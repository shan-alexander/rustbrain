//! Health checks for a project brain (`rustbrain doctor`).

use crate::error::Result;
use crate::query::PendingLink;
use crate::storage::{Database, SCHEMA_VERSION};
use crate::types::NodeType;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Severity of a doctor finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DoctorSeverity {
    /// Informational.
    Info,
    /// Should be fixed soon.
    Warn,
    /// Broken or unusable state.
    Error,
}

/// One line of doctor output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DoctorFinding {
    /// Severity.
    pub severity: DoctorSeverity,
    /// Stable machine code (e.g. `pending_links`).
    pub code: String,
    /// Human message.
    pub message: String,
}

/// Full doctor report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DoctorReport {
    /// Workspace path examined.
    pub workspace: PathBuf,
    /// Brain directory.
    pub brain_dir: PathBuf,
    /// Whether `db.sqlite` exists.
    pub db_exists: bool,
    /// Whether `graph.mmap` exists.
    pub mmap_exists: bool,
    /// Schema version if readable.
    pub schema_version: Option<u32>,
    /// Total nodes.
    pub nodes: usize,
    /// Total edges.
    pub edges: usize,
    /// FTS rows.
    pub fts_rows: usize,
    /// Pending links.
    pub pending_links: usize,
    /// Symbol anchors.
    pub symbol_anchors: usize,
    /// Notes with no **explicit** edges (auto-links ignored).
    pub orphan_notes: usize,
    /// Breakdown by node type.
    pub by_type: Vec<(String, usize)>,
    /// Pending link samples (up to 50).
    pub pending: Vec<PendingLink>,
    /// Detailed orphan list when [`DoctorOptions::detail_orphans`] is set.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub orphan_details: Vec<crate::autolink::OrphanNote>,
    /// Findings.
    pub findings: Vec<DoctorFinding>,
    /// True when any finding is Error (or pending policy fail).
    pub healthy: bool,
}

/// Options for [`run_doctor_with`].
#[derive(Debug, Clone, Default)]
pub struct DoctorOptions {
    /// Include full orphan list + suggestions in the report.
    pub detail_orphans: bool,
}

/// Run doctor against a workspace (opens DB if present).
///
/// Walks parent directories for `.brain/db.sqlite` (same as [`crate::Brain::open`]).
pub fn run_doctor(workspace: &Path) -> Result<DoctorReport> {
    run_doctor_with(workspace, &DoctorOptions::default())
}

