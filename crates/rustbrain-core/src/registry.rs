//! Global developer registry of rustbrain workspaces (XDG-aware).
//!
//! The registry powers `rustbrain query --all-workspaces`: each `rustbrain init`
//! / `sync` registers the workspace path under the user config directory so
//! multi-repo developers can search every brain on the machine.

use crate::error::{BrainError, Result};
use crate::query::{QueryOptions, RankedHit};
use crate::storage::Database;
use crate::types::Node;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Global registry of rustbrain workspaces on this machine.
///
/// Stored at `$XDG_CONFIG_HOME/rustbrain/registry.json` (Linux), or the platform
/// equivalent returned by the `dirs` crate.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GlobalRegistry {
    /// Absolute workspace paths (canonicalized when possible).
    #[serde(default)]
    pub workspaces: Vec<String>,
}

/// A ranked hit tagged with its originating workspace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalRankedHit {
    /// Absolute workspace path.
    pub workspace: String,
    /// Hit within that workspace's brain.
    pub hit: RankedHit,
}

impl GlobalRegistry {
    /// Config path: `$XDG_CONFIG_HOME/rustbrain/registry.json` or platform equivalent.
    pub fn config_path() -> Result<PathBuf> {
        let base = dirs::config_dir()
            .ok_or_else(|| BrainError::other("could not determine user config directory"))?;
        let dir = base.join("rustbrain");
        fs::create_dir_all(&dir)?;
        Ok(dir.join("registry.json"))
    }

    /// Load registry, pruning missing workspace paths (and persisting if anything dropped).
    pub fn load() -> Result<Self> {
        let path = Self::config_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = fs::read_to_string(&path)?;
        let mut reg: Self = serde_json::from_str(&content).unwrap_or_default();
        let before = reg.workspaces.len();
        reg.prune_missing();
        if reg.workspaces.len() != before {
            let _ = reg.save();
        }
        Ok(reg)
    }

    /// Prune missing paths and save. Returns number of entries removed.
    pub fn gc(&mut self) -> Result<usize> {
        let before = self.workspaces.len();
        self.prune_missing();
        let removed = before.saturating_sub(self.workspaces.len());
        if removed > 0 {
            self.save()?;
        }
        Ok(removed)
    }

    /// Persist registry atomically.
    pub fn save(&self) -> Result<()> {
        let path = Self::config_path()?;
        let tmp = path.with_extension("json.tmp");
        let json = serde_json::to_string_pretty(self)?;
        fs::write(&tmp, json)?;
        fs::rename(&tmp, &path)?;
        Ok(())
    }

    /// Drop workspace paths that no longer have a brain database.
    pub fn prune_missing(&mut self) {
        self.workspaces
            .retain(|ws| Path::new(ws).join(".brain").join("db.sqlite").exists());
    }

    /// Register a workspace directory path.
    pub fn register<P: AsRef<Path>>(&mut self, workspace_path: P) -> Result<()> {
        let abs_path = fs::canonicalize(workspace_path.as_ref())
            .unwrap_or_else(|_| workspace_path.as_ref().to_path_buf())
            .to_string_lossy()
            .to_string();

        if !self.workspaces.contains(&abs_path) {
            self.workspaces.push(abs_path);
            self.save()?;
        }
        Ok(())
    }

    /// Search FTS across all registered workspaces (unranked grouping).
    pub fn search_all(&self, query: &str) -> Result<Vec<(String, Vec<Node>)>> {
        let ranked = self.search_all_ranked(query, &QueryOptions::default())?;
        let mut by_ws: std::collections::BTreeMap<String, Vec<Node>> =
            std::collections::BTreeMap::new();
        for gh in ranked {
            by_ws.entry(gh.workspace).or_default().push(gh.hit.node);
        }
        Ok(by_ws.into_iter().collect())
    }

    /// Cross-workspace ranked merge: highest score first, workspace tag preserved.
    pub fn search_all_ranked(
        &self,
        query: &str,
        opts: &QueryOptions,
    ) -> Result<Vec<GlobalRankedHit>> {
        let mut merged: Vec<GlobalRankedHit> = Vec::new();

        for ws in &self.workspaces {
            let db_path = PathBuf::from(ws).join(".brain").join("db.sqlite");
            if !db_path.exists() {
                continue;
            }
            match Database::open(&db_path) {
                Ok(db) => match db.search_ranked(query, opts) {
                    Ok(hits) => {
                        for hit in hits {
                            merged.push(GlobalRankedHit {
                                workspace: ws.clone(),
                                hit,
                            });
                        }
                    }
                    Err(e) => {
                        eprintln!("rustbrain: skip workspace {ws}: {e}");
                    }
                },
                Err(e) => {
                    eprintln!("rustbrain: cannot open {ws}: {e}");
                }
            }
        }

        merged.sort_by(|a, b| {
            b.hit
                .score
                .partial_cmp(&a.hit.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        merged.truncate(opts.limit);
        Ok(merged)
    }
}
