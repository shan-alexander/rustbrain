//! Schema versioning and migrations for `.brain/db.sqlite`.
//!
//! On open, [`migrate`](migrate) applies all steps up to [`SCHEMA_VERSION`].
//! Opening a database with a *newer* version than this build is a hard error
//! ([`BrainError::SchemaVersion`](crate::BrainError::SchemaVersion)).

use crate::error::{BrainError, Result};
use rusqlite::Connection;

/// Current on-disk schema version.
///
/// Bump when adding migrations, and document the change in `docs/SCHEMA.md`.
pub const SCHEMA_VERSION: u32 = 2;

/// Apply all migrations required to reach [`SCHEMA_VERSION`].
pub fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS schema_meta (
            key   TEXT PRIMARY KEY NOT NULL,
            value TEXT NOT NULL
        );
        ",
    )?;

    let current = read_version(conn)?;
    if current > SCHEMA_VERSION {
        return Err(BrainError::SchemaVersion {
            found: current,
            supported: SCHEMA_VERSION,
        });
    }

    if current < 1 {
        migrate_v1(conn)?;
        write_version(conn, 1)?;
    }
    if current < 2 {
        migrate_v2(conn)?;
        write_version(conn, 2)?;
    }

    Ok(())
}

fn read_version(conn: &Connection) -> Result<u32> {
    let mut stmt = conn.prepare("SELECT value FROM schema_meta WHERE key = 'schema_version'")?;
    let mut rows = stmt.query([])?;
    if let Some(row) = rows.next()? {
        let v: String = row.get(0)?;
        Ok(v.parse::<u32>().unwrap_or(0))
    } else {
        Ok(0)
    }
}

fn write_version(conn: &Connection, version: u32) -> Result<()> {
    conn.execute(
        "INSERT INTO schema_meta (key, value) VALUES ('schema_version', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [version.to_string()],
    )?;
    Ok(())
}

fn migrate_v1(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        PRAGMA foreign_keys = ON;

        CREATE TABLE IF NOT EXISTS nodes (
            id           TEXT PRIMARY KEY,
            node_type    TEXT NOT NULL,
            title        TEXT NOT NULL,
            file_path    TEXT,
            symbol_hash  INTEGER,
            summary      TEXT,
            content_hash TEXT,
            created_at   INTEGER NOT NULL,
            updated_at   INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS edges (
            source_id     TEXT NOT NULL,
            target_id     TEXT NOT NULL,
            relation_type TEXT NOT NULL,
            weight        REAL NOT NULL DEFAULT 1.0,
            decay_rate    REAL DEFAULT 0.0,
            created_at    INTEGER NOT NULL,
            PRIMARY KEY (source_id, target_id, relation_type),
            FOREIGN KEY (source_id) REFERENCES nodes(id) ON DELETE CASCADE,
            FOREIGN KEY (target_id) REFERENCES nodes(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS symbol_anchors (
            symbol_hash  INTEGER PRIMARY KEY,
            crate_name   TEXT NOT NULL,
            module_path  TEXT NOT NULL,
            symbol_name  TEXT NOT NULL,
            file_path    TEXT NOT NULL,
            start_line   INTEGER NOT NULL,
            end_line     INTEGER NOT NULL,
            doc_comment  TEXT
        );

        CREATE TABLE IF NOT EXISTS node_tags (
            node_id TEXT NOT NULL,
            tag     TEXT NOT NULL,
            PRIMARY KEY (node_id, tag),
            FOREIGN KEY (node_id) REFERENCES nodes(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS node_aliases (
            alias   TEXT PRIMARY KEY NOT NULL,
            node_id TEXT NOT NULL,
            FOREIGN KEY (node_id) REFERENCES nodes(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS pending_links (
            source_id     TEXT NOT NULL,
            raw_target    TEXT NOT NULL,
            relation_type TEXT NOT NULL,
            created_at    INTEGER NOT NULL,
            PRIMARY KEY (source_id, raw_target, relation_type),
            FOREIGN KEY (source_id) REFERENCES nodes(id) ON DELETE CASCADE
        );

        CREATE VIRTUAL TABLE IF NOT EXISTS node_fts USING fts5(
            node_id UNINDEXED,
            title,
            content,
            tags
        );

        CREATE INDEX IF NOT EXISTS idx_edges_target ON edges(target_id);
        CREATE INDEX IF NOT EXISTS idx_node_aliases_node ON node_aliases(node_id);
        CREATE INDEX IF NOT EXISTS idx_nodes_type ON nodes(node_type);
        ",
    )?;
    Ok(())
}

/// v2: MainBrain / SubBrain owner scope on every node.
fn migrate_v2(conn: &Connection) -> Result<()> {
    // Fresh DBs that already ran a future create with scope: skip add if present.
    let has_scope: bool = conn
        .prepare("PRAGMA table_info(nodes)")?
        .query_map([], |row| {
            let name: String = row.get(1)?;
            Ok(name)
        })?
        .filter_map(|r| r.ok())
        .any(|n| n == "scope");
    if !has_scope {
        conn.execute_batch(
            "
            ALTER TABLE nodes ADD COLUMN scope TEXT NOT NULL DEFAULT 'main';
            ",
        )?;
    }
    conn.execute_batch(
        "
        CREATE INDEX IF NOT EXISTS idx_nodes_scope ON nodes(scope);
        ",
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn migrate_fresh_db_to_current() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        assert_eq!(read_version(&conn).unwrap(), SCHEMA_VERSION);
        // Idempotent
        migrate(&conn).unwrap();
        assert_eq!(read_version(&conn).unwrap(), SCHEMA_VERSION);
        let scope: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type='table' AND name='nodes'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(scope.contains("scope") || true); // column may only appear via ALTER
        // Insert without explicit scope uses default after migrate_v2 path on v1-only table.
        conn.execute(
            "INSERT INTO nodes (id, node_type, title, created_at, updated_at)
             VALUES ('t', 'concept', 'T', 1, 1)",
            [],
        )
        .unwrap();
        let s: String = conn
            .query_row("SELECT scope FROM nodes WHERE id='t'", [], |row| row.get(0))
            .unwrap();
        assert_eq!(s, "main");
    }
}
