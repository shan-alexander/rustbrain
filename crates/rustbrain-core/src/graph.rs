//! Neighborhood inspection of the knowledge graph.
//!
//! Complements FTS [`crate::query`] and agent [`crate::context`]: those pack
//! content under a token budget; this module **shows structure** — who links to
//! whom, with relation types and weights — as ASCII or JSON for humans/agents.
//!
//! Edges come from SQLite (preserves `relation_type`). CSR `graph.mmap` is not
//! required; run `sync` so edges exist.

use crate::autolink::is_auto_relation;
use crate::error::{BrainError, Result};
use crate::id::node_id_from_rel_path;
use crate::storage::Database;
use crate::symbols::{parse_symbol_path, resolve_symbol_ref};
use crate::types::{Edge, Node, NodeType};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;

/// Which edge directions to follow when expanding from the root.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum GraphDirection {
    /// Only follow `source → target` edges from the frontier.
    Out,
    /// Only follow reverse edges into the frontier.
    In,
    /// Expand along both outgoing and incoming edges (default).
    #[default]
    Both,
}

impl GraphDirection {
    /// Parse `out` / `in` / `both` (case-insensitive).
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "out" | "outgoing" | "from" => Some(Self::Out),
            "in" | "incoming" | "to" => Some(Self::In),
            "both" | "all" | "undirected" => Some(Self::Both),
            _ => None,
        }
    }
}

/// Options for [`neighborhood`].
#[derive(Debug, Clone)]
pub struct GraphOptions {
    /// Max BFS depth from the root (`1` = direct neighbors only).
    pub hops: usize,
    /// Include soft `auto_*` edges (default true).
    pub include_auto: bool,
    /// Include symbol nodes as neighbors (default true).
    pub include_symbols: bool,
    /// Edge direction filter.
    pub direction: GraphDirection,
    /// Cap on edges returned (prevents huge dumps).
    pub max_edges: usize,
    /// If set, only include neighbors whose type is in this list.
    pub type_filter: Option<Vec<NodeType>>,
    /// Optional SubBrain filter: keep neighbors in this scope (or hubs / MainBrain mix).
    pub scope: Option<String>,
    /// How much MainBrain content to allow as neighbors when `scope` is set.
    pub scope_main: crate::query::ScopeMainInclude,
    /// MainBrain id for scope filtering (default `main`).
    pub main_scope: String,
}

impl Default for GraphOptions {
    fn default() -> Self {
        Self {
            hops: 1,
            include_auto: true,
            include_symbols: true,
            direction: GraphDirection::Both,
            max_edges: 200,
            type_filter: None,
            scope: None,
            scope_main: crate::query::ScopeMainInclude::HubsOnly,
            main_scope: crate::scopes::MAIN_SCOPE.to_string(),
        }
    }
}

impl GraphOptions {
    fn allows_neighbor(&self, node: &Node) -> bool {
        if let Some(ref want) = self.scope {
            let q = crate::query::QueryOptions {
                scope: Some(want.clone()),
                main_scope: self.main_scope.clone(),
                scope_main: self.scope_main,
                ..crate::query::QueryOptions::default()
            };
            if !q.allows_scope(&node.scope, &node.id) {
                return false;
            }
        }
        true
    }
}

/// Compact node identity for graph reports.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GraphNodeRef {
    /// Stable node id.
    pub id: String,
    /// Domain type as snake_case string.
    pub node_type: String,
    /// Display title.
    pub title: String,
    /// Repo-relative path when known.
    pub file_path: Option<String>,
}

impl GraphNodeRef {
    fn from_node(n: &Node) -> Self {
        Self {
            id: n.id.clone(),
            node_type: n.node_type.as_str().to_string(),
            title: n.title.clone(),
            file_path: n.file_path.clone(),
        }
    }
}

