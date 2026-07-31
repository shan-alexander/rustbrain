//! Stable node ID scheme based on repository-relative path slugs.
//!
//! Collision-resistant across directories:
//!
//! | Relative path | Node id |
//! |---------------|---------|
//! | `docs/concepts/raft.md` | `docs/concepts/raft` |
//! | `Notes/Foo Bar.md` | `notes/foo-bar` |
//!
//! IDs are lowercase, `/`-separated, and extension-stripped. They are the
//! primary key in SQLite and the string key in the CSR id table.

use std::path::{Component, Path};

/// Build a stable node id from a workspace-relative file path.
///
/// Rules:
/// - strip extension
/// - lowercase
/// - normalize path separators to `/`
/// - collapse `//`, reject parent-dir escapes
/// - replace whitespace runs with `-`
pub fn node_id_from_rel_path(rel_path: &Path) -> String {
    let mut parts: Vec<String> = Vec::new();
    for comp in rel_path.components() {
        match comp {
            Component::Normal(os) => {
                let s = os.to_string_lossy();
                let cleaned = sanitize_segment(&s);
                if !cleaned.is_empty() {
                    parts.push(cleaned);
                }
            }
            Component::CurDir => {}
            // Drop prefix / root / parent — callers should pass relative paths.
            _ => {}
        }
    }

    if parts.is_empty() {
        return "untitled".to_string();
    }

    // Drop final extension if present on last segment only when it looks like a file.
    if let Some(last) = parts.last_mut() {
        if let Some((stem, _ext)) = last.rsplit_once('.') {
            if !stem.is_empty() {
                *last = stem.to_string();
            }
        }
    }

    parts.join("/")
}

/// Sanitize a single path segment for use in an id.
fn sanitize_segment(s: &str) -> String {
    let lower = s.to_lowercase();
    let mut out = String::with_capacity(lower.len());
    let mut prev_dash = false;
    for c in lower.chars() {
        if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' {
            out.push(c);
            prev_dash = c == '-';
        } else if c.is_whitespace() || c == '/' || c == '\\' {
            if !prev_dash && !out.is_empty() {
                out.push('-');
                prev_dash = true;
            }
        } else {
            // drop other punctuation
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// Resolve a WikiLink target string against known node ids / aliases / titles.
///
/// Matching order (first hit wins):
/// 1. exact id
/// 2. exact alias
/// 3. case-insensitive title
/// 4. id suffix match (`raft` → `docs/concepts/raft`) when unique
pub fn resolve_link_target(
    raw: &str,
    ids: &std::collections::HashSet<String>,
    aliases: &std::collections::HashMap<String, String>,
    titles: &std::collections::HashMap<String, String>,
) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let key = trimmed.to_lowercase();

    if ids.contains(&key) {
        return Some(key);
    }
    // Also try as-is for ids that already contain slashes in lowercase form.
    if ids.contains(trimmed) {
        return Some(trimmed.to_string());
    }

    if let Some(id) = aliases.get(&key) {
        return Some(id.clone());
    }

    if let Some(id) = titles.get(&key) {
        return Some(id.clone());
    }

    // Unique suffix match: "raft" matches "docs/concepts/raft" if only one ends with /raft or == raft
    let mut matches: Vec<&String> = ids
        .iter()
        .filter(|id| {
            id.as_str() == key
                || id.ends_with(&format!("/{key}"))
                || id.rsplit('/').next() == Some(key.as_str())
        })
        .collect();
    matches.sort();
    matches.dedup();
    if matches.len() == 1 {
        return Some(matches[0].clone());
    }

    None
}

/// Compute a hex BLAKE3 content hash for change detection.
pub fn content_hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

/// Make a path relative to workspace, falling back to the path itself.
pub fn rel_path_from_workspace(workspace: &Path, file: &Path) -> std::path::PathBuf {
    if let Ok(rel) = file.strip_prefix(workspace) {
        return rel.to_path_buf();
    }
    // Try canonicalize both sides.
    if let (Ok(ws), Ok(fp)) = (workspace.canonicalize(), file.canonicalize()) {
        if let Ok(rel) = fp.strip_prefix(&ws) {
            return rel.to_path_buf();
        }
    }
    file.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};
    use std::path::PathBuf;

    #[test]
    fn slug_from_nested_path() {
        let id = node_id_from_rel_path(Path::new("docs/concepts/Raft Consensus.md"));
        assert_eq!(id, "docs/concepts/raft-consensus");
    }

    #[test]
    fn slug_strips_extension() {
        assert_eq!(
            node_id_from_rel_path(Path::new("notes/foo.md")),
            "notes/foo"
        );
    }

    #[test]
    fn resolve_suffix_unique() {
        let mut ids = HashSet::new();
        ids.insert("docs/concepts/raft".to_string());
        ids.insert("docs/other/log".to_string());
        let aliases = HashMap::new();
        let titles = HashMap::new();
        assert_eq!(
            resolve_link_target("raft", &ids, &aliases, &titles).as_deref(),
            Some("docs/concepts/raft")
        );
    }

    #[test]
    fn resolve_ambiguous_suffix_none() {
        let mut ids = HashSet::new();
        ids.insert("a/raft".to_string());
        ids.insert("b/raft".to_string());
        let aliases = HashMap::new();
        let titles = HashMap::new();
        assert!(resolve_link_target("raft", &ids, &aliases, &titles).is_none());
    }

    #[test]
    fn rel_path_strips_workspace() {
        let ws = PathBuf::from("/proj");
        let f = PathBuf::from("/proj/docs/a.md");
        assert_eq!(
            rel_path_from_workspace(&ws, &f),
            PathBuf::from("docs/a.md")
        );
    }
}
