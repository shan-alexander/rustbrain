//! # rustbrain-core
//!
//! Project-scoped **knowledge graph engine** for software repositories: Markdown notes,
//! optional Rust AST symbols, ranked full-text search, and graph-aware AI agent context.
//!
//! ## Quick start
//!
//! ```no_run
//! use rustbrain_core::{Brain, ContextOptions, QueryOptions, Result};
//!
//! fn main() -> Result<()> {
//!     let mut brain = Brain::open_or_create(".")?;
//!     brain.sync()?;
//!     let hits = brain.query_ranked("raft", &QueryOptions::human())?;
//!     let ctx = brain.context_for_prompt("explain raft", 1024)?;
//!     println!("{}", ctx.to_markdown());
//!     let _ = hits;
//!     Ok(())
//! }
//! ```
//!
//! ## Published crates
//!
//! | Package | Role |
//! |---------|------|
//! | **`rustbrain-core`** | This library |
//! | **`rustbrain`** | CLI (`bootstrap`, `doctor`, `note`, `sync`, …) |
//!
//! ## Feature flags
//!
//! | Feature | Default | Purpose |
//! |---------|---------|---------|
//! | `ast` | yes | Tree-sitter Rust symbol extraction |
//! | `obsidian` | yes | WikiLinks, frontmatter, Canvas |
//! | `mmap` | yes | CSR `graph.mmap` |
//! | `watch` | no | Debounced filesystem watcher |
//! | `jshift` | no | In-place JSON helpers |
//! | `full` | no | All optional features |

#![warn(missing_docs)]

#[cfg(feature = "ast")]
pub mod ast;
pub mod bootstrap;
pub mod brain;
pub mod context;
pub mod doctor;
pub mod error;
pub mod exporter;
pub mod fts;
pub mod id;
pub mod ignore;
pub mod indexer;
pub mod mmap;
pub mod mutator;
pub mod note;
#[cfg(feature = "obsidian")]
pub mod obsidian;
pub mod query;
pub mod registry;
pub mod storage;
pub mod symbols;
pub mod types;
pub mod watch;

pub use bootstrap::{
    bootstrap_noninteractive, bootstrap_workspace, BootstrapAction, BootstrapMode,
    BootstrapOptions, BootstrapReport,
};
pub use brain::{find_brain_dir, Brain};
pub use context::ContextOptions;
pub use fts::{prepare_search_query, tokenize_query, PreparedQuery};
pub use doctor::{run_doctor, DoctorFinding, DoctorReport, DoctorSeverity};
pub use error::{BrainError, Result};
pub use exporter::{BrainExporter, BrainImporter, PortableBrainBundle, BUNDLE_VERSION};
pub use ignore::{recommended_ignore_extras, write_rustbrainignore, IgnoreSet};
pub use indexer::WorkspaceIndexer;
pub use mmap::{CsrCompiler, CsrMmapGraph, MMAP_VERSION};
pub use note::{create_note, default_dir_for_type, slugify_title, NoteCreated, NoteNewOptions};
pub use query::{PendingLink, QueryOptions, RankedHit};
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
