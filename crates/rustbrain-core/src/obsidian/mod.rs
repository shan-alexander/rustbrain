//! Obsidian-compatible WikiLink, frontmatter, and Canvas parsing.
//!
//! Gated by the `obsidian` feature (default). No dependency on the Obsidian
//! application — ingest-only for the knowledge graph.

pub mod canvas;
pub mod frontmatter;
pub mod wikilink;

pub use canvas::{CanvasEdge, CanvasNode, ObsidianCanvas};
pub use frontmatter::{parse_frontmatter, Frontmatter};
pub use wikilink::{extract_wikilinks, WikiLink};
