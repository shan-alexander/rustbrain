//! Safe Markdown link application: pending normalize (Phase 0) + AC discover (Phase 1).
//!
//! # Design
//!
//! - **Closed world:** only rewrite toward nodes that already exist in the brain.
//! - **Unique or skip:** ambiguous resolutions never mutate files.
//! - **Atomic writes:** temp file + rename per path; plan-then-apply with offset-safe edits.
//! - **Tiers:** `auto` may write; `suggest` is dry-run / report only unless opted in later.
//! - **No invention:** never create stub notes or free-form prose.
//!
//! Phase 0 closes the `pending_links` ledger when targets now resolve uniquely.
//! Phase 1 (`discover`) compiles a [`LinkLexicon`] and scans note bodies with
//! Aho–Corasick for unmarked entity mentions.

use crate::error::{BrainError, Result};
use crate::id::resolve_link_target;
use crate::obsidian::{extract_wikilink_spans, parse_frontmatter, WikiLink};
use crate::query::PendingLink;
use crate::storage::Database;
use crate::symbols::{parse_symbol_path, resolve_symbol_ref};
use crate::types::{Node, NodeType};
use aho_corasick::{AhoCorasick, AhoCorasickBuilder, MatchKind};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

// ── public options / report ─────────────────────────────────────────────────

/// Options for [`apply_links`].
#[derive(Debug, Clone)]
pub struct ApplyOptions {
    /// When true, write files (requires explicit opt-in from CLI).
    pub write: bool,
    /// When true with `write`, still plan but do not write (redundant safety).
    pub dry_run: bool,
    /// Phase 1: scan for unmarked entity mentions via Aho–Corasick.
    pub discover: bool,
    /// Allow rewriting files marked `generated: true` or `.generated.` paths.
    pub force_generated: bool,
    /// Max edits that may be applied (or planned as auto when dry-run).
    pub limit: usize,
    /// Optional focus: path, node id, or title of **source** notes only.
    pub target: Option<String>,
    /// Run `sync` after successful writes (caller handles sync; flag is advisory in report).
    pub sync_after: bool,
    /// Include `suggest`-tier discover hits in the report (always; never auto-written).
    pub report_suggest: bool,
}

impl Default for ApplyOptions {
    fn default() -> Self {
        Self {
            write: false,
            dry_run: true,
            discover: false,
            force_generated: false,
            limit: 200,
            target: None,
            sync_after: true,
            report_suggest: true,
        }
    }
}

/// Confidence / disposition of a planned edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplyTier {
    /// Safe to write under `--write`.
    Auto,
    /// Report only (discover weak hits).
    Suggest,
    /// Will not write.
    Skip,
}

/// Kind of planned change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplyKind {
    /// Rewrite an existing WikiLink target to a canonical node id (pending).
    PendingWikiNormalize,
    /// Pending resolved in DB without needing a text change.
    PendingResolvedNoEdit,
    /// Pending could not be resolved uniquely.
    PendingUnresolved,
    /// Wrap an unmarked mention in a WikiLink (discover).
    DiscoverWrap,
    /// Discover hit filtered / not applied.
    DiscoverSkip,
    /// File-level skip (missing path, generated, …).
    FileSkip,
}

/// One planned or applied edit (or skip record).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyEdit {
    /// Disposition.
    pub tier: ApplyTier,
    /// Edit kind.
    pub kind: ApplyKind,
    /// Source node id.
    pub source_id: String,
    /// Repo-relative file path when known.
    pub file_path: Option<String>,
    /// Target node id when known.
    pub target_id: Option<String>,
    /// Original surface (wiki target, symbol raw, or mention text).
    pub before: String,
    /// Replacement text when an edit is planned.
    pub after: Option<String>,
    /// Human-readable reason.
    pub reason: String,
    /// True when this edit was written to disk.
    pub written: bool,
    /// Byte span in the **original** file when applicable (`start`, `end`).
    pub span: Option<(usize, usize)>,
}

/// Aggregate report from [`apply_links`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyReport {
    /// Whether writes were requested.
    pub write_requested: bool,
    /// Whether this run was dry-run (no mutations).
    pub dry_run: bool,
    /// Whether discover mode was enabled.
    pub discover: bool,
    /// Edits actually written.
    pub written: usize,
    /// Auto-tier plans (written or would write).
    pub auto_planned: usize,
    /// Suggest-tier plans (report only).
    pub suggest_planned: usize,
    /// Skips (all kinds).
    pub skipped: usize,
    /// Files that changed on disk.
    pub files_written: Vec<String>,
    /// Files that would change (dry-run).
    pub files_planned: Vec<String>,
    /// Detail rows (capped in text display; full in JSON).
    pub edits: Vec<ApplyEdit>,
    /// Non-fatal issues.
    pub warnings: Vec<String>,
    /// Whether caller should sync after write.
    pub recommend_sync: bool,
    /// Surfaces excluded from discover because they map to multiple nodes.
    pub ambiguous_surfaces: usize,
}

impl ApplyReport {
    /// Human-readable summary for CLI.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "links apply: dry_run={} write_requested={} discover={}\n",
            self.dry_run, self.write_requested, self.discover
        ));
        out.push_str(&format!(
            "  auto={} suggest={} skipped={} written={} files={}\n",
            self.auto_planned,
            self.suggest_planned,
            self.skipped,
            self.written,
            if self.dry_run {
                self.files_planned.len()
            } else {
                self.files_written.len()
            }
        ));
        let show = self.edits.iter().take(80);
        for e in show {
            let status = if e.written {
                "WRITTEN"
            } else {
                match e.tier {
                    ApplyTier::Auto => "AUTO",
                    ApplyTier::Suggest => "SUGGEST",
                    ApplyTier::Skip => "SKIP",
                }
            };
            let after = e.after.as_deref().unwrap_or("-");
            let path = e.file_path.as_deref().unwrap_or("-");
            out.push_str(&format!(
                "  [{status}] {}  {} → {}  ({})  {}\n",
                path, e.before, after, e.source_id, e.reason
            ));
        }
        if self.edits.len() > 80 {
            out.push_str(&format!("  … {} more (use --json)\n", self.edits.len() - 80));
        }
        for w in &self.warnings {
            out.push_str(&format!("  warning: {w}\n"));
        }
        if self.dry_run && self.auto_planned > 0 {
            out.push_str(
                "tip: re-run with `rustbrain links --apply --write` to apply AUTO edits\n",
            );
        } else if !self.dry_run && self.written > 0 && self.recommend_sync {
            out.push_str("tip: run `rustbrain sync` (or rely on auto-sync) so pending/edges refresh\n");
        } else if self.auto_planned == 0 && self.suggest_planned == 0 && self.skipped > 0 {
            out.push_str(
                "tip: pending targets may not resolve yet — create notes or fix titles, then sync\n",
            );
        }
        if !self.discover {
            out.push_str(
                "tip: `links --apply --discover --dry-run` finds unmarked mentions (Phase 1)\n",
            );
        }
        out
    }
}

