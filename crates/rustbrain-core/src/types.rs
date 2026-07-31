//! Domain types for the rustbrain knowledge graph.
//!
//! These types are serialized into SQLite rows, `.brainbundle` JSON, and agent
//! context payloads. Field names and `NodeType` string forms are part of the
//! **v1 on-disk contract** — change them only with a schema migration.

use serde::{Deserialize, Serialize};

/// The seven first-class domain node types in a rustbrain repository.
///
/// Serialized as `snake_case` strings in YAML frontmatter and SQLite
/// (`goal`, `adr`, `alternative`, `concept`, `symbol`, `reference`, `edge_case`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeType {
    /// Overarching project goal, PRD requirement, SLA target, or scope limitation.
    Goal,
    /// Architectural Decision Record (decision made to achieve a goal).
    Adr,
    /// Alternatives considered (benchmarks, trade-offs, discarded options).
    Alternative,
    /// Pure atomic technical or domain concept note (Zettelkasten-style).
    Concept,
    /// Codebase AST entity (function, struct, trait, module) from Tree-Sitter.
    Symbol,
    /// External dependency, crate quirk, API caveat, or documentation link.
    Reference,
    /// Known bug, concurrency trap, memory aliasing quirk, or platform gotcha.
    EdgeCase,
}

impl NodeType {
    /// Stable string form stored in SQLite and frontmatter.
    pub fn as_str(&self) -> &'static str {
        match self {
            NodeType::Goal => "goal",
            NodeType::Adr => "adr",
            NodeType::Alternative => "alternative",
            NodeType::Concept => "concept",
            NodeType::Symbol => "symbol",
            NodeType::Reference => "reference",
            NodeType::EdgeCase => "edge_case",
        }
    }

    /// Parse a node type string.
    ///
    /// Returns `None` for unknown values — callers must not silently map garbage
    /// to [`NodeType::Concept`] at the API boundary.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "goal" => Some(NodeType::Goal),
            "adr" => Some(NodeType::Adr),
            "alternative" => Some(NodeType::Alternative),
            "concept" => Some(NodeType::Concept),
            "symbol" => Some(NodeType::Symbol),
            "reference" => Some(NodeType::Reference),
            "edge_case" => Some(NodeType::EdgeCase),
            _ => None,
        }
    }
}

impl std::fmt::Display for NodeType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A node entity in the brain graph (note, symbol, ADR, …).
///
/// # Identity
///
/// [`Node::id`] is a stable workspace-relative path slug for notes
/// (e.g. `docs/concepts/raft`) or a `symbol/…` path for AST entities.
/// See [`crate::id::node_id_from_rel_path`] and [`crate::symbols::symbol_node_id`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Node {
    /// Stable unique id (path slug or `symbol/…`).
    pub id: String,
    /// Domain taxonomy type.
    pub node_type: NodeType,
    /// Human-readable title (frontmatter `title:`, first H1, or file stem).
    pub title: String,
    /// Repository-relative file path when known.
    pub file_path: Option<String>,
    /// Optional 64-bit symbol signature (AST nodes).
    pub symbol_hash: Option<u64>,
    /// Short summary (first substantive line or doc comment).
    pub summary: Option<String>,
    /// BLAKE3 hex of source bytes for change detection (notes/symbols).
    pub content_hash: Option<String>,
    /// Unix epoch seconds; preserved across content updates.
    pub created_at: i64,
    /// Unix epoch seconds; bumped when content changes.
    pub updated_at: i64,
}

/// Directed weighted edge between two nodes.
///
/// Common `relation_type` values: `relates_to` (WikiLink), `anchors` (note→symbol),
/// plus free-form labels imported from Obsidian Canvas.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Edge {
    /// Source node id.
    pub source_id: String,
    /// Target node id.
    pub target_id: String,
    /// Edge label / relation kind.
    pub relation_type: String,
    /// Weight in a typical range near `[0, 1]` (not strictly enforced).
    pub weight: f32,
    /// Optional temporal decay rate (reserved for future ranking).
    pub decay_rate: f32,
    /// Unix epoch seconds when the edge was first created.
    pub created_at: i64,
}

