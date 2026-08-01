//! Harvest **crates.io → docs.rs** links from `Cargo.toml` (+ optional `Cargo.lock`).
//!
//! Used by bootstrap/setup so agents can `rustbrain query serde` and land on a
//! note with the correct docs.rs URL. Purely algorithmic — no network fetch of
//! docs HTML (URLs follow the public docs.rs convention).
//!
//! ## What is included
//!
//! - Direct deps from every `Cargo.toml` under the workspace (skip `target/`, …)
//! - Sections: `[dependencies]`, `[dev-dependencies]`, `[build-dependencies]`,
//!   and `[workspace.dependencies]`
//! - Exact versions from `Cargo.lock` when present (better docs.rs deep links)
//!
//! ## What is skipped
//!
//! - `path = "…"` dependencies (local crates — not on docs.rs)
//! - Renamed packages use the **package** name for docs.rs, not the key
//!
//! ## Output
//!
//! - `docs/references/crate-docs.generated.md` — index of all harvested deps
//! - `docs/references/crates/{crate}.md` — one note per crates.io package
//!   (`node_type: reference`, `generated: true`)

use crate::error::Result;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// One direct dependency harvested from a manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrateDep {
    /// crates.io / docs.rs package name.
    pub name: String,
    /// Version requirement from Cargo.toml (if any).
    pub req: Option<String>,
    /// Exact version from Cargo.lock (if any).
    pub version_exact: Option<String>,
    /// `normal` | `dev` | `build` | `workspace`.
    pub kind: String,
    /// Relative path of the Cargo.toml that declared it.
    pub declared_in: String,
}

/// docs.rs URL for a package (optionally pinned to an exact version).
pub fn docs_rs_url(name: &str, version_exact: Option<&str>) -> String {
    let name = name.trim();
    if let Some(v) = version_exact.map(str::trim).filter(|s| !s.is_empty()) {
        format!("https://docs.rs/{name}/{v}")
    } else {
        format!("https://docs.rs/{name}")
    }
}

/// crates.io URL for a package.
pub fn crates_io_url(name: &str) -> String {
    format!("https://crates.io/crates/{}", name.trim())
}

/// Collect unique crates.io dependencies from a workspace tree.
pub fn collect_crate_deps(workspace: &Path) -> Result<Vec<CrateDep>> {
    let mut by_name: BTreeMap<String, CrateDep> = BTreeMap::new();
    let lock_versions = parse_lock_versions(&workspace.join("Cargo.lock"));

    let mut manifests = Vec::new();
    find_cargo_tomls(workspace, workspace, &mut manifests)?;

    for manifest in &manifests {
        let rel = manifest
            .strip_prefix(workspace)
            .unwrap_or(manifest)
            .to_string_lossy()
            .replace('\\', "/");
        let text = match std::fs::read_to_string(manifest) {
            Ok(t) => t,
            Err(_) => continue,
        };
        let Ok(value) = text.parse::<toml::Value>() else {
            continue;
        };
        harvest_table(
            &value,
            "dependencies",
            "normal",
            &rel,
            &lock_versions,
            &mut by_name,
        );
        harvest_table(
            &value,
            "dev-dependencies",
            "dev",
            &rel,
            &lock_versions,
            &mut by_name,
        );
        harvest_table(
            &value,
            "build-dependencies",
            "build",
            &rel,
            &lock_versions,
            &mut by_name,
        );
        if let Some(ws) = value.get("workspace").and_then(|v| v.as_table()) {
            if let Some(deps) = ws.get("dependencies") {
                harvest_value_table(
                    deps,
                    "workspace",
                    &rel,
                    &lock_versions,
                    &mut by_name,
                );
            }
        }
    }

    Ok(by_name.into_values().collect())
}

