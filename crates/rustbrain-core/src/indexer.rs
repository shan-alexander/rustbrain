//! Workspace walker: Markdown notes, optional Canvas, optional Rust AST.
//!
//! [`WorkspaceIndexer`] is the engine behind [`crate::Brain::sync`]. It owns a
//! [`Database`], walks the workspace (skipping `target/`, `.git/`, etc.), and:
//!
//! 1. Upserts Markdown nodes transactionally (FTS + tags + aliases + edges)
//! 2. Indexes Obsidian Canvas relationships when the `obsidian` feature is on
//! 3. Extracts Rust symbols when the `ast` feature is on
//! 4. Resolves pending WikiLink / `symbol:` targets
//! 5. Compiles `.brain/graph.mmap` when the `mmap` feature is on

use crate::error::{BrainError, Result};
use crate::id::{content_hash, node_id_from_rel_path, rel_path_from_workspace, resolve_link_target};
use crate::ignore::IgnoreSet;
use crate::storage::Database;
use crate::types::{Edge, Node, NodeType, SyncStats};
use chrono::Utc;
use std::path::{Path, PathBuf};

/// Indexes a workspace directory into a [`Database`].
pub struct WorkspaceIndexer {
    db: Database,
    workspace: PathBuf,
    ignore: IgnoreSet,
    /// Scope assignment (MainBrain / SubBrain).
    scopes: crate::scopes::WorkspaceManifest,
}

impl WorkspaceIndexer {
    /// Create an indexer for `workspace` writing into `db`.
    ///
    /// Loads `.rustbrainignore` (if present) plus built-in skips. When the env
    /// var `RUSTBRAIN_IMPORT_GITIGNORE=1` is set, also merges root `.gitignore`.
    pub fn new(db: Database, workspace: impl Into<PathBuf>) -> Self {
        let workspace = workspace.into();
        let import_gi = std::env::var_os("RUSTBRAIN_IMPORT_GITIGNORE")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        // Also auto-import gitignore when .rustbrainignore asks for it.
        let import_gi = import_gi || rustbrainignore_requests_gitignore(&workspace);
        let ignore = IgnoreSet::load(&workspace, import_gi).unwrap_or_default();
        let scopes = crate::scopes::load_manifest(&workspace).unwrap_or_else(|_| {
            crate::scopes::WorkspaceManifest::single(&workspace)
        });
        Self {
            db,
            workspace,
            ignore,
            scopes,
        }
    }

    fn owner_scope(&self, rel_str: &str) -> String {
        self.scopes.resolve_scope(rel_str)
    }

    /// Borrow the database.
    pub fn database(&self) -> &Database {
        &self.db
    }

    /// Consume the indexer and return the database.
    pub fn into_database(self) -> Database {
        self.db
    }

    /// Index all supported files under the workspace and bake the mmap cache.
    pub fn index_workspace(&self) -> Result<SyncStats> {
        let mut stats = SyncStats::default();

        #[cfg(feature = "ast")]
        let mut ast_parser = crate::ast::CodeAstParser::new_rust()
            .map_err(|e| BrainError::Ast(e.to_string()))?;

        self.walk_and_index(
            &self.workspace,
            &mut stats,
            #[cfg(feature = "ast")]
            &mut ast_parser,
        )?;

        // Phase 2: resolve any pending WikiLinks now that all nodes exist.
        let (resolved, still) = self.db.resolve_pending_links()?;
        stats.edges_created += resolved;
        stats.edges_pending = still;

        // Bake CSR mmap if feature enabled and .brain dir exists.
        let brain_dir = self.workspace.join(".brain");
        if brain_dir.exists() {
            #[cfg(feature = "mmap")]
            {
                let mmap_path = brain_dir.join("graph.mmap");
                self.compile_mmap(&mmap_path)?;
                stats.mmap_written = true;
            }
        }

        Ok(stats)
    }

    /// Bake current database into zero-copy CSR mmap (atomic replace).
    pub fn compile_mmap(&self, output_path: &Path) -> Result<()> {
        #[cfg(feature = "mmap")]
        {
            let node_ids = self.db.get_all_node_ids()?;
            let edges = self.db.get_csr_edges()?;
            // Phase A: no embeddings yet — vector_dim = 0.
            crate::mmap::CsrCompiler::compile(output_path, &node_ids, &edges, None, 0)?;
            Ok(())
        }
        #[cfg(not(feature = "mmap"))]
        {
            let _ = output_path;
            Err(BrainError::FeatureDisabled("mmap"))
        }
    }