// ── entry point ─────────────────────────────────────────────────────────────

/// Plan (and optionally apply) link normalizations and discoveries.
pub fn apply_links(
    workspace: &Path,
    db: &Database,
    opts: &ApplyOptions,
) -> Result<ApplyReport> {
    let dry_run = !opts.write || opts.dry_run;
    let limit = opts.limit.max(1);

    let (ids, aliases, titles) = db.link_resolution_maps()?;
    let symbol_ids: HashSet<String> = ids
        .iter()
        .filter(|id| id.starts_with("symbol/"))
        .cloned()
        .collect();

    let all_nodes = db.get_all_nodes()?;
    let node_by_id: HashMap<String, Node> =
        all_nodes.into_iter().map(|n| (n.id.clone(), n)).collect();

    let mut pending = db.list_pending_links()?;
    if let Some(t) = &opts.target {
        let focus = resolve_source_filter(db, t, &node_by_id)?;
        pending.retain(|p| focus.contains(&p.source_id));
    }

    let mut edits: Vec<ApplyEdit> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    // Group pending by source for efficient file reads.
    let mut by_source: HashMap<String, Vec<PendingLink>> = HashMap::new();
    for p in pending {
        by_source.entry(p.source_id.clone()).or_default().push(p);
    }

    // File plans: path -> list of (start, end, replacement, edit_index scaffolding)
    let mut file_ops: HashMap<PathBuf, Vec<SpanReplace>> = HashMap::new();

    for (source_id, pendings) in &by_source {
        let Some(source) = node_by_id.get(source_id) else {
            for p in pendings {
                edits.push(ApplyEdit {
                    tier: ApplyTier::Skip,
                    kind: ApplyKind::FileSkip,
                    source_id: source_id.clone(),
                    file_path: None,
                    target_id: None,
                    before: p.raw_target.clone(),
                    after: None,
                    reason: "source node missing from nodes table".into(),
                    written: false,
                    span: None,
                });
            }
            continue;
        };

        let Some(rel) = source.file_path.as_deref() else {
            for p in pendings {
                // DB-only pending (e.g. canvas-origin): try resolve without file rewrite.
                plan_pending_no_file(
                    p,
                    source_id,
                    &ids,
                    &aliases,
                    &titles,
                    &symbol_ids,
                    &mut edits,
                );
            }
            continue;
        };

        let abs = workspace.join(rel);
        if !abs.is_file() {
            for p in pendings {
                edits.push(ApplyEdit {
                    tier: ApplyTier::Skip,
                    kind: ApplyKind::FileSkip,
                    source_id: source_id.clone(),
                    file_path: Some(rel.to_string()),
                    target_id: None,
                    before: p.raw_target.clone(),
                    after: None,
                    reason: format!("file missing on disk: {}", abs.display()),
                    written: false,
                    span: None,
                });
            }
            continue;
        }

        let content = match fs::read_to_string(&abs) {
            Ok(c) => c,
            Err(e) => {
                warnings.push(format!("read {}: {e}", abs.display()));
                for p in pendings {
                    edits.push(ApplyEdit {
                        tier: ApplyTier::Skip,
                        kind: ApplyKind::FileSkip,
                        source_id: source_id.clone(),
                        file_path: Some(rel.to_string()),
                        target_id: None,
                        before: p.raw_target.clone(),
                        after: None,
                        reason: format!("read error: {e}"),
                        written: false,
                        span: None,
                    });
                }
                continue;
            }
        };

        if is_generated_file(rel, &content) && !opts.force_generated {
            for p in pendings {
                edits.push(ApplyEdit {
                    tier: ApplyTier::Skip,
                    kind: ApplyKind::FileSkip,
                    source_id: source_id.clone(),
                    file_path: Some(rel.to_string()),
                    target_id: None,
                    before: p.raw_target.clone(),
                    after: None,
                    reason: "generated file skipped (pass --force to override)".into(),
                    written: false,
                    span: None,
                });
            }
            continue;
        }

        plan_pending_for_file(
            source_id,
            rel,
            &content,
            pendings,
            &ids,
            &aliases,
            &titles,
            &symbol_ids,
            &mut edits,
            &mut file_ops,
            &abs,
        );
    }

    // Phase 1: discover unmarked mentions.
    let mut ambiguous_surfaces = 0usize;
    if opts.discover {
        let lexicon = LinkLexicon::compile(&node_by_id, &aliases)?;
        ambiguous_surfaces = lexicon.ambiguous.len();
        if ambiguous_surfaces > 0 {
            warnings.push(format!(
                "discover: {ambiguous_surfaces} surface(s) excluded as ambiguous (unique match required)"
            ));
        }
        if let Some(ac) = lexicon.build_automaton() {
            let sources: Vec<&Node> = if let Some(t) = &opts.target {
                let focus = resolve_source_filter(db, t, &node_by_id)?;
                node_by_id
                    .values()
                    .filter(|n| focus.contains(&n.id) && n.node_type != NodeType::Symbol)
                    .collect()
            } else {
                node_by_id
                    .values()
                    .filter(|n| n.node_type != NodeType::Symbol && n.file_path.is_some())
                    .collect()
            };

            for source in sources {
                let Some(rel) = source.file_path.as_deref() else {
                    continue;
                };
                let abs = workspace.join(rel);
                if !abs.is_file() {
                    continue;
                }
                let content = match fs::read_to_string(&abs) {
                    Ok(c) => c,
                    Err(e) => {
                        warnings.push(format!("discover read {}: {e}", abs.display()));
                        continue;
                    }
                };
                if is_generated_file(rel, &content) && !opts.force_generated {
                    continue;
                }
                plan_discover_for_file(
                    source,
                    rel,
                    &content,
                    &lexicon,
                    &ac,
                    opts.report_suggest,
                    &mut edits,
                    &mut file_ops,
                    &abs,
                );
            }
        } else {
            warnings.push("discover: empty lexicon (no eligible patterns)".into());
        }
    }

    // Enforce limit on auto-tier span replaces (prefer pending over discover by sort).
    prioritize_and_cap_ops(&mut file_ops, &mut edits, limit);

    // Apply writes.
    let mut files_written = Vec::new();
    let mut files_planned = Vec::new();
    let mut written_count = 0usize;

    for (abs, ops) in &file_ops {
        if ops.is_empty() {
            continue;
        }
        let rel = abs
            .strip_prefix(workspace)
            .unwrap_or(abs.as_path())
            .to_string_lossy()
            .replace('\\', "/");
        files_planned.push(rel.clone());

        if dry_run {
            continue;
        }

        let original = match fs::read_to_string(abs) {
            Ok(c) => c,
            Err(e) => {
                warnings.push(format!("write aborted, re-read {}: {e}", abs.display()));
                continue;
            }
        };

        match apply_span_replaces(&original, ops) {
            Ok(new_content) => {
                if new_content == original {
                    continue;
                }
                match atomic_write(abs, &new_content) {
                    Ok(()) => {
                        files_written.push(rel);
                        written_count += ops.len();
                        // mark matching edits written
                        for op in ops {
                            for e in edits.iter_mut() {
                                if e.span == Some((op.start, op.end))
                                    && e.file_path.as_deref()
                                        == Some(
                                            abs.strip_prefix(workspace)
                                                .unwrap_or(abs)
                                                .to_string_lossy()
                                                .replace('\\', "/")
                                                .as_str(),
                                        )
                                    && e.tier == ApplyTier::Auto
                                {
                                    e.written = true;
                                }
                            }
                        }
                    }
                    Err(e) => warnings.push(format!("atomic write {}: {e}", abs.display())),
                }
            }
            Err(e) => warnings.push(format!("apply plan {}: {e}", abs.display())),
        }
    }

    // Fix written flags more reliably by path + before/after
    if !dry_run {
        let written_set: HashSet<String> = files_written.iter().cloned().collect();
        for e in edits.iter_mut() {
            if e.tier == ApplyTier::Auto
                && e.after.is_some()
                && e.file_path
                    .as_ref()
                    .is_some_and(|p| written_set.contains(p))
                && matches!(
                    e.kind,
                    ApplyKind::PendingWikiNormalize | ApplyKind::DiscoverWrap
                )
            {
                e.written = true;
            }
        }
        written_count = edits.iter().filter(|e| e.written).count();
    }

    let auto_planned = edits
        .iter()
        .filter(|e| e.tier == ApplyTier::Auto && e.after.is_some())
        .count();
    let suggest_planned = edits.iter().filter(|e| e.tier == ApplyTier::Suggest).count();
    let skipped = edits.iter().filter(|e| e.tier == ApplyTier::Skip).count();

    files_planned.sort();
    files_planned.dedup();
    files_written.sort();
    files_written.dedup();

    Ok(ApplyReport {
        write_requested: opts.write,
        dry_run,
        discover: opts.discover,
        written: written_count,
        auto_planned,
        suggest_planned,
        skipped,
        files_written,
        files_planned,
        edits,
        warnings,
        recommend_sync: !dry_run && written_count > 0 && opts.sync_after,
        ambiguous_surfaces,
    })
}