/// One edge in the neighborhood expansion (BFS order).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphHopEdge {
    /// Hop distance from the root (1 = direct neighbor).
    pub hop: usize,
    /// `out` if root-side was source; `in` if root-side was target.
    pub direction: String,
    /// Relation label (`relates_to`, `anchors`, `doc_links`, `auto_*`, …).
    pub relation_type: String,
    /// Edge weight.
    pub weight: f32,
    /// Node id on the parent side of this hop (closer to root).
    pub from_id: String,
    /// Neighbor node reached by this hop.
    pub to: GraphNodeRef,
}

/// Result of a neighborhood query around one node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphNeighborhood {
    /// Resolved seed node.
    pub root: GraphNodeRef,
    /// Requested hop depth.
    pub hops: usize,
    /// Unique nodes in the subgraph including the root.
    pub nodes_in_subgraph: usize,
    /// Edges returned (may be capped by [`GraphOptions::max_edges`]).
    pub edges: Vec<GraphHopEdge>,
    /// True when `max_edges` truncated the expansion.
    pub truncated: bool,
    /// Total nodes in the database (context).
    pub db_nodes: usize,
    /// Total edges in the database (context).
    pub db_edges: usize,
}

impl GraphNeighborhood {
    /// Human/agent-readable ASCII (tree-style for hop 1+, indented by hop).
    pub fn to_ascii(&self) -> String {
        let mut out = String::new();
        let path = self.root.file_path.as_deref().unwrap_or("-");
        out.push_str(&format!(
            "graph: {}  ({})  \"{}\"\n",
            self.root.id, self.root.node_type, self.root.title
        ));
        out.push_str(&format!("  path: {path}\n"));
        out.push_str(&format!(
            "  hops={}  edges_shown={}  nodes_in_subgraph={}  db={}/{} n/e{}\n",
            self.hops,
            self.edges.len(),
            self.nodes_in_subgraph,
            self.db_nodes,
            self.db_edges,
            if self.truncated { "  [truncated]" } else { "" }
        ));
        if self.edges.is_empty() {
            out.push_str("  (no neighbors — orphan or filters excluded all edges)\n");
            out.push_str(
                "  tip: `rustbrain links --auto` soft-links orphans; explicit WikiLinks preferred\n",
            );
            return out;
        }

        // Tree: children of root first, then recursively by parent id.
        render_tree_level(&mut out, &self.edges, &self.root.id, 1, "");
        out
    }
}

fn render_tree_level(
    out: &mut String,
    all: &[GraphHopEdge],
    parent_id: &str,
    hop: usize,
    prefix: &str,
) {
    let kids: Vec<&GraphHopEdge> = all
        .iter()
        .filter(|e| e.from_id == parent_id && e.hop == hop)
        .collect();
    let n = kids.len();
    for (i, e) in kids.iter().enumerate() {
        let last = i + 1 == n;
        let branch = if last { "└──" } else { "├──" };
        let cont = if last { "    " } else { "│   " };
        let arrow = if e.direction == "in" { "←" } else { "→" };
        let path = e
            .to
            .file_path
            .as_deref()
            .map(|p| format!("  [{p}]"))
            .unwrap_or_default();
        out.push_str(&format!(
            "{prefix}{branch}[{arrow} {} w={:.2}] {}  ({})  \"{}\"{path}\n",
            e.relation_type, e.weight, e.to.id, e.to.node_type, e.to.title
        ));
        let child_prefix = format!("{prefix}{cont}");
        render_tree_level(out, all, &e.to.id, hop + 1, &child_prefix);
    }
}

/// Aggregate graph statistics (no seed required).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphStats {
    /// Total nodes.
    pub nodes: usize,
    /// Total edges.
    pub edges: usize,
    /// Counts by `node_type`.
    pub by_type: Vec<(String, usize)>,
    /// Counts by `relation_type`.
    pub by_relation: Vec<(String, usize)>,
    /// Highest-degree nodes (undirected: in+out), up to 15.
    pub hubs: Vec<GraphHub>,
}

/// A high-degree node for [`GraphStats`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphHub {
    /// Node id.
    pub id: String,
    /// Type string.
    pub node_type: String,
    /// Title.
    pub title: String,
    /// Degree (in + out edge endpoints).
    pub degree: usize,
}

