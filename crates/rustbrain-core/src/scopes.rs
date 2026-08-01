//! MainBrain / SubBrain workspace scopes (multi-crate / multi-root).
//!
//! **Default is single-brain:** every node owns scope [`MAIN_SCOPE`] (`main`).
//! Multi-brain is **opt-in** via [`enable_multi`] / `rustbrain scopes enable`.
//!
//! Design rules:
//! - Flat sibling SubBrains only (no nested brains).
//! - One canonical owner scope per node.
//! - Cross-scope WikiLinks / symbols stay in one workspace graph.
//! - Distinct from `--all-workspaces` (machine-global registry of projects).

use crate::error::{BrainError, Result};
use crate::storage::Database;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

/// Canonical MainBrain scope id.
pub const MAIN_SCOPE: &str = "main";

/// Workspace brain mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BrainMode {
    /// One bag of knowledge; all nodes are `main`. Default for most repos.
    #[default]
    Single,
    /// Multi-scope: MainBrain + optional SubBrains with path roots.
    Multi,
}

impl BrainMode {
    /// Parse `single` / `multi`.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "single" | "one" | "flat" => Some(Self::Single),
            "multi" | "multi-brain" | "multibrain" | "scopes" => Some(Self::Multi),
            _ => None,
        }
    }

    /// Wire string.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Single => "single",
            Self::Multi => "multi",
        }
    }
}

/// How a scope entry was created.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ScopeSource {
    /// User / agent added via CLI.
    #[default]
    Manual,
    /// Discovered from Cargo `[workspace].members`.
    CargoWorkspace,
    /// Imported from another brain / workspace.
    Imported,
    /// Explicit extra root (non-Cargo).
    ExtraRoot,
}

impl ScopeSource {
    /// Wire string.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::CargoWorkspace => "cargo-workspace",
            Self::Imported => "imported",
            Self::ExtraRoot => "extra-root",
        }
    }
}

/// One SubBrain (or extra root) under the MainBrain.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScopeDef {
    /// Flat scope id (path-stable, e.g. `rustbrain-core`).
    pub id: String,
    /// Workspace-relative directory roots owned by this scope.
    pub roots: Vec<String>,
    /// Optional aliases (Cargo package name when different from id).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    /// Provenance.
    #[serde(default)]
    pub source: ScopeSource,
}

/// Versioned `.brain/workspace.json` manifest.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkspaceManifest {
    /// Manifest format version (2 = scopes).
    pub version: u32,
    /// Absolute or display path of workspace root.
    pub workspace: String,
    /// Single vs multi-brain.
    #[serde(default)]
    pub mode: BrainMode,
    /// MainBrain id (always present conceptually; default `main`).
    #[serde(default = "default_main_id")]
    pub main_id: String,
    /// SubBrain definitions (never includes main itself).
    #[serde(default)]
    pub scopes: Vec<ScopeDef>,
    /// Discovery toggles.
    #[serde(default)]
    pub discovery: DiscoveryOptions,
}

fn default_main_id() -> String {
    MAIN_SCOPE.to_string()
}

/// How scopes were / should be discovered.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct DiscoveryOptions {
    /// Auto-refresh Cargo workspace members on enable / discover.
    #[serde(default)]
    pub cargo_workspace: bool,
}

impl WorkspaceManifest {
    /// New single-mode marker (compatible with older tiny markers).
    pub fn single(workspace: &Path) -> Self {
        Self {
            version: 2,
            workspace: workspace.display().to_string(),
            mode: BrainMode::Single,
            main_id: MAIN_SCOPE.to_string(),
            scopes: Vec::new(),
            discovery: DiscoveryOptions::default(),
        }
    }

    /// Whether multi-brain filtering is active.
    pub fn is_multi(&self) -> bool {
        self.mode == BrainMode::Multi
    }

    /// Resolve owner scope for a workspace-relative path (`/`-separated).
    ///
    /// Longest matching SubBrain root wins; otherwise MainBrain.
    /// In **single** mode always returns [`MAIN_SCOPE`].
    pub fn resolve_scope(&self, rel_path: &str) -> String {
        if !self.is_multi() {
            return self.main_id.clone();
        }
        let rel = normalize_rel(rel_path);
        let mut best: Option<(&str, usize)> = None;
        for sc in &self.scopes {
            for root in &sc.roots {
                let r = normalize_rel(root);
                if r.is_empty() {
                    continue;
                }
                if rel == r || rel.starts_with(&(r.clone() + "/")) {
                    let len = r.len();
                    if best.map(|(_, l)| len > l).unwrap_or(true) {
                        best = Some((sc.id.as_str(), len));
                    }
                }
            }
        }
        best.map(|(id, _)| id.to_string())
            .unwrap_or_else(|| self.main_id.clone())
    }

    /// Look up a SubBrain by id or alias.
    pub fn find_scope(&self, id_or_alias: &str) -> Option<&ScopeDef> {
        let key = id_or_alias.trim();
        self.scopes.iter().find(|s| {
            s.id == key || s.aliases.iter().any(|a| a == key)
        })
    }

    /// Mut look up.
    pub fn find_scope_mut(&mut self, id_or_alias: &str) -> Option<&mut ScopeDef> {
        let key = id_or_alias.trim().to_string();
        self.scopes.iter_mut().find(|s| {
            s.id == key || s.aliases.iter().any(|a| a == &key)
        })
    }

    /// All known scope ids including main.
    pub fn all_scope_ids(&self) -> Vec<String> {
        let mut v = vec![self.main_id.clone()];
        for s in &self.scopes {
            v.push(s.id.clone());
        }
        v
    }