    /// Index a single Markdown note (transactional, content-hash aware).
    pub fn index_markdown_file(&self, file_path: &Path, stats: &mut SyncStats) -> Result<()> {
        if !file_path.exists() || file_path.extension().and_then(|e| e.to_str()) != Some("md") {
            return Ok(());
        }

        let raw_bytes = std::fs::read(file_path)?;
        let hash = content_hash(&raw_bytes);
        let content = String::from_utf8_lossy(&raw_bytes);

        let rel = rel_path_from_workspace(&self.workspace, file_path);
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        if self.ignore.is_ignored(&rel_str, false) {
            return Ok(());
        }

        let hub = crate::hubs::detect_project_hub(&rel);
        let node_id = hub
            .map(|h| h.node_id().to_string())
            .unwrap_or_else(|| node_id_from_rel_path(&rel));

        let mut scope = self.owner_scope(&rel_str);
        // Frontmatter `scope:` may override path ownership when multi-brain and id is known.
        if self.scopes.is_multi() {
            #[cfg(feature = "obsidian")]
            {
                let (fm_early, _) = crate::obsidian::parse_frontmatter(&content);
                if let Some(fm) = fm_early.as_ref() {
                    if let Some(s) = fm
                        .extra
                        .get("scope")
                        .and_then(|v| v.as_str())
                        .map(|s| s.trim().to_ascii_lowercase().replace('_', "-"))
                        .filter(|s| !s.is_empty())
                    {
                        if s == self.scopes.main_id
                            || self.scopes.find_scope(&s).is_some()
                            || s == crate::scopes::MAIN_SCOPE
                        {
                            scope = if s == crate::scopes::MAIN_SCOPE {
                                self.scopes.main_id.clone()
                            } else {
                                self.scopes
                                    .find_scope(&s)
                                    .map(|sc| sc.id.clone())
                                    .unwrap_or(s)
                            };
                        }
                    }
                }
            }
        }
        if let Some(existing) = self.db.get_content_hash(&node_id)? {
            if existing == hash {
                // Keep scope aligned when multi-brain roots change without content edits.
                let _ = self.db.set_node_scope(&node_id, &scope);
                stats.nodes_skipped_unchanged += 1;
                return Ok(());
            }
        }

        #[cfg(feature = "obsidian")]
        let (frontmatter, body) = crate::obsidian::parse_frontmatter(&content);
        #[cfg(not(feature = "obsidian"))]
        let (frontmatter, body): (Option<()>, &str) = (None, content.as_ref());

        let title = resolve_title(
            #[cfg(feature = "obsidian")]
            frontmatter.as_ref(),
            #[cfg(not(feature = "obsidian"))]
            None,
            body,
            &rel,
        );

        let node_type = {
            #[cfg(feature = "obsidian")]
            {
                let parsed = frontmatter
                    .as_ref()
                    .and_then(|fm| fm.node_type.as_deref())
                    .and_then(NodeType::parse);
                match hub {
                    // Root README is the project front door; default to Goal unless author overrides.
                    Some(crate::hubs::ProjectHub::Readme) => parsed.unwrap_or(NodeType::Goal),
                    // Keep a Changelog → first-class Changelog type.
                    Some(crate::hubs::ProjectHub::Changelog) => {
                        parsed.unwrap_or(NodeType::Changelog)
                    }
                    // ROADMAP / BACKLOG → Plan (roadmaps, tasklists, todos).
                    Some(
                        crate::hubs::ProjectHub::Roadmap | crate::hubs::ProjectHub::Backlog,
                    ) => parsed.unwrap_or(NodeType::Plan),
                    None => parsed.unwrap_or(NodeType::Concept),
                }
            }
            #[cfg(not(feature = "obsidian"))]
            {
                match hub {
                    Some(crate::hubs::ProjectHub::Readme) => NodeType::Goal,
                    Some(crate::hubs::ProjectHub::Changelog) => NodeType::Changelog,
                    Some(
                        crate::hubs::ProjectHub::Roadmap | crate::hubs::ProjectHub::Backlog,
                    ) => NodeType::Plan,
                    None => NodeType::Concept,
                }
            }
        };

        let now = Utc::now().timestamp();
        let base_summary = if hub == Some(crate::hubs::ProjectHub::Changelog) {
            crate::hubs::changelog_latest_heading(body)
                .unwrap_or_else(|| first_substantive_line(body))
        } else {
            first_substantive_line(body)
        };

        let mut tags: Vec<String> = {
            #[cfg(feature = "obsidian")]
            {
                frontmatter
                    .as_ref()
                    .map(|fm| fm.tags.clone())
                    .unwrap_or_default()
            }
            #[cfg(not(feature = "obsidian"))]
            {
                Vec::new()
            }
        };

        let aliases: Vec<String> = {
            #[cfg(feature = "obsidian")]
            {
                frontmatter
                    .as_ref()
                    .map(|fm| fm.aliases.clone())
                    .unwrap_or_default()
            }
            #[cfg(not(feature = "obsidian"))]
            {
                Vec::new()
            }
        };

        // Optional frontmatter status/state (plan lifecycle).
        let fm_status: Option<String> = {
            #[cfg(feature = "obsidian")]
            {
                frontmatter.as_ref().and_then(|fm| {
                    fm.extra
                        .get("status")
                        .or_else(|| fm.extra.get("state"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                })
            }
            #[cfg(not(feature = "obsidian"))]
            {
                None
            }
        };

        // Plan densification: status tokens for FTS + compact summary (optional; no-op if no signal).
        let (fts_body, summary) = if node_type == NodeType::Plan
            || hub == Some(crate::hubs::ProjectHub::Roadmap)
            || hub == Some(crate::hubs::ProjectHub::Backlog)
        {
            crate::plan_status::enrich_plan_index_fields(
                body,
                fm_status.as_deref(),
                &base_summary,
            )
        } else {
            (body.to_string(), base_summary)
        };

        // Collect wikilinks before the transaction writes.
        #[cfg(feature = "obsidian")]
        let wikilinks = crate::obsidian::extract_wikilinks(body);
        #[cfg(not(feature = "obsidian"))]
        let wikilinks: Vec<RawLink> = Vec::new();

        // Architecture plan: symbol:crate::module::Name anchors in notes.
        let symbol_refs = crate::symbols::extract_symbol_refs(body);

        let node = Node {
            id: node_id.clone(),
            node_type,
            title: title.clone(),
            file_path: Some(rel_str.clone()),
            symbol_hash: None,
            summary: Some(summary),
            content_hash: Some(hash),
            scope: scope.clone(),
            created_at: now,
            updated_at: now,
        };

        // Densify scope token for FTS when multi-brain is on.
        if self.scopes.is_multi() {
            let token = format!("scope:{scope}");
            if !tags.iter().any(|t| t == &token) {
                tags.push(token);
            }
        }
        let tags_str = tags.join(" ");
        let mut links_for_tx: Vec<(String, String)> = {
            #[cfg(feature = "obsidian")]
            {
                wikilinks
                    .iter()
                    .map(|l| {
                        // WikiLinks of form [[symbol:…]] become anchors, not relates_to.
                        if let Some(rest) = l.target_node.strip_prefix("symbol:") {
                            (format!("symbol:{rest}"), "anchors".to_string())
                        } else {
                            (l.target_node.clone(), "relates_to".to_string())
                        }
                    })
                    .collect()
            }
            #[cfg(not(feature = "obsidian"))]
            {
                let _ = &wikilinks;
                Vec::new()
            }
        };
        for sref in &symbol_refs {
            links_for_tx.push((format!("symbol:{}", sref.raw), "anchors".to_string()));
        }

        // Also register file stem and title as aliases for link resolution.
        let mut extra_aliases = aliases.clone();
        if let Some(stem) = Path::new(&node.file_path.as_deref().unwrap_or(""))
            .file_stem()
            .and_then(|s| s.to_str())
        {
            extra_aliases.push(stem.to_string());
        }
        extra_aliases.push(title.clone());
        if let Some(h) = hub {
            for a in h.aliases() {
                extra_aliases.push((*a).to_string());
            }
            if h == crate::hubs::ProjectHub::Readme {
                if let Some(name) = self.workspace.file_name().and_then(|n| n.to_str()) {
                    extra_aliases.push(name.to_string());
                }
            }
            if h == crate::hubs::ProjectHub::Changelog {
                // Recent SemVer labels from Keep a Changelog headings → FTS aliases.
                for v in crate::hubs::changelog_version_aliases(body, 8) {
                    extra_aliases.push(v);
                }
            }
            // Plan hub aliases include overall status for resolution (e.g. "in_progress").
            if matches!(
                h,
                crate::hubs::ProjectHub::Roadmap | crate::hubs::ProjectHub::Backlog
            ) {
                if let Some(st) = fm_status.as_deref().and_then(crate::plan_status::PlanStatus::parse)
                {
                    extra_aliases.push(st.as_str().to_string());
                }
            }
        }
        if node_type == NodeType::Plan {
            if let Some(st) = fm_status.as_deref().and_then(crate::plan_status::PlanStatus::parse) {
                extra_aliases.push(st.as_str().to_string());
                extra_aliases.push(format!("status:{}", st.as_str()));
            }
        }

        self.db.with_transaction(|conn| {
            self.db.insert_node_on(conn, &node)?;
            self.db.replace_node_tags_on(conn, &node_id, &tags)?;
            self.db
                .replace_node_aliases_on(conn, &node_id, &extra_aliases)?;
            self.db
                .index_fts_on(conn, &node_id, &title, &fts_body, &tags_str)?;

            // Clear prior outbound edges + pending for this source (idempotent).
            self.db
                .clear_edges_from_on(conn, &node_id, "relates_to")?;
            self.db.clear_edges_from_on(conn, &node_id, "anchors")?;
            self.db.clear_pending_links_for_on(conn, &node_id)?;

            for (raw_target, rel_type) in &links_for_tx {
                let resolved = if let Some(sym_path) = raw_target.strip_prefix("symbol:") {
                    resolve_symbol_against_conn(conn, sym_path)?
                } else {
                    resolve_against_conn(conn, raw_target)?
                };

                if let Some(target_id) = resolved {
                    let edge = Edge {
                        source_id: node_id.clone(),
                        target_id,
                        relation_type: rel_type.clone(),
                        weight: 1.0,
                        decay_rate: 0.0,
                        created_at: now,
                    };
                    self.db.insert_edge_on(conn, &edge)?;
                    stats.edges_created += 1;
                } else {
                    self.db.insert_pending_link_on(
                        conn,
                        &node_id,
                        raw_target,
                        rel_type,
                        now,
                    )?;
                    stats.edges_pending += 1;
                }
            }
            Ok(())
        })?;

        stats.markdown_files += 1;
        stats.nodes_upserted += 1;
        Ok(())
    }

    /// Index an Obsidian Canvas (`.canvas`) file into graph edges.
    ///
    /// Resolves file/text node labels to existing note ids when possible;
    /// otherwise records pending links. Requires the `obsidian` feature.
    #[cfg(feature = "obsidian")]
    pub fn index_canvas_file(&self, file_path: &Path, stats: &mut SyncStats) -> Result<()> {
        if !file_path.exists() || file_path.extension().and_then(|e| e.to_str()) != Some("canvas")
        {
            return Ok(());
        }

        let content = std::fs::read_to_string(file_path)?;
        let canvas = crate::obsidian::ObsidianCanvas::parse_str(&content)
            .map_err(|e| BrainError::indexer(e.to_string()))?;
        let now = Utc::now().timestamp();
        let relationships = canvas.extract_relationships();

        let (ids, aliases, titles) = self.db.link_resolution_maps()?;

        self.db.with_transaction(|conn| {
            for (src_raw, dst_raw, rel) in &relationships {
                let src = resolve_link_target(src_raw, &ids, &aliases, &titles);
                let dst = resolve_link_target(dst_raw, &ids, &aliases, &titles);
                match (src, dst) {
                    (Some(s), Some(d)) => {
                        let edge = Edge {
                            source_id: s,
                            target_id: d,
                            relation_type: rel.clone(),
                            weight: 1.0,
                            decay_rate: 0.0,
                            created_at: now,
                        };
                        self.db.insert_edge_on(conn, &edge)?;
                        stats.edges_created += 1;
                    }
                    (Some(s), None) => {
                        self.db
                            .insert_pending_link_on(conn, &s, dst_raw, rel, now)?;
                        stats.edges_pending += 1;
                    }
                    _ => {
                        // Source unknown — skip with pending if we can map nothing.
                        stats.edges_pending += 1;
                    }
                }
            }
            Ok(())
        })?;

        stats.canvas_files += 1;
        Ok(())
    }

    fn walk_and_index(
        &self,
        dir: &Path,
        stats: &mut SyncStats,
        #[cfg(feature = "ast")] ast_parser: &mut crate::ast::CodeAstParser,
    ) -> Result<()> {
        if !dir.exists() {
            return Ok(());
        }

        let entries = std::fs::read_dir(dir)?;
        for entry in entries {
            let entry = entry?;
            let path = entry.path();

            if path.is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if self.ignore.skip_dir_name(name) {
                        continue;
                    }
                }
                let rel = rel_path_from_workspace(&self.workspace, &path);
                let rel_str = rel.to_string_lossy().replace('\\', "/");
                if self.ignore.is_ignored(&rel_str, true) {
                    continue;
                }
                self.walk_and_index(
                    &path,
                    stats,
                    #[cfg(feature = "ast")]
                    ast_parser,
                )?;
            } else {
                let rel = rel_path_from_workspace(&self.workspace, &path);
                let rel_str = rel.to_string_lossy().replace('\\', "/");
                if self.ignore.is_ignored(&rel_str, false) {
                    continue;
                }
                let ext = path.extension().and_then(|e| e.to_str());
                let result = match ext {
                    Some("md") => self.index_markdown_file(&path, stats),
                    #[cfg(feature = "obsidian")]
                    Some("canvas") => self.index_canvas_file(&path, stats),
                    #[cfg(feature = "ast")]
                    Some("rs") => self.index_rust_file(&path, ast_parser, stats),
                    _ => Ok(()),
                };
                if let Err(e) = result {
                    // Never abort a full-workspace sync for one bad file.
                    stats.file_errors += 1;
                    eprintln!(
                        "rustbrain: skip {} ({e})",
                        path.display()
                    );
                }
            }
        }
        Ok(())
    }

