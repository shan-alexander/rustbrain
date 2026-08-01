//! Shared shapes for serde_json vs jshift benchmarks (rustbrain-like).

use jshift::JsonView;
use serde::{Deserialize, Serialize};

/// Tiny marker like `.brain/workspace.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkspaceMeta {
    pub version: u32,
    pub workspace: String,
}

#[derive(Debug, Clone, PartialEq, JsonView)]
pub struct WorkspaceMetaView {
    #[json(path = "version")]
    pub version: u32,
    #[json(path = "workspace")]
    pub workspace: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Finding {
    pub severity: String,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DoctorReportLite {
    pub workspace: String,
    pub nodes: usize,
    pub edges: usize,
    pub pending_links: usize,
    pub orphan_notes: usize,
    pub findings: Vec<Finding>,
    pub healthy: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BundleNode {
    pub id: String,
    pub node_type: String,
    pub title: String,
    pub file_path: Option<String>,
    pub summary: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BundleEdge {
    pub source_id: String,
    pub target_id: String,
    pub relation_type: String,
    pub weight: f32,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BrainBundleLite {
    pub version: u32,
    pub name: String,
    pub created_at: i64,
    pub nodes: Vec<BundleNode>,
    pub edges: Vec<BundleEdge>,
}

pub fn sample_workspace() -> WorkspaceMeta {
    WorkspaceMeta {
        version: 1,
        workspace: "/home/farmer/dev-other/parqview".into(),
    }
}

pub fn sample_doctor() -> DoctorReportLite {
    DoctorReportLite {
        workspace: "/home/farmer/dev-other/parqview".into(),
        nodes: 120,
        edges: 95,
        pending_links: 2,
        orphan_notes: 4,
        findings: vec![
            Finding {
                severity: "info".into(),
                code: "orphan_notes".into(),
                message: "4 orphan note(s)".into(),
            },
            Finding {
                severity: "info".into(),
                code: "sparse_readme".into(),
                message: "README is thin".into(),
            },
        ],
        healthy: true,
    }
}

pub fn sample_bundle(n_nodes: usize, n_edges: usize) -> BrainBundleLite {
    let nodes: Vec<BundleNode> = (0..n_nodes)
        .map(|i| BundleNode {
            id: format!("docs/concepts/n{i}"),
            node_type: if i % 5 == 0 {
                "adr".into()
            } else {
                "concept".into()
            },
            title: format!("Note {i}"),
            file_path: Some(format!("docs/concepts/n{i}.md")),
            summary: Some(format!("summary for note {i} with some body text")),
            created_at: 1_700_000_000 + i as i64,
            updated_at: 1_700_000_100 + i as i64,
        })
        .collect();
    let edges: Vec<BundleEdge> = (0..n_edges)
        .map(|i| {
            let a = i % n_nodes.max(1);
            let b = (i * 3 + 1) % n_nodes.max(1);
            BundleEdge {
                source_id: format!("docs/concepts/n{a}"),
                target_id: format!("docs/concepts/n{b}"),
                relation_type: if i % 4 == 0 {
                    "anchors".into()
                } else {
                    "relates_to".into()
                },
                weight: 0.5 + (i % 10) as f32 * 0.05,
                created_at: 1_700_000_000 + i as i64,
            }
        })
        .collect();
    BrainBundleLite {
        version: 1,
        name: "bench-bundle".into(),
        created_at: 1_700_000_000,
        nodes,
        edges,
    }
}

/// Large JSON with noise fields — jshift path-get specialty.
pub fn large_catalog_json(n: usize) -> Vec<u8> {
    let mut s = String::from(r#"{"version":1,"noise":""#);
    s.push_str(&"x".repeat(4096));
    s.push_str(r#"","nodes":["#);
    for i in 0..n {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&format!(
            r#"{{"id":"docs/concepts/n{i}","node_type":"concept","title":"Note {i}","extra":{{"a":{i},"b":"pad-{i}","c":[1,2,3,4,5]}},"file_path":"docs/concepts/n{i}.md"}}"#
        ));
    }
    s.push_str(r#"],"meta":{"workspace":"/tmp/ws","ok":true}}"#);
    s.into_bytes()
}