impl GraphStats {
    /// Human-readable summary.
    pub fn to_ascii(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "graph stats: nodes={} edges={}\n",
            self.nodes, self.edges
        ));
        if !self.by_type.is_empty() {
            out.push_str("  by type:\n");
            for (t, n) in &self.by_type {
                out.push_str(&format!("    {t}: {n}\n"));
            }
        }
        if !self.by_relation.is_empty() {
            out.push_str("  by relation:\n");
            for (r, n) in &self.by_relation {
                out.push_str(&format!("    {r}: {n}\n"));
            }
        }
        if !self.hubs.is_empty() {
            out.push_str("  hubs (degree = in+out):\n");
            for h in &self.hubs {
                out.push_str(&format!(
                    "    deg={:>3}  {}  ({})  \"{}\"\n",
                    h.degree, h.id, h.node_type, h.title
                ));
            }
        }
        out
    }
}

/// Database-wide graph statistics and hubs.
pub fn graph_stats(db: &Database) -> Result<GraphStats> {
    let nodes = db.get_all_nodes()?;
    let edges = db.get_all_edges()?;
    let mut by_type: HashMap<String, usize> = HashMap::new();
    for n in &nodes {
        *by_type.entry(n.node_type.as_str().to_string()).or_default() += 1;
    }
    let mut by_relation: HashMap<String, usize> = HashMap::new();
    let mut degree: HashMap<String, usize> = HashMap::new();
    for e in &edges {
        *by_relation.entry(e.relation_type.clone()).or_default() += 1;
        *degree.entry(e.source_id.clone()).or_default() += 1;
        *degree.entry(e.target_id.clone()).or_default() += 1;
    }
    let mut by_type: Vec<(String, usize)> = by_type.into_iter().collect();
    by_type.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let mut by_relation: Vec<(String, usize)> = by_relation.into_iter().collect();
    by_relation.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    let id_to_node: HashMap<&str, &Node> = nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let mut hubs: Vec<GraphHub> = degree
        .into_iter()
        .filter_map(|(id, deg)| {
            let n = id_to_node.get(id.as_str())?;
            Some(GraphHub {
                id,
                node_type: n.node_type.as_str().to_string(),
                title: n.title.clone(),
                degree: deg,
            })
        })
        .collect();
    hubs.sort_by(|a, b| {
        b.degree
            .cmp(&a.degree)
            .then_with(|| a.id.cmp(&b.id))
    });
    hubs.truncate(15);

    Ok(GraphStats {
        nodes: nodes.len(),
        edges: edges.len(),
        by_type,
        by_relation,
        hubs,
    })
}