    #[cfg(feature = "ast")]
    fn index_rust_file(
        &self,
        path: &Path,
        ast_parser: &mut crate::ast::CodeAstParser,
        stats: &mut SyncStats,
    ) -> Result<()> {
        let rel = rel_path_from_workspace(&self.workspace, path);
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        if self.ignore.is_ignored(&rel_str, false) {
            return Ok(());
        }
        let crate_name = infer_crate_name(&self.workspace, path);
        let source = std::fs::read_to_string(path)?;
        let anchors = ast_parser
            .parse_symbols(&crate_name, &rel_str, &source)
            .map_err(|e| BrainError::Ast(e.to_string()))?;

        for anchor in &anchors {
            self.db.insert_symbol_anchor(anchor)?;

            let node_id = crate::symbols::symbol_node_id(
                &anchor.crate_name,
                &anchor.module_path,
                &anchor.symbol_name,
            );
            // Include doc comments in the hash so WikiLink edits in /// re-index.
            let sig = format!(
                "{}::{}::{}@{}-{}#{}",
                anchor.crate_name,
                anchor.module_path,
                anchor.symbol_name,
                anchor.start_line,
                anchor.end_line,
                anchor.doc_comment.as_deref().unwrap_or("")
            );
            let chash = crate::id::content_hash(sig.as_bytes());
            let unchanged = self
                .db
                .get_content_hash(&node_id)?
                .map(|existing| existing == chash)
                .unwrap_or(false);

            let scope = self.owner_scope(&anchor.file_path);
            if unchanged {
                let _ = self.db.set_node_scope(&node_id, &scope);
            }
            if !unchanged {
                let now = Utc::now().timestamp();
                let node = Node {
                    id: node_id.clone(),
                    node_type: NodeType::Symbol,
                    title: anchor.symbol_name.clone(),
                    file_path: Some(anchor.file_path.clone()),
                    symbol_hash: Some(anchor.symbol_hash),
                    summary: anchor.doc_comment.clone(),
                    content_hash: Some(chash),
                    scope: scope.clone(),
                    created_at: now,
                    updated_at: now,
                };
                self.db.insert_node(&node)?;

                // Aliases for resolution: bare name, Type::method, full path.
                let mut aliases = vec![anchor.symbol_name.clone()];
                if let Some((_, method)) = anchor.symbol_name.split_once("::") {
                    aliases.push(method.to_string());
                }
                aliases.push(format!(
                    "{}::{}::{}",
                    anchor.crate_name, anchor.module_path, anchor.symbol_name
                ));
                self.db.replace_node_aliases(&node_id, &aliases)?;

                let fts_body = format!(
                    "{} {} {} {}",
                    anchor.symbol_name,
                    anchor.module_path,
                    anchor.crate_name,
                    anchor.doc_comment.as_deref().unwrap_or("")
                );
                self.db
                    .index_fts(&node_id, &anchor.symbol_name, &fts_body, "symbol")?;

                stats.nodes_upserted += 1;
            } else {
                stats.nodes_skipped_unchanged += 1;
            }

            // Always (re)link rustdoc WikiLinks → notes so new notes resolve later.
            self.link_symbol_doc_wikis(&node_id, anchor.doc_comment.as_deref(), stats)?;

            stats.symbol_anchors += 1;
        }
        stats.rust_files += 1;
        Ok(())
    }

