//! Recognized **project hub** Markdown files at the workspace root.
//!
//! Rust / open-source norms:
//! - [`README.md`](https://doc.rust-lang.org/cargo/guide/project-layout.html) — project front door
//! - [`CHANGELOG.md`](https://keepachangelog.com/) — Keep a Changelog + SemVer ship history
//!
//! **Optional** planning surfaces (agent HITL — never required for a healthy brain):
//! - `ROADMAP.md`, `BACKLOG.md` — priorities / unshipped work
//!
//! These are indexed under **stable node ids** (`readme`, `changelog`, …) so agents
//! can query and context-pack them reliably when the files exist. Absence is fine:
//! doctor tips are info-level only. Rustbrain never invents changelog entries or
//! plan tasks — it only densifies what is already on disk after `sync`.

use std::path::{Component, Path};

/// Stable node id for the root README hub.
pub const HUB_README: &str = "readme";
/// Stable node id for the root CHANGELOG hub (Keep a Changelog).
pub const HUB_CHANGELOG: &str = "changelog";
/// Stable node id for an optional root ROADMAP.
pub const HUB_ROADMAP: &str = "roadmap";
/// Stable node id for an optional root BACKLOG.
pub const HUB_BACKLOG: &str = "backlog";

/// Which hub (if any) a workspace-relative path maps to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectHub {
    /// Root README.md
    Readme,
    /// Root CHANGELOG.md / CHANGES.md / HISTORY.md
    Changelog,
    /// Root ROADMAP.md
    Roadmap,
    /// Root BACKLOG.md
    Backlog,
}

impl ProjectHub {
    /// Stable graph / FTS node id.
    pub fn node_id(self) -> &'static str {
        match self {
            Self::Readme => HUB_README,
            Self::Changelog => HUB_CHANGELOG,
            Self::Roadmap => HUB_ROADMAP,
            Self::Backlog => HUB_BACKLOG,
        }
    }

    /// Default display title when the file has no H1.
    pub fn default_title(self) -> &'static str {
        match self {
            Self::Readme => "README",
            Self::Changelog => "Changelog",
            Self::Roadmap => "Roadmap",
            Self::Backlog => "Backlog",
        }
    }

    /// Extra resolution aliases (lowercased by alias storage).
    pub fn aliases(self) -> &'static [&'static str] {
        match self {
            Self::Readme => &["readme", "hub", "home"],
            Self::Changelog => &[
                "changelog",
                "change-log",
                "changes",
                "history",
                "releases",
                "release-notes",
                "keepachangelog",
                "semver",
                "versions",
                "unreleased",
            ],
            Self::Roadmap => &["roadmap", "milestones", "plan", "future"],
            Self::Backlog => &["backlog", "todo", "work-queue", "kanban"],
        }
    }
}

/// Detect a root-level project hub from a workspace-relative path.
pub fn detect_project_hub(rel: &Path) -> Option<ProjectHub> {
    let comps: Vec<_> = rel
        .components()
        .filter(|c| matches!(c, Component::Normal(_)))
        .collect();
    if comps.len() != 1 {
        return None;
    }
    let name = rel.file_name()?.to_str()?;
    let lower = name.to_ascii_lowercase();
    match lower.as_str() {
        "readme.md" => Some(ProjectHub::Readme),
        "changelog.md" | "changes.md" | "history.md" => Some(ProjectHub::Changelog),
        "roadmap.md" => Some(ProjectHub::Roadmap),
        "backlog.md" => Some(ProjectHub::Backlog),
        _ => None,
    }
}

/// True when `id` is a recognized project hub node.
pub fn is_hub_node_id(id: &str) -> bool {
    matches!(
        id,
        HUB_README | HUB_CHANGELOG | HUB_ROADMAP | HUB_BACKLOG
    )
}

