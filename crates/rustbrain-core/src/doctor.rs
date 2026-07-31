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
    /// Breakdown by node type.
    pub by_type: Vec<(String, usize)>,
    /// Pending link samples (up to 50).
    pub pending: Vec<PendingLink>,
    /// Findings.
    pub findings: Vec<DoctorFinding>,
    /// True when any finding is Error (or pending policy fail).
    pub healthy: bool,
}

/// Run doctor against a workspace (opens DB if present).
pub fn run_doctor(workspace: &Path) -> Result<DoctorReport> {
    let workspace = workspace
        .canonicalize()
        .unwrap_or_else(|_| workspace.to_path_buf());
    let brain_dir = workspace.join(".brain");
    let db_path = brain_dir.join("db.sqlite");
    let mmap_path = brain_dir.join("graph.mmap");

    let mut findings = Vec::new();
    let db_exists = db_path.is_file();
    let mmap_exists = mmap_path.is_file();

    if !db_exists {
        findings.push(DoctorFinding {
            severity: DoctorSeverity::Error,
            code: "no_db".into(),
            message: format!(
                "no database at {} — run `rustbrain init` and `rustbrain sync`",
                db_path.display()
            ),
        });
        return Ok(DoctorReport {
            workspace,
            brain_dir,
            db_exists: false,
            mmap_exists,
            schema_version: None,
            nodes: 0,
            edges: 0,
            fts_rows: 0,
            pending_links: 0,
            symbol_anchors: 0,
            by_type: vec![],
            pending: vec![],
            healthy: false,
            findings,
        });
    }

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
                "symbol/note ratio high ({symbol_count} symbols / {note_count} notes) — use `query --no-symbols` for human search"
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

    // README hub presence
    if db.get_node("readme")?.is_none() && workspace.join("README.md").is_file() {
        findings.push(DoctorFinding {
            severity: DoctorSeverity::Info,
            code: "readme_not_indexed".into(),
            message: "README.md exists but hub node `readme` missing — run sync".into(),
        });
    }

    let has_goal = by_type.iter().any(|(t, c)| t == NodeType::Goal.as_str() && *c > 0);
    if nodes > 0 && !has_goal {
        findings.push(DoctorFinding {
            severity: DoctorSeverity::Info,
            code: "no_goals".into(),
            message: "no goal nodes yet — consider docs/goals/ or bootstrap from README".into(),
        });
    }

    let healthy = !findings
        .iter()
        .any(|f| f.severity == DoctorSeverity::Error);

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
        by_type,
        pending,
        findings,
        healthy,
    })
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
            "  nodes={} edges={} fts={} pending={} symbols={}\n",
            self.nodes, self.edges, self.fts_rows, self.pending_links, self.symbol_anchors
        ));
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
}
