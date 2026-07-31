//! SQLite master store for rustbrain.
//!
//! [`Database`] owns a single connection with `foreign_keys=ON`, WAL (best-effort),
//! and a busy timeout. Schema evolution is handled by [`SCHEMA_VERSION`] migrations
//! in `schema_meta`.
//!
//! Application code should prefer [`crate::Brain`]; use `Database` directly when
//! building custom indexers or offline maintenance tools.

mod db;
mod migrations;

pub use db::Database;
/// Current on-disk SQLite schema version supported by this build.
pub use migrations::SCHEMA_VERSION;
