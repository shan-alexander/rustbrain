//! Orphan detection and soft auto-links (algorithmic, low weight).
//!
//! **Explicit** edges (`relates_to`, `anchors`, …) are user/agent-authored.
//! **Auto** edges use `relation_type` values prefixed with `auto_` and lower
//! weights so context hops prefer human links.

use crate::error::Result;
use crate::storage::Database;
use crate::types::{Edge, Node, NodeType};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Soft edge: same filename stem under different paths (e.g. goals/x.md ↔ concepts/x.md).
pub const REL_AUTO_FILENAME: &str = "auto_filename";
/// Soft edge: shared tag (excluding trivial type-only tags).
pub const REL_AUTO_TAG: &str = "auto_tag";

/// Default weight for filename-stem matches.
pub const WEIGHT_AUTO_FILENAME: f32 = 0.40;
/// Default weight for shared-tag matches.
pub const WEIGHT_AUTO_TAG: f32 = 0.25;

/// True when `relation_type` is a soft auto-link.
pub fn is_auto_relation(relation_type: &str) -> bool {
    relation_type.starts_with("auto_")
}

/// True when the relation is user/agent-authored (not auto).
pub fn is_explicit_relation(relation_type: &str) -> bool {
    !is_auto_relation(relation_type)
}

/// A note with no explicit (non-auto) graph edges.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrphanNote {
    /// Node id.
    pub id: String,
    /// Domain type.
    pub node_type: String,
    /// Title.
    pub title: String,
    /// Repo-relative path if known.
    pub file_path: Option<String>,
    /// Suggested soft links (not yet written unless auto-link is run).
    pub suggestions: Vec<AutoLinkSuggestion>,
}

/// One proposed or applied soft edge endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoLinkSuggestion {
    /// Other node id.
    pub target_id: String,
    /// Other node title (best-effort).
    pub target_title: String,
    /// Other path if known.
    pub target_path: Option<String>,
    /// `auto_filename` / `auto_tag` / …
    pub relation_type: String,
    /// Soft weight.
    pub weight: f32,
    /// Human-readable reason.
    pub reason: String,
}

/// Report from [`run_auto_link`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoLinkReport {
    /// Edges inserted or updated this run.
    pub edges_upserted: usize,
    /// Unique undirected pairs considered.
    pub pairs_considered: usize,
    /// Target scope description.
    pub scope: String,
    /// Sample of applied links (up to 50).
    pub applied: Vec<AutoLinkSuggestion>,
}

/// Filename stem for auto-filename matching (`docs/goals/rust-fluency.md` → `rust-fluency`).
pub fn path_stem(file_path: &str) -> Option<String> {
    let p = Path::new(file_path);
    let stem = p.file_stem()?.to_str()?.to_ascii_lowercase();
    if stem.is_empty() || stem == "readme" || stem == "template" {
        return None;
    }
    // Ignore common generated / index stems
    if stem.contains("generated") || stem == "from-readme" || stem == "bootstrap-checklist" {
        return None;
    }
    Some(stem)
}

fn trivial_tag(tag: &str) -> bool {
    matches!(
        tag.to_ascii_lowercase().as_str(),
        "goal"
            | "adr"
            | "concept"
            | "symbol"
            | "reference"
            | "edge_case"
            | "alternative"
            | "analysis"
            | "readme"
            | "generated"
            | "index"
    )
}

