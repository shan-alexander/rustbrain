//! Obsidian WikiLink (`[[...]]`) extraction.
//!
//! Links inside fenced code blocks and inline code spans are ignored.

use serde::{Deserialize, Serialize};

/// Parsed Obsidian WikiLink representation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WikiLink {
    /// Target note name / id fragment.
    pub target_node: String,
    /// Optional `#section` fragment.
    pub section: Option<String>,
    /// Optional display alias after `|`.
    pub display_alias: Option<String>,
}

impl WikiLink {
    /// Format as standard Obsidian markdown.
    pub fn to_markdown(&self) -> String {
        match (&self.section, &self.display_alias) {
            (Some(sec), Some(alias)) => format!("[[{}#{}|{}]]", self.target_node, sec, alias),
            (Some(sec), None) => format!("[[{}#{}]]", self.target_node, sec),
            (None, Some(alias)) => format!("[[{}|{}]]", self.target_node, alias),
            (None, None) => format!("[[{}]]", self.target_node),
        }
    }
}

/// Extract all WikiLinks from markdown, skipping fenced and inline code.
pub fn extract_wikilinks(markdown: &str) -> Vec<WikiLink> {
    let mut links = Vec::new();
    let bytes = markdown.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let mut in_fence = false;

    while i < len {
        // Fence toggle on lines starting with ```
        if is_line_start(bytes, i) && i + 2 < len && &bytes[i..i + 3] == b"```" {
            in_fence = !in_fence;
            i += 3;
            continue;
        }

        if in_fence {
            i += 1;
            continue;
        }

        // Inline code span: skip until next unescaped `
        if bytes[i] == b'`' {
            i += 1;
            while i < len && bytes[i] != b'`' {
                i += 1;
            }
            if i < len {
                i += 1;
            }
            continue;
        }

        if i + 1 < len && bytes[i] == b'[' && bytes[i + 1] == b'[' {
            let start = i + 2;
            if let Some(rel_end) = markdown[start..].find("]]") {
                let inner = &markdown[start..start + rel_end];
                if !inner.trim().is_empty() && !inner.contains('\n') {
                    let mut alias_parts = inner.splitn(2, '|');
                    let target_and_sec = alias_parts.next().unwrap_or("").trim();
                    let display_alias = alias_parts.next().map(|s| s.trim().to_string());

                    let mut sec_parts = target_and_sec.splitn(2, '#');
                    let target_node = sec_parts.next().unwrap_or("").trim().to_string();
                    let section = sec_parts.next().map(|s| s.trim().to_string());

                    if !target_node.is_empty() {
                        links.push(WikiLink {
                            target_node,
                            section,
                            display_alias,
                        });
                    }
                }
                i = start + rel_end + 2;
                continue;
            }
        }
        i += 1;
    }

    links
}

fn is_line_start(bytes: &[u8], i: usize) -> bool {
    i == 0 || bytes[i - 1] == b'\n'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_wikilinks_basic() {
        let text =
            "This relates to [[LogCompaction#Section|Log Compaction]] and [[RaftConsensus]].";
        let links = extract_wikilinks(text);
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].target_node, "LogCompaction");
        assert_eq!(links[0].section.as_deref(), Some("Section"));
        assert_eq!(links[0].display_alias.as_deref(), Some("Log Compaction"));
        assert_eq!(links[1].target_node, "RaftConsensus");
        assert_eq!(
            links[0].to_markdown(),
            "[[LogCompaction#Section|Log Compaction]]"
        );
    }

    #[test]
    fn ignores_code_fences_and_inline() {
        let text = "See [[Real]] and `[[NotThis]]` and:\n```\n[[AlsoNot]]\n```\n[[Yes]]";
        let links = extract_wikilinks(text);
        let names: Vec<_> = links.iter().map(|l| l.target_node.as_str()).collect();
        assert_eq!(names, vec!["Real", "Yes"]);
    }
}