/// True for release / history / version-oriented agent prompts.
pub fn is_release_intent(tokens: &[String]) -> bool {
    tokens.iter().any(|t| {
        matches!(
            t.as_str(),
            "changelog"
                | "changelogs"
                | "release"
                | "releases"
                | "released"
                | "version"
                | "versions"
                | "semver"
                | "shipped"
                | "unreleased"
                | "history"
                | "breaking"
                | "migration"
                | "upgrade"
                | "v1"
                | "v2"
                | "v3"
        ) || (t.starts_with('v')
            && t.len() >= 2
            && t[1..].chars().next().is_some_and(|c| c.is_ascii_digit()))
            || (t.chars().all(|c| c.is_ascii_digit() || c == '.')
                && t.contains('.')
                && t.len() >= 3)
    })
}

/// True for planning / prioritization agent prompts (roadmap, backlog, kanban-ish).
pub fn is_planning_intent(tokens: &[String]) -> bool {
    tokens.iter().any(|t| {
        matches!(
            t.as_str(),
            "roadmap"
                | "backlog"
                | "priority"
                | "priorities"
                | "prioritize"
                | "epic"
                | "epics"
                | "story"
                | "stories"
                | "sprint"
                | "kanban"
                | "milestone"
                | "milestones"
                | "todo"
                | "todos"
                | "status"
                | "plan"
                | "planning"
                | "next"
        )
    })
}

/// Keep a Changelog: first `## […]` heading as a short summary (latest section).
pub fn changelog_latest_heading(body: &str) -> Option<String> {
    for line in body.lines() {
        let t = line.trim();
        if t.starts_with("## [") || t.starts_with("##[") {
            let s = t.trim_start_matches('#').trim();
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
        // Also accept ## Unreleased
        if t.eq_ignore_ascii_case("## unreleased") || t.eq_ignore_ascii_case("## [unreleased]") {
            return Some(t.trim_start_matches('#').trim().to_string());
        }
    }
    None
}

/// Version labels from Keep a Changelog headings, newest first, capped.
pub fn changelog_version_aliases(body: &str, max: usize) -> Vec<String> {
    let mut out = Vec::new();
    for line in body.lines() {
        let t = line.trim();
        let rest = if let Some(r) = t.strip_prefix("## [") {
            r
        } else if let Some(r) = t.strip_prefix("##[") {
            r
        } else {
            continue;
        };
        let ver = rest.split(']').next().unwrap_or("").trim();
        if ver.is_empty() || ver.eq_ignore_ascii_case("unreleased") {
            continue;
        }
        // Skip dates accidentally captured
        if ver.len() >= 3 && !out.iter().any(|x: &String| x.eq_ignore_ascii_case(ver)) {
            out.push(ver.to_string());
        }
        if out.len() >= max {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn detects_root_hubs() {
        assert_eq!(
            detect_project_hub(Path::new("CHANGELOG.md")),
            Some(ProjectHub::Changelog)
        );
        assert_eq!(
            detect_project_hub(Path::new("changelog.md")),
            Some(ProjectHub::Changelog)
        );
        assert_eq!(
            detect_project_hub(Path::new("HISTORY.md")),
            Some(ProjectHub::Changelog)
        );
        assert_eq!(
            detect_project_hub(Path::new("README.md")),
            Some(ProjectHub::Readme)
        );
        assert_eq!(
            detect_project_hub(Path::new("ROADMAP.md")),
            Some(ProjectHub::Roadmap)
        );
        assert_eq!(detect_project_hub(Path::new("docs/CHANGELOG.md")), None);
    }

    #[test]
    fn parses_keep_a_changelog_heading() {
        let body = "# Changelog\n\n## [0.3.14] - 2026-07-31\n\n### Added\n- foo\n";
        assert_eq!(
            changelog_latest_heading(body).as_deref(),
            Some("[0.3.14] - 2026-07-31")
        );
        assert_eq!(
            changelog_version_aliases(body, 3),
            vec!["0.3.14".to_string()]
        );
    }

    #[test]
    fn release_intent_tokens() {
        assert!(is_release_intent(&[
            "what".into(),
            "shipped".into(),
            "0.3.14".into()
        ]));
        assert!(is_release_intent(&["changelog".into()]));
        assert!(!is_release_intent(&["raft".into(), "consensus".into()]));
    }
}
