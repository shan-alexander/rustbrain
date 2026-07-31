//! Symbol anchors and location-independent hashing.

use serde::{Deserialize, Serialize};

/// Errors from AST hashing / parsing helpers.
#[derive(Debug, thiserror::Error)]
pub enum AstError {
    /// Tree-sitter or language load failure.
    #[error("AST error: {0}")]
    Message(String),
}

/// AST code symbol anchor details.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SymbolAnchor {
    /// Stable 64-bit BLAKE3-derived hash of the symbol signature.
    pub symbol_hash: u64,
    /// Crate name owning the symbol.
    pub crate_name: String,
    /// Module path (logical, not absolute filesystem).
    pub module_path: String,
    /// Symbol name (function, struct, trait, …).
    pub symbol_name: String,
    /// Repository-relative file path for navigation.
    pub file_path: String,
    /// 1-based start line.
    pub start_line: u32,
    /// 1-based end line.
    pub end_line: u32,
    /// Adjacent doc comment, if any.
    pub doc_comment: Option<String>,
}

/// Compute a 64-bit BLAKE3 hash for a code symbol signature.
///
/// Signature format: `crate_name::module_path::symbol_name`
///
/// **Does not include file path** so the hash is stable across file moves
/// within the same logical module path.
pub fn compute_symbol_hash(crate_name: &str, module_path: &str, symbol_name: &str) -> u64 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(crate_name.as_bytes());
    hasher.update(b"::");
    hasher.update(module_path.as_bytes());
    hasher.update(b"::");
    hasher.update(symbol_name.as_bytes());

    let result = hasher.finalize();
    let bytes = &result.as_bytes()[0..8];
    u64::from_le_bytes(bytes.try_into().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_symbol_hash() {
        let hash1 = compute_symbol_hash("rustbrain", "storage::engine", "CsrMatrix");
        let hash2 = compute_symbol_hash("rustbrain", "storage::engine", "CsrMatrix");
        assert_eq!(hash1, hash2);
        assert_ne!(hash1, 0);
    }

    #[test]
    fn hash_independent_of_path_param() {
        // Same module path → same hash (file path is not an input).
        let a = compute_symbol_hash("c", "foo::bar", "Baz");
        let b = compute_symbol_hash("c", "foo::bar", "Baz");
        assert_eq!(a, b);
        let c = compute_symbol_hash("c", "foo::other", "Baz");
        assert_ne!(a, c);
    }
}
