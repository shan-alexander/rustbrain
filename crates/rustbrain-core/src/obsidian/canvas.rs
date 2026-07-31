//! Obsidian Canvas (`.canvas`) JSON parsing.

use serde::{Deserialize, Serialize};

/// Errors parsing Canvas documents.
#[derive(Debug, thiserror::Error)]
pub enum CanvasError {
    /// JSON deserialization failed.
    #[error("failed to parse Obsidian .canvas JSON: {0}")]
    Json(#[from] serde_json::Error),
}

/// An Obsidian Canvas (`.canvas`) document.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ObsidianCanvas {
    /// Canvas nodes.
    #[serde(default)]
    pub nodes: Vec<CanvasNode>,
    /// Canvas edges.
    #[serde(default)]
    pub edges: Vec<CanvasEdge>,
}

/// A node on an Obsidian canvas.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanvasNode {
    /// Node id (canvas-local).
    pub id: String,
    /// Node type: `text`, `file`, `link`, `group`.
    #[serde(rename = "type")]
    pub node_type: String,
    /// Text content for text nodes.
    pub text: Option<String>,
    /// File path for file nodes.
    pub file: Option<String>,
    /// URL for link nodes.
    pub url: Option<String>,
    /// X position.
    pub x: f64,
    /// Y position.
    pub y: f64,
    /// Width.
    pub width: f64,
    /// Height.
    pub height: f64,
}

/// An edge on an Obsidian canvas.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanvasEdge {
    /// Edge id.
    pub id: String,
    /// Source canvas node id.
    #[serde(rename = "fromNode")]
    pub from_node: String,
    /// Target canvas node id.
    #[serde(rename = "toNode")]
    pub to_node: String,
    /// Optional label (used as relation type).
    pub label: Option<String>,
    /// Optional from side.
    #[serde(rename = "fromSide")]
    pub from_side: Option<String>,
    /// Optional to side.
    #[serde(rename = "toSide")]
    pub to_side: Option<String>,
}

impl ObsidianCanvas {
    /// Parse an Obsidian `.canvas` JSON document.
    pub fn parse_str(json_content: &str) -> Result<Self, CanvasError> {
        Ok(serde_json::from_str::<Self>(json_content)?)
    }

    /// Extract relational triples `(source, target, relation_type)` from canvas edges.
    ///
    /// File nodes resolve to their path stem (without `.md`); text nodes use the
    /// first heading line when present.
    pub fn extract_relationships(&self) -> Vec<(String, String, String)> {
        let mut relationships = Vec::new();

        let mut id_map = std::collections::HashMap::new();
        for node in &self.nodes {
            let target_name = if let Some(file) = &node.file {
                file.trim_end_matches(".md").to_string()
            } else if let Some(text) = &node.text {
                text.lines()
                    .next()
                    .unwrap_or(&node.id)
                    .trim_start_matches("# ")
                    .to_string()
            } else {
                node.id.clone()
            };
            id_map.insert(node.id.as_str(), target_name);
        }

        for edge in &self.edges {
            if let (Some(src), Some(dst)) = (
                id_map.get(edge.from_node.as_str()),
                id_map.get(edge.to_node.as_str()),
            ) {
                let rel = edge
                    .label
                    .clone()
                    .unwrap_or_else(|| "relates_to".to_string());
                relationships.push((src.clone(), dst.clone(), rel));
            }
        }

        relationships
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_obsidian_canvas() {
        let json = r##"{
            "nodes": [
                { "id": "n1", "type": "file", "file": "concepts/raft.md", "x": 0, "y": 0, "width": 200, "height": 100 },
                { "id": "n2", "type": "text", "text": "# Log Compaction", "x": 300, "y": 0, "width": 200, "height": 100 }
            ],
            "edges": [
                { "id": "e1", "fromNode": "n1", "toNode": "n2", "label": "implements" }
            ]
        }"##;

        let canvas = ObsidianCanvas::parse_str(json).unwrap();
        assert_eq!(canvas.nodes.len(), 2);
        assert_eq!(canvas.edges.len(), 1);

        let rels = canvas.extract_relationships();
        assert_eq!(rels.len(), 1);
        assert_eq!(rels[0].0, "concepts/raft");
        assert_eq!(rels[0].1, "Log Compaction");
        assert_eq!(rels[0].2, "implements");
    }
}