    /// Validate no overlapping roots; returns error messages.
    pub fn validate(&self) -> Vec<String> {
        let mut errs = Vec::new();
        let mut seen_ids = BTreeSet::new();
        if self.main_id.trim().is_empty() {
            errs.push("main_id must not be empty".into());
        }
        for sc in &self.scopes {
            if sc.id == self.main_id || sc.id == MAIN_SCOPE && self.main_id != MAIN_SCOPE {
                // allow only if main_id is literally that id — subbrains must not use main id
            }
            if sc.id == self.main_id {
                errs.push(format!("scope id {:?} collides with main_id", sc.id));
            }
            if !seen_ids.insert(sc.id.clone()) {
                errs.push(format!("duplicate scope id {:?}", sc.id));
            }
            if sc.id.trim().is_empty() {
                errs.push("empty scope id".into());
            }
            if sc.roots.is_empty() {
                errs.push(format!("scope {:?} has no roots", sc.id));
            }
            for root in &sc.roots {
                if root.trim().is_empty() || root.contains("..") {
                    errs.push(format!("invalid root {:?} on scope {:?}", root, sc.id));
                }
            }
        }
        // Overlap detection (equal roots or one prefix of another across scopes).
        let mut root_owners: Vec<(String, String)> = Vec::new();
        for sc in &self.scopes {
            for r in &sc.roots {
                root_owners.push((normalize_rel(r), sc.id.clone()));
            }
        }
        for i in 0..root_owners.len() {
            for j in (i + 1)..root_owners.len() {
                let (ra, sa) = &root_owners[i];
                let (rb, sb) = &root_owners[j];
                if sa == sb {
                    continue;
                }
                if ra == rb || ra.starts_with(&(rb.clone() + "/")) || rb.starts_with(&(ra.clone() + "/"))
                {
                    errs.push(format!(
                        "overlapping roots {ra:?} (scope {sa}) and {rb:?} (scope {sb})"
                    ));
                }
            }
        }
        errs
    }
}

fn normalize_rel(p: &str) -> String {
    p.replace('\\', "/")
        .trim_matches('/')
        .trim()
        .to_string()
}

/// Sanitize a user-provided scope id.
pub fn sanitize_scope_id(raw: &str) -> Result<String> {
    let s = raw.trim().to_ascii_lowercase().replace('_', "-");
    if s.is_empty() {
        return Err(BrainError::other("scope id must not be empty"));
    }
    if s == MAIN_SCOPE {
        return Err(BrainError::other(
            "scope id 'main' is reserved for MainBrain — use a SubBrain id",
        ));
    }
    if !s
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
    {
        return Err(BrainError::other(format!(
            "invalid scope id {raw:?}: use letters, digits, '-', '.'"
        )));
    }
    Ok(s)
}

/// Path to `.brain/workspace.json`.
pub fn manifest_path(workspace: &Path) -> PathBuf {
    workspace.join(".brain").join("workspace.json")
}

/// Load manifest if present; otherwise single-mode default (does not write).
pub fn load_manifest(workspace: &Path) -> Result<WorkspaceManifest> {
    let path = manifest_path(workspace);
    if !path.is_file() {
        return Ok(WorkspaceManifest::single(workspace));
    }
    let raw = fs::read_to_string(&path)?;
    parse_manifest_json(&raw, workspace)
}

fn parse_manifest_json(raw: &str, workspace: &Path) -> Result<WorkspaceManifest> {
    // Backward compat: tiny v1 marker `{ "version": 1, "workspace": "..." }`
    let v: serde_json::Value = serde_json::from_str(raw)
        .map_err(|e| BrainError::other(format!("workspace.json: {e}")))?;
    let version = v.get("version").and_then(|x| x.as_u64()).unwrap_or(1) as u32;
    if version <= 1 && v.get("mode").is_none() && v.get("scopes").is_none() {
        let mut m = WorkspaceManifest::single(workspace);
        if let Some(ws) = v.get("workspace").and_then(|x| x.as_str()) {
            m.workspace = ws.to_string();
        }
        return Ok(m);
    }
    let mut m: WorkspaceManifest = serde_json::from_value(v)
        .map_err(|e| BrainError::other(format!("workspace.json: {e}")))?;
    if m.main_id.trim().is_empty() {
        m.main_id = MAIN_SCOPE.to_string();
    }
    if m.version < 2 {
        m.version = 2;
    }
    Ok(m)
}

/// Write manifest (pretty JSON). Creates `.brain/` if needed.
pub fn save_manifest(workspace: &Path, manifest: &WorkspaceManifest) -> Result<()> {
    let errs = manifest.validate();
    if !errs.is_empty() {
        return Err(BrainError::other(format!(
            "invalid workspace manifest: {}",
            errs.join("; ")
        )));
    }
    let brain = workspace.join(".brain");
    fs::create_dir_all(&brain)?;
    let path = manifest_path(workspace);
    let mut m = manifest.clone();
    m.version = 2;
    m.workspace = workspace.display().to_string();
    let text = serde_json::to_string_pretty(&m)?;
    fs::write(path, text + "\n")?;
    Ok(())
}

/// Ensure a workspace.json exists (single mode) without enabling multi.
pub fn ensure_manifest(workspace: &Path) -> Result<WorkspaceManifest> {
    let path = manifest_path(workspace);
    if path.is_file() {
        return load_manifest(workspace);
    }
    let m = WorkspaceManifest::single(workspace);
    save_manifest(workspace, &m)?;
    Ok(m)
}

// ── Cargo discovery ─────────────────────────────────────────────────────────