/// Doctor with options (e.g. detailed orphan analysis).
pub fn run_doctor_with(workspace: &Path, opts: &DoctorOptions) -> Result<DoctorReport> {
    let start = workspace
        .canonicalize()
        .unwrap_or_else(|_| workspace.to_path_buf());

    let (workspace, brain_dir) = if let Some(found) = crate::brain::find_brain_dir(&start) {
        found
    } else {
        let brain_dir = start.join(".brain");
        let mut findings = Vec::new();
        findings.push(DoctorFinding {
            severity: DoctorSeverity::Error,
            code: "no_db".into(),
            message: format!(
                "no database at {} or parents — run `rustbrain setup --yes`",
                brain_dir.display()
            ),
        });
        return Ok(DoctorReport {
            workspace: start,
            brain_dir,
            db_exists: false,
            mmap_exists: false,
            schema_version: None,
            nodes: 0,
            edges: 0,
            fts_rows: 0,
            pending_links: 0,
            symbol_anchors: 0,
            orphan_notes: 0,
            by_type: vec![],
            pending: vec![],
            orphan_details: vec![],
            healthy: false,
            findings,
        });
    };

    let db_path = brain_dir.join("db.sqlite");
    let mmap_path = brain_dir.join("graph.mmap");
    let mut findings = Vec::new();
    let mmap_exists = mmap_path.is_file();

    let db = Database::open(&db_path)?;
    let nodes = db.count_nodes()?;
    let edges = db.count_edges()?;
    let fts_rows = db.count_fts_rows()?;
    let pending_links = db.count_pending_links()?;
    let symbol_anchors = db.count_symbol_anchors()?;
    let by_type = db.count_nodes_by_type()?;
    let pending = db.list_pending_links()?;
    let schema_version = Some(SCHEMA_VERSION);

    if !mmap_exists {
        findings.push(DoctorFinding {
            severity: DoctorSeverity::Warn,
            code: "no_mmap".into(),
            message: "graph.mmap missing — run `rustbrain sync` to bake CSR cache".into(),
        });
    }

    if nodes == 0 {
        findings.push(DoctorFinding {
            severity: DoctorSeverity::Warn,
            code: "empty_brain".into(),
            message: "brain has zero nodes — add docs/ notes or run `rustbrain bootstrap --write` then sync"
                .into(),
        });
    }

    let note_count: usize = by_type
        .iter()
        .filter(|(t, _)| t != "symbol")
        .map(|(_, c)| c)
        .sum();
    let symbol_count = by_type
        .iter()
        .find(|(t, _)| t == "symbol")
        .map(|(_, c)| *c)
        .unwrap_or(0);

    if note_count == 0 && symbol_count > 0 {
        findings.push(DoctorFinding {
            severity: DoctorSeverity::Warn,
            code: "symbols_only".into(),
            message: format!(
                "brain has {symbol_count} symbols but no notes — bootstrap or write Markdown under docs/"
            ),
        });
    } else if note_count > 0 && symbol_count > note_count.saturating_mul(20) {
        findings.push(DoctorFinding {
            severity: DoctorSeverity::Info,
            code: "symbol_flood".into(),
            message: format!(
                "symbol/note ratio high ({symbol_count} symbols / {note_count} notes) — default `query` is note-first; use `--with-symbols` when you need code"
            ),
        });
    }

    if pending_links > 0 {
        findings.push(DoctorFinding {
            severity: DoctorSeverity::Warn,
            code: "pending_links".into(),
            message: format!(
                "{pending_links} unresolved WikiLink/symbol refs — see pending list; create stub notes or fix links"
            ),
        });
    }

    if fts_rows != note_count + symbol_count && fts_rows != nodes {
        // Soft check: FTS should track indexed content nodes
        if fts_rows == 0 && nodes > 0 {
            findings.push(DoctorFinding {
                severity: DoctorSeverity::Error,
                code: "fts_empty".into(),
                message: "nodes exist but FTS is empty — re-run `rustbrain sync`".into(),
            });
        }
    }

    if !workspace.join("docs").is_dir() {
        findings.push(DoctorFinding {
            severity: DoctorSeverity::Info,
            code: "no_docs_dir".into(),
            message: "no docs/ directory — `rustbrain bootstrap --write` can scaffold one".into(),
        });
    }

    let ignore = workspace.join(".rustbrainignore");
    if !ignore.is_file() {
        findings.push(DoctorFinding {
            severity: DoctorSeverity::Info,
            code: "no_ignore".into(),
            message: "no .rustbrainignore — bootstrap can create one from .gitignore defaults"
                .into(),
        });
    }

    // --- README / harvest / knowledge density (info-level; never invent content) ---
    assess_readme_and_harvest(&workspace, &db, &mut findings)?;
    assess_knowledge_density(&workspace, &db, note_count, symbol_count, &mut findings)?;

    let has_goal = by_type.iter().any(|(t, c)| t == NodeType::Goal.as_str() && *c > 0);
    if nodes > 0 && !has_goal {
        findings.push(DoctorFinding {
            severity: DoctorSeverity::Info,
            code: "no_goals".into(),
            message: "no goal nodes yet — consider docs/goals/ or bootstrap from README".into(),
        });
    }

    // ADR quality: TEMPLATE alone is not a real decision record.
    let adr_ids = db.list_node_ids_by_type(NodeType::Adr.as_str())?;
    let real_adrs = adr_ids
        .iter()
        .filter(|id| !id.to_lowercase().contains("template"))
        .count();
    if !adr_ids.is_empty() && real_adrs == 0 {
        findings.push(DoctorFinding {
            severity: DoctorSeverity::Info,
            code: "adr_template_only".into(),
            message: "only ADR template present — write real decisions under docs/adr/ (or `note new --type adr`)"
                .into(),
        });
    } else if adr_ids.is_empty() && note_count > 0 {
        findings.push(DoctorFinding {
            severity: DoctorSeverity::Info,
            code: "no_adrs".into(),
            message: "no ADR notes yet — capture decisions with `rustbrain note new --type adr --title … --note …`"
                .into(),
        });
    }

    if !workspace.join("AGENTS.md").is_file() && nodes > 0 {
        findings.push(DoctorFinding {
            severity: DoctorSeverity::Info,
            code: "no_agents_md".into(),
            message: "no AGENTS.md — `rustbrain bootstrap --yes --write` can add the agent cookbook (or --no-agents-md to skip intentionally)"
                .into(),
        });
    }

    let orphan_list = crate::autolink::list_orphan_notes(&db)?;
    let orphan_notes = orphan_list.len();
    if orphan_notes > 0 {
        findings.push(DoctorFinding {
            severity: DoctorSeverity::Info,
            code: "orphan_notes".into(),
            message: format!(
                "{orphan_notes} orphan note(s) (no explicit links) — run `rustbrain doctor --orphans` for details, or `rustbrain links --auto` to create soft auto-links"
            ),
        });
    }

    let orphan_details = if opts.detail_orphans {
        orphan_list
    } else {
        vec![]
    };

    let healthy = !findings
        .iter()
        .any(|f| f.severity == DoctorSeverity::Error);

    // "ok" only when nothing else to report (knowledge infos replace a bare ok).
    if healthy && findings.is_empty() {
        findings.push(DoctorFinding {
            severity: DoctorSeverity::Info,
            code: "ok".into(),
            message: "brain looks healthy".into(),
        });
    }

    Ok(DoctorReport {
        workspace,
        brain_dir,
        db_exists: true,
        mmap_exists,
        schema_version,
        nodes,
        edges,
        fts_rows,
        pending_links,
        symbol_anchors,
        orphan_notes,
        by_type,
        pending,
        orphan_details,
        findings,
        healthy,
    })
}