    /// Parse `[[WikiLinks]]` in rustdoc (`///` / `//!` / `/**`) and edge symbol → note.
    ///
    /// Relation type: `doc_links` (code documents/references a brain node).
    /// Unresolved targets become pending links (resolved on later syncs).
    #[cfg(feature = "ast")]
    fn link_symbol_doc_wikis(
        &self,
        symbol_node_id: &str,
        doc_comment: Option<&str>,
        stats: &mut SyncStats,
    ) -> Result<()> {
        let now = Utc::now().timestamp();
        let doc_plain = doc_comment.map(strip_rustdoc_prefixes).unwrap_or_default();

        self.db.with_transaction(|conn| {
            self.db
                .clear_edges_from_on(conn, symbol_node_id, "doc_links")?;
            // Drop only pending rows for this symbol that were doc_links (full clear is ok:
            // symbols do not use other pending kinds).
            self.db.clear_pending_links_for_on(conn, symbol_node_id)?;

            if doc_plain.trim().is_empty() {
                return Ok(());
            }

            // WikiLink extraction lives under the `obsidian` feature (default on).
            #[cfg(feature = "obsidian")]
            {
                let wikis = crate::obsidian::extract_wikilinks(&doc_plain);
                for w in wikis {
                    let target = w.target_node.trim();
                    if target.is_empty() {
                        continue;
                    }
                    // Skip pure symbol: refs in docs — notes own note→code anchors.
                    if target.starts_with("symbol:") {
                        continue;
                    }
                    let resolved = resolve_against_conn(conn, target)?;
                    if let Some(target_id) = resolved {
                        if target_id == symbol_node_id {
                            continue;
                        }
                        let edge = Edge {
                            source_id: symbol_node_id.to_string(),
                            target_id,
                            relation_type: "doc_links".into(),
                            weight: 1.0,
                            decay_rate: 0.0,
                            created_at: now,
                        };
                        self.db.insert_edge_on(conn, &edge)?;
                        stats.edges_created += 1;
                    } else {
                        self.db.insert_pending_link_on(
                            conn,
                            symbol_node_id,
                            &format!("[[{target}]]"),
                            "doc_links",
                            now,
                        )?;
                        stats.edges_pending += 1;
                    }
                }
            }
            Ok(())
        })
    }
}