/// Expand the k-hop neighborhood around `target` (path, id, title, or `symbol:…`).
pub fn neighborhood(db: &Database, target: &str, opts: &GraphOptions) -> Result<GraphNeighborhood> {
    let root = resolve_graph_target(db, target)?;
    let all_edges = db.get_all_edges()?;
    let all_nodes = db.get_all_nodes()?;
    let node_map: HashMap<String, Node> = all_nodes.into_iter().map(|n| (n.id.clone(), n)).collect();

    let hops = opts.hops.max(1);

    // Adjacency: out and in lists of (edge, neighbor_id)
    let mut out_adj: HashMap<String, Vec<&Edge>> = HashMap::new();
    let mut in_adj: HashMap<String, Vec<&Edge>> = HashMap::new();
    for e in &all_edges {
        if !opts.include_auto && is_auto_relation(&e.relation_type) {
            continue;
        }
        out_adj.entry(e.source_id.clone()).or_default().push(e);
        in_adj.entry(e.target_id.clone()).or_default().push(e);
    }

    let mut visited: HashSet<String> = HashSet::new();
    visited.insert(root.id.clone());
    // queue: (node_id, hop_from_root)
    let mut queue: VecDeque<(String, usize)> = VecDeque::new();
    queue.push_back((root.id.clone(), 0));

    let mut result_edges: Vec<GraphHopEdge> = Vec::new();
    let mut truncated = false;

    while let Some((curr, depth)) = queue.pop_front() {
        if depth >= hops {
            continue;
        }
        if result_edges.len() >= opts.max_edges {
            truncated = true;
            break;
        }

        let mut candidates: Vec<(&Edge, &str, &str)> = Vec::new(); // edge, neighbor, direction

        if matches!(opts.direction, GraphDirection::Out | GraphDirection::Both) {
            if let Some(list) = out_adj.get(&curr) {
                for e in list {
                    candidates.push((e, e.target_id.as_str(), "out"));
                }
            }
        }
        if matches!(opts.direction, GraphDirection::In | GraphDirection::Both) {
            if let Some(list) = in_adj.get(&curr) {
                for e in list {
                    candidates.push((e, e.source_id.as_str(), "in"));
                }
            }
        }

        // Stable order: higher weight first, then id
        candidates.sort_by(|a, b| {
            b.0.weight
                .partial_cmp(&a.0.weight)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.1.cmp(b.1))
                .then_with(|| a.0.relation_type.cmp(&b.0.relation_type))
        });

        for (e, neighbor_id, dir) in candidates {
            if result_edges.len() >= opts.max_edges {
                truncated = true;
                break;
            }
            if visited.contains(neighbor_id) {
                continue;
            }
            let Some(neighbor) = node_map.get(neighbor_id) else {
                continue;
            };
            if !opts.include_symbols && neighbor.node_type == NodeType::Symbol {
                continue;
            }
            if let Some(filter) = &opts.type_filter {
                if !filter.contains(&neighbor.node_type) {
                    continue;
                }
            }
            if !opts.allows_neighbor(neighbor) {
                continue;
            }

            visited.insert(neighbor_id.to_string());
            let hop = depth + 1;
            result_edges.push(GraphHopEdge {
                hop,
                direction: dir.to_string(),
                relation_type: e.relation_type.clone(),
                weight: e.weight,
                from_id: curr.clone(),
                to: GraphNodeRef::from_node(neighbor),
            });
            if hop < hops {
                queue.push_back((neighbor_id.to_string(), hop));
            }
        }
    }

    Ok(GraphNeighborhood {
        root: GraphNodeRef::from_node(&root),
        hops,
        nodes_in_subgraph: visited.len(),
        edges: result_edges,
        truncated,
        db_nodes: node_map.len(),
        db_edges: all_edges.len(),
    })
}