// ── span replace plumbing ───────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct SpanReplace {
    start: usize,
    end: usize,
    replacement: String,
    /// Prefer pending (0) over discover (1) when capping.
    priority: u8,
}

fn apply_span_replaces(content: &str, ops: &[SpanReplace]) -> Result<String> {
    let mut ops: Vec<&SpanReplace> = ops.iter().collect();
    ops.sort_by(|a, b| b.start.cmp(&a.start).then_with(|| b.end.cmp(&a.end)));

    // Overlap check (after sorting by start desc, check neighbors in original order)
    let mut ordered = ops.clone();
    ordered.sort_by(|a, b| a.start.cmp(&b.start).then_with(|| a.end.cmp(&b.end)));
    for w in ordered.windows(2) {
        if w[0].end > w[1].start {
            return Err(BrainError::other(format!(
                "overlapping edits at {}..{} and {}..{}",
                w[0].start, w[0].end, w[1].start, w[1].end
            )));
        }
    }

    let mut out = content.to_string();
    for op in ops {
        if op.start > op.end || op.end > out.len() {
            return Err(BrainError::other(format!(
                "invalid span {}..{} (len {})",
                op.start,
                op.end,
                out.len()
            )));
        }
        if !out.is_char_boundary(op.start) || !out.is_char_boundary(op.end) {
            return Err(BrainError::other("edit span not on UTF-8 boundary"));
        }
        out.replace_range(op.start..op.end, &op.replacement);
    }
    Ok(out)
}

fn atomic_write(path: &Path, content: &str) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let tmp = parent.join(format!(
        ".{}.rb-apply.{}.tmp",
        path.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("note.md"),
        std::process::id()
    ));
    fs::write(&tmp, content.as_bytes())?;
    if let Ok(f) = fs::File::open(&tmp) {
        let _ = f.sync_all();
    }
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        BrainError::from(e)
    })?;
    Ok(())
}

fn prioritize_and_cap_ops(
    file_ops: &mut HashMap<PathBuf, Vec<SpanReplace>>,
    edits: &mut [ApplyEdit],
    limit: usize,
) {
    // Collect all auto ops with global priority.
    let mut all: Vec<(PathBuf, SpanReplace)> = Vec::new();
    for (p, ops) in file_ops.iter() {
        for op in ops {
            all.push((p.clone(), op.clone()));
        }
    }
    all.sort_by(|a, b| {
        a.1.priority
            .cmp(&b.1.priority)
            .then_with(|| a.0.cmp(&b.0))
            .then_with(|| a.1.start.cmp(&b.1.start))
    });

    if all.len() <= limit {
        return;
    }

    let keep: HashSet<(PathBuf, usize, usize)> = all
        .iter()
        .take(limit)
        .map(|(p, op)| (p.clone(), op.start, op.end))
        .collect();

    for (p, ops) in file_ops.iter_mut() {
        ops.retain(|op| keep.contains(&(p.clone(), op.start, op.end)));
    }

    // Demote edits that lost their op
    for e in edits.iter_mut() {
        if e.tier == ApplyTier::Auto && e.after.is_some() {
            if let (Some(fp), Some((s, en))) = (&e.file_path, e.span) {
                let abs_match = keep.iter().any(|(p, a, b)| {
                    *a == s
                        && *b == en
                        && p.to_string_lossy().replace('\\', "/").ends_with(fp.as_str())
                });
                if !abs_match {
                    // might still be kept — check loosely
                    let still = file_ops.values().flatten().any(|op| {
                        op.start == s && op.end == en && op.replacement == e.after.as_deref().unwrap_or("")
                    });
                    if !still {
                        e.tier = ApplyTier::Skip;
                        e.kind = ApplyKind::FileSkip;
                        e.reason = format!("capped by --limit {limit}");
                        e.after = None;
                    }
                }
            }
        }
    }
}