/// Normalize `///` / `//!` / block-doc lines into plain text for WikiLink extraction.
#[cfg(feature = "ast")]
fn strip_rustdoc_prefixes(doc: &str) -> String {
    let mut out = String::new();
    for line in doc.lines() {
        let t = line.trim();
        let body = if let Some(rest) = t.strip_prefix("///") {
            rest.strip_prefix(' ').unwrap_or(rest)
        } else if let Some(rest) = t.strip_prefix("//!") {
            rest.strip_prefix(' ').unwrap_or(rest)
        } else if t.starts_with("/**") || t.starts_with("*/") {
            continue;
        } else if let Some(rest) = t.strip_prefix('*') {
            rest.strip_prefix(' ').unwrap_or(rest)
        } else {
            t
        };
        out.push_str(body);
        out.push('\n');
    }
    out
}

fn rustbrainignore_requests_gitignore(workspace: &Path) -> bool {
    let path = workspace.join(".rustbrainignore");
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    text.lines().any(|l| {
        let t = l.trim().to_ascii_lowercase();
        t == "# rustbrain: import-gitignore"
            || t == "#!import-gitignore"
            || t.contains("rustbrain: import-gitignore")
    })
}

fn first_substantive_line(body: &str) -> String {
    for line in body.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        // Skip pure heading markers only lines handled below
        let cleaned = t.trim_start_matches('#').trim();
        if !cleaned.is_empty() {
            return cleaned.to_string();
        }
    }
    String::new()
}