/// Statistics produced by a workspace sync / index run.
///
/// Useful for CLI progress lines and agent telemetry. Counts are best-effort
/// totals for the run, not a full database census (except where noted).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncStats {
    /// Markdown files successfully processed this run.
    pub markdown_files: usize,
    /// Canvas files processed this run.
    pub canvas_files: usize,
    /// Rust source files visited this run.
    pub rust_files: usize,
    /// Nodes inserted or updated (including symbols).
    pub nodes_upserted: usize,
    /// Nodes skipped because `content_hash` matched.
    pub nodes_skipped_unchanged: usize,
    /// Edges created or upserted (including resolve-pending phase).
    pub edges_created: usize,
    /// Unresolved links remaining in `pending_links` after resolve.
    pub edges_pending: usize,
    /// Symbol anchors written or confirmed this run.
    pub symbol_anchors: usize,
    /// Whether `graph.mmap` was rewritten.
    pub mmap_written: bool,
    /// Files that failed to index (errors logged; sync continues).
    pub file_errors: usize,
}

/// Role of a node inside a context bundle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ContextRole {
    /// Direct hit from ranked FTS / tag-alias search.
    #[default]
    Seed,
    /// Reached via CSR graph expansion from a seed.
    Neighbor,
}

impl std::fmt::Display for ContextRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContextRole::Seed => write!(f, "seed"),
            ContextRole::Neighbor => write!(f, "neighbor"),
        }
    }
}

/// Context bundle returned by [`crate::Brain::context_for_prompt`].
///
/// Render with [`ContextBundle::to_markdown`] or [`ContextBundle::to_xml`]
/// (XML escapes special characters for safe agent injection).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextBundle {
    /// Original prompt / topic string.
    pub prompt: String,
    /// Requested maximum tokens (input budget).
    pub max_tokens: usize,
    /// Estimated tokens consumed by the packed bundle (~chars/4).
    pub tokens_used: usize,
    /// Packed nodes (seeds and optionally neighbors), highest score first.
    pub nodes: Vec<ContextNode>,
    /// Neighbor ids discovered via the graph (may include ids not packed).
    pub neighbor_ids: Vec<String>,
    /// Wall time for assembly in microseconds.
    pub latency_us: u64,
    /// Node count in the CSR graph when mmap was loaded (else 0).
    pub graph_nodes: usize,
    /// Edge count in the CSR graph when mmap was loaded (else 0).
    pub graph_edges: usize,
}

/// A single node entry inside an AI context bundle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextNode {
    /// Stable node id.
    pub id: String,
    /// Domain type.
    pub node_type: NodeType,
    /// Display title.
    pub title: String,
    /// Short summary if available.
    pub summary: Option<String>,
    /// Body / FTS excerpt for agent grounding (may be truncated).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub excerpt: Option<String>,
    /// Repo-relative path if available.
    pub file_path: Option<String>,
    /// Ranking score used during packing (higher is better).
    pub score_hint: f32,
    /// Whether this node was a search seed or a graph neighbor.
    pub role: ContextRole,
    /// Graph hop distance from a seed (`0` for seeds).
    pub hop: u8,
}