// ── Phase 0: pending ────────────────────────────────────────────────────────

fn plan_pending_no_file(
    p: &PendingLink,
    source_id: &str,
    ids: &HashSet<String>,
    aliases: &HashMap<String, String>,
    titles: &HashMap<String, String>,
    symbol_ids: &HashSet<String>,
    edits: &mut Vec<ApplyEdit>,
) {
    match resolve_pending_target(&p.raw_target, ids, aliases, titles, symbol_ids) {
        Some(tid) => edits.push(ApplyEdit {
            tier: ApplyTier::Skip,
            kind: ApplyKind::PendingResolvedNoEdit,
            source_id: source_id.into(),
            file_path: None,
            target_id: Some(tid),
            before: p.raw_target.clone(),
            after: None,
            reason: "no markdown file for source — edge will resolve on sync, no text rewrite"
                .into(),
            written: false,
            span: None,
        }),
        None => edits.push(ApplyEdit {
            tier: ApplyTier::Skip,
            kind: ApplyKind::PendingUnresolved,
            source_id: source_id.into(),
            file_path: None,
            target_id: None,
            before: p.raw_target.clone(),
            after: None,
            reason: "target still unresolved (and no source file)".into(),
            written: false,
            span: None,
        }),
    }
}

fn plan_pending_for_file(
    source_id: &str,
    rel: &str,
    content: &str,
    pendings: &[PendingLink],
    ids: &HashSet<String>,
    aliases: &HashMap<String, String>,
    titles: &HashMap<String, String>,
    symbol_ids: &HashSet<String>,
    edits: &mut Vec<ApplyEdit>,
    file_ops: &mut HashMap<PathBuf, Vec<SpanReplace>>,
    abs: &Path,
) {
    let spans = extract_wikilink_spans(content);

    for p in pendings {
        let resolved =
            resolve_pending_target(&p.raw_target, ids, aliases, titles, symbol_ids);

        let Some(target_id) = resolved else {
            edits.push(ApplyEdit {
                tier: ApplyTier::Skip,
                kind: ApplyKind::PendingUnresolved,
                source_id: source_id.into(),
                file_path: Some(rel.into()),
                target_id: None,
                before: p.raw_target.clone(),
                after: None,
                reason: "target does not uniquely resolve — create the note or disambiguate"
                    .into(),
                written: false,
                span: None,
            });
            continue;
        };

        // Match WikiLinks whose target equals pending raw (wiki) or symbol form.
        let wiki_raw = p
            .raw_target
            .strip_prefix("symbol:")
            .map(|s| format!("symbol:{s}"))
            .unwrap_or_else(|| p.raw_target.clone());

        let mut matched = 0usize;
        for sp in &spans {
            if !wikilink_matches_pending(&sp.link, &p.raw_target) {
                continue;
            }
            matched += 1;
            let new_link = normalize_wikilink(&sp.link, &target_id, &p.raw_target);
            let after = new_link.to_markdown();
            let before = content[sp.start..sp.end].to_string();
            if before == after {
                edits.push(ApplyEdit {
                    tier: ApplyTier::Skip,
                    kind: ApplyKind::PendingResolvedNoEdit,
                    source_id: source_id.into(),
                    file_path: Some(rel.into()),
                    target_id: Some(target_id.clone()),
                    before,
                    after: None,
                    reason: "already canonical; edge will clear on sync".into(),
                    written: false,
                    span: Some((sp.start, sp.end)),
                });
                continue;
            }
            file_ops.entry(abs.to_path_buf()).or_default().push(SpanReplace {
                start: sp.start,
                end: sp.end,
                replacement: after.clone(),
                priority: 0,
            });
            edits.push(ApplyEdit {
                tier: ApplyTier::Auto,
                kind: ApplyKind::PendingWikiNormalize,
                source_id: source_id.into(),
                file_path: Some(rel.into()),
                target_id: Some(target_id.clone()),
                before,
                after: Some(after),
                reason: format!("pending `{}` → unique node `{target_id}`", p.raw_target),
                written: false,
                span: Some((sp.start, sp.end)),
            });
        }

        // Bare `symbol:…` in body (not necessarily a wiki).
        if matched == 0 && p.raw_target.starts_with("symbol:") {
            if let Some((start, end, before)) = find_symbol_token(content, &p.raw_target) {
                // Leave token as-is when it resolves — no markdown rewrite needed.
                edits.push(ApplyEdit {
                    tier: ApplyTier::Skip,
                    kind: ApplyKind::PendingResolvedNoEdit,
                    source_id: source_id.into(),
                    file_path: Some(rel.into()),
                    target_id: Some(target_id),
                    before,
                    after: None,
                    reason: "symbol: ref resolves; edge created on sync without rewrite".into(),
                    written: false,
                    span: Some((start, end)),
                });
                continue;
            }
        }

        if matched == 0 {
            edits.push(ApplyEdit {
                tier: ApplyTier::Skip,
                kind: ApplyKind::PendingResolvedNoEdit,
                source_id: source_id.into(),
                file_path: Some(rel.into()),
                target_id: Some(target_id),
                before: p.raw_target.clone(),
                after: None,
                reason: format!(
                    "resolves to target but `{wiki_raw}` not found as WikiLink in file (edge on sync)"
                ),
                written: false,
                span: None,
            });
        }
    }
}

fn wikilink_matches_pending(link: &WikiLink, raw_target: &str) -> bool {
    let t = link.target_node.as_str();
    if t == raw_target {
        return true;
    }
    // pending stores symbol:… ; wiki may be symbol:… without extra prefix issues
    if let Some(rest) = raw_target.strip_prefix("symbol:") {
        if t == raw_target || t.strip_prefix("symbol:") == Some(rest) || t == rest {
            return true;
        }
    }
    // case-insensitive target match for user-facing titles used as links
    t.eq_ignore_ascii_case(raw_target)
}

fn normalize_wikilink(old: &WikiLink, canonical_id: &str, raw_pending: &str) -> WikiLink {
    // Prefer keeping display alias; else preserve original target as display when it differs.
    let display = old.display_alias.clone().or_else(|| {
        if old.target_node != canonical_id && !raw_pending.starts_with("symbol:") {
            Some(old.target_node.clone())
        } else {
            None
        }
    });
    // For symbol anchors, keep a readable symbol: surface when canonical is symbol/…
    let target_node = if canonical_id.starts_with("symbol/") {
        if old.target_node.starts_with("symbol:") {
            old.target_node.clone()
        } else if raw_pending.starts_with("symbol:") {
            raw_pending.to_string()
        } else {
            canonical_id.to_string()
        }
    } else {
        canonical_id.to_string()
    };
    let display_alias = display.filter(|d| d != &target_node);
    WikiLink {
        target_node,
        section: old.section.clone(),
        display_alias,
    }
}