fn resolve_title(
    #[cfg(feature = "obsidian")] frontmatter: Option<&crate::obsidian::Frontmatter>,
    #[cfg(not(feature = "obsidian"))] frontmatter: Option<&()>,
    body: &str,
    rel: &Path,
) -> String {
    #[cfg(feature = "obsidian")]
    if let Some(fm) = frontmatter {
        if let Some(title) = fm.extra.get("title").and_then(|v| v.as_str()) {
            let t = title.trim();
            if !t.is_empty() {
                return t.to_string();
            }
        }
    }
    #[cfg(not(feature = "obsidian"))]
    let _ = frontmatter;

    // First H1
    for line in body.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("# ") {
            let t = rest.trim();
            if !t.is_empty() {
                return t.to_string();
            }
        }
    }

    rel.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Untitled")
        .to_string()
}

#[cfg(feature = "ast")]
fn infer_crate_name(workspace: &Path, file: &Path) -> String {
    // Walk up looking for Cargo.toml
    let mut cur = file.parent();
    while let Some(dir) = cur {
        let cargo = dir.join("Cargo.toml");
        if cargo.exists() {
            if let Ok(text) = std::fs::read_to_string(&cargo) {
                if let Some(name) = parse_cargo_package_name(&text) {
                    return name;
                }
            }
            if let Some(n) = dir.file_name().and_then(|n| n.to_str()) {
                return n.to_string();
            }
        }
        if dir == workspace {
            break;
        }
        cur = dir.parent();
    }
    workspace
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("workspace")
        .to_string()
}

#[cfg(feature = "ast")]
fn parse_cargo_package_name(toml: &str) -> Option<String> {
    let mut in_package = false;
    for line in toml.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            in_package = t == "[package]";
            continue;
        }
        if in_package {
            if let Some(rest) = t.strip_prefix("name") {
                let rest = rest.trim().trim_start_matches('=').trim();
                let name = rest.trim_matches('"').trim_matches('\'').to_string();
                if !name.is_empty() {
                    return Some(name);
                }
            }
        }
    }
    None
}

/// Resolve `symbol:…` path against symbol nodes / aliases.
fn resolve_symbol_against_conn(
    conn: &rusqlite::Connection,
    raw_path: &str,
) -> Result<Option<String>> {
    let Some(sym) = crate::symbols::parse_symbol_path(raw_path) else {
        return Ok(None);
    };

    // Collect all symbol node ids (type=symbol).
    let mut stmt = conn.prepare("SELECT id FROM nodes WHERE node_type = 'symbol'")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    let mut ids = std::collections::HashSet::new();
    for r in rows {
        ids.insert(r?);
    }

    if let Some(id) = crate::symbols::resolve_symbol_ref(&sym, &ids) {
        return Ok(Some(id));
    }

    // Fall back to alias / title resolution for the bare name.
    resolve_against_conn(conn, &sym.symbol_name)
}

/// Resolve a link target using the live connection (aliases + ids + titles).
fn resolve_against_conn(conn: &rusqlite::Connection, raw: &str) -> Result<Option<String>> {
    let key = raw.trim().to_lowercase();
    if key.is_empty() {
        return Ok(None);
    }

    // Exact id
    let exact: Option<String> = conn
        .query_row("SELECT id FROM nodes WHERE id = ?1", [&key], |row| row.get(0))
        .optional_compat()?;
    if exact.is_some() {
        return Ok(exact);
    }

    // Alias
    let by_alias: Option<String> = conn
        .query_row(
            "SELECT node_id FROM node_aliases WHERE alias = ?1",
            [&key],
            |row| row.get(0),
        )
        .optional_compat()?;
    if by_alias.is_some() {
        return Ok(by_alias);
    }

    // Title (case-insensitive) — may be ambiguous; take if unique
    let mut stmt = conn.prepare("SELECT id FROM nodes WHERE lower(title) = ?1")?;
    let rows = stmt.query_map([&key], |row| row.get::<_, String>(0))?;
    let mut hits = Vec::new();
    for r in rows {
        hits.push(r?);
    }
    if hits.len() == 1 {
        return Ok(Some(hits.remove(0)));
    }

    // Unique suffix match
    let mut stmt = conn.prepare("SELECT id FROM nodes")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    let mut suffix_hits = Vec::new();
    for r in rows {
        let id = r?;
        if id == key || id.ends_with(&format!("/{key}")) || id.rsplit('/').next() == Some(key.as_str())
        {
            suffix_hits.push(id);
        }
    }
    suffix_hits.sort();
    suffix_hits.dedup();
    if suffix_hits.len() == 1 {
        return Ok(Some(suffix_hits.remove(0)));
    }

    Ok(None)
}