/// One discovered Cargo workspace member.
#[derive(Debug, Clone)]
pub struct CargoMember {
    /// Path-stable scope id (last segment of member path, sanitized).
    pub id: String,
    /// Workspace-relative member directory.
    pub root: String,
    /// Package name from member Cargo.toml if present.
    pub package_name: Option<String>,
}

/// Discover `[workspace].members` from root `Cargo.toml` (no recursion into nested workspaces).
pub fn discover_cargo_members(workspace: &Path) -> Result<Vec<CargoMember>> {
    let cargo = workspace.join("Cargo.toml");
    if !cargo.is_file() {
        return Ok(Vec::new());
    }
    let text = fs::read_to_string(&cargo)?;
    let value: toml::Value = text
        .parse()
        .map_err(|e| BrainError::other(format!("Cargo.toml: {e}")))?;
    let Some(members) = value
        .get("workspace")
        .and_then(|w| w.get("members"))
        .and_then(|m| m.as_array())
    else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for m in members {
        let Some(pat) = m.as_str() else { continue };
        // Simple globs: "crates/*" or exact path
        let paths = expand_member_pattern(workspace, pat)?;
        for rel in paths {
            let id = path_scope_id(&rel);
            if !seen.insert(id.clone()) {
                continue;
            }
            let pkg = read_package_name(&workspace.join(&rel));
            out.push(CargoMember {
                id,
                root: normalize_rel(&rel),
                package_name: pkg,
            });
        }
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

fn expand_member_pattern(workspace: &Path, pat: &str) -> Result<Vec<String>> {
    let pat = pat.trim().trim_matches('/').replace('\\', "/");
    if let Some(parent) = pat.strip_suffix("/*") {
        let dir = workspace.join(parent);
        if !dir.is_dir() {
            return Ok(Vec::new());
        }
        let mut v = Vec::new();
        for ent in fs::read_dir(&dir)? {
            let ent = ent?;
            if ent.file_type()?.is_dir() {
                let name = ent.file_name().to_string_lossy().to_string();
                if name.starts_with('.') {
                    continue;
                }
                v.push(format!("{}/{}", normalize_rel(parent), name));
            }
        }
        v.sort();
        return Ok(v);
    }
    if workspace.join(&pat).is_dir() {
        Ok(vec![normalize_rel(&pat)])
    } else {
        Ok(Vec::new())
    }
}

fn path_scope_id(rel: &str) -> String {
    let rel = normalize_rel(rel);
    let last = rel.rsplit('/').next().unwrap_or(&rel);
    // Prefer path segment; keep dots for crate names like foo.bar rarely
    let id = last.to_ascii_lowercase().replace('_', "-");
    if id.is_empty() {
        "crate".into()
    } else {
        id
    }
}

fn read_package_name(member_dir: &Path) -> Option<String> {
    let cargo = member_dir.join("Cargo.toml");
    let text = fs::read_to_string(cargo).ok()?;
    let value: toml::Value = text.parse().ok()?;
    value
        .get("package")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        .map(|s| s.to_string())
}

/// Apply Cargo members into manifest scopes (replaces prior cargo-workspace sources).
pub fn apply_cargo_discovery(manifest: &mut WorkspaceManifest, members: &[CargoMember]) {
    manifest.scopes.retain(|s| s.source != ScopeSource::CargoWorkspace);
    for m in members {
        if m.id == manifest.main_id {
            continue;
        }
        if manifest.find_scope(&m.id).is_some() {
            // Keep manual overrides; only fill if missing
            continue;
        }
        let mut aliases = Vec::new();
        if let Some(pkg) = &m.package_name {
            let a = pkg.to_ascii_lowercase().replace('_', "-");
            if a != m.id {
                aliases.push(a);
            }
        }
        manifest.scopes.push(ScopeDef {
            id: m.id.clone(),
            roots: vec![m.root.clone()],
            aliases,
            source: ScopeSource::CargoWorkspace,
        });
    }
    manifest.discovery.cargo_workspace = true;
}

// ── Enable / disable / mutate ───────────────────────────────────────────────

/// Enable multi-brain mode. Optionally discover Cargo members.
pub fn enable_multi(workspace: &Path, discover_cargo: bool) -> Result<WorkspaceManifest> {
    let mut m = load_manifest(workspace)?;
    m.mode = BrainMode::Multi;
    m.version = 2;
    if discover_cargo {
        let members = discover_cargo_members(workspace)?;
        apply_cargo_discovery(&mut m, &members);
    }
    save_manifest(workspace, &m)?;
    Ok(m)
}

/// Disable multi-brain (single mode). Does not change node scopes until absorb/re-sync.
pub fn disable_multi(workspace: &Path, clear_scopes: bool) -> Result<WorkspaceManifest> {
    let mut m = load_manifest(workspace)?;
    m.mode = BrainMode::Single;
    if clear_scopes {
        m.scopes.clear();
        m.discovery.cargo_workspace = false;
    }
    save_manifest(workspace, &m)?;
    Ok(m)
}

/// Add or replace a SubBrain root.
pub fn add_scope(
    workspace: &Path,
    id: &str,
    roots: &[String],
    aliases: &[String],
    source: ScopeSource,
) -> Result<WorkspaceManifest> {
    let id = sanitize_scope_id(id)?;
    if roots.is_empty() {
        return Err(BrainError::other("at least one --root is required"));
    }
    let mut m = load_manifest(workspace)?;
    if m.mode != BrainMode::Multi {
        m.mode = BrainMode::Multi;
    }
    let roots: Vec<String> = roots.iter().map(|r| normalize_rel(r)).collect();
    for r in &roots {
        let abs = workspace.join(r);
        if !abs.exists() {
            return Err(BrainError::other(format!(
                "root does not exist: {r} (under {})",
                workspace.display()
            )));
        }
    }
    if let Some(existing) = m.find_scope_mut(&id) {
        existing.roots = roots;
        for a in aliases {
            if !existing.aliases.contains(a) {
                existing.aliases.push(a.clone());
            }
        }
        existing.source = source;
    } else {
        m.scopes.push(ScopeDef {
            id: id.clone(),
            roots,
            aliases: aliases.to_vec(),
            source,
        });
    }
    save_manifest(workspace, &m)?;
    Ok(m)
}

/// Remove a SubBrain from the manifest (does not rewrite nodes).
pub fn remove_scope_def(workspace: &Path, id: &str) -> Result<WorkspaceManifest> {
    let mut m = load_manifest(workspace)?;
    let before = m.scopes.len();
    m.scopes.retain(|s| s.id != id && !s.aliases.iter().any(|a| a == id));
    if m.scopes.len() == before {
        return Err(BrainError::other(format!("unknown scope {id:?}")));
    }
    save_manifest(workspace, &m)?;
    Ok(m)
}

/// Reassign all nodes with `from_scope` → `to_scope` in SQLite.
pub fn reassign_node_scopes(db: &Database, from_scope: &str, to_scope: &str) -> Result<usize> {
    let n = db.conn.execute(
        "UPDATE nodes SET scope = ?1 WHERE scope = ?2",
        rusqlite::params![to_scope, from_scope],
    )?;
    Ok(n)
}

/// Absorb a SubBrain into MainBrain: reassign nodes + drop scope def.
///
/// Order: DB reassignment first (idempotent), then manifest update so a crash
/// mid-way never leaves a live SubBrain def with nodes already on main.
pub fn absorb_scope(workspace: &Path, db: &Database, id: &str) -> Result<AbsorbReport> {
    let m = load_manifest(workspace)?;
    let sc = m
        .find_scope(id)
        .ok_or_else(|| BrainError::other(format!("unknown scope {id:?}")))?;
    let scope_id = sc.id.clone();
    let main = m.main_id.clone();
    let mut stmt = db
        .conn
        .prepare("SELECT id FROM nodes WHERE scope = ?1")?;
    let ids: Vec<String> = stmt
        .query_map([&scope_id], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();
    let n = reassign_node_scopes(db, &scope_id, &main)?;
    for nid in &ids {
        db.set_node_scope(nid, &main)?;
    }
    let m = remove_scope_def(workspace, &scope_id)?;
    Ok(AbsorbReport {
        absorbed_id: scope_id,
        nodes_reassigned: n,
        manifest: m,
    })
}

/// Report from absorb.
#[derive(Debug, Clone)]
pub struct AbsorbReport {
    /// Scope that was removed.
    pub absorbed_id: String,
    /// Rows updated in SQLite.
    pub nodes_reassigned: usize,
    /// Resulting manifest.
    pub manifest: WorkspaceManifest,
}

/// Absorb **all** SubBrains into main and set single mode.
pub fn absorb_all_to_main(workspace: &Path, db: &Database) -> Result<AbsorbAllReport> {
    let m = load_manifest(workspace)?;
    let ids: Vec<String> = m.scopes.iter().map(|s| s.id.clone()).collect();
    let mut total = 0usize;
    let mut touched: Vec<String> = Vec::new();
    for id in &ids {
        let mut stmt = db
            .conn
            .prepare("SELECT id FROM nodes WHERE scope = ?1")?;
        for nid in stmt
            .query_map([id.as_str()], |row| row.get::<_, String>(0))?
            .flatten()
        {
            touched.push(nid);
        }
        total += reassign_node_scopes(db, id, MAIN_SCOPE)?;
    }
    // Stray scopes not in manifest
    let mut stmt = db
        .conn
        .prepare("SELECT id FROM nodes WHERE scope != ?1")?;
    for nid in stmt
        .query_map([MAIN_SCOPE], |row| row.get::<_, String>(0))?
        .flatten()
    {
        touched.push(nid);
    }
    total += db.conn.execute(
        "UPDATE nodes SET scope = ?1 WHERE scope != ?1",
        rusqlite::params![MAIN_SCOPE],
    )?;
    touched.sort();
    touched.dedup();
    for nid in &touched {
        db.set_node_scope(nid, MAIN_SCOPE)?;
    }
    let mut m = disable_multi(workspace, true)?;
    m.mode = BrainMode::Single;
    save_manifest(workspace, &m)?;
    Ok(AbsorbAllReport {
        scopes_removed: ids,
        nodes_reassigned: total,
        manifest: m,
    })
}

// ── Reconcile / attach (umbrella workspaces) ────────────────────────────────

/// Report from [`reconcile_scopes`].
#[derive(Debug, Clone, Serialize)]
pub struct ReconcileReport {
    /// Nodes whose scope column changed.
    pub updated: usize,
    /// Nodes already correct.
    pub unchanged: usize,
    /// Scope labels present in DB but not in manifest (reassigned to main when multi,
    /// or all → main when single).
    pub orphan_scopes_cleared: Vec<String>,
    /// Manifest mode after reconcile.
    pub mode: String,
}

/// Recompute every node's owner scope from the current manifest (+ optional frontmatter).
///
/// Run after `enable` / `add` / `attach` / moving roots, or when doctor reports orphans.
pub fn reconcile_scopes(workspace: &Path, db: &Database) -> Result<ReconcileReport> {
    let m = load_manifest(workspace)?;
    let mut updated = 0usize;
    let mut unchanged = 0usize;
    let mut stmt = db.conn.prepare(
        "SELECT id, file_path, scope FROM nodes",
    )?;
    let rows: Vec<(String, Option<String>, String)> = stmt
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get::<_, Option<String>>(2)?
                    .unwrap_or_else(|| MAIN_SCOPE.to_string()),
            ))
        })?
        .filter_map(|r| r.ok())
        .collect();

    let known: BTreeSet<String> = m.all_scope_ids().into_iter().collect();
    let mut orphan_scopes: BTreeSet<String> = BTreeSet::new();

    for (id, file_path, old_scope) in rows {
        let expected = if !m.is_multi() {
            m.main_id.clone()
        } else if let Some(ref fp) = file_path {
            let path_scope = m.resolve_scope(fp);
            // Frontmatter override when multi
            if let Some(fm) = read_frontmatter_scope(workspace, fp) {
                if known.contains(&fm) || fm == m.main_id {
                    fm
                } else {
                    path_scope
                }
            } else {
                path_scope
            }
        } else {
            // symbols without path: keep main unless already a known scope
            if known.contains(&old_scope) {
                old_scope.clone()
            } else {
                m.main_id.clone()
            }
        };

        if !known.contains(&old_scope) && old_scope != m.main_id {
            orphan_scopes.insert(old_scope.clone());
        }

        if expected != old_scope {
            db.set_node_scope(&id, &expected)?;
            updated += 1;
        } else {
            unchanged += 1;
        }
    }

    Ok(ReconcileReport {
        updated,
        unchanged,
        orphan_scopes_cleared: orphan_scopes.into_iter().collect(),
        mode: m.mode.as_str().to_string(),
    })
}