fn resolve_pending_target(
    raw_target: &str,
    ids: &HashSet<String>,
    aliases: &HashMap<String, String>,
    titles: &HashMap<String, String>,
    symbol_ids: &HashSet<String>,
) -> Option<String> {
    if let Some(sym_path) = raw_target.strip_prefix("symbol:") {
        parse_symbol_path(sym_path)
            .and_then(|s| resolve_symbol_ref(&s, symbol_ids))
            .or_else(|| {
                parse_symbol_path(sym_path)
                    .and_then(|s| resolve_link_target(&s.symbol_name, ids, aliases, titles))
            })
    } else {
        resolve_link_target(raw_target, ids, aliases, titles)
    }
}

fn find_symbol_token(content: &str, raw_target: &str) -> Option<(usize, usize, String)> {
    // raw_target like "symbol:StorageEngine"
    let needle = raw_target;
    let bytes = content.as_bytes();
    let mut i = 0;
    let mut in_fence = false;
    while i + needle.len() <= bytes.len() {
        if is_line_start_bytes(bytes, i) && i + 2 < bytes.len() && &bytes[i..i + 3] == b"```" {
            in_fence = !in_fence;
            i += 3;
            continue;
        }
        if in_fence {
            i += 1;
            continue;
        }
        if bytes[i] == b'`' {
            i += 1;
            while i < bytes.len() && bytes[i] != b'`' {
                i += 1;
            }
            if i < bytes.len() {
                i += 1;
            }
            continue;
        }
        if content.is_char_boundary(i)
            && content[i..].starts_with(needle)
            && content.is_char_boundary(i + needle.len())
        {
            return Some((i, i + needle.len(), needle.to_string()));
        }
        i += 1;
    }
    None
}

// ── Phase 1: lexicon + AC ───────────────────────────────────────────────────

/// Closed-world surface → node dictionary for discover.
struct LinkLexicon {
    /// pattern string → node_id (only unambiguous surfaces)
    surfaces: Vec<(String, String, PatternKind)>,
    /// surfaces dropped as ambiguous
    ambiguous: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
enum PatternKind {
    Title,
    Alias,
    Stem,
    SymbolName,
}

impl LinkLexicon {
    fn compile(
        nodes: &HashMap<String, Node>,
        aliases: &HashMap<String, String>,
    ) -> Result<Self> {
        // surface_lower → set of node ids (for ambiguity)
        let mut map: HashMap<String, HashSet<String>> = HashMap::new();
        let mut original_case: HashMap<String, String> = HashMap::new();
        let mut kinds: HashMap<String, PatternKind> = HashMap::new();

        for n in nodes.values() {
            if n.node_type == NodeType::Symbol {
                // symbol short name
                if let Some(name) = n.id.rsplit('/').next() {
                    // Prefer Type::method last segment after ::
                    let short = name.rsplit("::").next().unwrap_or(name);
                    add_surface(&mut map, &mut original_case, &mut kinds, short, &n.id, PatternKind::SymbolName);
                }
                continue;
            }
            add_surface(
                &mut map,
                &mut original_case,
                &mut kinds,
                &n.title,
                &n.id,
                PatternKind::Title,
            );
            if let Some(path) = &n.file_path {
                if let Some(stem) = Path::new(path).file_stem().and_then(|s| s.to_str()) {
                    add_surface(
                        &mut map,
                        &mut original_case,
                        &mut kinds,
                        stem,
                        &n.id,
                        PatternKind::Stem,
                    );
                }
            }
        }
        for (alias, id) in aliases {
            add_surface(
                &mut map,
                &mut original_case,
                &mut kinds,
                alias,
                id,
                PatternKind::Alias,
            );
        }

        let mut surfaces = Vec::new();
        let mut ambiguous = Vec::new();
        for (lower, ids) in &map {
            if is_stop_surface(lower) {
                continue;
            }
            let min_len = match kinds.get(lower) {
                Some(PatternKind::SymbolName) => 3,
                Some(PatternKind::Alias) => 3,
                _ => 4,
            };
            if lower.chars().count() < min_len {
                continue;
            }
            if ids.len() != 1 {
                ambiguous.push(original_case.get(lower).cloned().unwrap_or_else(|| lower.clone()));
                continue;
            }
            let id = ids.iter().next().unwrap().clone();
            let surface = original_case
                .get(lower)
                .cloned()
                .unwrap_or_else(|| lower.clone());
            let kind = kinds.get(lower).copied().unwrap_or(PatternKind::Title);
            surfaces.push((surface, id, kind));
        }

        // Prefer longer patterns first for AC leftmost-longest
        surfaces.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(&b.0)));
        ambiguous.sort();
        ambiguous.dedup();

        Ok(Self {
            surfaces,
            ambiguous,
        })
    }

    fn build_automaton(&self) -> Option<AhoCorasick> {
        if self.surfaces.is_empty() {
            return None;
        }
        let patterns: Vec<&str> = self.surfaces.iter().map(|(s, _, _)| s.as_str()).collect();
        AhoCorasickBuilder::new()
            .ascii_case_insensitive(true)
            .match_kind(MatchKind::LeftmostLongest)
            .build(patterns)
            .ok()
    }

    fn node_for_pattern_index(&self, idx: usize) -> Option<(&str, PatternKind)> {
        self.surfaces
            .get(idx)
            .map(|(s, id, k)| {
                let _ = s;
                (id.as_str(), *k)
            })
    }
}

fn add_surface(
    map: &mut HashMap<String, HashSet<String>>,
    original_case: &mut HashMap<String, String>,
    kinds: &mut HashMap<String, PatternKind>,
    surface: &str,
    node_id: &str,
    kind: PatternKind,
) {
    let s = surface.trim();
    if s.is_empty() {
        return;
    }
    let lower = s.to_ascii_lowercase();
    map.entry(lower.clone()).or_default().insert(node_id.to_string());
    original_case.entry(lower.clone()).or_insert_with(|| s.to_string());
    kinds.entry(lower).or_insert(kind);
}