impl ContextBundle {
    /// Render as Markdown suitable for human reading or agent system prompts.
    pub fn to_markdown(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "## rustbrain context for: `{}`\n\n",
            self.prompt
        ));
        out.push_str(&format!(
            "- latency: {} µs | tokens: ~{}/{} | packed: {} | neighbors: {} | graph: {}n/{}e\n\n",
            self.latency_us,
            self.tokens_used,
            self.max_tokens,
            self.nodes.len(),
            self.neighbor_ids.len(),
            self.graph_nodes,
            self.graph_edges
        ));
        for n in &self.nodes {
            out.push_str(&format!(
                "### [{}] {} ({}, hop {})\n",
                n.node_type, n.title, n.role, n.hop
            ));
            out.push_str(&format!("- id: `{}`\n", n.id));
            out.push_str(&format!("- score: {:.3}\n", n.score_hint));
            if let Some(p) = &n.file_path {
                out.push_str(&format!("- path: `{}`\n", p));
            }
            if let Some(s) = &n.summary {
                out.push_str(&format!("\n> {}\n", s));
            }
            if let Some(ex) = &n.excerpt {
                out.push_str("\n```\n");
                out.push_str(ex);
                if !ex.ends_with('\n') {
                    out.push('\n');
                }
                out.push_str("```\n");
            }
            out.push('\n');
        }
        if self.nodes.is_empty() {
            out.push_str("_No nodes packed._ Try a shorter topic (key nouns only), or:\n");
            out.push_str("```\nrustbrain query \"topic\" --no-symbols --scores\nrustbrain context -p \"topic\" --no-symbols -F markdown\n```\n\n");
        }
        if !self.neighbor_ids.is_empty() {
            out.push_str("### Graph neighbor ids\n\n");
            for id in &self.neighbor_ids {
                out.push_str(&format!("- `{}`\n", id));
            }
        }
        out
    }

    /// Render as XML with entity escaping for safe embedding in tool protocols.
    pub fn to_xml(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "<rustbrain_context graph_nodes=\"{}\" graph_edges=\"{}\" latency_us=\"{}\" packed=\"{}\" tokens_used=\"{}\" max_tokens=\"{}\">\n",
            self.graph_nodes,
            self.graph_edges,
            self.latency_us,
            self.nodes.len(),
            self.tokens_used,
            self.max_tokens
        ));
        out.push_str(&format!(
            "  <prompt>{}</prompt>\n",
            xml_escape(&self.prompt)
        ));
        for n in &self.nodes {
            out.push_str(&format!(
                "  <node type=\"{}\" id=\"{}\" role=\"{}\" hop=\"{}\" score=\"{:.4}\">\n",
                xml_escape(n.node_type.as_str()),
                xml_escape(&n.id),
                n.role,
                n.hop,
                n.score_hint
            ));
            out.push_str(&format!(
                "    <title>{}</title>\n",
                xml_escape(&n.title)
            ));
            if let Some(s) = &n.summary {
                out.push_str(&format!(
                    "    <summary>{}</summary>\n",
                    xml_escape(s)
                ));
            }
            if let Some(ex) = &n.excerpt {
                out.push_str(&format!(
                    "    <excerpt>{}</excerpt>\n",
                    xml_escape(ex)
                ));
            }
            if let Some(p) = &n.file_path {
                out.push_str(&format!(
                    "    <path>{}</path>\n",
                    xml_escape(p)
                ));
            }
            out.push_str("  </node>\n");
        }
        if !self.neighbor_ids.is_empty() {
            out.push_str("  <neighbors>\n");
            for id in &self.neighbor_ids {
                out.push_str(&format!(
                    "    <id>{}</id>\n",
                    xml_escape(id)
                ));
            }
            out.push_str("  </neighbors>\n");
        }
        out.push_str("</rustbrain_context>\n");
        out
    }
}

/// Escape text for inclusion in XML character data and attribute values.
///
/// Escapes `&`, `<`, `>`, `"`, and `'`.
pub fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_escape_entities() {
        assert_eq!(xml_escape(r#"a<b>&"c""#), "a&lt;b&gt;&amp;&quot;c&quot;");
    }

    #[test]
    fn node_type_no_silent_fallback() {
        assert!(NodeType::parse("not_a_type").is_none());
        assert_eq!(NodeType::parse("concept"), Some(NodeType::Concept));
    }
}
