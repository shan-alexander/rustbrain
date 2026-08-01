//! Algorithmic **plan / task status** densification for FTS and agent summaries.
//!
//! Plans are ordinary Markdown. Rustbrain does **not** require a kanban product.
//! On index, it extracts a compact status vocabulary so agents can query and pack
//! status without reading whole roadmaps:
//!
//! | Canonical | Accepted surface forms |
//! |-----------|------------------------|
//! | `backlog` | backlog, todo, open, pending, queued |
//! | `in_progress` | in_progress, in-progress, inprogress, wip, doing, active |
//! | `qa` | qa, review, in-review, testing, verification |
//! | `done` | done, complete, completed, finished, closed, shipped (task-level) |
//! | `cancelled` | cancelled, canceled, wontfix, dropped, obsolete |
//! | `undone` | undone, reopen, reopened, blocked, stuck |
//!
//! Sources (first wins for *overall* status when explicit):
//! 1. YAML frontmatter `status:` / `state:`
//! 2. `## Status` section body (first status token)
//! 3. Section headings (`## In Progress`, `## Done`, …)
//! 4. Checkbox lines (`- [ ]`, `- [x]`, `- [/]`, `- [~]`)
//!
//! Dense FTS tokens use stable prefixes (`status:…`, `task:status:slug`) so they
//! survive stopword stripping and stay machine-readable.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Canonical task / plan lifecycle status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    /// Not started / queue.
    Backlog,
    /// Actively being worked.
    InProgress,
    /// Review / QA / testing.
    Qa,
    /// Finished successfully.
    Done,
    /// Explicitly abandoned.
    Cancelled,
    /// Reopened, blocked, or otherwise not done after work started.
    Undone,
}

impl PlanStatus {
    /// Stable token used in FTS and summaries.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Backlog => "backlog",
            Self::InProgress => "in_progress",
            Self::Qa => "qa",
            Self::Done => "done",
            Self::Cancelled => "cancelled",
            Self::Undone => "undone",
        }
    }

    /// Parse free-form status text into a canonical value.
    pub fn parse(s: &str) -> Option<Self> {
        let t = s
            .trim()
            .trim_matches(|c: char| c == '"' || c == '\'' || c == '`')
            .to_ascii_lowercase()
            .replace('-', "_")
            .replace(' ', "_");
        match t.as_str() {
            "backlog" | "todo" | "to_do" | "open" | "pending" | "queued" | "queue" | "later"
            | "icebox" => Some(Self::Backlog),
            "in_progress" | "inprogress" | "wip" | "doing" | "active" | "started" | "working"
            | "progress" => Some(Self::InProgress),
            "qa" | "review" | "in_review" | "testing" | "test" | "verification" | "verify"
            | "code_review" | "pr_review" => Some(Self::Qa),
            "done" | "complete" | "completed" | "finished" | "closed" | "shipped" | "resolved"
            | "fixed" | "merged" => Some(Self::Done),
            "cancelled" | "canceled" | "wontfix" | "wont_fix" | "dropped" | "obsolete"
            | "abandoned" | "declined" => Some(Self::Cancelled),
            "undone" | "reopen" | "reopened" | "blocked" | "stuck" | "on_hold" | "paused"
            | "deferred" => Some(Self::Undone),
            _ => None,
        }
    }
}

/// One extracted checklist / task line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanTask {
    /// Canonical status.
    pub status: PlanStatus,
    /// Short task text (truncated).
    pub text: String,
}

/// Aggregated densification for a plan note body (+ optional frontmatter status).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanStatusDigest {
    /// Overall plan status when detectable.
    pub overall: Option<PlanStatus>,
    /// Counts by status (from tasks + section bulk).
    pub counts: BTreeMap<String, usize>,
    /// Individual tasks (capped).
    pub tasks: Vec<PlanTask>,
    /// Open = backlog + in_progress + qa + undone.
    pub open: usize,
    /// Done count.
    pub done: usize,
    /// Cancelled count.
    pub cancelled: usize,
}

impl PlanStatusDigest {
    /// Dense one-line summary for `Node.summary` / agent skimming.
    pub fn summary_line(&self) -> String {
        let overall = self
            .overall
            .map(|s| s.as_str())
            .unwrap_or("unspecified");
        format!(
            "plan status={overall} · open {} · done {} · cancelled {}",
            self.open, self.done, self.cancelled
        )
    }

    /// Tokens appended to FTS body (information-dense, stopword-resistant).
    pub fn fts_block(&self) -> String {
        let mut parts = Vec::new();
        parts.push("[plan-status]".to_string());
        if let Some(o) = self.overall {
            parts.push(format!("status:{}", o.as_str()));
            parts.push(format!("plan_status:{}", o.as_str()));
        }
        parts.push(format!("plan_open:{}", self.open));
        parts.push(format!("plan_done:{}", self.done));
        parts.push(format!("plan_cancelled:{}", self.cancelled));
        for (k, n) in &self.counts {
            if *n > 0 {
                parts.push(format!("count:{k}:{n}"));
                // Repeat status tokens for soft ranking weight (bounded).
                for _ in 0..(*n).min(5) {
                    parts.push(format!("status:{k}"));
                }
            }
        }
        for t in self.tasks.iter().take(40) {
            let slug = slug_token(&t.text);
            if !slug.is_empty() {
                parts.push(format!("task:{}:{}", t.status.as_str(), slug));
            }
            parts.push(format!("status:{}", t.status.as_str()));
        }
        parts.join(" ")
    }