/// Write index + per-crate notes under `docs/references/`.
///
/// Returns number of crate notes written/planned.
pub fn write_crate_docs_notes(
    workspace: &Path,
    deps: &[CrateDep],
    write: bool,
    force: bool,
) -> Result<(usize, Vec<String>)> {
    let mut actions = Vec::new();
    let refs_dir = workspace.join("docs/references/crates");
    if write {
        std::fs::create_dir_all(&refs_dir)?;
        std::fs::create_dir_all(workspace.join("docs/references"))?;
    }

    // Cap runaway trees (monorepos with huge trees still capped).
    let deps: Vec<&CrateDep> = deps.iter().take(300).collect();

    let mut index = String::from(
        "---\n\
         tags: [reference, generated, crates, docs-rs]\n\
         node_type: reference\n\
         aliases: [crate-docs, docs-rs, dependencies, crates-io]\n\
         generated: true\n\
         ---\n\
         # Crate docs (docs.rs) — generated\n\n\
         > Harvested from `Cargo.toml` (+ `Cargo.lock` versions when present) by\n\
         > `rustbrain bootstrap` / `setup`. **Not** a download of docs HTML — URLs only.\n\
         > Re-run with `--force` to refresh. Agents: `rustbrain query <crate> --scores`.\n\n",
    );

    if deps.is_empty() {
        index.push_str("_No crates.io dependencies found in this workspace._\n");
    } else {
        index.push_str(&format!("_{} unique crates.io package(s)._ \n\n", deps.len()));
        index.push_str("| Crate | docs.rs | Version | Kind |\n");
        index.push_str("|-------|---------|---------|------|\n");
        for d in &deps {
            let url = docs_rs_url(&d.name, d.version_exact.as_deref());
            let ver = d
                .version_exact
                .as_deref()
                .or(d.req.as_deref())
                .unwrap_or("—");
            index.push_str(&format!(
                "| [`{}`](./crates/{}.md) | [{url}]({url}) | `{ver}` | {} |\n",
                d.name,
                sanitize_filename(&d.name),
                d.kind
            ));
        }
        index.push('\n');
    }

    let index_rel = "docs/references/crate-docs.generated.md";
    let index_path = workspace.join(index_rel);
    write_generated_file(&index_path, index_rel, &index, write, force, &mut actions)?;

    let mut n = 0usize;
    for d in &deps {
        let body = render_crate_note(d);
        let rel = format!("docs/references/crates/{}.md", sanitize_filename(&d.name));
        let path = workspace.join(&rel);
        write_generated_file(&path, &rel, &body, write, force, &mut actions)?;
        n += 1;
    }

    Ok((n, actions))
}

fn render_crate_note(d: &CrateDep) -> String {
    let docs = docs_rs_url(&d.name, d.version_exact.as_deref());
    let crates = crates_io_url(&d.name);
    let ver_line = match (&d.version_exact, &d.req) {
        (Some(v), Some(r)) => format!("- **Resolved (lock):** `{v}`\n- **Requirement:** `{r}`\n"),
        (Some(v), None) => format!("- **Resolved (lock):** `{v}`\n"),
        (None, Some(r)) => format!("- **Requirement:** `{r}`\n"),
        (None, None) => String::new(),
    };
    format!(
        "---\n\
         tags: [reference, generated, crate, docs-rs, {name}]\n\
         node_type: reference\n\
         aliases: [{name}, crate:{name}, docs.rs/{name}]\n\
         generated: true\n\
         source: cargo-deps\n\
         ---\n\
         # Crate: `{name}`\n\n\
         > Generated by rustbrain from Cargo manifests. Re-run bootstrap `--force` to refresh.\n\n\
         ## Documentation\n\n\
         - **docs.rs:** [{docs}]({docs})\n\
         - **crates.io:** [{crates}]({crates})\n\
         {ver_line}\
         - **Declared as:** `{kind}` dependency\n\
         - **Manifest:** `{manifest}`\n\n\
         ## Agent tips\n\n\
         - Open the docs.rs URL for API reference (types, features, examples).\n\
         - Prefer this note over inventing crate APIs from memory.\n\
         - After upgrading the crate in Cargo.toml/lock, re-run `rustbrain bootstrap --yes --write --force` (or your setup path) and `sync`.\n",
        name = d.name,
        docs = docs,
        crates = crates,
        ver_line = ver_line,
        kind = d.kind,
        manifest = d.declared_in,
    )
}

