//! Workspace ignore patterns for indexing (`.rustbrainignore` + optional `.gitignore`).
//!
//! Patterns use a simple gitignore-inspired dialect sufficient for v0.2:
//! - `#` comments and blank lines ignored
//! - trailing `/` matches directories only (prefix of path segments)
//! - leading `/` anchors to workspace root
//! - `*` matches within a single path segment (no `/`)
//! - `**` matches across segments
//! - bare names match any path segment equal to the name
//!
//! Built-in defaults always apply: `target/`, `.git/`, `.brain/`, `node_modules/`, etc.

use crate::error::Result;
use std::path::{Path, PathBuf};

/// Compiled ignore set for a workspace root.
#[derive(Debug, Clone, Default)]
pub struct IgnoreSet {
    patterns: Vec<Pattern>,
    /// Paths of ignore files that were loaded (for doctor / bootstrap reports).
    pub sources: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
struct Pattern {
    /// Normalized pattern without leading `/` or trailing `/`.
    body: String,
    dir_only: bool,
    anchored: bool,
}

impl IgnoreSet {
    /// Load ignore rules for `workspace`.
    ///
    /// Always includes built-in defaults. Then loads `.rustbrainignore` if present.
    /// If `include_gitignore` is true and `.gitignore` exists, its patterns are appended.
    pub fn load(workspace: &Path, include_gitignore: bool) -> Result<Self> {
        let mut set = Self::default();
        for line in BUILTIN_PATTERNS {
            set.push_line(line);
        }

        let rb = workspace.join(".rustbrainignore");
        if rb.is_file() {
            set.load_file(&rb)?;
            set.sources.push(rb);
        }

        if include_gitignore {
            let gi = workspace.join(".gitignore");
            if gi.is_file() {
                set.load_file(&gi)?;
                set.sources.push(gi);
            }
        }

        Ok(set)
    }

    /// Whether a **relative** path (posix `/` separators preferred) should be skipped.
    ///
    /// `is_dir` should be true when `rel` names a directory.
    pub fn is_ignored(&self, rel: &str, is_dir: bool) -> bool {
        let rel = rel.trim_matches('/');
        if rel.is_empty() {
            return false;
        }
        let rel_slash = rel.replace('\\', "/");
        for p in &self.patterns {
            if p.matches(&rel_slash, is_dir) {
                return true;
            }
        }
        false
    }

    /// Whether a directory name (single segment) should be skipped during walk.
    pub fn skip_dir_name(&self, name: &str) -> bool {
        if name == "." || name == ".." {
            return true;
        }
        // Fast path built-ins
        if matches!(
            name,
            "target"
                | "node_modules"
                | "vendor"
                | ".git"
                | ".brain"
                | "dist"
                | "build"
                | ".svn"
                | ".hg"
                | ".direnv"
                | "result"
        ) || name.starts_with('.')
        {
            // Still allow going into dirs that are only ignored via custom rules differently —
            // for hidden dirs we skip by default (same as pre-0.2 indexer).
            return true;
        }
        // Check patterns against "name" and "name/" as directory
        self.is_ignored(name, true)
    }

    fn load_file(&mut self, path: &Path) -> Result<()> {
        let text = std::fs::read_to_string(path)?;
        for line in text.lines() {
            self.push_line(line);
        }
        Ok(())
    }

    fn push_line(&mut self, line: &str) {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            return;
        }
        // Negation not supported in v0.2 — skip.
        if line.starts_with('!') {
            return;
        }
        let anchored = line.starts_with('/');
        let dir_only = line.ends_with('/');
        let mut body = line.trim_start_matches('/').trim_end_matches('/').to_string();
        body = body.replace('\\', "/");
        if body.is_empty() {
            return;
        }
        self.patterns.push(Pattern {
            body,
            dir_only,
            anchored,
        });
    }
}

impl Pattern {
    fn matches(&self, rel: &str, is_dir: bool) -> bool {
        if self.dir_only && !is_dir {
            // Still match if any path prefix is this directory.
            // e.g. pattern `target/` matches file `target/foo.rlib` via prefix.
        }

        if self.body.contains("**") {
            return glob_double_star(&self.body, rel, self.anchored);
        }

        if self.body.contains('*') {
            if self.anchored {
                return glob_single_segment_path(&self.body, rel);
            }
            // Unanchored: match any path suffix or segment
            if glob_single_segment_path(&self.body, rel) {
                return true;
            }
            for part in rel.split('/') {
                if glob_segment(&self.body, part) {
                    return true;
                }
            }
            // also match full relative with pattern as suffix path
            if let Some(idx) = rel.rfind('/') {
                return glob_single_segment_path(&self.body, &rel[idx + 1..]);
            }
            return false;
        }

        // Literal
        if self.anchored {
            if self.dir_only {
                return rel == self.body || rel.starts_with(&format!("{}/", self.body));
            }
            return rel == self.body;
        }

        // Unanchored literal: any segment equals body, or path ends with /body
        if rel == self.body || rel.ends_with(&format!("/{}", self.body)) {
            return true;
        }
        // directory prefix anywhere
        if rel.starts_with(&format!("{}/", self.body)) {
            return true;
        }
        if rel.contains(&format!("/{}/", self.body)) {
            return true;
        }
        rel.split('/').any(|s| s == self.body)
    }
}