fn read_frontmatter_scope(workspace: &Path, rel: &str) -> Option<String> {
    #[cfg(feature = "obsidian")]
    {
        let path = workspace.join(rel);
        let text = fs::read_to_string(path).ok()?;
        let (fm, _) = crate::obsidian::parse_frontmatter(&text);
        let fm = fm?;
        fm.extra
            .get("scope")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_ascii_lowercase().replace('_', "-"))
            .filter(|s| !s.is_empty())
    }
    #[cfg(not(feature = "obsidian"))]
    {
        let _ = (workspace, rel);
        None
    }
}

/// Attach an existing directory under the workspace as a SubBrain **without copying**.
///
/// Umbrella pattern: three former mono-repos under one folder become SubBrains of a
/// new MainBrain:
///
/// ```text
/// umbrella/           ← rustbrain scopes enable --empty
///   .brain/           ← new MainBrain
///   project-a/        ← scopes attach project-a --root project-a
///   project-b/
///   project-c/
/// ```
///
/// Nested `project-a/.brain` may still exist for working inside that tree alone
/// (`Brain::open` finds the nearest `.brain`). The umbrella indexes path ownership.
pub fn attach_subbrain(
    workspace: &Path,
    id: &str,
    root: &str,
    aliases: &[String],
) -> Result<WorkspaceManifest> {
    let root = normalize_rel(root);
    let abs = workspace.join(&root);
    if !abs.is_dir() {
        return Err(BrainError::other(format!(
            "attach root does not exist or is not a directory: {root}"
        )));
    }
    // Refuse attaching the workspace root itself as a SubBrain.
    if root.is_empty() || root == "." {
        return Err(BrainError::other(
            "cannot attach workspace root as SubBrain — use MainBrain",
        ));
    }
    add_scope(
        workspace,
        id,
        &[root],
        aliases,
        ScopeSource::ExtraRoot,
    )
}