    /// True when we found any signal (worth densifying).
    pub fn has_signal(&self) -> bool {
        self.overall.is_some() || !self.tasks.is_empty() || self.counts.values().any(|n| *n > 0)
    }
}

/// Build a digest from optional frontmatter status string + markdown body.
pub fn densify_plan(frontmatter_status: Option<&str>, body: &str) -> PlanStatusDigest {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut tasks: Vec<PlanTask> = Vec::new();
    let mut overall = frontmatter_status.and_then(PlanStatus::parse);

    // ## Status section: first non-empty line / first status token
    if overall.is_none() {
        if let Some(sec) = section_body(body, "status") {
            overall = first_status_in_text(&sec);
        }
    }

    // Headings as section-scoped status for following checkboxes
    let mut section_status: Option<PlanStatus> = None;
    for line in body.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix('#') {
            let heading = rest.trim_start_matches('#').trim();
            let h = heading.to_ascii_lowercase();
            // "## In Progress" / "## Done" / "## Backlog" / "## QA"
            section_status = PlanStatus::parse(&h).or_else(|| {
                // heading may be longer: "In Progress (this week)"
                h.split_whitespace()
                    .find_map(|w| PlanStatus::parse(w))
                    .or_else(|| {
                        for key in [
                            "in_progress",
                            "in progress",
                            "backlog",
                            "done",
                            "qa",
                            "review",
                            "cancelled",
                            "canceled",
                            "todo",
                            "blocked",
                        ] {
                            if h.contains(key) {
                                return PlanStatus::parse(key);
                            }
                        }
                        None
                    })
            });
            continue;
        }

        if let Some((st, text)) = parse_checkbox_line(t) {
            let status = st.or(section_status).unwrap_or(PlanStatus::Backlog);
            *counts.entry(status.as_str().to_string()).or_default() += 1;
            if tasks.len() < 64 {
                tasks.push(PlanTask {
                    status,
                    text: truncate(&text, 80),
                });
            }
            continue;
        }

        // Bullet with leading status tag: "- in_progress: foo" or "- [done] foo"
        if let Some(rest) = t.strip_prefix('-').or_else(|| t.strip_prefix('*')) {
            let rest = rest.trim();
            if let Some((st, text)) = parse_tagged_bullet(rest) {
                *counts.entry(st.as_str().to_string()).or_default() += 1;
                if tasks.len() < 64 {
                    tasks.push(PlanTask {
                        status: st,
                        text: truncate(&text, 80),
                    });
                }
            }
        }
    }

    // If still no overall, infer from task mix.
    if overall.is_none() && !tasks.is_empty() {
        overall = Some(infer_overall(&counts));
    }

    let done = *counts.get("done").unwrap_or(&0);
    let cancelled = *counts.get("cancelled").unwrap_or(&0);
    let open = counts
        .iter()
        .filter(|(k, _)| matches!(k.as_str(), "backlog" | "in_progress" | "qa" | "undone"))
        .map(|(_, n)| *n)
        .sum();

    PlanStatusDigest {
        overall,
        counts,
        tasks,
        open,
        done,
        cancelled,
    }
}

fn infer_overall(counts: &BTreeMap<String, usize>) -> PlanStatus {
    let get = |k: &str| *counts.get(k).unwrap_or(&0);
    if get("in_progress") > 0 {
        return PlanStatus::InProgress;
    }
    if get("qa") > 0 {
        return PlanStatus::Qa;
    }
    if get("undone") > 0 {
        return PlanStatus::Undone;
    }
    if get("backlog") > 0 {
        return PlanStatus::Backlog;
    }
    if get("done") > 0 && get("cancelled") == 0 {
        return PlanStatus::Done;
    }
    if get("cancelled") > 0 && get("done") == 0 && get("backlog") == 0 {
        return PlanStatus::Cancelled;
    }
    PlanStatus::Backlog
}

fn parse_checkbox_line(t: &str) -> Option<(Option<PlanStatus>, String)> {
    let t = t.strip_prefix('-').or_else(|| t.strip_prefix('*'))?.trim();
    // - [ ] text  / - [x] text / - [/] / - [~]
    if !t.starts_with('[') {
        return None;
    }
    let close = t.find(']')?;
    let mark = t[1..close].trim();
    let text = t[close + 1..].trim().to_string();
    if text.is_empty() {
        return None;
    }
    let st = match mark.to_ascii_lowercase().as_str() {
        "" | " " => Some(PlanStatus::Backlog),
        "x" => Some(PlanStatus::Done),
        "/" | ">" | "o" => Some(PlanStatus::InProgress),
        "~" | "-" => Some(PlanStatus::Cancelled),
        "?" => Some(PlanStatus::Qa),
        "!" => Some(PlanStatus::Undone),
        other => PlanStatus::parse(other),
    };
    Some((st, text))
}

