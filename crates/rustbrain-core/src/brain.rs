//! Primary library façade: [`Brain`].
//!
//! Most applications should open a [`Brain`], call [`Brain::sync`] after note or
//! code edits, then use [`Brain::query_ranked`] / [`Brain::context_for_prompt`].

use crate::context::{assemble_context, ContextOptions};
use crate::error::{BrainError, Result};
use crate::exporter::{BrainExporter, BrainImporter};
use crate::indexer::WorkspaceIndexer;
use crate::query::{QueryOptions, RankedHit};
use crate::storage::Database;
use crate::types::{ContextBundle, Node, SyncStats};
use std::path::{Path, PathBuf};

/// A project-scoped rustbrain instance rooted at `workspace/.brain`.
///
/// # Lifecycle
///
/// ```text
/// Brain::create / open / open_or_create
///         │
///         ▼
///      sync()  ──► index Markdown + AST + resolve links + bake graph.mmap
///         │
///         ├─► query / query_ranked
///         ├─► context_for_prompt / context_for_prompt_with
///         ├─► export / import
///         └─► watch  (feature = "watch")
/// ```
///
/// # Threading
///
/// `Brain` is not `Sync`. Open separate instances (or serialize access) for
/// concurrent use. SQLite is opened with WAL and a busy timeout for multi-process
/// friendliness on the same machine.
pub struct Brain {
    workspace: PathBuf,
    brain_dir: PathBuf,
    db: Database,
}

impl Brain {
    /// Create a new brain directory and empty database under `workspace/.brain`.
    ///
    /// Creates the workspace directory if it does not exist. Writes a minimal
    /// `workspace.json` marker. Does **not** index notes — call [`Self::sync`].
    ///
    /// # Errors
    ///
    /// Returns I/O or SQLite errors if the directory or database cannot be created.
    pub fn create(workspace: impl AsRef<Path>) -> Result<Self> {
        let workspace = canonicalize_or_owned(workspace.as_ref())?;
        let brain_dir = workspace.join(".brain");
        std::fs::create_dir_all(&brain_dir)?;
        let db_path = brain_dir.join("db.sqlite");
        let db = Database::open(&db_path)?;
        let marker = brain_dir.join("workspace.json");
        if !marker.exists() {
            let meta = serde_json::json!({
                "version": 1,
                "workspace": workspace.to_string_lossy(),
            });
            std::fs::write(&marker, serde_json::to_string_pretty(&meta)?)?;
        }
        Ok(Self {
            workspace,
            brain_dir,
            db,
        })
    }

    /// Open an existing brain under `workspace/.brain`.
    ///
    /// If `workspace/.brain/db.sqlite` is missing, walks **parent directories**
    /// (like git) until a brain is found or the filesystem root is reached.
    ///
    /// # Errors
    ///
    /// Returns [`BrainError::BrainNotFound`] if no brain is found.
    pub fn open(workspace: impl AsRef<Path>) -> Result<Self> {
        let start = canonicalize_or_owned(workspace.as_ref())?;
        if let Some((ws, brain_dir)) = find_brain_dir(&start) {
            let db = Database::open(brain_dir.join("db.sqlite"))?;
            return Ok(Self {
                workspace: ws,
                brain_dir,
                db,
            });
        }
        Err(BrainError::BrainNotFound {
            path: start.join(".brain"),
        })
    }

    /// Open a brain **only** at `workspace` (no parent walk).
    ///
    /// Prefer [`Self::open`] for CLI tooling.
    pub fn open_exact(workspace: impl AsRef<Path>) -> Result<Self> {
        let workspace = canonicalize_or_owned(workspace.as_ref())?;
        let brain_dir = workspace.join(".brain");
        let db_path = brain_dir.join("db.sqlite");
        if !db_path.exists() {
            return Err(BrainError::BrainNotFound { path: brain_dir });
        }
        let db = Database::open(&db_path)?;
        Ok(Self {
            workspace,
            brain_dir,
            db,
        })
    }