/// Rough non-whitespace content length (chars).
fn content_mass(s: &str) -> usize {
    s.chars().filter(|c| !c.is_whitespace()).count()
}

/// Body-ish mass: strip common YAML frontmatter then count.
fn body_mass(s: &str) -> usize {
    let t = s.trim_start();
    let body = if let Some(rest) = t.strip_prefix("---") {
        if let Some(idx) = rest.find("\n---") {
            rest[idx + 4..].trim()
        } else {
            t
        }
    } else {
        t
    };
    content_mass(body)
}

fn file_body_mass(path: &Path) -> Option<usize> {
    let text = std::fs::read_to_string(path).ok()?;
    Some(body_mass(&text))
}

/// README / from-readme harvest quality (informational only).
fn assess_readme_and_harvest(
    workspace: &Path,
    db: &Database,
    findings: &mut Vec<DoctorFinding>,
) -> Result<()> {
    let readme_path = workspace.join("README.md");
    let from_readme_path = workspace.join("docs/goals/from-readme.md");

    if !readme_path.is_file() {
        findings.push(DoctorFinding {
            severity: DoctorSeverity::Info,
            code: "no_readme".into(),
            message: "no root README.md — bootstrap cannot harvest goals; context relies on docs/ notes, module map, and symbols. Optional: add a short README then `bootstrap --force` + sync"
                .into(),
        });
        if !from_readme_path.is_file() {
            findings.push(DoctorFinding {
                severity: DoctorSeverity::Info,
                code: "no_from_readme".into(),
                message: "no docs/goals/from-readme.md (expected without README) — write goals under docs/goals/ or add README.md"
                    .into(),
            });
        }
        return Ok(());
    }

    // README present
    let readme_text = std::fs::read_to_string(&readme_path).unwrap_or_default();
    let readme_mass = body_mass(&readme_text);
    let content_lines = readme_text
        .lines()
        .filter(|l| {
            let t = l.trim();
            !t.is_empty() && !t.starts_with('#')
        })
        .count();

    // Sparse: very little prose (not a hard error — many valid tiny READMEs exist).
    const SPARSE_MASS: usize = 120;
    const SPARSE_LINES: usize = 3;
    if readme_mass < SPARSE_MASS || content_lines < SPARSE_LINES {
        findings.push(DoctorFinding {
            severity: DoctorSeverity::Info,
            code: "sparse_readme".into(),
            message: format!(
                "README.md is thin (~{readme_mass} non-ws chars, {content_lines} content lines) — from-readme harvest will mirror that; enrich README or write real docs/goals and ADRs for better context"
            ),
        });
    }

    if db.get_node("readme")?.is_none() {
        findings.push(DoctorFinding {
            severity: DoctorSeverity::Info,
            code: "readme_not_indexed".into(),
            message: "README.md exists but hub node `readme` missing — run `rustbrain sync`".into(),
        });
    }

    if from_readme_path.is_file() {
        let mass = file_body_mass(&from_readme_path).unwrap_or(0);
        // Subtract typical boilerplate header roughly by threshold on body.
        if mass < 180 {
            findings.push(DoctorFinding {
                severity: DoctorSeverity::Info,
                code: "thin_from_readme".into(),
                message: format!(
                    "docs/goals/from-readme.md is thin (~{mass} non-ws chars) — harvest only copies README sections; expand README (Why/Features) or add hand-written goals/ADRs"
                ),
            });
        }
    } else if readme_mass >= SPARSE_MASS {
        findings.push(DoctorFinding {
            severity: DoctorSeverity::Info,
            code: "no_from_readme".into(),
            message: "README.md present but no docs/goals/from-readme.md — run `rustbrain bootstrap --yes --write` (or --force) then sync"
                .into(),
        });
    }

    Ok(())
}