fn parse_tagged_bullet(rest: &str) -> Option<(PlanStatus, String)> {
    // [done] text
    if let Some(inner) = rest.strip_prefix('[') {
        if let Some(end) = inner.find(']') {
            let tag = &inner[..end];
            let text = inner[end + 1..].trim();
            if let Some(st) = PlanStatus::parse(tag) {
                if !text.is_empty() {
                    return Some((st, text.to_string()));
                }
            }
        }
    }
    // status: text  or status - text
    for sep in [':', '—', '-'] {
        if let Some((left, right)) = rest.split_once(sep) {
            if let Some(st) = PlanStatus::parse(left.trim()) {
                let text = right.trim();
                if !text.is_empty() && text.len() < 200 {
                    return Some((st, text.to_string()));
                }
            }
        }
    }
    None
}

fn section_body(body: &str, name: &str) -> Option<String> {
    let name = name.to_ascii_lowercase();
    let mut lines = body.lines().peekable();
    let mut out = String::new();
    let mut in_sec = false;
    while let Some(line) = lines.next() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix('#') {
            let heading = rest.trim_start_matches('#').trim().to_ascii_lowercase();
            if in_sec {
                break;
            }
            if heading == name || heading.starts_with(&format!("{name} ")) {
                in_sec = true;
                continue;
            }
        }
        if in_sec {
            out.push_str(line);
            out.push('\n');
        }
    }
    if out.trim().is_empty() {
        None
    } else {
        Some(out)
    }
}

fn first_status_in_text(s: &str) -> Option<PlanStatus> {
    for raw in s.split(|c: char| c.is_whitespace() || c == ',' || c == ';' || c == '|') {
        if let Some(st) = PlanStatus::parse(raw) {
            return Some(st);
        }
    }
    // whole first line
    s.lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with("<!--"))
        .and_then(PlanStatus::parse)
}

fn slug_token(s: &str) -> String {
    let mut out = String::new();
    let mut prev = false;
    for c in s.chars().take(48) {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            prev = false;
        } else if !prev && !out.is_empty() {
            out.push('_');
            prev = true;
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    out
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let t: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{t}…")
    }
}

/// Enrich FTS body + summary for plan notes when status signal exists.
pub fn enrich_plan_index_fields(
    body: &str,
    frontmatter_status: Option<&str>,
    base_summary: &str,
) -> (String, String) {
    let dig = densify_plan(frontmatter_status, body);
    if !dig.has_signal() {
        return (body.to_string(), base_summary.to_string());
    }
    let fts = format!("{body}\n\n{}\n", dig.fts_block());
    let summary = if base_summary.is_empty() {
        dig.summary_line()
    } else {
        format!("{} · {}", dig.summary_line(), base_summary)
    };
    // Keep summary bounded for SQLite / agent skims
    let summary = if summary.chars().count() > 240 {
        truncate(&summary, 240)
    } else {
        summary
    };
    (fts, summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_checkboxes_and_sections() {
        let body = r#"
## Status

in_progress

## Backlog

- [ ] Design API
- [ ] Write docs

## In Progress

- [/] Implement parser

## Done

- [x] Scaffold types

## Cancelled

- [~] Old approach
"#;
        let d = densify_plan(None, body);
        assert_eq!(d.overall, Some(PlanStatus::InProgress));
        assert_eq!(d.done, 1);
        assert_eq!(d.cancelled, 1);
        assert!(d.open >= 3);
        let fts = d.fts_block();
        assert!(fts.contains("status:in_progress"));
        assert!(fts.contains("task:done:"));
        assert!(fts.contains("plan_open:"));
    }

    #[test]
    fn frontmatter_overall_wins() {
        let d = densify_plan(Some("qa"), "- [ ] still open\n");
        assert_eq!(d.overall, Some(PlanStatus::Qa));
    }

    #[test]
    fn tagged_bullets() {
        let body = "- done: ship 0.3\n- blocked: flaky CI\n";
        let d = densify_plan(None, body);
        assert!(d.tasks.iter().any(|t| t.status == PlanStatus::Done));
        assert!(d.tasks.iter().any(|t| t.status == PlanStatus::Undone));
    }

    #[test]
    fn parse_aliases() {
        assert_eq!(PlanStatus::parse("WIP"), Some(PlanStatus::InProgress));
        assert_eq!(PlanStatus::parse("wontfix"), Some(PlanStatus::Cancelled));
        assert_eq!(PlanStatus::parse("in-review"), Some(PlanStatus::Qa));
    }
}
