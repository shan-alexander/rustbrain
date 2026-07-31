//! Debounced filesystem watcher for live re-indexing.
//!
//! Requires the `watch` Cargo feature (pulls in `notify`). The watcher blocks
//! the calling thread until the event channel disconnects.
//!
//! Strategy: accumulate interesting paths (`.md` / `.rs` / `.canvas`), wait for
//! a quiet period ([`WatchConfig::debounce`]), then run a full
//! [`WorkspaceIndexer::index_workspace`] (content-hash skips unchanged files)
//! which also remaps `graph.mmap`.

use crate::error::{BrainError, Result};
use crate::indexer::WorkspaceIndexer;
use crate::storage::Database;
use crate::types::SyncStats;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

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

/// Watch `workspace` for Markdown / Rust / Canvas changes and re-index + remmap.
///
/// Blocks the current thread until the process is interrupted (channel disconnect).
/// Requires the `watch` feature.
#[cfg(feature = "watch")]
pub fn watch_workspace(workspace: impl AsRef<Path>, config: WatchConfig) -> Result<()> {
    use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};

    let workspace = workspace.as_ref().to_path_buf();
    let brain_dir = workspace.join(".brain");
    if !brain_dir.join("db.sqlite").exists() {
        return Err(BrainError::BrainNotFound { path: brain_dir });
    }

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
                    if is_indexable(&p) && !is_under_skipped(&workspace, &p) {
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
                    let paths: Vec<PathBuf> = pending.drain().collect();
                    dirty = false;
                    match reindex(&workspace, &paths, config.verbose) {
                        Ok(stats) => {
                            if config.verbose {
                                eprintln!(
                                    "rustbrain watch: reindexed (md≈{} rs≈{} edges+={} mmap={})",
                                    stats.markdown_files,
                                    stats.rust_files,
                                    stats.edges_created,
                                    stats.mmap_written
                                );
                            }
                        }
                        Err(e) => {
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
fn reindex(workspace: &Path, _paths: &[PathBuf], _verbose: bool) -> Result<SyncStats> {
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

fn is_indexable(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("md") | Some("rs") | Some("canvas")
    )
}

fn is_under_skipped(workspace: &Path, path: &Path) -> bool {
    let Ok(rel) = path.strip_prefix(workspace) else {
        return false;
    };
    rel.components().any(|c| {
        let s = c.as_os_str().to_string_lossy();
        matches!(
            s.as_ref(),
            "target" | "node_modules" | ".git" | ".brain" | "vendor" | "dist" | "build"
        ) || s.starts_with('.')
    })
}

#[cfg(not(feature = "watch"))]
pub fn watch_workspace(_workspace: impl AsRef<Path>, _config: WatchConfig) -> Result<()> {
    Err(BrainError::FeatureDisabled("watch"))
}