/// Report for absorb-all.
#[derive(Debug, Clone)]
pub struct AbsorbAllReport {
    /// Removed SubBrain ids.
    pub scopes_removed: Vec<String>,
    /// Nodes reassigned.
    pub nodes_reassigned: usize,
    /// Resulting manifest.
    pub manifest: WorkspaceManifest,
}

// ── Import another brain ────────────────────────────────────────────────────

/// Default max Markdown files copied in one import (DoS guard).
pub const IMPORT_DEFAULT_MAX_FILES: usize = 5_000;
/// Default max total bytes copied in one import.
pub const IMPORT_DEFAULT_MAX_BYTES: u64 = 50 * 1024 * 1024;

/// Options for importing one workspace brain into another.
#[derive(Debug, Clone)]
pub struct ImportBrainOptions {
    /// Destination scope id. Use [`MAIN_SCOPE`] to **merge** into MainBrain.
    /// Any other id registers/keeps a **separate SubBrain**.
    pub into_scope: String,
    /// Relative dest root for copied Markdown (default: `docs/subbrains/<id>` or `docs/imported`).
    pub dest_root: Option<String>,
    /// Copy Markdown notes from source workspace (ignored when `mount` succeeds).
    pub copy_markdown: bool,
    /// Overwrite existing files at destination.
    pub force: bool,
    /// If source lies under target, register that path as SubBrain root without copying.
    /// Umbrella: `scopes import --from ./project-a --as project-a --mount`.
    pub mount: bool,
    /// Max `.md` files to copy (default [`IMPORT_DEFAULT_MAX_FILES`]).
    pub max_files: usize,
    /// Max total bytes to copy (default [`IMPORT_DEFAULT_MAX_BYTES`]).
    pub max_bytes: u64,
}

impl Default for ImportBrainOptions {
    fn default() -> Self {
        Self {
            into_scope: MAIN_SCOPE.to_string(),
            dest_root: None,
            copy_markdown: true,
            force: false,
            mount: false,
            max_files: IMPORT_DEFAULT_MAX_FILES,
            max_bytes: IMPORT_DEFAULT_MAX_BYTES,
        }
    }
}

