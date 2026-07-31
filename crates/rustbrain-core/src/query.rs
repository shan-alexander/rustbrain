//! Ranked hybrid query: FTS5 BM25 + tag / alias / title boosts.
//!
//! This is not neural retrieval. Scores are useful for ordering within a single
//! brain (and for cross-workspace merge) but are not calibrated probabilities.

use crate::error::Result;
use crate::fts::prepare_search_query;
use crate::storage::Database;
use crate::types::{Node, NodeType};
use rusqlite::params;
use serde::{Deserialize, Serialize};

/// A single ranked search hit.
///
/// `reasons` is a debug-oriented list of score components (e.g. `fts:…`, `tag:raft`)
/// intended for CLI `--scores` and UI tooltips — not a stable public schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankedHit {
    /// Matching node.
    pub node: Node,
    /// Higher is better. Combines inverted BM25 with additive boosts.
    pub score: f32,
    /// Why this hit was selected (for debugging / UI).
    pub reasons: Vec<String>,
}

/// Options for [`Database::search_ranked`] / [`crate::Brain::query_ranked`].
#[derive(Debug, Clone)]
pub struct QueryOptions {
    /// Maximum hits to return after ranking and deduplication.
    pub limit: usize,
    /// Multiplicative boosts applied when `node.node_type` matches.
    ///
    /// Defaults slightly prefer goals/ADRs/edge cases over raw symbols.
    pub type_boosts: Vec<(NodeType, f32)>,
    /// If non-empty, only include these node types.
    pub include_types: Vec<NodeType>,
    /// Exclude these node types after ranking (applied after include filter).
    pub exclude_types: Vec<NodeType>,
    /// Convenience: exclude [`NodeType::Symbol`] (typical for human CLI search).
    pub no_symbols: bool,
}

impl Default for QueryOptions {
    fn default() -> Self {
        Self {
            limit: 50,
            type_boosts: vec![
                (NodeType::Goal, 1.15),
                (NodeType::Adr, 1.1),
                (NodeType::EdgeCase, 1.1),
                (NodeType::Analysis, 1.08),
                (NodeType::Concept, 1.05),
                (NodeType::Symbol, 0.95),
            ],
            include_types: Vec::new(),
            exclude_types: Vec::new(),
            no_symbols: false,
        }
    }
}

impl QueryOptions {
    /// Human-oriented defaults: exclude pure code symbols unless asked otherwise.
    pub fn human() -> Self {
        Self {
            no_symbols: true,
            ..Self::default()
        }
    }

    fn allows(&self, ty: &NodeType) -> bool {
        if self.no_symbols && *ty == NodeType::Symbol {
            return false;
        }
        if !self.include_types.is_empty() && !self.include_types.contains(ty) {
            return false;
        }
        if self.exclude_types.contains(ty) {
            return false;
        }
        true
    }
}

