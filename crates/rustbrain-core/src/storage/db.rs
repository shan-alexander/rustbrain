//! SQLite database handle with transactional, FTS-safe APIs.
//!
//! Prefer [`crate::Brain`] for application code. Use [`Database`] when building
//! custom tools that need direct control over transactions, FTS, or migrations.

use crate::error::{BrainError, Result};
use crate::fts::escape_fts5_query;
use crate::types::{Edge, Node, NodeType};
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::{HashMap, HashSet};
use std::path::Path;

#[cfg(feature = "ast")]
use crate::ast::SymbolAnchor;

/// Master transactional store for a single brain (`.brain/db.sqlite`).
///
/// The connection is crate-visible for ranked-query helpers; external
/// consumers use typed methods only.
pub struct Database {
    pub(crate) conn: Connection,
}

impl Database {
    /// Open or create a database at `path`, apply pragmas, and run migrations.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let conn = Connection::open(path.as_ref())?;
        let db = Self { conn };
        db.configure_connection()?;
        super::migrations::migrate(&db.conn)?;
        Ok(db)
    }

    /// Open an in-memory database (tests).
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let db = Self { conn };
        db.configure_connection()?;
        super::migrations::migrate(&db.conn)?;
        Ok(db)
    }

    fn configure_connection(&self) -> Result<()> {
        self.conn.pragma_update(None, "foreign_keys", true)?;
        self.conn.pragma_update(None, "busy_timeout", 5000i32)?;
        // WAL is best-effort (no-op / different for pure in-memory DBs).
        let _ = self.conn.pragma_update(None, "journal_mode", "WAL");
        let _ = self.conn.pragma_update(None, "synchronous", "NORMAL");
        Ok(())
    }

    /// Run `f` inside a single transaction (unchecked; suitable for &self).
    pub fn with_transaction<T, F>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&Connection) -> Result<T>,
    {
        let tx = self.conn.unchecked_transaction()?;
        let result = f(&tx)?;
        tx.commit()?;
        Ok(result)
    }

    /// Insert or update a node. Preserves `created_at` on conflict when the
    /// existing row is older (caller should pass original created_at when known).
    pub fn insert_node(&self, node: &Node) -> Result<()> {
        self.insert_node_on(&self.conn, node)
    }

    pub(crate) fn insert_node_on(&self, conn: &Connection, node: &Node) -> Result<()> {
        // Preserve created_at if node already exists.
        let existing_created: Option<i64> = conn
            .query_row(
                "SELECT created_at FROM nodes WHERE id = ?1",
                [&node.id],
                |row| row.get(0),
            )
            .optional()?;

        let created_at = existing_created.unwrap_or(node.created_at);

        conn.execute(
            "INSERT INTO nodes (id, node_type, title, file_path, symbol_hash, summary, content_hash, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(id) DO UPDATE SET
                node_type = excluded.node_type,
                title = excluded.title,
                file_path = excluded.file_path,
                symbol_hash = excluded.symbol_hash,
                summary = excluded.summary,
                content_hash = excluded.content_hash,
                updated_at = excluded.updated_at",
            params![
                node.id,
                node.node_type.as_str(),
                node.title,
                node.file_path,
                node.symbol_hash.map(|h| h as i64),
                node.summary,
                node.content_hash,
                created_at,
                node.updated_at,
            ],
        )?;
        Ok(())
    }

    /// Fetch content_hash for a node if present.
    pub fn get_content_hash(&self, node_id: &str) -> Result<Option<String>> {
        let v: Option<String> = self
            .conn
            .query_row(
                "SELECT content_hash FROM nodes WHERE id = ?1",
                [node_id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(v)
    }

    /// Insert or update a weighted edge. Both endpoints must exist.
    pub fn insert_edge(&self, edge: &Edge) -> Result<()> {
        self.insert_edge_on(&self.conn, edge)
    }

    pub(crate) fn insert_edge_on(&self, conn: &Connection, edge: &Edge) -> Result<()> {
        conn.execute(
            "INSERT INTO edges (source_id, target_id, relation_type, weight, decay_rate, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(source_id, target_id, relation_type) DO UPDATE SET
                weight = excluded.weight,
                decay_rate = excluded.decay_rate",
            params![
                edge.source_id,
                edge.target_id,
                edge.relation_type,
                edge.weight,
                edge.decay_rate,
                edge.created_at,
            ],
        )?;
        Ok(())
    }

    /// Record an unresolved WikiLink for later resolution / reporting.
    pub fn insert_pending_link(
        &self,
        source_id: &str,
        raw_target: &str,
        relation_type: &str,
        created_at: i64,
    ) -> Result<()> {
        self.insert_pending_link_on(&self.conn, source_id, raw_target, relation_type, created_at)
    }

    pub(crate) fn insert_pending_link_on(
        &self,
        conn: &Connection,
        source_id: &str,
        raw_target: &str,
        relation_type: &str,
        created_at: i64,
    ) -> Result<()> {
        conn.execute(
            "INSERT INTO pending_links (source_id, raw_target, relation_type, created_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(source_id, raw_target, relation_type) DO NOTHING",
            params![source_id, raw_target, relation_type, created_at],
        )?;
        Ok(())
    }

    /// Clear all pending links originating from `source_id` (before re-resolve).
    pub fn clear_pending_links_for(&self, source_id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM pending_links WHERE source_id = ?1", [source_id])?;
        Ok(())
    }

    pub(crate) fn clear_pending_links_for_on(
        &self,
        conn: &Connection,
        source_id: &str,
    ) -> Result<()> {
        conn.execute("DELETE FROM pending_links WHERE source_id = ?1", [source_id])?;
        Ok(())
    }

    /// Clear outbound relates_to edges for a source before re-link (idempotent reindex).
    pub(crate) fn clear_edges_from_on(
        &self,
        conn: &Connection,
        source_id: &str,
        relation_type: &str,
    ) -> Result<()> {
        conn.execute(
            "DELETE FROM edges WHERE source_id = ?1 AND relation_type = ?2",
            params![source_id, relation_type],
        )?;
        Ok(())
    }

    /// Delete all soft auto-edges (`relation_type` like `auto_%`).
    pub fn clear_all_auto_edges(&self) -> Result<()> {
        self.conn
            .execute("DELETE FROM edges WHERE relation_type LIKE 'auto_%'", [])?;
        Ok(())
    }

    /// Delete soft auto-edges where `node_id` is source or target.
    pub fn clear_auto_edges_involving(&self, node_id: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM edges WHERE relation_type LIKE 'auto_%' AND (source_id = ?1 OR target_id = ?1)",
            [node_id],
        )?;
        Ok(())
    }

    /// Insert or update a code symbol anchor row (AST metadata).
    #[cfg(feature = "ast")]
    pub fn insert_symbol_anchor(&self, anchor: &SymbolAnchor) -> Result<()> {
        self.conn.execute(
            "INSERT INTO symbol_anchors (symbol_hash, crate_name, module_path, symbol_name, file_path, start_line, end_line, doc_comment)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(symbol_hash) DO UPDATE SET
                crate_name = excluded.crate_name,
                module_path = excluded.module_path,
                symbol_name = excluded.symbol_name,
                file_path = excluded.file_path,
                start_line = excluded.start_line,
                end_line = excluded.end_line,
                doc_comment = excluded.doc_comment",
            params![
                anchor.symbol_hash as i64,
                anchor.crate_name,
                anchor.module_path,
                anchor.symbol_name,
                anchor.file_path,
                anchor.start_line,
                anchor.end_line,
                anchor.doc_comment,
            ],
        )?;
        Ok(())
    }

    /// Replace the full tag set for a node.
    pub fn replace_node_tags(&self, node_id: &str, tags: &[String]) -> Result<()> {
        self.replace_node_tags_on(&self.conn, node_id, tags)
    }

    pub(crate) fn replace_node_tags_on(
        &self,
        conn: &Connection,
        node_id: &str,
        tags: &[String],
    ) -> Result<()> {
        conn.execute("DELETE FROM node_tags WHERE node_id = ?1", [node_id])?;
        for tag in tags {
            conn.execute(
                "INSERT OR IGNORE INTO node_tags (node_id, tag) VALUES (?1, ?2)",
                params![node_id, tag],
            )?;
        }
        Ok(())
    }

    /// Replace aliases for a node.
    pub fn replace_node_aliases(&self, node_id: &str, aliases: &[String]) -> Result<()> {
        self.replace_node_aliases_on(&self.conn, node_id, aliases)
    }

    pub(crate) fn replace_node_aliases_on(
        &self,
        conn: &Connection,
        node_id: &str,
        aliases: &[String],
    ) -> Result<()> {
        conn.execute("DELETE FROM node_aliases WHERE node_id = ?1", [node_id])?;
        for alias in aliases {
            let key = alias.trim().to_lowercase();
            if key.is_empty() {
                continue;
            }
            conn.execute(
                "INSERT INTO node_aliases (alias, node_id) VALUES (?1, ?2)
                 ON CONFLICT(alias) DO UPDATE SET node_id = excluded.node_id",
                params![key, node_id],
            )?;
        }
        Ok(())
    }

    /// Idempotent FTS upsert: delete existing rows for `node_id`, then insert once.
    pub fn index_fts(&self, node_id: &str, title: &str, content: &str, tags: &str) -> Result<()> {
        self.index_fts_on(&self.conn, node_id, title, content, tags)
    }

    /// Fetch indexed FTS body/content for a node (for context excerpts).
    pub fn get_fts_content(&self, node_id: &str) -> Result<Option<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT content FROM node_fts WHERE node_id = ?1 LIMIT 1")?;
        let content: Option<String> = stmt
            .query_row(params![node_id], |row| row.get(0))
            .optional()?;
        Ok(content)
    }

    pub(crate) fn index_fts_on(
        &self,
        conn: &Connection,
        node_id: &str,
        title: &str,
        content: &str,
        tags: &str,
    ) -> Result<()> {
        conn.execute("DELETE FROM node_fts WHERE node_id = ?1", [node_id])?;
        conn.execute(
            "INSERT INTO node_fts (node_id, title, content, tags) VALUES (?1, ?2, ?3, ?4)",
            params![node_id, title, content, tags],
        )?;
        Ok(())
    }

    /// Search nodes using FTS5 BM25 ranking with a safely escaped query.
    pub fn search_fts(&self, query: &str) -> Result<Vec<Node>> {
        let escaped = escape_fts5_query(query)?;
        let mut stmt = self.conn.prepare(
            "SELECT n.id, n.node_type, n.title, n.file_path, n.symbol_hash, n.summary,
                    n.content_hash, n.created_at, n.updated_at
             FROM node_fts f
             JOIN nodes n ON n.id = f.node_id
             WHERE node_fts MATCH ?1
             ORDER BY rank
             LIMIT 50",
        )?;

        let rows = stmt.query_map(params![escaped], |row| {
            let type_str: String = row.get(1)?;
            let node_type = NodeType::parse(&type_str).unwrap_or(NodeType::Concept);
            let symbol_hash_raw: Option<i64> = row.get(4)?;
            Ok(Node {
                id: row.get(0)?,
                node_type,
                title: row.get(2)?,
                file_path: row.get(3)?,
                symbol_hash: symbol_hash_raw.map(|h| h as u64),
                summary: row.get(5)?,
                content_hash: row.get(6)?,
                created_at: row.get(7)?,
                updated_at: row.get(8)?,
            })
        })?;

        let mut results = Vec::new();
        for r in rows {
            results.push(r.map_err(BrainError::from)?);
        }
        Ok(results)
    }

    /// List node ids with the given `node_type` string (e.g. `"adr"`).
    pub fn list_node_ids_by_type(&self, node_type: &str) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM nodes WHERE node_type = ?1 ORDER BY id")?;
        let rows = stmt.query_map(params![node_type], |row| row.get(0))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Fetch a single node by id, if present.
    pub fn get_node(&self, id: &str) -> Result<Option<Node>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, node_type, title, file_path, symbol_hash, summary, content_hash, created_at, updated_at
             FROM nodes WHERE id = ?1",
        )?;
        let node = stmt
            .query_row(params![id], Self::map_node_row)
            .optional()?;
        Ok(node)
    }

    fn map_node_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Node> {
        let type_str: String = row.get(1)?;
        let node_type = NodeType::parse(&type_str).unwrap_or(NodeType::Concept);
        let symbol_hash_raw: Option<i64> = row.get(4)?;
        Ok(Node {
            id: row.get(0)?,
            node_type,
            title: row.get(2)?,
            file_path: row.get(3)?,
            symbol_hash: symbol_hash_raw.map(|h| h as u64),
            summary: row.get(5)?,
            content_hash: row.get(6)?,
            created_at: row.get(7)?,
            updated_at: row.get(8)?,
        })
    }

    /// Number of rows in `nodes`.
    pub fn count_nodes(&self) -> Result<usize> {
        let count: usize = self
            .conn
            .query_row("SELECT COUNT(*) FROM nodes", [], |row| row.get(0))?;
        Ok(count)
    }

    /// Number of rows in `edges`.
    pub fn count_edges(&self) -> Result<usize> {
        let count: usize = self
            .conn
            .query_row("SELECT COUNT(*) FROM edges", [], |row| row.get(0))?;
        Ok(count)
    }

    /// Number of rows in the FTS5 `node_fts` table (should equal indexed notes).
    pub fn count_fts_rows(&self) -> Result<usize> {
        let count: usize = self
            .conn
            .query_row("SELECT COUNT(*) FROM node_fts", [], |row| row.get(0))?;
        Ok(count)
    }

    /// Number of unresolved WikiLink / symbol refs in `pending_links`.
    pub fn count_pending_links(&self) -> Result<usize> {
        let count: usize = self
            .conn
            .query_row("SELECT COUNT(*) FROM pending_links", [], |row| row.get(0))?;
        Ok(count)
    }

    /// Number of AST symbol anchor rows.
    pub fn count_symbol_anchors(&self) -> Result<usize> {
        let count: usize = self
            .conn
            .query_row("SELECT COUNT(*) FROM symbol_anchors", [], |row| row.get(0))?;
        Ok(count)
    }

    /// All node IDs in stable ascending order (CSR index order).
    pub fn get_all_node_ids(&self) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM nodes ORDER BY id ASC")?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        let mut ids = Vec::new();
        for r in rows {
            ids.push(r?);
        }
        Ok(ids)
    }

    /// Full edge records (relation_type preserved).
    pub fn get_all_edges(&self) -> Result<Vec<Edge>> {
        let mut stmt = self.conn.prepare(
            "SELECT source_id, target_id, relation_type, weight, decay_rate, created_at FROM edges",
        )?;
        let rows = stmt.query_map([], |row| {
            let weight: f64 = row.get(3)?;
            let decay: f64 = row.get(4)?;
            Ok(Edge {
                source_id: row.get(0)?,
                target_id: row.get(1)?,
                relation_type: row.get(2)?,
                weight: weight as f32,
                decay_rate: decay as f32,
                created_at: row.get(5)?,
            })
        })?;

        let mut edges = Vec::new();
        for r in rows {
            edges.push(r?);
        }
        Ok(edges)
    }

    /// Lightweight edge triples for CSR compile: (source, target, weight).
    pub fn get_csr_edges(&self) -> Result<Vec<(String, String, f32)>> {
        Ok(self
            .get_all_edges()?
            .into_iter()
            .map(|e| (e.source_id, e.target_id, e.weight))
            .collect())
    }

    /// All nodes ordered by id ascending (export / CSR compile order).
    pub fn get_all_nodes(&self) -> Result<Vec<Node>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, node_type, title, file_path, symbol_hash, summary, content_hash, created_at, updated_at
             FROM nodes ORDER BY id ASC",
        )?;
        let rows = stmt.query_map([], Self::map_node_row)?;
        let mut nodes = Vec::new();
        for r in rows {
            nodes.push(r?);
        }
        Ok(nodes)
    }

    /// Build resolution maps: ids, alias→id, lowercase title→id.
    #[allow(clippy::type_complexity)]
    pub fn link_resolution_maps(
        &self,
    ) -> Result<(
        HashSet<String>,
        HashMap<String, String>,
        HashMap<String, String>,
    )> {
        let mut ids = HashSet::new();
        let mut titles: HashMap<String, String> = HashMap::new();
        {
            let mut stmt = self.conn.prepare("SELECT id, title FROM nodes")?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            for r in rows {
                let (id, title) = r?;
                titles
                    .entry(title.to_lowercase())
                    .or_insert_with(|| id.clone());
                ids.insert(id);
            }
        }

        let mut aliases: HashMap<String, String> = HashMap::new();
        {
            let mut stmt = self.conn.prepare("SELECT alias, node_id FROM node_aliases")?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            for r in rows {
                let (alias, node_id) = r?;
                aliases.insert(alias, node_id);
            }
        }

        Ok((ids, aliases, titles))
    }

    /// Attempt to resolve all pending links; returns (resolved, still_pending).
    pub fn resolve_pending_links(&self) -> Result<(usize, usize)> {
        let (ids, aliases, titles) = self.link_resolution_maps()?;
        let pending: Vec<(String, String, String, i64)> = {
            let mut stmt = self.conn.prepare(
                "SELECT source_id, raw_target, relation_type, created_at FROM pending_links",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })?;
            let mut v = Vec::new();
            for r in rows {
                v.push(r?);
            }
            v
        };

        // Symbol node id set for `symbol:…` pending targets.
        let symbol_ids: std::collections::HashSet<String> = ids
            .iter()
            .filter(|id| id.starts_with("symbol/"))
            .cloned()
            .collect();

        let mut resolved = 0usize;
        for (source_id, raw_target, relation_type, created_at) in &pending {
            let target_id = if let Some(sym_path) = raw_target.strip_prefix("symbol:") {
                crate::symbols::parse_symbol_path(sym_path)
                    .and_then(|s| crate::symbols::resolve_symbol_ref(&s, &symbol_ids))
                    .or_else(|| {
                        // bare name via aliases/titles
                        crate::symbols::parse_symbol_path(sym_path).and_then(|s| {
                            crate::id::resolve_link_target(
                                &s.symbol_name,
                                &ids,
                                &aliases,
                                &titles,
                            )
                        })
                    })
            } else {
                crate::id::resolve_link_target(raw_target, &ids, &aliases, &titles)
            };

            if let Some(target_id) = target_id {
                let edge = Edge {
                    source_id: source_id.clone(),
                    target_id,
                    relation_type: relation_type.clone(),
                    weight: 1.0,
                    decay_rate: 0.0,
                    created_at: *created_at,
                };
                self.insert_edge(&edge)?;
                self.conn.execute(
                    "DELETE FROM pending_links WHERE source_id = ?1 AND raw_target = ?2 AND relation_type = ?3",
                    params![source_id, raw_target, relation_type],
                )?;
                resolved += 1;
            }
        }

        let still = self.count_pending_links()?;
        Ok((resolved, still))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::NodeType;

    fn sample_node(id: &str) -> Node {
        Node {
            id: id.to_string(),
            node_type: NodeType::Concept,
            title: id.to_string(),
            file_path: Some(format!("{id}.md")),
            symbol_hash: None,
            summary: Some("summary".into()),
            content_hash: Some("abc".into()),
            created_at: 100,
            updated_at: 100,
        }
    }

    #[test]
    fn fts_idempotent_reindex() {
        let db = Database::open_in_memory().unwrap();
        let n = sample_node("docs/raft");
        db.insert_node(&n).unwrap();
        db.index_fts(&n.id, &n.title, "raft consensus protocol", "raft").unwrap();
        db.index_fts(&n.id, &n.title, "raft consensus protocol v2", "raft").unwrap();
        assert_eq!(db.count_fts_rows().unwrap(), 1);
        let hits = db.search_fts("raft").unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn edge_requires_endpoints() {
        let db = Database::open_in_memory().unwrap();
        let edge = Edge {
            source_id: "a".into(),
            target_id: "b".into(),
            relation_type: "relates_to".into(),
            weight: 1.0,
            decay_rate: 0.0,
            created_at: 1,
        };
        assert!(db.insert_edge(&edge).is_err());
    }

    #[test]
    fn full_edge_roundtrip() {
        let db = Database::open_in_memory().unwrap();
        db.insert_node(&sample_node("a")).unwrap();
        db.insert_node(&sample_node("b")).unwrap();
        let edge = Edge {
            source_id: "a".into(),
            target_id: "b".into(),
            relation_type: "implements".into(),
            weight: 0.75,
            decay_rate: 0.1,
            created_at: 42,
        };
        db.insert_edge(&edge).unwrap();
        let edges = db.get_all_edges().unwrap();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].relation_type, "implements");
        assert!((edges[0].weight - 0.75).abs() < 1e-6);
        assert!((edges[0].decay_rate - 0.1).abs() < 1e-6);
        assert_eq!(edges[0].created_at, 42);
    }

    #[test]
    fn preserves_created_at_on_upsert() {
        let db = Database::open_in_memory().unwrap();
        let mut n = sample_node("x");
        n.created_at = 10;
        n.updated_at = 10;
        db.insert_node(&n).unwrap();
        n.created_at = 999;
        n.updated_at = 20;
        n.title = "updated".into();
        db.insert_node(&n).unwrap();
        let got = db.get_node("x").unwrap().unwrap();
        assert_eq!(got.created_at, 10);
        assert_eq!(got.updated_at, 20);
        assert_eq!(got.title, "updated");
    }
}