/// List non-symbol notes that have zero **explicit** edges (auto edges ignored).
pub fn list_orphan_notes(db: &Database) -> Result<Vec<OrphanNote>> {
    let notes = list_note_nodes(db)?;
    let explicit = explicit_degree(db)?;
    let mut out = Vec::new();
    for n in &notes {
        let deg = explicit.get(&n.id).copied().unwrap_or(0);
        if deg > 0 {
            continue;
        }
        // Skip pure scaffold ids for the orphan list noise (still linkable via --auto path)
        if is_noise_orphan_id(&n.id) {
            continue;
        }
        let suggestions = suggest_for_node(db, n, &notes)?;
        out.push(OrphanNote {
            id: n.id.clone(),
            node_type: n.node_type.as_str().to_string(),
            title: n.title.clone(),
            file_path: n.file_path.clone(),
            suggestions,
        });
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

fn is_noise_orphan_id(id: &str) -> bool {
    let id = id.to_lowercase();
    id.contains("template")
        || id.contains("bootstrap")
        || id.contains("module-map.generated")
        || id == "agents"
        || id.ends_with("/agents")
        || id == "docs/goals/readme"
}

fn list_note_nodes(db: &Database) -> Result<Vec<Node>> {
    let mut nodes = Vec::new();
    for ty in [
        NodeType::Goal,
        NodeType::Adr,
        NodeType::Concept,
        NodeType::Analysis,
        NodeType::EdgeCase,
        NodeType::Reference,
        NodeType::Alternative,
        NodeType::Changelog,
        NodeType::Plan,
    ] {
        for id in db.list_node_ids_by_type(ty.as_str())? {
            if let Some(n) = db.get_node(&id)? {
                nodes.push(n);
            }
        }
    }
    // Root project hubs (README / CHANGELOG / optional planning docs)
    for id in [
        crate::hubs::HUB_README,
        crate::hubs::HUB_CHANGELOG,
        crate::hubs::HUB_ROADMAP,
        crate::hubs::HUB_BACKLOG,
    ] {
        if let Some(n) = db.get_node(id)? {
            if !nodes.iter().any(|x| x.id == id) {
                nodes.push(n);
            }
        }
    }
    Ok(nodes)
}

fn explicit_degree(db: &Database) -> Result<HashMap<String, usize>> {
    let edges = db.get_all_edges()?;
    let mut deg: HashMap<String, usize> = HashMap::new();
    for e in edges {
        if !is_explicit_relation(&e.relation_type) {
            continue;
        }
        *deg.entry(e.source_id).or_default() += 1;
        *deg.entry(e.target_id).or_default() += 1;
    }
    Ok(deg)
}

fn suggest_for_node(db: &Database, node: &Node, all: &[Node]) -> Result<Vec<AutoLinkSuggestion>> {
    let mut suggestions = Vec::new();
    let mut seen = HashSet::new();

    // Filename stem matches
    if let Some(path) = &node.file_path {
        if let Some(stem) = path_stem(path) {
            for other in all {
                if other.id == node.id {
                    continue;
                }
                let Some(op) = other.file_path.as_ref() else {
                    continue;
                };
                let Some(ostem) = path_stem(op) else {
                    continue;
                };
                if ostem != stem {
                    continue;
                }
                let key = format!("fn:{}:{}", node.id, other.id);
                if !seen.insert(key) {
                    continue;
                }
                suggestions.push(AutoLinkSuggestion {
                    target_id: other.id.clone(),
                    target_title: other.title.clone(),
                    target_path: other.file_path.clone(),
                    relation_type: REL_AUTO_FILENAME.into(),
                    weight: WEIGHT_AUTO_FILENAME,
                    reason: format!("same filename stem `{stem}`"),
                });
            }
        }
    }

    // Shared tags
    let tags: HashSet<String> = db
        .get_tags_for(&node.id)?
        .into_iter()
        .filter(|t| !trivial_tag(t))
        .map(|t| t.to_ascii_lowercase())
        .collect();
    if !tags.is_empty() {
        for other in all {
            if other.id == node.id {
                continue;
            }
            let otags = db.get_tags_for(&other.id)?;
            let shared: Vec<String> = otags
                .into_iter()
                .filter(|t| !trivial_tag(t) && tags.contains(&t.to_ascii_lowercase()))
                .collect();
            if shared.is_empty() {
                continue;
            }
            let key = format!("tag:{}:{}", node.id, other.id);
            if !seen.insert(key) {
                continue;
            }
            suggestions.push(AutoLinkSuggestion {
                target_id: other.id.clone(),
                target_title: other.title.clone(),
                target_path: other.file_path.clone(),
                relation_type: REL_AUTO_TAG.into(),
                weight: WEIGHT_AUTO_TAG,
                reason: format!("shared tag(s): {}", shared.join(", ")),
            });
        }
    }

    suggestions.sort_by(|a, b| {
        b.weight
            .partial_cmp(&a.weight)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.target_id.cmp(&b.target_id))
    });
    suggestions.truncate(24);
    Ok(suggestions)
}

/// Create soft auto-links for all orphans, or for one file/path/node id.
///
/// When `target` is `None`, processes every orphan (plus always applies
/// filename-stem matches among all notes so `goals/x.md` ↔ `concepts/x.md`
/// even if neither was classified orphan).
pub fn run_auto_link(db: &Database, target: Option<&Path>) -> Result<AutoLinkReport> {
    let all = list_note_nodes(db)?;
    let now = Utc::now().timestamp();
    let mut applied = Vec::new();
    let mut edges_upserted = 0usize;
    let mut pairs = HashSet::new();

    let scope = if let Some(t) = target {
        let focus = resolve_target_node(db, t, &all)?;
        // Clear prior auto edges involving this node so re-run is idempotent.
        db.clear_auto_edges_involving(&focus.id)?;
        let suggestions = suggest_for_node(db, &focus, &all)?;
        for s in &suggestions {
            edges_upserted += upsert_auto_pair(
                db,
                &focus.id,
                &s.target_id,
                &s.relation_type,
                s.weight,
                now,
                &mut pairs,
            )?;
            applied.push(s.clone());
        }
        format!("node {}", focus.id)
    } else {
        // Full rebuild of auto edges from filename + tags (orphans prioritized in report).
        db.clear_all_auto_edges()?;
        // Filename: group by stem
        let mut by_stem: HashMap<String, Vec<&Node>> = HashMap::new();
        for n in &all {
            if let Some(p) = &n.file_path {
                if let Some(stem) = path_stem(p) {
                    by_stem.entry(stem).or_default().push(n);
                }
            }
        }
        for (stem, group) in &by_stem {
            if group.len() < 2 {
                continue;
            }
            for i in 0..group.len() {
                for j in (i + 1)..group.len() {
                    let a = group[i];
                    let b = group[j];
                    edges_upserted += upsert_auto_pair(
                        db,
                        &a.id,
                        &b.id,
                        REL_AUTO_FILENAME,
                        WEIGHT_AUTO_FILENAME,
                        now,
                        &mut pairs,
                    )?;
                    if applied.len() < 50 {
                        applied.push(AutoLinkSuggestion {
                            target_id: b.id.clone(),
                            target_title: b.title.clone(),
                            target_path: b.file_path.clone(),
                            relation_type: REL_AUTO_FILENAME.into(),
                            weight: WEIGHT_AUTO_FILENAME,
                            reason: format!("same filename stem `{stem}` ({} ↔ {})", a.id, b.id),
                        });
                    }
                }
            }
        }
        // Tags among all notes
        let mut tag_index: HashMap<String, Vec<String>> = HashMap::new();
        for n in &all {
            for t in db.get_tags_for(&n.id)? {
                if trivial_tag(&t) {
                    continue;
                }
                tag_index
                    .entry(t.to_ascii_lowercase())
                    .or_default()
                    .push(n.id.clone());
            }
        }
        for (tag, ids) in &tag_index {
            let mut uniq = ids.clone();
            uniq.sort();
            uniq.dedup();
            if uniq.len() < 2 {
                continue;
            }
            for i in 0..uniq.len() {
                for j in (i + 1)..uniq.len() {
                    let a = &uniq[i];
                    let b = &uniq[j];
                    edges_upserted += upsert_auto_pair(
                        db,
                        a,
                        b,
                        REL_AUTO_TAG,
                        WEIGHT_AUTO_TAG,
                        now,
                        &mut pairs,
                    )?;
                    if applied.len() < 50 {
                        let bt = all
                            .iter()
                            .find(|n| n.id == *b)
                            .map(|n| n.title.clone())
                            .unwrap_or_else(|| b.clone());
                        let bp = all
                            .iter()
                            .find(|n| n.id == *b)
                            .and_then(|n| n.file_path.clone());
                        applied.push(AutoLinkSuggestion {
                            target_id: b.clone(),
                            target_title: bt,
                            target_path: bp,
                            relation_type: REL_AUTO_TAG.into(),
                            weight: WEIGHT_AUTO_TAG,
                            reason: format!("shared tag `{tag}` ({a} ↔ {b})"),
                        });
                    }
                }
            }
        }
        "all notes (filename stems + shared tags)".into()
    };

    Ok(AutoLinkReport {
        edges_upserted,
        pairs_considered: pairs.len(),
        scope,
        applied,
    })
}

fn upsert_auto_pair(
    db: &Database,
    a: &str,
    b: &str,
    rel: &str,
    weight: f32,
    now: i64,
    pairs: &mut HashSet<String>,
) -> Result<usize> {
    if a == b {
        return Ok(0);
    }
    // Canonical undirected pair key
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    let key = format!("{rel}:{lo}:{hi}");
    if !pairs.insert(key) {
        return Ok(0);
    }
    // Store both directions so CSR hops work either way.
    let mut n = 0;
    for (src, tgt) in [(lo, hi), (hi, lo)] {
        db.insert_edge(&Edge {
            source_id: src.to_string(),
            target_id: tgt.to_string(),
            relation_type: rel.to_string(),
            weight,
            decay_rate: 0.0,
            created_at: now,
        })?;
        n += 1;
    }
    Ok(n)
}

fn resolve_target_node(db: &Database, target: &Path, all: &[Node]) -> Result<Node> {
    let raw = target.to_string_lossy().replace('\\', "/");
    let raw = raw.trim_start_matches("./");

    // Exact node id
    if let Some(n) = db.get_node(raw)? {
        return Ok(n);
    }

    // Match file_path suffix / equality
    let candidates: Vec<&Node> = all
        .iter()
        .filter(|n| {
            n.file_path
                .as_ref()
                .map(|p| {
                    let p = p.replace('\\', "/");
                    p == raw || p.ends_with(raw) || p.ends_with(&format!("/{raw}"))
                })
                .unwrap_or(false)
        })
        .collect();
    if candidates.len() == 1 {
        return Ok(candidates[0].clone());
    }
    if candidates.len() > 1 {
        return Err(crate::error::BrainError::other(format!(
            "ambiguous path `{raw}` matches {} nodes",
            candidates.len()
        )));
    }

    // Try with docs/ prefix
    let with_docs = if raw.starts_with("docs/") {
        raw.to_string()
    } else {
        format!("docs/{raw}")
    };
    if let Some(n) = all.iter().find(|n| {
        n.file_path
            .as_ref()
            .map(|p| p.replace('\\', "/") == with_docs)
            .unwrap_or(false)
    }) {
        return Ok(n.clone());
    }

    Err(crate::error::BrainError::other(format!(
        "no note node found for `{raw}` — pass a path like docs/goals/foo.md or a node id"
    )))
}

/// Normalize a user path argument to something resolve_target can use.
pub fn normalize_target_arg(s: &str) -> PathBuf {
    PathBuf::from(s.trim().trim_start_matches("./"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::NodeType;

    fn note(id: &str, ty: NodeType, path: &str, title: &str) -> Node {
        Node {
            id: id.into(),
            node_type: ty,
            title: title.into(),
            file_path: Some(path.into()),
            symbol_hash: None,
            summary: Some(title.into()),
            content_hash: None,
            created_at: 1,
            updated_at: 1,
        }
    }

    #[test]
    fn path_stem_extracts_filename() {
        assert_eq!(
            path_stem("docs/goals/rust-fluency.md").as_deref(),
            Some("rust-fluency")
        );
        assert_eq!(
            path_stem("docs/concepts/rust-fluency.md").as_deref(),
            Some("rust-fluency")
        );
        assert!(path_stem("docs/adr/TEMPLATE.md").is_none());
    }

    #[test]
    fn filename_stem_auto_links_and_orphans() {
        let db = Database::open_in_memory().unwrap();
        let g = note(
            "docs/goals/rust-fluency",
            NodeType::Goal,
            "docs/goals/rust-fluency.md",
            "Rust fluency goal",
        );
        let c = note(
            "docs/concepts/rust-fluency",
            NodeType::Concept,
            "docs/concepts/rust-fluency.md",
            "Rust fluency deep dive",
        );
        db.insert_node(&g).unwrap();
        db.insert_node(&c).unwrap();
        db.index_fts(&g.id, &g.title, "goal body", "goal").unwrap();
        db.index_fts(&c.id, &c.title, "concept body", "concept")
            .unwrap();

        let orphans = list_orphan_notes(&db).unwrap();
        assert_eq!(orphans.len(), 2);
        assert!(orphans
            .iter()
            .any(|o| o.suggestions.iter().any(|s| s.relation_type == REL_AUTO_FILENAME)));

        let report = run_auto_link(&db, None).unwrap();
        assert!(report.edges_upserted >= 2);
        let edges = db.get_all_edges().unwrap();
        assert!(edges.iter().any(|e| e.relation_type == REL_AUTO_FILENAME));

        // After auto-link only, still orphan re: *explicit* edges
        let orphans2 = list_orphan_notes(&db).unwrap();
        assert_eq!(
            orphans2.len(),
            2,
            "auto edges must not count as explicit"
        );
    }

    #[test]
    fn targeted_auto_link_by_path() {
        let db = Database::open_in_memory().unwrap();
        let g = note(
            "docs/goals/foo",
            NodeType::Goal,
            "docs/goals/foo.md",
            "Foo goal",
        );
        let c = note(
            "docs/concepts/foo",
            NodeType::Concept,
            "docs/concepts/foo.md",
            "Foo concept",
        );
        db.insert_node(&g).unwrap();
        db.insert_node(&c).unwrap();
        let report = run_auto_link(&db, Some(Path::new("docs/goals/foo.md"))).unwrap();
        assert!(report.edges_upserted >= 2);
        assert!(report.scope.contains("docs/goals/foo"));
    }
}