/// Result of importing a brain.
#[derive(Debug, Clone, Serialize)]
pub struct ImportBrainReport {
    /// Source workspace path.
    pub source: String,
    /// Target scope id.
    pub into_scope: String,
    /// Files copied.
    pub files_copied: usize,
    /// Files skipped (exist, no force).
    pub files_skipped: usize,
    /// Destination or mount root used.
    pub dest_root: String,
    /// Whether a SubBrain was registered.
    pub scope_registered: bool,
    /// True when source was attached in-place (no file copy).
    pub mounted: bool,
    /// Bytes copied (0 when mounted).
    pub bytes_copied: u64,
}

/// Import knowledge from `source_workspace` into `target_workspace`.
///
/// | Mode | Behavior |
/// |------|----------|
/// | `--as id` (copy) | Copy Markdown under `docs/subbrains/id/`; SubBrain stays separate |
/// | `--as id --mount` | If source is under target, attach path as root (umbrella, no copy) |
/// | `--into main` | **Merge** copy into MainBrain tree (`docs/imported/` by default) |
///
/// Nested source `.brain/` is **not** deleted; mount keeps projects shareable and
/// independently openable. Call `sync` + `scopes reconcile` after.
pub fn import_brain(
    target_workspace: &Path,
    source_workspace: &Path,
    opts: &ImportBrainOptions,
) -> Result<ImportBrainReport> {
    let source_workspace = source_workspace
        .canonicalize()
        .unwrap_or_else(|_| source_workspace.to_path_buf());
    let target_workspace = target_workspace
        .canonicalize()
        .unwrap_or_else(|_| target_workspace.to_path_buf());
    if source_workspace == target_workspace {
        return Err(BrainError::other(
            "cannot import a workspace into itself — use scopes attach/absorb instead",
        ));
    }
    if !source_workspace.is_dir() {
        return Err(BrainError::other(format!(
            "source workspace not found: {}",
            source_workspace.display()
        )));
    }

    let into_main = opts.into_scope == MAIN_SCOPE || opts.into_scope.eq_ignore_ascii_case("main");
    let scope_id = if into_main {
        MAIN_SCOPE.to_string()
    } else {
        sanitize_scope_id(&opts.into_scope)?
    };

    // Mount path: source is a subdirectory of target → attach without copy.
    if opts.mount {
        if into_main {
            return Err(BrainError::other(
                "--mount cannot target main; use --as <subbrain-id> to keep it separate",
            ));
        }
        let rel = source_workspace
            .strip_prefix(&target_workspace)
            .map_err(|_| {
                BrainError::other(format!(
                    "mount requires source under target workspace (source={}, target={}). \
                     Move/clone the project under the umbrella, or omit --mount to copy.",
                    source_workspace.display(),
                    target_workspace.display()
                ))
            })?;
        let root = normalize_rel(&rel.to_string_lossy());
        let m = attach_subbrain(&target_workspace, &scope_id, &root, &[])?;
        let _ = m;
        return Ok(ImportBrainReport {
            source: source_workspace.display().to_string(),
            into_scope: scope_id,
            files_copied: 0,
            files_skipped: 0,
            dest_root: root,
            scope_registered: true,
            mounted: true,
            bytes_copied: 0,
        });
    }

    let dest_rel = opts.dest_root.clone().unwrap_or_else(|| {
        if into_main {
            "docs/imported".into()
        } else {
            format!("docs/subbrains/{scope_id}")
        }
    });
    let dest_rel = normalize_rel(&dest_rel);
    let dest_abs = target_workspace.join(&dest_rel);
    fs::create_dir_all(&dest_abs)?;

    let mut files_copied = 0usize;
    let mut files_skipped = 0usize;
    let mut bytes_copied = 0u64;

    if opts.copy_markdown {
        copy_markdown_tree(
            &source_workspace,
            &source_workspace,
            &dest_abs,
            opts.force,
            opts.max_files,
            opts.max_bytes,
            &mut files_copied,
            &mut files_skipped,
            &mut bytes_copied,
        )?;
    }

    let mut scope_registered = false;
    if !into_main {
        add_scope(
            &target_workspace,
            &scope_id,
            &[dest_rel.clone()],
            &[],
            ScopeSource::Imported,
        )?;
        scope_registered = true;
    } else {
        let _ = ensure_manifest(&target_workspace)?;
    }

    Ok(ImportBrainReport {
        source: source_workspace.display().to_string(),
        into_scope: scope_id,
        files_copied,
        files_skipped,
        dest_root: dest_rel,
        scope_registered,
        mounted: false,
        bytes_copied,
    })
}

fn copy_markdown_tree(
    source_root: &Path,
    dir: &Path,
    dest_root: &Path,
    force: bool,
    max_files: usize,
    max_bytes: u64,
    copied: &mut usize,
    skipped: &mut usize,
    bytes: &mut u64,
) -> Result<()> {
    let skip_names = [
        ".brain",
        "target",
        ".git",
        "node_modules",
        "vendor",
        "dist",
        "build",
        ".next",
    ];
    for ent in fs::read_dir(dir)? {
        let ent = ent?;
        let path = ent.path();
        let name = ent.file_name().to_string_lossy().to_string();
        if skip_names.iter().any(|s| *s == name) {
            continue;
        }
        if path.is_dir() {
            copy_markdown_tree(
                source_root,
                &path,
                dest_root,
                force,
                max_files,
                max_bytes,
                copied,
                skipped,
                bytes,
            )?;
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        if *copied >= max_files {
            return Err(BrainError::other(format!(
                "import exceeded max_files={max_files}; raise limit or narrow source"
            )));
        }
        let meta = fs::metadata(&path)?;
        let len = meta.len();
        if *bytes + len > max_bytes {
            return Err(BrainError::other(format!(
                "import exceeded max_bytes={max_bytes}; raise limit or narrow source"
            )));
        }
        let rel = path
            .strip_prefix(source_root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let dest = dest_root.join(&rel);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        if dest.exists() && !force {
            *skipped += 1;
            continue;
        }
        fs::copy(&path, &dest)?;
        *copied += 1;
        *bytes += len;
    }
    Ok(())
}

/// Counts of nodes per scope (from SQLite).
pub fn count_nodes_by_scope(db: &Database) -> Result<BTreeMap<String, usize>> {
    let mut stmt = db
        .conn
        .prepare("SELECT scope, COUNT(*) FROM nodes GROUP BY scope ORDER BY scope")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, usize>(1)?))
    })?;
    let mut map = BTreeMap::new();
    for r in rows {
        let (s, n) = r?;
        map.insert(s, n);
    }
    Ok(map)
}