/// Local trait shim so we can call `.optional()` style without importing OptionalExtension in every use.
trait OptionalCompat<T> {
    fn optional_compat(self) -> Result<Option<T>>;
}

impl<T> OptionalCompat<T> for std::result::Result<T, rusqlite::Error> {
    fn optional_compat(self) -> Result<Option<T>> {
        match self {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(BrainError::from(e)),
        }
    }
}

#[cfg(not(feature = "obsidian"))]
struct RawLink {
    target_node: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn index_workspace_and_fts_idempotent() {
        let dir = tempdir().unwrap();
        let docs = dir.path().join("docs");
        std::fs::create_dir_all(&docs).unwrap();
        std::fs::write(
            docs.join("raft.md"),
            "---\ntags: [raft]\nnode_type: concept\naliases: [Raft Consensus]\n---\n# Raft Consensus\nRelates to [[log-compaction]].\n",
        )
        .unwrap();
        std::fs::write(
            docs.join("log-compaction.md"),
            "---\ntags: [log]\nnode_type: concept\n---\n# Log Compaction\nSee [[raft]].\n",
        )
        .unwrap();

        let brain = dir.path().join(".brain");
        std::fs::create_dir_all(&brain).unwrap();
        let db = Database::open(brain.join("db.sqlite")).unwrap();
        let indexer = WorkspaceIndexer::new(db, dir.path());

        let s1 = indexer.index_workspace().unwrap();
        assert_eq!(s1.markdown_files, 2);
        assert!(s1.edges_created >= 2);
        let fts1 = indexer.database().count_fts_rows().unwrap();
        assert_eq!(fts1, 2);

        // Second sync should skip unchanged content and keep FTS row count stable.
        // Touch is not done — content hash matches → nodes_skipped.
        let s2 = indexer.index_workspace().unwrap();
        assert_eq!(s2.nodes_skipped_unchanged, 2);
        assert_eq!(indexer.database().count_fts_rows().unwrap(), 2);

        // Force reindex by changing content
        std::fs::write(
            docs.join("raft.md"),
            "---\ntags: [raft]\nnode_type: concept\n---\n# Raft Consensus\nUpdated body [[log-compaction]].\n",
        )
        .unwrap();
        let s3 = indexer.index_workspace().unwrap();
        assert_eq!(s3.nodes_upserted, 1);
        assert_eq!(indexer.database().count_fts_rows().unwrap(), 2);

        let hits = indexer.database().search_fts("raft").unwrap();
        assert!(!hits.is_empty());
        assert!(hits.iter().any(|n| n.id.contains("raft")));
    }

    #[test]
    fn strip_rustdoc_prefixes_keeps_wikilink() {
        let raw = "/// Primary engine. See [[use-sqlite]].\n/// Second line.";
        let plain = strip_rustdoc_prefixes(raw);
        assert!(plain.contains("[[use-sqlite]]"));
        assert!(!plain.contains("///"));
    }

    #[cfg(all(feature = "ast", feature = "obsidian"))]
    #[test]
    fn rustdoc_wikilink_creates_doc_links_edge() {
        let dir = tempdir().unwrap();
        let docs = dir.path().join("docs/adr");
        let src = dir.path().join("src");
        std::fs::create_dir_all(&docs).unwrap();
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::write(
            docs.join("use-sqlite.md"),
            "---\nnode_type: adr\naliases: [use-sqlite]\n---\n# Use SQLite\n\nLocal store.\n",
        )
        .unwrap();
        std::fs::write(
            src.join("lib.rs"),
            r#"/// Primary engine. See [[use-sqlite]] and [[docs/adr/use-sqlite]].
pub struct StorageEngine;

impl StorageEngine {
    /// Open the store.
    pub fn open() {}
}
"#,
        )
        .unwrap();

        let brain = dir.path().join(".brain");
        std::fs::create_dir_all(&brain).unwrap();
        let db = Database::open(brain.join("db.sqlite")).unwrap();
        let indexer = WorkspaceIndexer::new(db, dir.path());
        // Two syncs: notes first pass may race walk order; second resolves pending.
        let _ = indexer.index_workspace().unwrap();
        let s = indexer.index_workspace().unwrap();
        let _ = s;

        let edges = indexer.database().get_all_edges().unwrap();
        let doc_links: Vec<_> = edges
            .iter()
            .filter(|e| e.relation_type == "doc_links")
            .collect();
        assert!(
            !doc_links.is_empty(),
            "expected doc_links from rustdoc WikiLink, edges={edges:?}"
        );
        assert!(doc_links.iter().any(|e| e.source_id.contains("StorageEngine")));
        assert!(doc_links.iter().any(|e| e.target_id.contains("use-sqlite")
            || e.target_id.contains("docs/adr/use-sqlite")));
    }