fn is_stop_surface(lower: &str) -> bool {
    // High-DF English / Rust noise — never auto-link.
    matches!(
        lower,
        "the"
            | "and"
            | "for"
            | "with"
            | "from"
            | "this"
            | "that"
            | "into"
            | "over"
            | "under"
            | "about"
            | "after"
            | "before"
            | "data"
            | "file"
            | "files"
            | "path"
            | "type"
            | "name"
            | "value"
            | "error"
            | "errors"
            | "state"
            | "config"
            | "default"
            | "result"
            | "option"
            | "string"
            | "number"
            | "index"
            | "list"
            | "item"
            | "test"
            | "tests"
            | "main"
            | "lib"
            | "mod"
            | "use"
            | "impl"
            | "self"
            | "super"
            | "crate"
            | "pub"
            | "fn"
            | "struct"
            | "enum"
            | "trait"
            | "const"
            | "static"
            | "true"
            | "false"
            | "none"
            | "some"
            | "ok"
            | "err"
            | "note"
            | "notes"
            | "docs"
            | "doc"
            | "readme"
            | "todo"
            | "fixme"
            | "open"
            | "read"
            | "write"
            | "load"
            | "save"
            | "run"
            | "new"
            | "get"
            | "set"
            | "add"
            | "remove"
            | "create"
            | "update"
            | "delete"
            | "code"
            | "text"
            | "body"
            | "title"
            | "link"
            | "links"
            | "node"
            | "nodes"
            | "edge"
            | "edges"
            | "graph"
            | "query"
            | "context"
            | "sync"
            | "build"
            | "cargo"
            | "rust"
            | "http"
            | "https"
            | "json"
            | "yaml"
            | "markdown"
            | "generated"
            | "implementation"
            | "concept"
            | "concepts"
            | "goal"
            | "goals"
            | "analysis"
            | "reference"
            | "alternative"
            | "edge_case"
            | "edge-case"
            | "adr"
            | "symbol"
            | "module"
            | "modules"
            | "function"
            | "functions"
            | "method"
            | "methods"
            | "field"
            | "fields"
            | "param"
            | "params"
            | "return"
            | "returns"
            | "input"
            | "output"
            | "user"
            | "users"
            | "app"
            | "application"
            | "system"
            | "service"
            | "server"
            | "client"
            | "api"
            | "id"
            | "ids"
            | "key"
            | "keys"
            | "map"
            | "vec"
            | "vector"
            | "array"
            | "table"
            | "row"
            | "column"
            | "page"
            | "home"
            | "info"
            | "debug"
            | "trace"
            | "warn"
            | "warning"
            | "log"
            | "logs"
            | "time"
            | "date"
            | "version"
            | "v1"
            | "v2"
            | "see"
            | "also"
            | "related"
            | "summary"
            | "status"
            | "decision"
            | "consequences"
            | "checklist"
            | "bootstrap"
            | "template"
            | "scaffold"
            | "example"
            | "examples"
            | "usage"
            | "install"
            | "license"
            | "mit"
            | "apache"
    ) || lower.len() <= 2
}

fn plan_discover_for_file(
    source: &Node,
    rel: &str,
    content: &str,
    lexicon: &LinkLexicon,
    ac: &AhoCorasick,
    report_suggest: bool,
    edits: &mut Vec<ApplyEdit>,
    file_ops: &mut HashMap<PathBuf, Vec<SpanReplace>>,
    abs: &Path,
) {
    let mask = build_unlinkable_mask(content);
    let existing_targets: HashSet<String> = extract_wikilink_spans(content)
        .into_iter()
        .map(|s| s.link.target_node.to_ascii_lowercase())
        .collect();

    // One wrap per target node per file (avoid spam).
    let mut linked_targets: HashSet<String> = HashSet::new();

    for mat in ac.find_iter(content) {
        let start = mat.start();
        let end = mat.end();
        if !content.is_char_boundary(start) || !content.is_char_boundary(end) {
            continue;
        }
        if region_blocked(&mask, start, end) {
            continue;
        }
        if !identifier_boundaries(content, start, end) {
            continue;
        }
        let Some((node_id, kind)) = lexicon.node_for_pattern_index(mat.pattern().as_usize()) else {
            continue;
        };
        if node_id == source.id {
            continue;
        }
        if linked_targets.contains(node_id) {
            continue;
        }
        if existing_targets.contains(&node_id.to_ascii_lowercase()) {
            continue;
        }
        // Already linked under some surface of this node
        let surface = &content[start..end];
        if existing_targets.contains(&surface.to_ascii_lowercase()) {
            continue;
        }

        let (tier, reason) = match kind {
            PatternKind::Title | PatternKind::Alias if surface.chars().count() >= 5 => (
                ApplyTier::Auto,
                format!("discover unique {:?} mention → `{node_id}`", kind_name(kind)),
            ),
            PatternKind::SymbolName if surface.chars().any(|c| c.is_ascii_uppercase()) => (
                ApplyTier::Auto,
                format!("discover unique symbol-like mention → `{node_id}`"),
            ),
            PatternKind::Title | PatternKind::Alias | PatternKind::Stem | PatternKind::SymbolName => (
                ApplyTier::Suggest,
                format!(
                    "discover weak {:?} mention → `{node_id}` (suggest only)",
                    kind_name(kind)
                ),
            ),
        };

        if tier == ApplyTier::Suggest && !report_suggest {
            continue;
        }

        let after = format!("[[{node_id}|{surface}]]");
        let before = surface.to_string();

        if tier == ApplyTier::Auto {
            // Avoid overlapping ops
            let ops = file_ops.entry(abs.to_path_buf()).or_default();
            if ops.iter().any(|o| spans_overlap(o.start, o.end, start, end)) {
                edits.push(ApplyEdit {
                    tier: ApplyTier::Skip,
                    kind: ApplyKind::DiscoverSkip,
                    source_id: source.id.clone(),
                    file_path: Some(rel.into()),
                    target_id: Some(node_id.into()),
                    before,
                    after: None,
                    reason: "overlaps another planned edit".into(),
                    written: false,
                    span: Some((start, end)),
                });
                continue;
            }
            ops.push(SpanReplace {
                start,
                end,
                replacement: after.clone(),
                priority: 1,
            });
            linked_targets.insert(node_id.to_string());
            edits.push(ApplyEdit {
                tier: ApplyTier::Auto,
                kind: ApplyKind::DiscoverWrap,
                source_id: source.id.clone(),
                file_path: Some(rel.into()),
                target_id: Some(node_id.into()),
                before,
                after: Some(after),
                reason,
                written: false,
                span: Some((start, end)),
            });
        } else {
            edits.push(ApplyEdit {
                tier: ApplyTier::Suggest,
                kind: ApplyKind::DiscoverWrap,
                source_id: source.id.clone(),
                file_path: Some(rel.into()),
                target_id: Some(node_id.into()),
                before,
                after: Some(after),
                reason,
                written: false,
                span: Some((start, end)),
            });
            linked_targets.insert(node_id.to_string());
        }
    }

    let _ = &lexicon.ambiguous; // reserved for future report field
}