fn glob_segment(pat: &str, seg: &str) -> bool {
    // only * wildcards, no ?
    let parts: Vec<&str> = pat.split('*').collect();
    if parts.len() == 1 {
        return pat == seg;
    }
    if !seg.starts_with(parts[0]) {
        return false;
    }
    if !seg.ends_with(parts[parts.len() - 1]) {
        return false;
    }
    let mut rest = &seg[parts[0].len()..seg.len() - parts[parts.len() - 1].len()];
    for mid in &parts[1..parts.len() - 1] {
        if mid.is_empty() {
            continue;
        }
        if let Some(pos) = rest.find(mid) {
            rest = &rest[pos + mid.len()..];
        } else {
            return false;
        }
    }
    true
}

fn glob_single_segment_path(pat: &str, rel: &str) -> bool {
    // Pattern may include `/` for multi-segment without `**`
    let pat_parts: Vec<&str> = pat.split('/').collect();
    let rel_parts: Vec<&str> = rel.split('/').collect();
    if pat_parts.len() != rel_parts.len() {
        // allow pattern match against suffix of equal length
        if rel_parts.len() < pat_parts.len() {
            return false;
        }
        let start = rel_parts.len() - pat_parts.len();
        return pat_parts
            .iter()
            .zip(rel_parts[start..].iter())
            .all(|(p, r)| glob_segment(p, r));
    }
    pat_parts
        .iter()
        .zip(rel_parts.iter())
        .all(|(p, r)| glob_segment(p, r))
}

fn glob_double_star(pat: &str, rel: &str, anchored: bool) -> bool {
    // Very small ** implementation: split on ** and check sequential find.
    let parts: Vec<&str> = pat.split("**").map(|s| s.trim_matches('/')).collect();
    if parts.is_empty() {
        return true;
    }
    let mut cursor = 0usize;
    let rel_bytes = rel.as_bytes();
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        // For first part with anchored, must be prefix
        if i == 0 && anchored && !part.contains('*') {
            if !rel.starts_with(part)
                && !rel.starts_with(&format!("{part}/"))
            {
                return false;
            }
            cursor = part.len().min(rel.len());
            if cursor < rel.len() && rel.as_bytes()[cursor] == b'/' {
                cursor += 1;
            }
            continue;
        }
        // Find part in remaining (segment-aware for pure literals)
        let remaining = &rel[cursor..];
        if part.contains('*') {
            // fallback: substring after simplifying
            if let Some(pos) = remaining.find(part.trim_matches('*')) {
                cursor += pos + part.trim_matches('*').len();
            } else {
                return false;
            }
        } else if let Some(pos) = remaining.find(part) {
            cursor += pos + part.len();
        } else {
            return false;
        }
        let _ = rel_bytes;
    }
    true
}

const BUILTIN_PATTERNS: &[&str] = &[
    "target/",
    "node_modules/",
    "vendor/",
    ".git/",
    ".brain/",
    "dist/",
    "build/",
    ".svn/",
    ".hg/",
    ".direnv/",
    "result/",
    "*.sqlite",
    "*.sqlite-journal",
    "*.sqlite-wal",
    "*.sqlite-shm",
];

/// Default recommended extra patterns for a new `.rustbrainignore`.
pub fn recommended_ignore_extras() -> &'static [&'static str] {
    &[
        "# Build / IDE",
        "target/",
        ".idea/",
        ".vscode/",
        "*.swp",
        ".DS_Store",
        "",
        "# Derived data / large blobs",
        "*.parquet",
        "*.arrow",
        "data/",
        "datasets/",
        "",
        "# Logs and local env",
        "*.log",
        ".env",
        ".env.*",
    ]
}

/// Write a `.rustbrainignore` file.
pub fn write_rustbrainignore(workspace: &Path, import_gitignore: bool, extras: &[&str]) -> Result<PathBuf> {
    let path = workspace.join(".rustbrainignore");
    let mut out = String::from(
        "# rustbrain index ignore (gitignore-inspired)\n\
         # Built-in defaults always apply; this file adds more skips.\n\n",
    );

    if import_gitignore {
        let gi = workspace.join(".gitignore");
        if gi.is_file() {
            out.push_str("# --- imported from .gitignore ---\n");
            out.push_str(&std::fs::read_to_string(&gi)?);
            if !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str("# --- end import ---\n\n");
        }
    }

    for line in extras {
        out.push_str(line);
        out.push('\n');
    }

    // Don't overwrite silently if exists — caller should decide.
    std::fs::write(&path, out)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn builtin_skips_target() {
        let dir = tempdir().unwrap();
        let set = IgnoreSet::load(dir.path(), false).unwrap();
        assert!(set.is_ignored("target", true));
        assert!(set.is_ignored("target/debug/foo", false));
        assert!(set.skip_dir_name("target"));
        assert!(!set.is_ignored("src/main.rs", false));
    }

    #[test]
    fn custom_pattern_file() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join(".rustbrainignore"),
            "data/\n*.parquet\n",
        )
        .unwrap();
        let set = IgnoreSet::load(dir.path(), false).unwrap();
        assert!(set.is_ignored("data/x", false));
        assert!(set.is_ignored("foo.parquet", false));
        assert!(!set.is_ignored("src/lib.rs", false));
    }
}