    /// Open an existing brain, or [`Self::create`] if none is present at `workspace`
    /// (does not walk parents for create — create is always local).
    pub fn open_or_create(workspace: impl AsRef<Path>) -> Result<Self> {
        let workspace = workspace.as_ref();
        let db_path = workspace.join(".brain").join("db.sqlite");
        if db_path.exists() {
            Self::open_exact(workspace)
        } else if let Ok(b) = Self::open(workspace) {
            // Found a parent brain when CWD is a subdir.
            Ok(b)
        } else {
            Self::create(workspace)
        }
    }

    /// Absolute path to the workspace root (parent of `.brain`).
    pub fn workspace(&self) -> &Path {
        &self.workspace
    }

    /// Absolute path to the `.brain` directory.
    pub fn brain_dir(&self) -> &Path {
        &self.brain_dir
    }

    /// Borrow the underlying [`Database`] for advanced queries or tooling.
    pub fn database(&self) -> &Database {
        &self.db
    }

    /// Index Markdown (and optional AST / Canvas) and recompile the CSR mmap cache.
    ///
    /// Unchanged files (matching `content_hash`) are skipped. WikiLinks and
    /// `symbol:…` refs that cannot be resolved are stored as pending links and
    /// retried at the end of the run (and on later syncs).
    pub fn sync(&mut self) -> Result<SyncStats> {
        let db_path = self.brain_dir.join("db.sqlite");
        let db = Database::open(&db_path)?;
        let indexer = WorkspaceIndexer::new(db, self.workspace.clone());
        let stats = indexer.index_workspace()?;
        // Refresh our handle so we see committed WAL state.
        self.db = Database::open(&db_path)?;
        Ok(stats)
    }

    /// Ranked search returning only nodes (score order preserved, scores discarded).
    ///
    /// Prefer [`Self::query_ranked`] when scores or match reasons matter.
    pub fn query(&self, q: &str) -> Result<Vec<Node>> {
        let hits = self.query_ranked(q, &QueryOptions::default())?;
        Ok(hits.into_iter().map(|h| h.node).collect())
    }

    /// Ranked search with scores and human-readable match reasons.
    ///
    /// Combines FTS5 BM25 with title, id, tag, and alias boosts. See [`QueryOptions`].
    pub fn query_ranked(&self, q: &str, opts: &QueryOptions) -> Result<Vec<RankedHit>> {
        self.db.search_ranked(q, opts)
    }

    /// Build a graph-aware prompt context with a simple token budget.
    ///
    /// Equivalent to [`Self::context_for_prompt_with`] using default hop depth and
    /// packing limits. Token accounting uses a ~4 characters/token heuristic.
    pub fn context_for_prompt(&self, prompt: &str, max_tokens: usize) -> Result<ContextBundle> {
        let opts = ContextOptions {
            max_tokens,
            ..ContextOptions::default()
        };
        self.context_for_prompt_with(prompt, &opts)
    }

    /// Context assembly with full control over hops, seed counts, and packing.
    ///
    /// When `graph.mmap` exists and `opts.hop_depth > 0`, CSR neighbors of seed
    /// hits are scored and packed alongside FTS seeds.
    pub fn context_for_prompt_with(
        &self,
        prompt: &str,
        opts: &ContextOptions,
    ) -> Result<ContextBundle> {
        assemble_context(&self.db, &self.brain_dir, prompt, opts)
    }

    /// Export nodes/edges to a portable `.brainbundle` JSON file.
    ///
    /// When `decouple_ast` is true, symbol nodes and file/symbol path fields are
    /// stripped so the bundle can move across repositories cleanly.
    pub fn export(&self, out: impl AsRef<Path>, decouple_ast: bool) -> Result<()> {
        BrainExporter::export_bundle(&self.db, out, decouple_ast)
    }

    /// Import a `.brainbundle`, upserting nodes and edges.
    ///
    /// Recompiles `graph.mmap` when the `mmap` feature is enabled.
    ///
    /// # Returns
    ///
    /// Number of nodes upserted from the bundle.
    pub fn import(&mut self, input: impl AsRef<Path>) -> Result<usize> {
        let n = BrainImporter::import_bundle(&self.db, input)?;
        #[cfg(feature = "mmap")]
        {
            let indexer = WorkspaceIndexer::new(
                Database::open(self.brain_dir.join("db.sqlite"))?,
                self.workspace.clone(),
            );
            let _ = indexer.compile_mmap(&self.brain_dir.join("graph.mmap"));
            self.db = Database::open(self.brain_dir.join("db.sqlite"))?;
        }
        Ok(n)
    }