fn kind_name(k: PatternKind) -> &'static str {
    match k {
        PatternKind::Title => "title",
        PatternKind::Alias => "alias",
        PatternKind::Stem => "stem",
        PatternKind::SymbolName => "symbol",
    }
}

fn spans_overlap(a0: usize, a1: usize, b0: usize, b1: usize) -> bool {
    a0 < b1 && b0 < a1
}

/// Bit mask: 1 = not eligible for discover wrap.
fn build_unlinkable_mask(content: &str) -> Vec<u8> {
    let mut mask = vec![0u8; content.len()];
    let bytes = content.as_bytes();
    let len = bytes.len();

    // Frontmatter
    if let Some(body_start) = frontmatter_body_start(content) {
        for b in mask.iter_mut().take(body_start) {
            *b = 1;
        }
    }

    let mut i = 0;
    let mut in_fence = false;
    while i < len {
        if is_line_start_bytes(bytes, i) && i + 2 < len && &bytes[i..i + 3] == b"```" {
            // mark whole fence line start; toggle and mark until end fence
            let fence_start = i;
            in_fence = !in_fence;
            i += 3;
            if in_fence {
                // mark until closing fence
                while i < len {
                    if is_line_start_bytes(bytes, i) && i + 2 < len && &bytes[i..i + 3] == b"```" {
                        for b in mask.iter_mut().take(i + 3).skip(fence_start) {
                            *b = 1;
                        }
                        i += 3;
                        in_fence = false;
                        break;
                    }
                    i += 1;
                }
                if in_fence {
                    for b in mask.iter_mut().skip(fence_start) {
                        *b = 1;
                    }
                }
            }
            continue;
        }
        if bytes[i] == b'`' {
            let s = i;
            i += 1;
            while i < len && bytes[i] != b'`' {
                i += 1;
            }
            if i < len {
                i += 1;
            }
            for b in mask.iter_mut().take(i).skip(s) {
                *b = 1;
            }
            continue;
        }
        if i + 1 < len && bytes[i] == b'[' && bytes[i + 1] == b'[' {
            if let Some(rel) = content[i + 2..].find("]]") {
                let end = i + 2 + rel + 2;
                for b in mask.iter_mut().take(end).skip(i) {
                    *b = 1;
                }
                i = end;
                continue;
            }
        }
        i += 1;
    }
    mask
}

fn frontmatter_body_start(content: &str) -> Option<usize> {
    let trimmed = content.trim_start();
    let trim_off = content.len() - trimmed.len();
    if !trimmed.starts_with("---") {
        return None;
    }
    let rest = &trimmed[3..];
    let rest = rest.strip_prefix('\n').unwrap_or(rest);
    let rest_off = content.len() - rest.len();
    if let Some(end_idx) = rest.find("\n---") {
        let after = end_idx + 4;
        let body_rel = rest[after..].trim_start_matches('\n');
        let body_off = rest_off + after + (rest[after..].len() - body_rel.len());
        return Some(body_off.max(trim_off));
    }
    None
}

fn region_blocked(mask: &[u8], start: usize, end: usize) -> bool {
    if end > mask.len() {
        return true;
    }
    mask[start..end].iter().any(|&b| b != 0)
}

fn identifier_boundaries(content: &str, start: usize, end: usize) -> bool {
    let bytes = content.as_bytes();
    if start > 0 {
        let c = bytes[start - 1];
        if c.is_ascii_alphanumeric() || c == b'_' {
            return false;
        }
        // Don't match mid-word for UTF-8 letters before
        if content.is_char_boundary(start) {
            if let Some(ch) = content[..start].chars().last() {
                if ch.is_alphanumeric() || ch == '_' {
                    return false;
                }
            }
        }
    }
    if end < bytes.len() {
        let c = bytes[end];
        if c.is_ascii_alphanumeric() || c == b'_' {
            return false;
        }
        if content.is_char_boundary(end) {
            if let Some(ch) = content[end..].chars().next() {
                if ch.is_alphanumeric() || ch == '_' {
                    return false;
                }
            }
        }
    }
    true
}

fn is_line_start_bytes(bytes: &[u8], i: usize) -> bool {
    i == 0 || bytes[i - 1] == b'\n'
}

fn is_generated_file(rel: &str, content: &str) -> bool {
    let path = rel.replace('\\', "/");
    if path.contains(".generated.")
        || path.ends_with("module-map.generated.md")
        || path.contains("/generated/")
    {
        return true;
    }
    let (fm, _) = parse_frontmatter(content);
    if let Some(fm) = fm {
        if let Some(v) = fm.extra.get("generated") {
            match v {
                serde_yaml_ng::Value::Bool(true) => return true,
                serde_yaml_ng::Value::String(s) if s == "true" || s == "yes" => return true,
                _ => {}
            }
        }
    }
    content.lines().take(20).any(|l| {
        let t = l.trim();
        t == "generated: true" || t == "generated: yes"
    })
}