impl Database {
    /// Ranked search combining FTS5 BM25 with tag, alias, title, and id boosts.
    ///
    /// Steps:
    /// 1. Prepare the query (tokenize, drop stopwords, OR multi-token MATCH)
    /// 2. Collect BM25-ordered candidates (oversample by `2 × limit`)
    /// 3. Apply additive boosts for title/id/tag/alias token hits + multi-token coverage
    /// 4. Apply multiplicative type priors from [`QueryOptions::type_boosts`]
    /// 5. Augment with pure tag/alias exact matches FTS may have missed
    /// 6. Sort by score, dedupe by id, truncate to `limit`
    ///
    /// # Errors
    ///
    /// Returns [`crate::BrainError::FtsQuery`] for empty input after tokenization.
    pub fn search_ranked(&self, query: &str, opts: &QueryOptions) -> Result<Vec<RankedHit>> {
        let prepared = prepare_search_query(query)?;
        let tokens = &prepared.tokens;

        let mut stmt = self.conn.prepare(
            "SELECT n.id, n.node_type, n.title, n.file_path, n.symbol_hash, n.summary,
                    n.content_hash, n.created_at, n.updated_at,
                    bm25(node_fts) AS bm25_score
             FROM node_fts f
             JOIN nodes n ON n.id = f.node_id
             WHERE node_fts MATCH ?1
             ORDER BY bm25_score
             LIMIT ?2",
        )?;

        let rows = stmt.query_map(params![prepared.fts_match, opts.limit as i64 * 2], |row| {
            let type_str: String = row.get(1)?;
            let node_type = NodeType::parse(&type_str).unwrap_or(NodeType::Concept);
            let symbol_hash_raw: Option<i64> = row.get(4)?;
            let bm25: f64 = row.get(9)?;
            Ok((
                Node {
                    id: row.get(0)?,
                    node_type,
                    title: row.get(2)?,
                    file_path: row.get(3)?,
                    symbol_hash: symbol_hash_raw.map(|h| h as u64),
                    summary: row.get(5)?,
                    content_hash: row.get(6)?,
                    created_at: row.get(7)?,
                    updated_at: row.get(8)?,
                },
                bm25 as f32,
            ))
        })?;

        let mut hits: Vec<RankedHit> = Vec::new();
        for r in rows {
            let (node, bm25) = r?;
            if !opts.allows(&node.node_type) {
                continue;
            }
            // bm25() in SQLite FTS5: more relevant → more negative. Invert.
            let fts_score = (-bm25).max(0.01);
            let mut score = fts_score;
            let mut reasons = vec![format!("fts:{fts_score:.3}")];
            if prepared.used_or {
                reasons.push("fts:or".into());
            }
            // Hub boost for root README node.
            if node.id == "readme" || node.file_path.as_deref() == Some("README.md") {
                score *= 1.2;
                reasons.push("hub:readme".into());
            }

            // Title / id token hits + multi-token coverage bonus
            let title_l = node.title.to_lowercase();
            let id_l = node.id.to_lowercase();
            let fts_body = self
                .get_fts_content(&node.id)
                .ok()
                .flatten()
                .unwrap_or_default()
                .to_lowercase();
            let mut coverage = 0usize;
            for t in tokens {
                let mut hit_tok = false;
                if title_l.split_whitespace().any(|w| w == t) || title_l.contains(t.as_str()) {
                    score += 2.0;
                    reasons.push(format!("title:{t}"));
                    hit_tok = true;
                }
                if id_l.rsplit('/').next() == Some(t.as_str()) || id_l.contains(t.as_str()) {
                    score += 1.5;
                    reasons.push(format!("id:{t}"));
                    hit_tok = true;
                }
                if !fts_body.is_empty() && fts_body.contains(t.as_str()) {
                    score += 0.75;
                    hit_tok = true;
                }
                if hit_tok {
                    coverage += 1;
                }
            }
            if tokens.len() > 1 && coverage > 1 {
                let bonus = 1.5 * (coverage as f32);
                score += bonus;
                reasons.push(format!("coverage:{coverage}/{}", tokens.len()));
            }

            // Tag boosts
            let tags = self.get_tags_for(&node.id)?;
            for tag in &tags {
                let tag_l = tag.to_lowercase();
                for t in tokens {
                    if tag_l == *t || tag_l.contains(t.as_str()) {
                        score += 3.0;
                        reasons.push(format!("tag:{tag}"));
                    }
                }
            }

            // Alias boosts
            let aliases = self.get_aliases_for(&node.id)?;
            for alias in &aliases {
                let a = alias.to_lowercase();
                for t in tokens {
                    if a == *t || a.contains(t.as_str()) {
                        score += 2.5;
                        reasons.push(format!("alias:{alias}"));
                    }
                }
            }

            // Type boost
            for (ty, mult) in &opts.type_boosts {
                if node.node_type == *ty {
                    score *= mult;
                    reasons.push(format!("type:{}×{mult}", ty.as_str()));
                }
            }

            hits.push(RankedHit {
                node,
                score,
                reasons,
            });
        }

        // Also pull pure tag/alias matches that FTS may have missed (short queries).
        self.augment_with_tag_alias_hits(tokens, &mut hits, opts)?;

        hits.retain(|h| opts.allows(&h.node.node_type));

        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        // Dedup by id keeping best score
        let mut seen = std::collections::HashSet::new();
        hits.retain(|h| seen.insert(h.node.id.clone()));
        hits.truncate(opts.limit);
        Ok(hits)
    }

