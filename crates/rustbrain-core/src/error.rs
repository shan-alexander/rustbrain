//! Typed errors for the rustbrain library boundary.
//!
//! CLI code may wrap these in `anyhow`; library consumers should match on
//! [`BrainError`] directly so callers can distinguish “brain missing” from I/O
//! or schema mismatches.

use std::path::PathBuf;

/// Error type returned by all public `rustbrain-core` APIs.
///
/// Variants map roughly to subsystems: storage, mmap cache, FTS, indexing, and
/// portable bundles. Display messages are stable enough for CLI stderr; the
/// enum itself is not yet `#[non_exhaustive]` so minor releases may add variants
/// carefully (consider matching with a wildcard in long-lived apps).
#[derive(Debug, thiserror::Error)]
pub enum BrainError {
    /// Filesystem or OS I/O failure.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Underlying SQLite / rusqlite error (constraints, SQL, busy, etc.).
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    /// JSON serialization or deserialization failure (bundles, markers, registry).
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// No `.brain/db.sqlite` at the expected path (call [`crate::Brain::create`] or `sync` first).
    #[error("brain not found at {path}")]
    BrainNotFound {
        /// Path to the missing `.brain` directory (or parent).
        path: PathBuf,
    },

    /// `.brain` exists but is structurally invalid.
    #[error("invalid brain directory: {path}: {reason}")]
    InvalidBrain {
        /// Path that failed validation.
        path: PathBuf,
        /// Human-readable reason.
        reason: String,
    },

    /// On-disk schema is newer than this library build (upgrade rustbrain).
    #[error("schema version mismatch: found {found}, supported {supported}")]
    SchemaVersion {
        /// Version found in `schema_meta`.
        found: u32,
        /// Maximum version this binary understands.
        supported: u32,
    },

    /// CSR `graph.mmap` magic/version/bounds validation failure.
    #[error("mmap format error: {0}")]
    MmapFormat(String),

    /// Empty or illegal full-text query after sanitization.
    #[error("FTS query error: {0}")]
    FtsQuery(String),

    /// Called an API that requires a Cargo feature not enabled for this build.
    #[error("feature not enabled: {0}")]
    FeatureDisabled(&'static str),

    /// Tree-sitter / AST extraction failure.
    #[error("AST parse error: {0}")]
    Ast(String),

    /// Workspace walk / index pipeline failure.
    #[error("indexer error: {0}")]
    Indexer(String),

    /// Portable `.brainbundle` export or import failure.
    #[error("export/import error: {0}")]
    Bundle(String),

    /// Catch-all for rare internal conditions with a message.
    #[error("{0}")]
    Other(String),
}

impl BrainError {
    /// Construct an [`BrainError::Other`] from any displayable message.
    pub fn other(msg: impl Into<String>) -> Self {
        Self::Other(msg.into())
    }

    /// Construct an [`BrainError::MmapFormat`] from any displayable message.
    pub fn mmap(msg: impl Into<String>) -> Self {
        Self::MmapFormat(msg.into())
    }

    /// Construct an [`BrainError::Indexer`] from any displayable message.
    pub fn indexer(msg: impl Into<String>) -> Self {
        Self::Indexer(msg.into())
    }

    /// Construct an [`BrainError::Bundle`] from any displayable message.
    pub fn bundle(msg: impl Into<String>) -> Self {
        Self::Bundle(msg.into())
    }
}

/// Convenient result alias for library APIs: `Result<T, BrainError>`.
pub type Result<T> = std::result::Result<T, BrainError>;