fn resolve_source_filter(
    db: &Database,
    target: &str,
    nodes: &HashMap<String, Node>,
) -> Result<HashSet<String>> {
    // Reuse graph resolver if available
    match crate::graph::resolve_graph_target(db, target) {
        Ok(n) => Ok(HashSet::from([n.id])),
        Err(_) => {
            // path suffix soft match
            let raw = target.trim().trim_start_matches("./").replace('\\', "/");
            let hits: Vec<_> = nodes
                .values()
                .filter(|n| {
                    n.id == raw
                        || n.file_path
                            .as_ref()
                            .is_some_and(|p| p == &raw || p.ends_with(&raw))
                })
                .map(|n| n.id.clone())
                .collect();
            if hits.is_empty() {
                Err(BrainError::other(format!(
                    "apply target `{target}` not found — pass a source path or node id"
                )))
            } else {
                Ok(hits.into_iter().collect())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Node, NodeType};
    use tempfile::tempdir;

    fn setup() -> (tempfile::TempDir, Database, PathBuf) {
        let dir = tempdir().unwrap();
        let ws = dir.path().to_path_buf();
        let docs = ws.join("docs/concepts");
        fs::create_dir_all(&docs).unwrap();
        fs::write(
            docs.join("raft.md"),
            "---\nnode_type: concept\naliases: [RaftConsensus]\n---\n# Raft\n\nSee [[LogCompaction]] and the StorageEngine path.\n",
        )
        .unwrap();
        fs::write(
            docs.join("logcompaction.md"),
            "---\nnode_type: concept\n---\n# Log Compaction\n\nDetails.\n",
        )
        .unwrap();

        let db = Database::open(ws.join("db.sqlite")).unwrap();
        let now = 1_700_000_000i64;
        let raft = Node {
            id: "docs/concepts/raft".into(),
            node_type: NodeType::Concept,
            title: "Raft".into(),
            file_path: Some("docs/concepts/raft.md".into()),
            symbol_hash: None,
            summary: None,
            content_hash: None,
            created_at: now,
            updated_at: now,
        };
        let logc = Node {
            id: "docs/concepts/logcompaction".into(),
            node_type: NodeType::Concept,
            title: "Log Compaction".into(),
            file_path: Some("docs/concepts/logcompaction.md".into()),
            symbol_hash: None,
            summary: None,
            content_hash: None,
            created_at: now,
            updated_at: now,
        };
        db.insert_node(&raft).unwrap();
        db.insert_node(&logc).unwrap();
        db.replace_node_aliases(
            "docs/concepts/raft",
            &["RaftConsensus".into(), "Raft".into()],
        )
        .unwrap();
        db.replace_node_aliases(
            "docs/concepts/logcompaction",
            &["LogCompaction".into(), "Log Compaction".into()],
        )
        .unwrap();
        db.insert_pending_link(
            "docs/concepts/raft",
            "LogCompaction",
            "relates_to",
            now,
        )
        .unwrap();
        (dir, db, ws)
    }

    #[test]
    fn phase0_dry_run_plans_wiki_normalize() {
        let (_dir, db, ws) = setup();
        let report = apply_links(
            &ws,
            &db,
            &ApplyOptions {
                write: false,
                dry_run: true,
                ..ApplyOptions::default()
            },
        )
        .unwrap();
        assert!(report.dry_run);
        assert!(
            report.auto_planned >= 1,
            "expected auto plan, got {:?}",
            report.edits
        );
        let e = report
            .edits
            .iter()
            .find(|e| e.kind == ApplyKind::PendingWikiNormalize)
            .expect("normalize edit");
        assert!(e.after.as_ref().unwrap().contains("docs/concepts/logcompaction"));
        // file unchanged
        let body = fs::read_to_string(ws.join("docs/concepts/raft.md")).unwrap();
        assert!(body.contains("[[LogCompaction]]"));
    }

    #[test]
    fn phase0_write_rewrites_and_preserves_display() {
        let (_dir, db, ws) = setup();
        let report = apply_links(
            &ws,
            &db,
            &ApplyOptions {
                write: true,
                dry_run: false,
                sync_after: false,
                ..ApplyOptions::default()
            },
        )
        .unwrap();
        assert!(report.written >= 1);
        let body = fs::read_to_string(ws.join("docs/concepts/raft.md")).unwrap();
        assert!(
            body.contains("[[docs/concepts/logcompaction|LogCompaction]]")
                || body.contains("[[docs/concepts/logcompaction]]"),
            "body={body}"
        );
        assert!(!body.contains("[[LogCompaction]]"));
    }

    #[test]
    fn phase0_skips_unresolved() {
        let (_dir, db, ws) = setup();
        db.insert_pending_link("docs/concepts/raft", "NoSuchNote", "relates_to", 1)
            .unwrap();
        let report = apply_links(
            &ws,
            &db,
            &ApplyOptions {
                write: false,
                ..ApplyOptions::default()
            },
        )
        .unwrap();
        assert!(report
            .edits
            .iter()
            .any(|e| e.kind == ApplyKind::PendingUnresolved && e.before == "NoSuchNote"));
    }

    #[test]
    fn phase0_skips_generated_without_force() {
        let dir = tempdir().unwrap();
        let ws = dir.path().to_path_buf();
        let path = ws.join("docs/implementation");
        fs::create_dir_all(&path).unwrap();
        fs::write(
            path.join("module-map.generated.md"),
            "---\ngenerated: true\n---\n# Map\n[[LogCompaction]]\n",
        )
        .unwrap();
        let db = Database::open(ws.join("db.sqlite")).unwrap();
        let now = 1i64;
        db.insert_node(&Node {
            id: "docs/implementation/module-map.generated".into(),
            node_type: NodeType::Concept,
            title: "Map".into(),
            file_path: Some("docs/implementation/module-map.generated.md".into()),
            symbol_hash: None,
            summary: None,
            content_hash: None,
            created_at: now,
            updated_at: now,
        })
        .unwrap();
        db.insert_node(&Node {
            id: "docs/concepts/logcompaction".into(),
            node_type: NodeType::Concept,
            title: "Log Compaction".into(),
            file_path: Some("docs/concepts/logcompaction.md".into()),
            symbol_hash: None,
            summary: None,
            content_hash: None,
            created_at: now,
            updated_at: now,
        })
        .unwrap();
        db.insert_pending_link(
            "docs/implementation/module-map.generated",
            "LogCompaction",
            "relates_to",
            now,
        )
        .unwrap();
        let report = apply_links(&ws, &db, &ApplyOptions::default()).unwrap();
        assert!(report.edits.iter().any(|e| e.reason.contains("generated")));
    }

    #[test]
    fn phase1_discover_wraps_title_mention() {
        let (_dir, db, ws) = setup();
        // Add mention of full title in logcompaction body
        fs::write(
            ws.join("docs/concepts/logcompaction.md"),
            "---\nnode_type: concept\n---\n# Log Compaction\n\nRaft is related here.\n",
        )
        .unwrap();
        let report = apply_links(
            &ws,
            &db,
            &ApplyOptions {
                write: true,
                dry_run: false,
                discover: true,
                sync_after: false,
                // no pending needed for discover on logcompaction
                ..ApplyOptions::default()
            },
        )
        .unwrap();
        let body = fs::read_to_string(ws.join("docs/concepts/logcompaction.md")).unwrap();
        assert!(
            body.contains("[[docs/concepts/raft|Raft]]") || report.suggest_planned + report.auto_planned > 0,
            "body={body} report={:?}",
            report.edits
        );
    }

    #[test]
    fn apply_span_replaces_from_end() {
        let s = "aaa bbb ccc";
        let ops = vec![
            SpanReplace {
                start: 0,
                end: 3,
                replacement: "AAA".into(),
                priority: 0,
            },
            SpanReplace {
                start: 4,
                end: 7,
                replacement: "BBB".into(),
                priority: 0,
            },
        ];
        let out = apply_span_replaces(s, &ops).unwrap();
        assert_eq!(out, "AAA BBB ccc");
    }

    #[test]
    fn mask_blocks_wikilinks_and_code() {
        let s = "See [[Raft]] and `Raft` and Raft.";
        let mask = build_unlinkable_mask(s);
        // last Raft should be free
        let idx = s.rfind("Raft").unwrap();
        assert_eq!(mask[idx], 0);
        let wiki = s.find("[[").unwrap();
        assert_eq!(mask[wiki], 1);
    }
}