fn write_generated_file(
    path: &Path,
    rel: &str,
    content: &str,
    write: bool,
    force: bool,
    actions: &mut Vec<String>,
) -> Result<()> {
    if path.exists() && !force {
        let existing = std::fs::read_to_string(path).unwrap_or_default();
        if existing.contains("generated: true") {
            // Still skip without force — same policy as module-map.
            actions.push(format!("skip {rel} (exists; use --force)"));
            return Ok(());
        }
        actions.push(format!("skip {rel} (exists, not marked generated)"));
        return Ok(());
    }
    if write {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, content)?;
        actions.push(format!("write {rel}"));
    } else {
        actions.push(format!("would_write {rel}"));
    }
    Ok(())
}

fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn find_cargo_tomls(workspace: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    let cargo = dir.join("Cargo.toml");
    if cargo.is_file() {
        out.push(cargo);
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if matches!(
            name,
            "target" | ".git" | ".brain" | "node_modules" | "vendor" | "dist" | "build"
        ) || name.starts_with('.')
        {
            continue;
        }
        // Stay under workspace
        if path.starts_with(workspace) {
            find_cargo_tomls(workspace, &path, out)?;
        }
    }
    Ok(())
}

fn harvest_table(
    root: &toml::Value,
    key: &str,
    kind: &str,
    rel: &str,
    lock: &BTreeMap<String, String>,
    out: &mut BTreeMap<String, CrateDep>,
) {
    if let Some(table) = root.get(key) {
        harvest_value_table(table, kind, rel, lock, out);
    }
}

fn harvest_value_table(
    table: &toml::Value,
    kind: &str,
    rel: &str,
    lock: &BTreeMap<String, String>,
    out: &mut BTreeMap<String, CrateDep>,
) {
    let Some(map) = table.as_table() else {
        return;
    };
    for (key, val) in map {
        if let Some(dep) = parse_dep_entry(key, val, kind, rel, lock) {
            // Prefer entry that has an exact version / merge kinds lightly.
            out.entry(dep.name.clone())
                .and_modify(|existing| {
                    if existing.version_exact.is_none() && dep.version_exact.is_some() {
                        existing.version_exact = dep.version_exact.clone();
                    }
                    if existing.req.is_none() && dep.req.is_some() {
                        existing.req = dep.req.clone();
                    }
                    // Prefer normal over workspace/dev labeling when both exist.
                    if existing.kind != "normal" && dep.kind == "normal" {
                        existing.kind = "normal".into();
                    }
                })
                .or_insert(dep);
        }
    }
}

fn parse_dep_entry(
    key: &str,
    val: &toml::Value,
    kind: &str,
    rel: &str,
    lock: &BTreeMap<String, String>,
) -> Option<CrateDep> {
    match val {
        toml::Value::String(req) => {
            let name = key.to_string();
            let version_exact = lock.get(&name).cloned();
            Some(CrateDep {
                name,
                req: Some(req.clone()),
                version_exact,
                kind: kind.into(),
                declared_in: rel.into(),
            })
        }
        toml::Value::Table(t) => {
            // Skip local path crates (not published docs).
            if t.contains_key("path") {
                return None;
            }
            let package = t
                .get("package")
                .and_then(|v| v.as_str())
                .unwrap_or(key)
                .to_string();
            let req = t
                .get("version")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            // Optional: still allow git deps — docs.rs may or may not host them.
            let version_exact = lock.get(&package).cloned();
            Some(CrateDep {
                name: package,
                req,
                version_exact,
                kind: kind.into(),
                declared_in: rel.into(),
            })
        }
        _ => None,
    }
}