/// Detect bootstrap-scaffold-only brains vs hand-written knowledge.
fn assess_knowledge_density(
    workspace: &Path,
    db: &Database,
    note_count: usize,
    symbol_count: usize,
    findings: &mut Vec<DoctorFinding>,
) -> Result<()> {
    if note_count == 0 {
        return Ok(());
    }

    // Collect note ids that look like hand-written project knowledge.
    let mut substantive = 0usize;
    let mut scaffoldish = 0usize;

    for ty in [
        NodeType::Goal,
        NodeType::Adr,
        NodeType::Concept,
        NodeType::Analysis,
        NodeType::EdgeCase,
        NodeType::Reference,
        NodeType::Alternative,
    ] {
        let ids = db.list_node_ids_by_type(ty.as_str())?;
        for id in ids {
            if is_hard_scaffold_id(&id) {
                scaffoldish += 1;
                continue;
            }
            let mass = note_body_mass(workspace, db, &id)?;
            // Rich README harvest counts as useful knowledge (still generated, but real prose).
            if id_is_from_readme(&id) {
                if mass >= 200 {
                    substantive += 1;
                } else {
                    scaffoldish += 1;
                }
                continue;
            }
            if mass >= 80 {
                substantive += 1;
            } else {
                scaffoldish += 1;
            }
        }
    }

    if substantive == 0 && (scaffoldish > 0 || symbol_count > 0) {
        findings.push(DoctorFinding {
            severity: DoctorSeverity::Info,
            code: "scaffold_only".into(),
            message: format!(
                "notes look bootstrap-scaffold only (stubs/templates/thin harvest); no substantial goals/ADRs/concepts yet — use `note new` or edit docs/; symbols={symbol_count}"
            ),
        });
    } else if substantive > 0 && symbol_count > substantive.saturating_mul(30) {
        findings.push(DoctorFinding {
            severity: DoctorSeverity::Info,
            code: "knowledge_thin".into(),
            message: format!(
                "few substantial notes ({substantive}) vs many symbols ({symbol_count}) — context for *why* may be thin; capture decisions as ADRs"
            ),
        });
    }

    let _ = scaffoldish;
    Ok(())
}

