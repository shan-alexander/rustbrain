//! Tree-sitter Rust symbol extraction and BLAKE3 hashing.
//!
//! Gated by the `ast` feature (default). Used during workspace sync to create
//! `symbol/…` graph nodes and `symbol_anchors` rows.

pub mod parser;
pub mod symbol;

pub use parser::CodeAstParser;
pub use symbol::{compute_symbol_hash, AstError, SymbolAnchor};