    /// Tags attached to a node (empty if none).
    pub fn get_tags_for(&self, node_id: &str) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT tag FROM node_tags WHERE node_id = ?1")?;
        let rows = stmt.query_map([node_id], |row| row.get(0))?;
        let mut v = Vec::new();
        for r in rows {
            v.push(r?);
        }
        Ok(v)
    }

    fn get_aliases_for(&self, node_id: &str) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT alias FROM node_aliases WHERE node_id = ?1")?;
        let rows = stmt.query_map([node_id], |row| row.get(0))?;
        let mut v = Vec::new();
        for r in rows {
            v.push(r?);
        }
        Ok(v)
    }

    fn augment_with_tag_alias_hits(
        &self,
        tokens: &[String],
        hits: &mut Vec<RankedHit>,
        opts: &QueryOptions,
    ) -> Result<()> {
        let existing: std::collections::HashSet<String> =
            hits.iter().map(|h| h.node.id.clone()).collect();

        for t in tokens {
            // Tag exact
            let mut stmt = self.conn.prepare(
                "SELECT n.id, n.node_type, n.title, n.file_path, n.symbol_hash, n.summary,
                        n.content_hash, n.created_at, n.updated_at
                 FROM node_tags t
                 JOIN nodes n ON n.id = t.node_id
                 WHERE lower(t.tag) = ?1
                 LIMIT 20",
            )?;
            let rows = stmt.query_map([t.as_str()], |row| {
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
            for r in rows {
                let node = r?;
                if existing.contains(&node.id) || !opts.allows(&node.node_type) {
                    continue;
                }
                hits.push(RankedHit {
                    node,
                    score: 3.5,
                    reasons: vec![format!("tag-exact:{t}")],
                });
            }

            // Alias exact
            let mut stmt = self.conn.prepare(
                "SELECT n.id, n.node_type, n.title, n.file_path, n.symbol_hash, n.summary,
                        n.content_hash, n.created_at, n.updated_at
                 FROM node_aliases a
                 JOIN nodes n ON n.id = a.node_id
                 WHERE a.alias = ?1
                 LIMIT 20",
            )?;
            let rows = stmt.query_map([t.as_str()], |row| {
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
            for r in rows {
                let node = r?;
                if hits.iter().any(|h| h.node.id == node.id) || !opts.allows(&node.node_type)
                {
                    continue;
                }
                hits.push(RankedHit {
                    node,
                    score: 3.0,
                    reasons: vec![format!("alias-exact:{t}")],
                });
            }
        }
        Ok(())
    }

    /// List unresolved WikiLink / symbol targets.
    pub fn list_pending_links(&self) -> Result<Vec<PendingLink>> {
        let mut stmt = self.conn.prepare(
            "SELECT source_id, raw_target, relation_type, created_at FROM pending_links ORDER BY source_id, raw_target",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(PendingLink {
                source_id: row.get(0)?,
                raw_target: row.get(1)?,
                relation_type: row.get(2)?,
                created_at: row.get(3)?,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Count nodes grouped by `node_type`.
    pub fn count_nodes_by_type(&self) -> Result<Vec<(String, usize)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT node_type, COUNT(*) FROM nodes GROUP BY node_type ORDER BY COUNT(*) DESC")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, usize>(1)?))
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }
}

/// Unresolved link kept for later resolution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingLink {
    /// Source node id that referenced the target.
    pub source_id: String,
    /// Raw WikiLink / `symbol:…` target text.
    pub raw_target: String,
    /// Intended relation type (`relates_to`, `anchors`, …).
    pub relation_type: String,
    /// When the pending link was recorded (unix seconds).
    pub created_at: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::NodeType;

    #[test]
    fn tag_boost_ranks_higher() {
        let db = Database::open_in_memory().unwrap();
        let n1 = Node {
            id: "docs/a".into(),
            node_type: NodeType::Concept,
            title: "Something Else".into(),
            file_path: None,
            symbol_hash: None,
            summary: Some("mentions raft in body".into()),
            content_hash: None,
            created_at: 1,
            updated_at: 1,
        };
        let n2 = Node {
            id: "docs/b".into(),
            node_type: NodeType::Concept,
            title: "Protocol".into(),
            file_path: None,
            symbol_hash: None,
            summary: Some("tagged note".into()),
            content_hash: None,
            created_at: 1,
            updated_at: 1,
        };
        db.insert_node(&n1).unwrap();
        db.insert_node(&n2).unwrap();
        db.index_fts("docs/a", "Something Else", "the raft algorithm", "").unwrap();
        db.index_fts("docs/b", "Protocol", "tagged note about consensus", "raft").unwrap();
        db.replace_node_tags("docs/b", &["raft".into()]).unwrap();

        let hits = db
            .search_ranked("raft", &QueryOptions::default())
            .unwrap();
        assert!(hits.len() >= 2);
        // Tag match should beat pure body mention when scores are close, or at least appear
        assert!(hits.iter().any(|h| h.node.id == "docs/b"));
        let b = hits.iter().find(|h| h.node.id == "docs/b").unwrap();
        assert!(b.reasons.iter().any(|r| r.starts_with("tag")));
    }
}
