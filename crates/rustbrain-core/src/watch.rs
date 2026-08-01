//! Debounced filesystem watcher for live re-indexing.
//!
//! Requires the `watch` Cargo feature (pulls in `notify`). The watcher blocks
//! the calling thread until the event channel disconnects.
//!
//! Strategy: accumulate interesting paths (`.md` / `.rs` / `.canvas`), wait for
//! a quiet period ([`WatchConfig::debounce`]), then run a full
//! [`WorkspaceIndexer::index_workspace`] (content-hash skips unchanged files)
//! which also remaps `graph.mmap`.
//!
//! Paths under `.brain/`, `target/`, `.git/`, and entries matching
//! `.rustbrainignore` (when present) are ignored.

use crate::error::{BrainError, Result};
use std::path::Path;
use std::time::Duration;

/// Configuration for [`watch_workspace`].
#[derive(Debug, Clone)]
pub struct WatchConfig {
    /// Debounce window: wait this long after the last event before re-syncing.
    pub debounce: Duration,
    /// If true, print status lines to stderr.
    pub verbose: bool,
}

impl Default for WatchConfig {
    fn default() -> Self {
        Self {
            debounce: Duration::from_millis(300),
            verbose: true,
        }
    }
}

/// Watch `workspace` for Markdown / Rust / Canvas changes and re-index + remap.
///
/// Blocks the current thread until the process is interrupted (channel disconnect).
/// Requires the `watch` feature.
#[cfg(feature = "watch")]
pub fn watch_workspace(workspace: impl AsRef<Path>, config: WatchConfig) -> Result<()> {
    use crate::ignore::IgnoreSet;
    use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
    use std::collections::HashSet;
    use std::path::PathBuf;
    use std::sync::mpsc::{self, RecvTimeoutError};
    use std::time::Instant;

    let workspace = workspace.as_ref().to_path_buf();
    let brain_dir = workspace.join(".brain");
    if !brain_dir.join("db.sqlite").exists() {
        return Err(BrainError::BrainNotFound { path: brain_dir });
    }

    let ignore = IgnoreSet::load(&workspace, false).unwrap_or_default();

    let (tx, rx) = mpsc::channel();
    let mut watcher = RecommendedWatcher::new(
        move |res| {
            let _ = tx.send(res);
        },
        notify::Config::default(),
    )
    .map_err(|e| BrainError::indexer(format!("watcher init: {e}")))?;

    watcher
        .watch(&workspace, RecursiveMode::Recursive)
        .map_err(|e| BrainError::indexer(format!("watch start: {e}")))?;

    if config.verbose {
        eprintln!(
            "rustbrain watch: {} (debounce {}ms)",
            workspace.display(),
            config.debounce.as_millis()
        );
    }

    let mut pending: HashSet<PathBuf> = HashSet::new();
    let mut last_event = Instant::now();
    let mut dirty = false;

    loop {
        let timeout = if dirty {
            config
                .debounce
                .saturating_sub(last_event.elapsed())
                .max(Duration::from_millis(10))
        } else {
            Duration::from_secs(3600)
        };

        match rx.recv_timeout(timeout) {
            Ok(Ok(Event { kind, paths, .. })) => {
                if !is_interesting_event(&kind) {
                    continue;
                }
                for p in paths {
                    if should_reindex_path(&workspace, &p, &ignore) {
                        pending.insert(p);
                        dirty = true;
                        last_event = Instant::now();
                    }
                }
            }
            Ok(Err(e)) => {
                if config.verbose {
                    eprintln!("rustbrain watch: event error: {e}");
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                if dirty && last_event.elapsed() >= config.debounce {
                    let n = pending.len();
                    pending.clear();
                    dirty = false;
                    match reindex(&workspace) {
                        Ok(stats) => {
                            if config.verbose {
                                eprintln!(
                                    "rustbrain watch: reindexed after {n} path event(s) (md≈{} rs≈{} edges+={} mmap={} pending_links={})",
                                    stats.markdown_files,
                                    stats.rust_files,
                                    stats.edges_created,
                                    stats.mmap_written,
                                    stats.edges_pending
                                );
                            }
                        }
                        Err(e) => {
                            // Stay alive; next quiet period can retry.
                            eprintln!("rustbrain watch: reindex failed: {e}");
                        }
                    }
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }

    Ok(())
}

#[cfg(feature = "watch")]
fn reindex(workspace: &Path) -> Result<crate::types::SyncStats> {
    use crate::indexer::WorkspaceIndexer;
    use crate::storage::Database;

    // Full workspace reindex is correct and simple; content-hash skips unchanged files.
    let db_path = workspace.join(".brain").join("db.sqlite");
    let db = Database::open(db_path)?;
    let indexer = WorkspaceIndexer::new(db, workspace);
    indexer.index_workspace()
}

#[cfg(feature = "watch")]
fn is_interesting_event(kind: &notify::EventKind) -> bool {
    use notify::EventKind;
    matches!(
        kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) | EventKind::Any
    )
}

#[cfg(feature = "watch")]
fn should_reindex_path(workspace: &Path, path: &Path, ignore: &crate::ignore::IgnoreSet) -> bool {
    if !is_indexable(path) {
        return false;
    }
    if is_under_skipped(workspace, path) {
        return false;
    }
    if let Ok(rel) = path.strip_prefix(workspace) {
        let rel_s = rel.to_string_lossy().replace('\\', "/");
        let is_dir = path.is_dir();
        if ignore.is_ignored(&rel_s, is_dir) {
            return false;
        }
    }
    true
}

/// True when the path extension is one rustbrain indexes.
pub fn is_indexable(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("md") | Some("rs") | Some("canvas")
    )
}

/// True when the path sits under a directory rustbrain never indexes.
pub fn is_under_skipped(workspace: &Path, path: &Path) -> bool {
    let Ok(rel) = path.strip_prefix(workspace) else {
        // Absolute path outside workspace — still skip known noise components.
        return path.components().any(|c| {
            let s = c.as_os_str().to_string_lossy();
            is_skipped_component(s.as_ref())
        });
    };
    rel.components().any(|c| {
        let s = c.as_os_str().to_string_lossy();
        is_skipped_component(s.as_ref())
    })
}

fn is_skipped_component(s: &str) -> bool {
    matches!(
        s,
        "target" | "node_modules" | ".git" | ".brain" | "vendor" | "dist" | "build" | ".cargo"
    ) || (s.starts_with('.') && s != "." && s != "..")
}

/// Stub when the `watch` feature is disabled at compile time.
#[cfg(not(feature = "watch"))]
pub fn watch_workspace(_workspace: impl AsRef<Path>, _config: WatchConfig) -> Result<()> {
    Err(BrainError::FeatureDisabled("watch"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    #[test]
    fn indexable_extensions() {
        assert!(is_indexable(Path::new("docs/a.md")));
        assert!(is_indexable(Path::new("src/lib.rs")));
        assert!(is_indexable(Path::new("map.canvas")));
        assert!(!is_indexable(Path::new("Cargo.toml")));
        assert!(!is_indexable(Path::new("notes.txt")));
    }

    #[test]
    fn skips_brain_and_target() {
        let ws = PathBuf::from("/proj");
        assert!(is_under_skipped(&ws, &ws.join(".brain/db.sqlite")));
        assert!(is_under_skipped(&ws, &ws.join("target/debug/foo.rs")));
        assert!(is_under_skipped(&ws, &ws.join(".git/config")));
        assert!(!is_under_skipped(&ws, &ws.join("docs/a.md")));
        assert!(!is_under_skipped(&ws, &ws.join("src/lib.rs")));
    }
}