fn note_body_mass(workspace: &Path, db: &Database, id: &str) -> Result<usize> {
    if let Some(c) = db.get_fts_content(id)? {
        let m = body_mass(&c);
        if m > 0 {
            return Ok(m);
        }
    }
    if let Some(node) = db.get_node(id)? {
        if let Some(path) = node.file_path.as_ref() {
            return Ok(file_body_mass(&workspace.join(path)).unwrap_or(0));
        }
    }
    Ok(0)
}

fn id_is_from_readme(id: &str) -> bool {
    let id = id.to_lowercase();
    id == "docs/goals/from-readme" || id.contains("from-readme")
}

/// Pure scaffold: never counts as project knowledge by itself.
fn is_hard_scaffold_id(id: &str) -> bool {
    let id = id.to_lowercase();
    id.contains("template")
        || id == "docs/goals/readme"
        || id.ends_with("bootstrap_checklist")
        || id.contains("bootstrap-checklist")
        || id.contains("bootstrap_checklist")
        || id.contains("module-map.generated")
        || id == "agents"
        || id.ends_with("/agents")
        || id == "readme" // root hub is inventory, not a decision record
}

impl DoctorReport {
    /// Render a human-readable multi-line report.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("rustbrain doctor — {}\n", self.workspace.display()));
        out.push_str(&format!(
            "  db: {}  mmap: {}  schema: {}\n",
            if self.db_exists { "yes" } else { "NO" },
            if self.mmap_exists { "yes" } else { "no" },
            self.schema_version
                .map(|v| v.to_string())
                .unwrap_or_else(|| "-".into())
        ));
        out.push_str(&format!(
            "  nodes={} edges={} fts={} pending={} symbols={}",
            self.nodes, self.edges, self.fts_rows, self.pending_links, self.symbol_anchors
        ));
        if self.orphan_notes > 0 {
            out.push_str(&format!(" orphans={}", self.orphan_notes));
        }
        out.push('\n');
        if !self.by_type.is_empty() {
            out.push_str("  by type: ");
            let parts: Vec<_> = self
                .by_type
                .iter()
                .map(|(t, c)| format!("{t}={c}"))
                .collect();
            out.push_str(&parts.join(" "));
            out.push('\n');
        }
        out.push_str("\nfindings:\n");
        for f in &self.findings {
            let tag = match f.severity {
                DoctorSeverity::Info => "info",
                DoctorSeverity::Warn => "WARN",
                DoctorSeverity::Error => "ERR ",
            };
            out.push_str(&format!("  [{tag}] {}: {}\n", f.code, f.message));
        }
        if !self.orphan_details.is_empty() {
            out.push_str("\norphan notes (no explicit WikiLink/symbol edges; auto-links ignored):\n");
            for o in &self.orphan_details {
                let path = o.file_path.as_deref().unwrap_or("-");
                out.push_str(&format!(
                    "  • [{}] {} (id: {})\n    path: {}\n",
                    o.node_type, o.title, o.id, path
                ));
                if o.suggestions.is_empty() {
                    out.push_str("    suggestions: (none — add tags, matching filenames, or WikiLinks)\n");
                } else {
                    out.push_str("    suggestions:\n");
                    for s in o.suggestions.iter().take(8) {
                        let tp = s.target_path.as_deref().unwrap_or("-");
                        out.push_str(&format!(
                            "      - [{} w={:.2}] {} → {} ({})\n",
                            s.relation_type, s.weight, s.reason, s.target_id, tp
                        ));
                    }
                }
            }
            out.push_str(
                "\n  apply soft links: `rustbrain links --auto`\n  one file: `rustbrain links --auto docs/goals/foo.md`\n",
            );
        }
        if !self.pending.is_empty() {
            out.push_str("\npending links (up to 50):\n");
            for p in self.pending.iter().take(50) {
                out.push_str(&format!(
                    "  {} -[{}]-> {}\n",
                    p.source_id, p.relation_type, p.raw_target
                ));
            }
        }
        out.push_str(&format!(
            "\nstatus: {}\n",
            if self.healthy { "OK" } else { "UNHEALTHY" }
        ));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brain::Brain;
    use tempfile::tempdir;