    /// Block the current thread and watch the workspace for changes.
    ///
    /// Requires the `watch` Cargo feature. Debounces filesystem events for
    /// `debounce_ms` milliseconds, then runs a full sync (content-hash skips
    /// unchanged files) and remaps the CSR cache.
    pub fn watch(&self, debounce_ms: u64) -> Result<()> {
        crate::watch::watch_workspace(
            &self.workspace,
            crate::watch::WatchConfig {
                debounce: std::time::Duration::from_millis(debounce_ms),
                verbose: true,
            },
        )
    }

    /// Deterministic docs/ignore bootstrap for mature repositories.
    ///
    /// See [`crate::bootstrap::bootstrap_workspace`]. Does not replace a full
    /// [`Self::sync`] — call sync afterward to index new files.
    pub fn bootstrap(
        workspace: impl AsRef<Path>,
        opts: crate::bootstrap::BootstrapOptions,
    ) -> Result<crate::bootstrap::BootstrapReport> {
        crate::bootstrap::bootstrap_workspace(workspace.as_ref(), opts)
    }

    /// Health check for this brain (pending links, ratios, schema).
    pub fn doctor(&self) -> Result<crate::doctor::DoctorReport> {
        crate::doctor::run_doctor(&self.workspace)
    }

    /// Create a Markdown note on disk (`docs/…`) without opening a second DB.
    ///
    /// Call [`Self::sync`] afterward so FTS/graph pick it up.
    pub fn note_new(&self, opts: &crate::note::NoteNewOptions) -> Result<crate::note::NoteCreated> {
        crate::note::create_note(&self.workspace, opts)
    }

    /// Notes with no explicit graph edges (soft `auto_*` edges do not count).
    pub fn list_orphans(&self) -> Result<Vec<crate::autolink::OrphanNote>> {
        crate::autolink::list_orphan_notes(&self.db)
    }

    /// Create soft auto-links (filename stem + shared tags). Optional path focuses one note.
    ///
    /// Recompiles `graph.mmap` when the `mmap` feature is enabled.
    pub fn auto_link(
        &mut self,
        target: Option<&std::path::Path>,
    ) -> Result<crate::autolink::AutoLinkReport> {
        let report = crate::autolink::run_auto_link(&self.db, target)?;
        #[cfg(feature = "mmap")]
        {
            let indexer = WorkspaceIndexer::new(
                Database::open(self.brain_dir.join("db.sqlite"))?,
                self.workspace.clone(),
            );
            let _ = indexer.compile_mmap(&self.brain_dir.join("graph.mmap"));
            self.db = Database::open(self.brain_dir.join("db.sqlite"))?;
        }
        Ok(report)
    }

    /// Plan/apply pending WikiLink normalizations and optional AC discovery.
    ///
    /// See [`crate::apply_links::apply_links`]. When `opts.write` is true and
    /// `opts.sync_after`, call [`Self::sync`] after a successful write.
    pub fn apply_links(
        &self,
        opts: &crate::apply_links::ApplyOptions,
    ) -> Result<crate::apply_links::ApplyReport> {
        crate::apply_links::apply_links(&self.workspace, &self.db, opts)
    }

    /// k-hop neighborhood around a node (path, id, title, or `symbol:…`).
    ///
    /// Uses SQLite edges (relation types preserved). Prefer for inspection;
    /// use [`Self::context_for_prompt`] when packing content for agents.
    pub fn graph_neighborhood(
        &self,
        target: &str,
        opts: &crate::graph::GraphOptions,
    ) -> Result<crate::graph::GraphNeighborhood> {
        crate::graph::neighborhood(&self.db, target, opts)
    }

    /// Aggregate node/edge counts, relation breakdown, and high-degree hubs.
    pub fn graph_stats(&self) -> Result<crate::graph::GraphStats> {
        crate::graph::graph_stats(&self.db)
    }
}