fn parse_lock_versions(lock_path: &Path) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    let Ok(text) = std::fs::read_to_string(lock_path) else {
        return map;
    };
    // Cargo.lock is TOML with [[package]] arrays.
    let Ok(value) = text.parse::<toml::Value>() else {
        return map;
    };
    let Some(packages) = value.get("package").and_then(|v| v.as_array()) else {
        return map;
    };
    for pkg in packages {
        let name = pkg.get("name").and_then(|v| v.as_str());
        let ver = pkg.get("version").and_then(|v| v.as_str());
        if let (Some(n), Some(v)) = (name, ver) {
            // Keep first (usually unique per name+version pairs; multiple versions possible —
            // prefer the last written / highest by naive string is wrong; keep first seen then
            // upgrade if we see a longer patch-like version).
            map.entry(n.to_string())
                .and_modify(|old| {
                    if version_is_newer(v, old) {
                        *old = v.to_string();
                    }
                })
                .or_insert_with(|| v.to_string());
        }
    }
    map
}

fn version_is_newer(a: &str, b: &str) -> bool {
    // Simple numeric compare of dotted versions; fall back to string.
    let pa: Vec<u64> = a
        .split(|c: char| !c.is_ascii_digit())
        .filter_map(|s| s.parse().ok())
        .collect();
    let pb: Vec<u64> = b
        .split(|c: char| !c.is_ascii_digit())
        .filter_map(|s| s.parse().ok())
        .collect();
    if pa.is_empty() || pb.is_empty() {
        return a > b;
    }
    for i in 0..pa.len().max(pb.len()) {
        let x = pa.get(i).copied().unwrap_or(0);
        let y = pb.get(i).copied().unwrap_or(0);
        if x != y {
            return x > y;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn docs_rs_url_shapes() {
        assert_eq!(docs_rs_url("serde", None), "https://docs.rs/serde");
        assert_eq!(
            docs_rs_url("serde", Some("1.0.200")),
            "https://docs.rs/serde/1.0.200"
        );
    }

    #[test]
    fn harvests_simple_and_table_deps() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("Cargo.toml"),
            r#"[package]
name = "demo"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = "1.0"
tokio = { version = "1", features = ["full"] }
mine = { path = "../mine" }
renamed = { package = "once_cell", version = "1.19" }

[dev-dependencies]
tempfile = "3"
"#,
        )
        .unwrap();
        fs::write(
            dir.path().join("Cargo.lock"),
            r#"# This file is automatically @generated by Cargo.
version = 3

[[package]]
name = "once_cell"
version = "1.19.0"

[[package]]
name = "serde"
version = "1.0.210"

[[package]]
name = "tempfile"
version = "3.10.1"

[[package]]
name = "tokio"
version = "1.40.0"
"#,
        )
        .unwrap();

        let deps = collect_crate_deps(dir.path()).unwrap();
        let names: BTreeSet<_> = deps.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains("serde"));
        assert!(names.contains("tokio"));
        assert!(names.contains("once_cell"));
        assert!(names.contains("tempfile"));
        assert!(!names.contains("mine"), "path deps skipped");
        let serde = deps.iter().find(|d| d.name == "serde").unwrap();
        assert_eq!(serde.version_exact.as_deref(), Some("1.0.210"));
        assert_eq!(
            docs_rs_url("serde", serde.version_exact.as_deref()),
            "https://docs.rs/serde/1.0.210"
        );
    }

    #[test]
    fn writes_notes() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("Cargo.toml"),
            r#"[package]
name = "demo"
version = "0.1.0"
[dependencies]
serde = "1"
"#,
        )
        .unwrap();
        let deps = collect_crate_deps(dir.path()).unwrap();
        let (n, _) = write_crate_docs_notes(dir.path(), &deps, true, true).unwrap();
        assert!(n >= 1);
        assert!(dir
            .path()
            .join("docs/references/crate-docs.generated.md")
            .is_file());
        assert!(dir
            .path()
            .join("docs/references/crates/serde.md")
            .is_file());
        let note = fs::read_to_string(dir.path().join("docs/references/crates/serde.md")).unwrap();
        assert!(note.contains("https://docs.rs/serde"));
        assert!(note.contains("generated: true"));
        assert!(note.contains("node_type: reference"));
    }
}