    #[test]
    fn pending_link_then_resolve() {
        let dir = tempdir().unwrap();
        let docs = dir.path().join("docs");
        std::fs::create_dir_all(&docs).unwrap();
        std::fs::write(
            docs.join("a.md"),
            "---\nnode_type: concept\n---\n# A\nLink to [[b]].\n",
        )
        .unwrap();

        let brain = dir.path().join(".brain");
        std::fs::create_dir_all(&brain).unwrap();
        let db = Database::open(brain.join("db.sqlite")).unwrap();
        let indexer = WorkspaceIndexer::new(db, dir.path());
        let s1 = indexer.index_workspace().unwrap();
        assert!(s1.edges_pending >= 1 || indexer.database().count_pending_links().unwrap() >= 1);

        std::fs::write(
            docs.join("b.md"),
            "---\nnode_type: concept\n---\n# B\nBack.\n",
        )
        .unwrap();
        let s2 = indexer.index_workspace().unwrap();
        assert!(s2.edges_created >= 1 || indexer.database().count_edges().unwrap() >= 1);
    }

    #[test]
    fn plan_note_status_densified_in_fts() {
        let dir = tempdir().unwrap();
        let plans = dir.path().join("docs/plans");
        std::fs::create_dir_all(&plans).unwrap();
        std::fs::write(
            plans.join("sprint.md"),
            "---\nnode_type: plan\nstatus: in_progress\n---\n# Sprint\n\n## Backlog\n\n- [ ] Write docs\n\n## Done\n\n- [x] Scaffold hub\n",
        )
        .unwrap();
        let brain = dir.path().join(".brain");
        std::fs::create_dir_all(&brain).unwrap();
        let db = Database::open(brain.join("db.sqlite")).unwrap();
        let indexer = WorkspaceIndexer::new(db, dir.path());
        indexer.index_workspace().unwrap();
        let node = indexer
            .database()
            .get_node("docs/plans/sprint")
            .unwrap()
            .expect("plan node");
        assert_eq!(node.node_type, NodeType::Plan);
        assert!(
            node.summary
                .as_deref()
                .is_some_and(|s| s.contains("status=in_progress") && s.contains("open")),
            "summary={:?}",
            node.summary
        );
        let fts = indexer
            .database()
            .get_fts_content("docs/plans/sprint")
            .unwrap()
            .unwrap_or_default();
        assert!(fts.contains("status:in_progress"), "fts={fts}");
        assert!(fts.contains("status:done") || fts.contains("task:done:"), "fts={fts}");
        let hits = indexer
            .database()
            .search_ranked("status:in_progress", &crate::query::QueryOptions::human())
            .unwrap();
        assert!(
            hits.iter().any(|h| h.node.id == "docs/plans/sprint"),
            "hits={:?}",
            hits.iter().map(|h| &h.node.id).collect::<Vec<_>>()
        );
    }

    #[test]
    fn root_changelog_indexes_as_stable_hub() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("CHANGELOG.md"),
            "# Changelog\n\n## [0.3.15] - 2026-07-31\n\n### Added\n- changelog hub\n\n## [0.3.14] - 2026-07-30\n\n### Fixed\n- prior\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("README.md"), "# Demo\n\nA crate.\n").unwrap();
        let brain = dir.path().join(".brain");
        std::fs::create_dir_all(&brain).unwrap();
        let db = Database::open(brain.join("db.sqlite")).unwrap();
        let indexer = WorkspaceIndexer::new(db, dir.path());
        indexer.index_workspace().unwrap();

        let node = indexer
            .database()
            .get_node(crate::hubs::HUB_CHANGELOG)
            .unwrap()
            .expect("changelog hub");
        assert_eq!(node.node_type, NodeType::Changelog);
        assert_eq!(node.file_path.as_deref(), Some("CHANGELOG.md"));
        assert!(
            node.summary
                .as_deref()
                .is_some_and(|s| s.contains("0.3.15")),
            "summary={:?}",
            node.summary
        );
        // Version alias for FTS resolution
        let hits = indexer
            .database()
            .search_ranked("0.3.15", &crate::query::QueryOptions::default())
            .unwrap();
        assert!(
            hits.iter().any(|h| h.node.id == crate::hubs::HUB_CHANGELOG),
            "hits={:?}",
            hits.iter().map(|h| &h.node.id).collect::<Vec<_>>()
        );
    }
}
