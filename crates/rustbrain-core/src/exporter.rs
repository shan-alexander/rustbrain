//! Portable `.brainbundle` export and import (schema v1).
//!
//! A brainbundle is pretty-printed JSON describing nodes and edges so a “brain”
//! can move between repositories. With `decouple_ast = true`, Layer B (repo-local
//! file paths and symbol nodes) is stripped — suitable for sharing concepts
//! without binding to a specific checkout.

use crate::error::{BrainError, Result};
use crate::storage::Database;
use crate::types::{Edge, Node, NodeType};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

/// Version of the portable bundle JSON schema.
///
/// Increment when adding required fields or changing semantics. Readers must
/// reject bundles with `version > BUNDLE_VERSION`.
pub const BUNDLE_VERSION: u32 = 1;

/// Portable knowledge bundle (Layer A concept core; optional AST decoupling).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortableBrainBundle {
    /// Bundle format version (must be ≤ [`BUNDLE_VERSION`] to import).
    pub version: u32,
    /// Display name (usually the output file stem).
    pub name: String,
    /// Unix epoch seconds when the bundle was written.
    pub created_at: i64,
    /// Nodes included in the bundle.
    pub nodes: Vec<Node>,
    /// Edges with full metadata (`relation_type`, weights, timestamps).
    pub edges: Vec<Edge>,
}

/// Export helper for writing `.brainbundle` files.
pub struct BrainExporter;

impl BrainExporter {
    /// Export database nodes and edges into a portable `.brainbundle` JSON file.
    ///
    /// Writes via a temporary file then `rename` for crash safety.
    ///
    /// When `decouple_ast` is true:
    /// - drops [`NodeType::Symbol`] nodes
    /// - clears `file_path` / `symbol_hash` on remaining nodes
    /// - drops edges whose endpoints were removed
    pub fn export_bundle<P: AsRef<Path>>(
        db: &Database,
        output_path: P,
        decouple_ast: bool,
    ) -> Result<()> {
        let mut nodes = db.get_all_nodes()?;
        if decouple_ast {
            nodes.retain(|n| n.node_type != NodeType::Symbol);
            for node in &mut nodes {
                node.file_path = None;
                node.symbol_hash = None;
            }
        }

        let mut edges = db.get_all_edges()?;
        if decouple_ast {
            let keep: std::collections::HashSet<String> =
                nodes.iter().map(|n| n.id.clone()).collect();
            edges.retain(|e| keep.contains(&e.source_id) && keep.contains(&e.target_id));
        }

        let bundle = PortableBrainBundle {
            version: BUNDLE_VERSION,
            name: output_path
                .as_ref()
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("brain")
                .to_string(),
            created_at: chrono::Utc::now().timestamp(),
            nodes,
            edges,
        };

        let json = serde_json::to_string_pretty(&bundle)?;
        // Atomic write
        let path = output_path.as_ref();
        let tmp = path.with_extension("brainbundle.tmp");
        fs::write(&tmp, json)?;
        fs::rename(&tmp, path)?;
        Ok(())
    }
}

/// Import helper for reading `.brainbundle` files.
pub struct BrainImporter;

impl BrainImporter {
    /// Import a `.brainbundle` into the database (upsert nodes, then edges).
    ///
    /// Runs inside a single transaction. Edge FK failures abort the import
    /// (integrity over partial success).
    ///
    /// # Returns
    ///
    /// Number of nodes upserted.
    pub fn import_bundle<P: AsRef<Path>>(db: &Database, input_path: P) -> Result<usize> {
        let text = fs::read_to_string(input_path.as_ref())?;
        let bundle: PortableBrainBundle = serde_json::from_str(&text)?;
        if bundle.version > BUNDLE_VERSION {
            return Err(BrainError::bundle(format!(
                "bundle version {} is newer than supported {}",
                bundle.version, BUNDLE_VERSION
            )));
        }

        let mut count = 0usize;
        db.with_transaction(|conn| {
            for node in &bundle.nodes {
                db.insert_node_on(conn, node)?;
                count += 1;
            }
            for edge in &bundle.edges {
                // FK failures surface immediately (data integrity over silence).
                db.insert_edge_on(conn, edge)?;
            }
            Ok(())
        })?;
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::NodeType;
    use tempfile::tempdir;

    #[test]
    fn export_preserves_relation_type_and_decouples_ast() {
        let dir = tempdir().unwrap();
        let db = Database::open_in_memory().unwrap();
        let node = Node {
            id: "docs/raft".into(),
            node_type: NodeType::Concept,
            title: "Raft".into(),
            file_path: Some("docs/raft.md".into()),
            symbol_hash: Some(99),
            summary: Some("s".into()),
            content_hash: Some("h".into()),
            created_at: 1,
            updated_at: 1,
        };
        let sym = Node {
            id: "symbol/c/m/Foo".into(),
            node_type: NodeType::Symbol,
            title: "Foo".into(),
            file_path: Some("src/lib.rs".into()),
            symbol_hash: Some(1),
            summary: None,
            content_hash: None,
            created_at: 1,
            updated_at: 1,
        };
        db.insert_node(&node).unwrap();
        db.insert_node(&sym).unwrap();
        db.insert_edge(&Edge {
            source_id: "docs/raft".into(),
            target_id: "symbol/c/m/Foo".into(),
            relation_type: "implements".into(),
            weight: 0.5,
            decay_rate: 0.0,
            created_at: 7,
        })
        .unwrap();

        let path = dir.path().join("export.brainbundle");
        BrainExporter::export_bundle(&db, &path, true).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        let bundle: PortableBrainBundle = serde_json::from_str(&text).unwrap();
        assert_eq!(bundle.version, BUNDLE_VERSION);
        assert_eq!(bundle.nodes.len(), 1);
        assert!(bundle.nodes[0].file_path.is_none());
        assert!(bundle.nodes[0].symbol_hash.is_none());
        // Edge to symbol stripped because target removed
        assert!(bundle.edges.is_empty());
    }

    #[test]
    fn export_import_roundtrip_edges() {
        let dir = tempdir().unwrap();
        let db = Database::open_in_memory().unwrap();
        for id in ["a", "b"] {
            db.insert_node(&Node {
                id: id.into(),
                node_type: NodeType::Concept,
                title: id.into(),
                file_path: None,
                symbol_hash: None,
                summary: None,
                content_hash: None,
                created_at: 1,
                updated_at: 1,
            })
            .unwrap();
        }
        db.insert_edge(&Edge {
            source_id: "a".into(),
            target_id: "b".into(),
            relation_type: "blocks".into(),
            weight: 1.0,
            decay_rate: 0.2,
            created_at: 9,
        })
        .unwrap();

        let path = dir.path().join("b.brainbundle");
        BrainExporter::export_bundle(&db, &path, false).unwrap();

        let db2 = Database::open_in_memory().unwrap();
        BrainImporter::import_bundle(&db2, &path).unwrap();
        let edges = db2.get_all_edges().unwrap();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].relation_type, "blocks");
        assert!((edges[0].decay_rate - 0.2).abs() < 1e-6);
    }
}