/// Human-readable scopes listing.
pub fn format_scopes_text(
    manifest: &WorkspaceManifest,
    counts: &BTreeMap<String, usize>,
) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "mode: {}  main: {}\n",
        manifest.mode.as_str(),
        manifest.main_id
    ));
    if !manifest.is_multi() {
        out.push_str("multi-brain: off (all nodes are MainBrain)\n");
        out.push_str("enable: rustbrain scopes enable --cargo   # or --empty\n");
        if let Some(n) = counts.get(MAIN_SCOPE).or_else(|| counts.values().next()) {
            out.push_str(&format!("nodes: {n}\n"));
        }
        return out;
    }
    let main_n = counts.get(&manifest.main_id).copied().unwrap_or(0);
    out.push_str(&format!(
        "  [{:>6} nodes]  {}  (MainBrain)\n",
        main_n, manifest.main_id
    ));
    if manifest.scopes.is_empty() {
        out.push_str("  (no SubBrains yet — scopes add <id> --root <path>)\n");
    }
    for sc in &manifest.scopes {
        let n = counts.get(&sc.id).copied().unwrap_or(0);
        out.push_str(&format!(
            "  [{:>6} nodes]  {}  roots={}  source={}\n",
            n,
            sc.id,
            sc.roots.join(","),
            sc.source.as_str()
        ));
        if !sc.aliases.is_empty() {
            out.push_str(&format!("               aliases: {}\n", sc.aliases.join(", ")));
        }
    }
    // Stray scopes in DB not in manifest
    for (s, n) in counts {
        if s != &manifest.main_id && manifest.find_scope(s).is_none() {
            out.push_str(&format!(
                "  [{n:>6} nodes]  {s}  (orphan scope in DB — absorb or re-sync)\n"
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn longest_prefix_wins() {
        let mut m = WorkspaceManifest::single(Path::new("/tmp/ws"));
        m.mode = BrainMode::Multi;
        m.scopes.push(ScopeDef {
            id: "core".into(),
            roots: vec!["crates/core".into()],
            aliases: vec![],
            source: ScopeSource::Manual,
        });
        m.scopes.push(ScopeDef {
            id: "cli".into(),
            roots: vec!["crates/cli".into()],
            aliases: vec![],
            source: ScopeSource::Manual,
        });
        assert_eq!(m.resolve_scope("crates/core/src/lib.rs"), "core");
        assert_eq!(m.resolve_scope("crates/cli/src/main.rs"), "cli");
        assert_eq!(m.resolve_scope("docs/adr/x.md"), "main");
        assert_eq!(m.resolve_scope("README.md"), "main");
    }

    #[test]
    fn single_mode_always_main() {
        let m = WorkspaceManifest::single(Path::new("/tmp/ws"));
        assert_eq!(m.resolve_scope("crates/core/src/lib.rs"), "main");
    }

    #[test]
    fn sanitize_rejects_main() {
        assert!(sanitize_scope_id("main").is_err());
        assert_eq!(sanitize_scope_id("RustBrain_Core").unwrap(), "rustbrain-core");
    }

    #[test]
    fn roundtrip_manifest() {
        let dir = tempdir().unwrap();
        let ws = dir.path();
        fs::create_dir_all(ws.join(".brain")).unwrap();
        fs::create_dir_all(ws.join("crates/foo")).unwrap();
        let mut m = enable_multi(ws, false).unwrap();
        assert!(m.is_multi());
        m = add_scope(
            ws,
            "foo",
            &["crates/foo".into()],
            &["pkg-foo".into()],
            ScopeSource::Manual,
        )
        .unwrap();
        assert_eq!(m.scopes.len(), 1);
        let loaded = load_manifest(ws).unwrap();
        assert_eq!(loaded.scopes[0].id, "foo");
    }

    #[test]
    fn cargo_discover_this_repo_shape() {
        // Pattern expansion unit: create mini workspace
        let dir = tempdir().unwrap();
        let ws = dir.path();
        fs::create_dir_all(ws.join("crates/a")).unwrap();
        fs::create_dir_all(ws.join("crates/b")).unwrap();
        fs::write(
            ws.join("Cargo.toml"),
            r#"[workspace]
members = ["crates/*"]
"#,
        )
        .unwrap();
        fs::write(
            ws.join("crates/a/Cargo.toml"),
            r#"[package]
name = "pkg_a"
version = "0.1.0"
"#,
        )
        .unwrap();
        fs::write(
            ws.join("crates/b/Cargo.toml"),
            r#"[package]
name = "b"
version = "0.1.0"
"#,
        )
        .unwrap();
        let members = discover_cargo_members(ws).unwrap();
        assert_eq!(members.len(), 2);
        let a = members.iter().find(|m| m.id == "a").unwrap();
        assert_eq!(a.package_name.as_deref(), Some("pkg_a"));
    }

    #[test]
    fn multi_scope_index_and_query_filter() {
        use crate::{Brain, QueryOptions};
        let dir = tempdir().unwrap();
        let ws = dir.path();
        fs::create_dir_all(ws.join("crates/core/src")).unwrap();
        fs::create_dir_all(ws.join("crates/cli/src")).unwrap();
        fs::create_dir_all(ws.join("docs/concepts")).unwrap();
        fs::write(
            ws.join("Cargo.toml"),
            r#"[workspace]
members = ["crates/*"]
"#,
        )
        .unwrap();
        fs::write(
            ws.join("crates/core/Cargo.toml"),
            r#"[package]
name = "core"
version = "0.1.0"
edition = "2021"
"#,
        )
        .unwrap();
        fs::write(
            ws.join("crates/cli/Cargo.toml"),
            r#"[package]
name = "cli"
version = "0.1.0"
edition = "2021"
"#,
        )
        .unwrap();
        fs::write(
            ws.join("crates/core/src/lib.rs"),
            "/// Storage engine core.\npub struct StorageEngine;\n",
        )
        .unwrap();
        fs::write(
            ws.join("crates/cli/src/main.rs"),
            "fn main() { println!(\"cli clap parse\"); }\n",
        )
        .unwrap();
        fs::write(
            ws.join("docs/concepts/shared.md"),
            "---\nnode_type: concept\n---\n# Shared\n\nShared note about clap and storage.\n",
        )
        .unwrap();
        fs::write(
            ws.join("crates/core/README.md"),
            "# Core crate\n\nStorage engine details only in core.\n",
        )
        .unwrap();

        let mut brain = Brain::create(ws).unwrap();
        enable_multi(ws, true).unwrap();
        brain.sync().unwrap();

        let counts = count_nodes_by_scope(brain.database()).unwrap();
        assert!(counts.get("core").copied().unwrap_or(0) >= 1);
        assert!(counts.get("cli").copied().unwrap_or(0) >= 1);

        let mut opts = QueryOptions::human();
        opts.scope = Some("core".into());
        opts.scope_main = crate::query::ScopeMainInclude::Strict;
        opts.limit = 20;
        let hits = brain.query_ranked("storage", &opts).unwrap();
        assert!(
            hits.iter().all(|h| h.node.scope == "core"),
            "strict core scope should not leak other scopes: {:?}",
            hits.iter().map(|h| (&h.node.id, &h.node.scope)).collect::<Vec<_>>()
        );

        // Absorb core into main
        let rep = absorb_scope(ws, brain.database(), "core").unwrap();
        assert!(rep.nodes_reassigned >= 1);
        let m = load_manifest(ws).unwrap();
        assert!(m.find_scope("core").is_none());
    }

    #[test]
    fn import_brain_as_subbrain_copies_md() {
        let src = tempdir().unwrap();
        let dst = tempdir().unwrap();
        fs::create_dir_all(src.path().join("docs")).unwrap();
        fs::write(
            src.path().join("docs/note.md"),
            "---\nnode_type: concept\n---\n# Imported Note\n\nHello from source brain.\n",
        )
        .unwrap();
        let report = import_brain(
            dst.path(),
            src.path(),
            &ImportBrainOptions {
                into_scope: "foreign".into(),
                dest_root: None,
                copy_markdown: true,
                force: false,
                mount: false,
                max_files: IMPORT_DEFAULT_MAX_FILES,
                max_bytes: IMPORT_DEFAULT_MAX_BYTES,
            },
        )
        .unwrap();
        assert_eq!(report.files_copied, 1);
        assert!(report.scope_registered);
        assert!(!report.mounted);
        assert!(dst
            .path()
            .join("docs/subbrains/foreign/docs/note.md")
            .is_file());
        let m = load_manifest(dst.path()).unwrap();
        assert!(m.is_multi());
        assert!(m.find_scope("foreign").is_some());
    }

    #[test]
    fn umbrella_mount_three_former_mainbrains() {
        let umbrella = tempdir().unwrap();
        let u = umbrella.path();
        for name in ["alpha", "beta", "gamma"] {
            fs::create_dir_all(u.join(name).join("docs")).unwrap();
            fs::write(
                u.join(name).join("docs/note.md"),
                format!("---\nnode_type: concept\n---\n# {name}\n\nNote in {name}.\n"),
            )
            .unwrap();
            // Nested former brain marker (still valid for local open)
            fs::create_dir_all(u.join(name).join(".brain")).unwrap();
            fs::write(
                u.join(name).join(".brain/workspace.json"),
                r#"{"version":1,"workspace":"nested"}"#,
            )
            .unwrap();
        }
        use crate::Brain;
        let mut brain = Brain::create(u).unwrap();
        enable_multi(u, false).unwrap();
        for name in ["alpha", "beta", "gamma"] {
            import_brain(
                u,
                &u.join(name),
                &ImportBrainOptions {
                    into_scope: name.into(),
                    mount: true,
                    copy_markdown: false,
                    ..Default::default()
                },
            )
            .unwrap();
        }
        let m = load_manifest(u).unwrap();
        assert_eq!(m.scopes.len(), 3);
        assert_eq!(m.resolve_scope("alpha/docs/note.md"), "alpha");
        assert_eq!(m.resolve_scope("beta/docs/note.md"), "beta");
        assert_eq!(m.resolve_scope("docs/x.md"), "main");

        brain.sync().unwrap();
        let rep = reconcile_scopes(u, brain.database()).unwrap();
        assert!(rep.updated + rep.unchanged > 0);
        let counts = count_nodes_by_scope(brain.database()).unwrap();
        assert!(counts.get("alpha").copied().unwrap_or(0) >= 1);
        assert!(counts.get("beta").copied().unwrap_or(0) >= 1);
    }
}