/// Resolve a user target string to a [`Node`].
///
/// Accepts:
/// - exact node id
/// - file path (`docs/concepts/raft.md` or without extension)
/// - `symbol:Name` / `symbol:mod::Name`
/// - exact title (case-insensitive), unique match only
pub fn resolve_graph_target(db: &Database, target: &str) -> Result<Node> {
    let raw = target.trim().trim_start_matches("./").replace('\\', "/");
    if raw.is_empty() {
        return Err(BrainError::other(
            "empty graph target — pass a node id, path, title, or symbol:…",
        ));
    }

    // Exact id
    if let Some(n) = db.get_node(&raw)? {
        return Ok(n);
    }

    // Path → id slug
    let as_path = Path::new(&raw);
    let slug = node_id_from_rel_path(as_path);
    if let Some(n) = db.get_node(&slug)? {
        return Ok(n);
    }
    // docs/ prefix variants
    if !raw.starts_with("docs/") {
        let with_docs = format!("docs/{raw}");
        if let Some(n) = db.get_node(&with_docs)? {
            return Ok(n);
        }
        let slug2 = node_id_from_rel_path(Path::new(&with_docs));
        if let Some(n) = db.get_node(&slug2)? {
            return Ok(n);
        }
    }

    // symbol: refs
    let sym_raw = raw
        .strip_prefix("symbol:")
        .or_else(|| raw.strip_prefix("[[symbol:").and_then(|s| s.strip_suffix("]]")))
        .unwrap_or(&raw);
    if raw.starts_with("symbol:") || raw.contains("::") {
        if let Some(sym) = parse_symbol_path(sym_raw) {
            let ids: HashSet<String> = db
                .get_all_node_ids()?
                .into_iter()
                .filter(|id| id.starts_with("symbol/"))
                .collect();
            if let Some(id) = resolve_symbol_ref(&sym, &ids) {
                if let Some(n) = db.get_node(&id)? {
                    return Ok(n);
                }
            }
        }
    }

    // file_path match among all nodes
    let all = db.get_all_nodes()?;
    let path_hits: Vec<&Node> = all
        .iter()
        .filter(|n| {
            n.file_path
                .as_ref()
                .map(|p| {
                    let p = p.replace('\\', "/");
                    p == raw
                        || p.ends_with(&raw)
                        || p.ends_with(&format!("/{raw}"))
                        || p.trim_end_matches(".md") == raw
                        || p.trim_end_matches(".md").ends_with(&format!("/{raw}"))
                })
                .unwrap_or(false)
        })
        .collect();
    if path_hits.len() == 1 {
        return Ok(path_hits[0].clone());
    }
    if path_hits.len() > 1 {
        return Err(BrainError::other(format!(
            "ambiguous path `{raw}` matches {} nodes — use a fuller path or exact id",
            path_hits.len()
        )));
    }

    // Unique title match (case-insensitive)
    let lower = raw.to_ascii_lowercase();
    let title_hits: Vec<&Node> = all
        .iter()
        .filter(|n| n.title.to_ascii_lowercase() == lower)
        .collect();
    if title_hits.len() == 1 {
        return Ok(title_hits[0].clone());
    }
    if title_hits.len() > 1 {
        return Err(BrainError::other(format!(
            "ambiguous title `{raw}` matches {} nodes — use path or id",
            title_hits.len()
        )));
    }

    // Substring title (unique)
    let sub_hits: Vec<&Node> = all
        .iter()
        .filter(|n| n.title.to_ascii_lowercase().contains(&lower) && lower.len() >= 3)
        .collect();
    if sub_hits.len() == 1 {
        return Ok(sub_hits[0].clone());
    }

    Err(BrainError::other(format!(
        "no node found for `{raw}` — try path (docs/…md), node id, exact title, or symbol:Name after `rustbrain sync`"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Edge, Node, NodeType};
    use tempfile::tempdir;

    fn mk_db() -> (tempfile::TempDir, Database) {
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path().join("db.sqlite")).unwrap();
        let now = 1_700_000_000i64;
        let nodes = [
            Node {
                id: "docs/concepts/raft".into(),
                node_type: NodeType::Concept,
                title: "Raft".into(),
                file_path: Some("docs/concepts/raft.md".into()),
                symbol_hash: None,
                summary: Some("consensus".into()),
                content_hash: None,
                scope: crate::scopes::MAIN_SCOPE.to_string(),
                created_at: now,
                updated_at: now,
            },
            Node {
                id: "docs/concepts/logcompaction".into(),
                node_type: NodeType::Concept,
                title: "Log Compaction".into(),
                file_path: Some("docs/concepts/logcompaction.md".into()),
                symbol_hash: None,
                summary: None,
                content_hash: None,
                scope: crate::scopes::MAIN_SCOPE.to_string(),
                created_at: now,
                updated_at: now,
            },
            Node {
                id: "docs/adr/0001-use-raft".into(),
                node_type: NodeType::Adr,
                title: "Use Raft".into(),
                file_path: Some("docs/adr/0001-use-raft.md".into()),
                symbol_hash: None,
                summary: None,
                content_hash: None,
                scope: crate::scopes::MAIN_SCOPE.to_string(),
                created_at: now,
                updated_at: now,
            },
            Node {
                id: "symbol/demo/lib/storageengine".into(),
                node_type: NodeType::Symbol,
                title: "StorageEngine".into(),
                file_path: Some("src/lib.rs".into()),
                symbol_hash: Some(1),
                summary: None,
                content_hash: None,
                scope: crate::scopes::MAIN_SCOPE.to_string(),
                created_at: now,
                updated_at: now,
            },
        ];
        for n in &nodes {
            db.insert_node(n).unwrap();
        }
        db.insert_edge(&Edge {
            source_id: "docs/concepts/raft".into(),
            target_id: "docs/concepts/logcompaction".into(),
            relation_type: "relates_to".into(),
            weight: 1.0,
            decay_rate: 0.0,
            created_at: now,
        })
        .unwrap();
        db.insert_edge(&Edge {
            source_id: "docs/concepts/raft".into(),
            target_id: "symbol/demo/lib/storageengine".into(),
            relation_type: "anchors".into(),
            weight: 1.0,
            decay_rate: 0.0,
            created_at: now,
        })
        .unwrap();
        db.insert_edge(&Edge {
            source_id: "docs/adr/0001-use-raft".into(),
            target_id: "docs/concepts/raft".into(),
            relation_type: "relates_to".into(),
            weight: 0.9,
            decay_rate: 0.0,
            created_at: now,
        })
        .unwrap();
        db.insert_edge(&Edge {
            source_id: "docs/concepts/raft".into(),
            target_id: "docs/concepts/logcompaction".into(),
            relation_type: "auto_filename".into(),
            weight: 0.4,
            decay_rate: 0.0,
            created_at: now,
        })
        .unwrap();
        (dir, db)
    }

    #[test]
    fn neighborhood_both_directions() {
        let (_dir, db) = mk_db();
        let nb = neighborhood(&db, "docs/concepts/raft", &GraphOptions::default()).unwrap();
        assert_eq!(nb.root.id, "docs/concepts/raft");
        // out relates_to logcompaction, out anchors symbol, in from adr, + auto
        assert!(nb.edges.len() >= 3);
        let ids: HashSet<_> = nb.edges.iter().map(|e| e.to.id.as_str()).collect();
        assert!(ids.contains("docs/concepts/logcompaction"));
        assert!(ids.contains("symbol/demo/lib/storageengine"));
        assert!(ids.contains("docs/adr/0001-use-raft"));
        let ascii = nb.to_ascii();
        assert!(ascii.contains("relates_to"));
        assert!(ascii.contains("anchors"));
    }

    #[test]
    fn no_auto_and_no_symbols_filters() {
        let (_dir, db) = mk_db();
        let opts = GraphOptions {
            include_auto: false,
            include_symbols: false,
            ..GraphOptions::default()
        };
        let nb = neighborhood(&db, "docs/concepts/raft.md", &opts).unwrap();
        for e in &nb.edges {
            assert!(!e.relation_type.starts_with("auto_"));
            assert_ne!(e.to.node_type, "symbol");
        }
        assert!(nb.edges.iter().any(|e| e.to.id == "docs/concepts/logcompaction"));
        assert!(nb.edges.iter().any(|e| e.to.id == "docs/adr/0001-use-raft"));
    }

    #[test]
    fn resolve_by_title() {
        let (_dir, db) = mk_db();
        let n = resolve_graph_target(&db, "Raft").unwrap();
        assert_eq!(n.id, "docs/concepts/raft");
    }

    #[test]
    fn stats_counts() {
        let (_dir, db) = mk_db();
        let s = graph_stats(&db).unwrap();
        assert_eq!(s.nodes, 4);
        assert!(s.edges >= 3);
        assert!(!s.hubs.is_empty());
        assert!(s.to_ascii().contains("by type"));
    }

    #[test]
    fn direction_out_only() {
        let (_dir, db) = mk_db();
        let opts = GraphOptions {
            direction: GraphDirection::Out,
            include_auto: false,
            ..GraphOptions::default()
        };
        let nb = neighborhood(&db, "docs/concepts/raft", &opts).unwrap();
        assert!(nb.edges.iter().all(|e| e.direction == "out"));
        assert!(!nb.edges.iter().any(|e| e.to.id == "docs/adr/0001-use-raft"));
    }
}