    #[test]
    fn doctor_empty_workspace() {
        let dir = tempdir().unwrap();
        let report = run_doctor(dir.path()).unwrap();
        assert!(!report.healthy);
        assert!(report.findings.iter().any(|f| f.code == "no_db"));
    }

    #[test]
    fn doctor_after_init_sync() {
        let dir = tempdir().unwrap();
        let docs = dir.path().join("docs");
        std::fs::create_dir_all(&docs).unwrap();
        std::fs::write(
            docs.join("a.md"),
            "---\nnode_type: concept\n---\n# A\nHello.\n",
        )
        .unwrap();
        let mut brain = Brain::create(dir.path()).unwrap();
        brain.sync().unwrap();
        let report = run_doctor(dir.path()).unwrap();
        assert!(report.db_exists);
        assert!(report.nodes >= 1);
        assert!(report.healthy || report.findings.iter().all(|f| f.severity != DoctorSeverity::Error));
    }

    #[test]
    fn doctor_walks_parent_for_brain() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let sub = root.join("src").join("nested");
        std::fs::create_dir_all(&sub).unwrap();
        let mut brain = Brain::create(root).unwrap();
        brain.sync().unwrap();
        let report = run_doctor(&sub).unwrap();
        assert!(report.db_exists, "doctor should find parent .brain");
        assert_eq!(
            report.workspace.canonicalize().unwrap(),
            root.canonicalize().unwrap()
        );
    }

    #[test]
    fn doctor_reports_no_readme() {
        let dir = tempdir().unwrap();
        let mut brain = Brain::create(dir.path()).unwrap();
        brain.sync().unwrap();
        let report = run_doctor(dir.path()).unwrap();
        assert!(
            report.findings.iter().any(|f| f.code == "no_readme"),
            "findings={:?}",
            report.findings
        );
        assert!(report.healthy);
    }

    #[test]
    fn doctor_reports_sparse_readme() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("README.md"), "# x\n\ntodo\n").unwrap();
        let mut brain = Brain::create(dir.path()).unwrap();
        brain.sync().unwrap();
        let report = run_doctor(dir.path()).unwrap();
        assert!(
            report.findings.iter().any(|f| f.code == "sparse_readme"),
            "findings={:?}",
            report.findings
        );
    }

    #[test]
    fn doctor_counts_orphans_and_details() {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("docs/concepts")).unwrap();
        std::fs::write(
            dir.path().join("docs/concepts/lonely.md"),
            "---\nnode_type: concept\n---\n# Lonely\n\nNo links here.\n",
        )
        .unwrap();
        let mut brain = Brain::create(dir.path()).unwrap();
        brain.sync().unwrap();
        let report = run_doctor(dir.path()).unwrap();
        assert!(report.orphan_notes >= 1, "orphan_notes={}", report.orphan_notes);
        assert!(report.findings.iter().any(|f| f.code == "orphan_notes"));

        let detailed = run_doctor_with(
            dir.path(),
            &DoctorOptions {
                detail_orphans: true,
            },
        )
        .unwrap();
        assert!(!detailed.orphan_details.is_empty());
        assert!(detailed
            .orphan_details
            .iter()
            .any(|o| o.id.contains("lonely")));
    }

    #[test]
    fn doctor_scaffold_only_after_empty_bootstrap_ish() {
        let dir = tempdir().unwrap();
        // Minimal notes that look like stubs
        std::fs::create_dir_all(dir.path().join("docs/adr")).unwrap();
        std::fs::write(
            dir.path().join("docs/adr/TEMPLATE.md"),
            "---\nnode_type: adr\n---\n# ADR template\n\nProposed\n",
        )
        .unwrap();
        let mut brain = Brain::create(dir.path()).unwrap();
        brain.sync().unwrap();
        let report = run_doctor(dir.path()).unwrap();
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.code == "scaffold_only" || f.code == "adr_template_only"),
            "findings={:?}",
            report.findings
        );
    }
}
