//! YAML frontmatter parsing for Obsidian-style Markdown notes.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Parsed YAML frontmatter metadata from a Markdown file.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Frontmatter {
    /// Note tags.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Optional rustbrain node type (`concept`, `adr`, …).
    #[serde(default)]
    pub node_type: Option<String>,
    /// Obsidian-style aliases.
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Additional free-form fields (e.g. `title`).
    #[serde(flatten)]
    pub extra: HashMap<String, serde_yaml_ng::Value>,
}

/// Parse a YAML frontmatter block from a markdown header (`--- ... ---`).
///
/// On malformed YAML, returns `(None, original markdown)` without panicking.
pub fn parse_frontmatter(markdown: &str) -> (Option<Frontmatter>, &str) {
    let trimmed = markdown.trim_start();
    if !trimmed.starts_with("---") {
        return (None, markdown);
    }

    let rest = &trimmed[3..];
    // Allow optional trailing spaces after opening ---
    let rest = rest.strip_prefix('\n').unwrap_or(rest);

    if let Some(end_idx) = rest.find("\n---") {
        let yaml_str = &rest[..end_idx];
        let body = rest[end_idx + 4..].trim_start_matches('\n');
        match serde_yaml_ng::from_str::<Frontmatter>(yaml_str) {
            Ok(fm) => return (Some(fm), body),
            Err(_) => return (None, markdown),
        }
    }

    (None, markdown)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_frontmatter_basic() {
        let content =
            "---\ntags: [consensus, raft]\nnode_type: concept\naliases: [Raft]\n---\n# Raft Consensus\nNotes here.";
        let (fm, body) = parse_frontmatter(content);
        assert!(fm.is_some());
        let fm = fm.unwrap();
        assert_eq!(fm.tags, vec!["consensus", "raft"]);
        assert_eq!(fm.node_type.as_deref(), Some("concept"));
        assert_eq!(fm.aliases, vec!["Raft"]);
        assert!(body.starts_with("# Raft Consensus"));
    }

    #[test]
    fn no_frontmatter() {
        let (fm, body) = parse_frontmatter("# Hello");
        assert!(fm.is_none());
        assert_eq!(body, "# Hello");
    }
}