fn canonicalize_or_owned(path: &Path) -> Result<PathBuf> {
    if path.exists() {
        Ok(fs_canonicalize(path)?)
    } else {
        std::fs::create_dir_all(path)?;
        Ok(fs_canonicalize(path)?)
    }
}

fn fs_canonicalize(path: &Path) -> Result<PathBuf> {
    std::fs::canonicalize(path).map_err(BrainError::from)
}

/// Walk `start` and its parents looking for `.brain/db.sqlite`.
///
/// Returns `(workspace_root, brain_dir)`.
pub fn find_brain_dir(start: &Path) -> Option<(PathBuf, PathBuf)> {
    let mut cur = start.to_path_buf();
    loop {
        let brain_dir = cur.join(".brain");
        if brain_dir.join("db.sqlite").is_file() {
            return Some((cur, brain_dir));
        }
        if !cur.pop() {
            break;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ContextRole;
    use tempfile::tempdir;

    #[test]
    fn open_walks_parent_for_brain() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let sub = root.join("src").join("nested");
        std::fs::create_dir_all(&sub).unwrap();
        Brain::create(root).unwrap();
        let opened = Brain::open(&sub).unwrap();
        assert_eq!(
            opened.workspace().canonicalize().unwrap(),
            root.canonicalize().unwrap()
        );
    }

    #[test]
    fn create_sync_query_context_export() {
        let dir = tempdir().unwrap();
        let docs = dir.path().join("docs");
        std::fs::create_dir_all(&docs).unwrap();
        std::fs::write(
            docs.join("raft.md"),
            "---\ntags: [raft]\nnode_type: concept\n---\n# Raft\nSee [[logcompaction]].\n",
        )
        .unwrap();
        std::fs::write(
            docs.join("logcompaction.md"),
            "---\ntags: [log]\nnode_type: concept\n---\n# Log Compaction\nSee [[raft]].\n",
        )
        .unwrap();

        let mut brain = Brain::create(dir.path()).unwrap();
        let stats = brain.sync().unwrap();
        assert_eq!(stats.markdown_files, 2);

        let hits = brain.query("raft").unwrap();
        assert!(!hits.is_empty());

        let ranked = brain
            .query_ranked("raft", &QueryOptions::default())
            .unwrap();
        assert!(ranked[0].score > 0.0);

        let ctx = brain.context_for_prompt("raft", 512).unwrap();
        assert!(
            !ctx.nodes.is_empty(),
            "expected FTS hits for 'raft', got none"
        );
        assert!(ctx.tokens_used > 0);
        assert!(ctx.nodes.iter().any(|n| n.role == ContextRole::Seed));
        let xml = ctx.to_xml();
        assert!(xml.contains("<rustbrain_context"));
        assert!(xml.contains("tokens_used="));

        let out = dir.path().join("out.brainbundle");
        brain.export(&out, true).unwrap();
        assert!(out.exists());

        let _ = brain.sync().unwrap();
        assert_eq!(brain.database().count_fts_rows().unwrap(), 2);
    }

    #[test]
    fn note_anchors_to_symbol() {
        let dir = tempdir().unwrap();
        let docs = dir.path().join("docs");
        let src = dir.path().join("src");
        std::fs::create_dir_all(&docs).unwrap();
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::write(
            src.join("lib.rs"),
            "/// Storage engine\npub struct StorageEngine;\nimpl StorageEngine { pub fn open() {} }\n",
        )
        .unwrap();
        std::fs::write(
            docs.join("design.md"),
            "---\nnode_type: adr\n---\n# Design\nUses symbol:StorageEngine for persistence.\n",
        )
        .unwrap();

        let mut brain = Brain::create(dir.path()).unwrap();
        let stats = brain.sync().unwrap();
        assert!(stats.symbol_anchors >= 1);
        assert!(stats.markdown_files >= 1);

        let _ = brain.sync().unwrap();
        let edges = brain.database().get_all_edges().unwrap();
        let anchors: Vec<_> = edges
            .iter()
            .filter(|e| e.relation_type == "anchors")
            .collect();
        assert!(
            !anchors.is_empty(),
            "expected anchors edge note→symbol, edges={edges:?}"
        );
    }
}
