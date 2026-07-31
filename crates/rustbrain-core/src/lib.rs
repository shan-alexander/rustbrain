//! # rustbrain-core
//!
//! Project-scoped **knowledge graph engine** for software repositories: Markdown notes,
//! optional Rust AST symbols, ranked full-text search, and graph-aware AI agent context.
//!
//! This is the primary library crate for embedding rustbrain in tools, agents, and CLIs.
//! Humans edit plain Markdown (Obsidian-compatible WikiLinks and YAML frontmatter);
//! `rustbrain-core` derives a SQLite graph + FTS index and an optional CSR `graph.mmap` cache.
//!
//! ## Quick start
//!
//! ```no_run
//! use rustbrain_core::{Brain, ContextOptions, QueryOptions, Result};
//!
//! fn main() -> Result<()> {
//!     // Creates `<cwd>/.brain/db.sqlite` if missing
//!     let mut brain = Brain::open_or_create(".")?;
//!
//!     // Walk Markdown / Rust / Canvas, update FTS, resolve links, bake graph.mmap
//!     let stats = brain.sync()?;
//!     let _ = stats.markdown_files;
//!
//!     // Ranked search: BM25 + tag/alias/title boosts
//!     let hits = brain.query_ranked("raft consensus", &QueryOptions::default())?;
//!     for hit in hits.iter().take(3) {
//!         println!("{:.2} {}", hit.score, hit.node.title);
//!     }
//!
//!     // Agent context: seeds + CSR neighbors, packed under a token budget
//!     let ctx = brain.context_for_prompt_with(
//!         "explain log compaction",
//!         &ContextOptions {
//!             max_tokens: 1024,
//!             hop_depth: 1,
//!             ..ContextOptions::default()
//!         },
//!     )?;
//!     print!("{}", ctx.to_xml());
//!     Ok(())
//! }
//! ```
//!
//! ## Crates.io layout
//!
//! Only two packages are published:
//!
//! | Package | Role |
//! |---------|------|
//! | **`rustbrain-core`** (this crate) | Library for apps and agents |
//! | **`rustbrain`** | CLI binary (`cargo install rustbrain`) |
//!
//! AST and Obsidian parsing live **inside** this crate behind feature flags
//! (`ast`, `obsidian`) — not as separate published crates.
//!
//! ## On-disk layout
//!
//! Under `<workspace>/.brain/`:
//!
//! | Path | Role |
//! |------|------|
//! | `db.sqlite` | Source of truth (nodes, edges, FTS5, aliases, symbols) |
//! | `graph.mmap` | CSR adjacency + node-id table for fast neighborhood walks |
//! | `workspace.json` | Workspace marker |
//!
//! ## Feature flags
//!
//! | Feature | Default | Purpose |
//! |---------|---------|---------|
//! | `ast` | yes | Tree-sitter Rust symbol extraction |
//! | `obsidian` | yes | WikiLinks, frontmatter, Canvas |
//! | `mmap` | yes | Compile and read `.brain/graph.mmap` |
//! | `watch` | no | Debounced filesystem watcher |
//! | `jshift` | no | In-place JSON field mutation helpers |
//! | `full` | no | Enables every optional feature |
//!
//! ## Error handling
//!
//! Fallible APIs return [`Result<T>`] aliasing [`BrainError`]. Match on variants at
//! library boundaries; the CLI may wrap errors in `anyhow`.
//!
//! ## Non-goals (v0.1)
//!
//! - Neural embeddings / ANN indexes (product path uses `vector_dim = 0`)
//! - Multi-user servers or cloud sync
//! - Full Obsidian vault write-back (ingest only)

#![warn(missing_docs)]

#[cfg(feature = "ast")]
pub mod ast;
pub mod brain;
pub mod context;
pub mod error;
pub mod exporter;
pub mod fts;
pub mod id;
pub mod indexer;
pub mod mmap;
pub mod mutator;
#[cfg(feature = "obsidian")]
pub mod obsidian;
pub mod query;
pub mod registry;
pub mod storage;
pub mod symbols;
pub mod types;
pub mod watch;

pub use brain::Brain;
pub use context::ContextOptions;
pub use error::{BrainError, Result};
pub use exporter::{BrainExporter, BrainImporter, PortableBrainBundle, BUNDLE_VERSION};
pub use indexer::WorkspaceIndexer;
pub use mmap::{CsrCompiler, CsrMmapGraph, MMAP_VERSION};
pub use query::{QueryOptions, RankedHit};
pub use registry::{GlobalRankedHit, GlobalRegistry};
pub use storage::{Database, SCHEMA_VERSION};
pub use symbols::{extract_symbol_refs, symbol_node_id, SymbolRef};
pub use types::{
    xml_escape, ContextBundle, ContextNode, ContextRole, Edge, Node, NodeType, SyncStats,
};
pub use watch::WatchConfig;

#[cfg(feature = "ast")]
pub use ast::{compute_symbol_hash, AstError, CodeAstParser, SymbolAnchor};

#[cfg(feature = "obsidian")]
pub use obsidian::{
    extract_wikilinks, parse_frontmatter, CanvasEdge, CanvasNode, Frontmatter, ObsidianCanvas,
    WikiLink,
};

#[cfg(feature = "jshift")]
pub use mutator::{extract_json_field_slice, update_json_field_inplace};

#[cfg(feature = "watch")]
pub use watch::watch_workspace;
