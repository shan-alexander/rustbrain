//! Graph-aware AI context assembly with token budgeting.
//!
//! Pipeline used by [`crate::Brain::context_for_prompt`]:
//!
//! 1. Ranked FTS seeds ([`crate::query`]) with stopword-aware query rewrite
//! 2. Optional CSR k-hop expansion from `.brain/graph.mmap` (doc seeds preferred)
//! 3. Score fusion (seed score × edge weight × hop decay) + symbol quality filters
//! 4. Pack nodes with body excerpts until the approximate character/token budget is exhausted
//!
//! Token accounting is intentionally simple (`chars / 4`). It is good enough for
//! agent prompt packing, not for billing-grade tokenizer parity.

use crate::error::Result;
use crate::fts::{prepare_search_query, tokenize_query};
use crate::query::{QueryOptions, RankedHit};
use crate::storage::Database;
use crate::types::{ContextBundle, ContextNode, ContextRole, Node, NodeType};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Instant;

/// Max characters of body excerpt packed per note/seed.
const MAX_EXCERPT_CHARS: usize = 900;
/// Max characters for symbol neighbor excerpts (usually doc comments).
const MAX_SYMBOL_EXCERPT_CHARS: usize = 280;
/// Soft cap on packed symbol neighbors even when hop_to_symbols is on.
const MAX_PACKED_SYMBOL_NEIGHBORS: usize = 8;

/// Options controlling [`assemble_context`] and [`crate::Brain::context_for_prompt_with`].
#[derive(Debug, Clone)]
pub struct ContextOptions {
    /// Approximate max tokens for the packed bundle (`chars ≈ tokens × 4`).
    pub max_tokens: usize,
    /// Graph expansion depth (`0` = seeds only; `1` is the usual default).
    pub hop_depth: usize,
    /// Max seed nodes taken from the ranked query before expansion.
    pub max_seeds: usize,
    /// Max total nodes packed into the bundle (seeds + neighbors).
    pub max_nodes: usize,
    /// Multiplicative decay applied per hop for neighbor scores.
    pub hop_decay: f32,
    /// Exclude pure code symbols from seeds (and from neighbors unless [`Self::hop_to_symbols`]).
    pub no_symbols: bool,
    /// Only include these node types when non-empty.
    pub include_types: Vec<crate::types::NodeType>,
    /// Always exclude these node types.
    pub exclude_types: Vec<crate::types::NodeType>,
    /// When true, allow graph hops *to* symbols even if `no_symbols` filters seeds.
    /// Default true so ADR → `symbol:foo` remains useful for agents.
    pub hop_to_symbols: bool,
    /// Prefer expanding the graph from non-symbol seeds (docs/README). Default true.
    pub hop_from_docs_only: bool,
    /// Include body excerpts from FTS content (or summary fallback). Default true.
    pub include_excerpts: bool,
}

impl Default for ContextOptions {
    fn default() -> Self {
        Self {
            max_tokens: 2048,
            hop_depth: 1,
            max_seeds: 8,
            max_nodes: 24,
            hop_decay: 0.65,
            // Agent-friendly: notes first; symbols arrive via hops when useful.
            no_symbols: true,
            include_types: Vec::new(),
            exclude_types: Vec::new(),
            hop_to_symbols: true,
            hop_from_docs_only: true,
            include_excerpts: true,
        }
    }
}

impl ContextOptions {
    /// Agent-oriented defaults (same as [`Default`] as of 0.3.0).
    pub fn agent() -> Self {
        Self::default()
    }

    /// Include symbols as seeds and neighbors (power-user / debugging).
    pub fn with_symbols(mut self) -> Self {
        self.no_symbols = false;
        self
    }

    fn seed_query_opts(&self) -> QueryOptions {
        QueryOptions {
            limit: self.max_seeds.saturating_mul(2).max(8),
            no_symbols: self.no_symbols,
            include_types: self.include_types.clone(),
            exclude_types: self.exclude_types.clone(),
            ..QueryOptions::default()
        }
    }

    fn allows_pack(&self, ty: &crate::types::NodeType, role: ContextRole) -> bool {
        use crate::types::NodeType;
        if !self.include_types.is_empty() && !self.include_types.contains(ty) {
            // Allow symbol neighbors when hop_to_symbols and role is Neighbor
            if !(self.hop_to_symbols && role == ContextRole::Neighbor && *ty == NodeType::Symbol) {
                return false;
            }
        }
        if self.exclude_types.contains(ty) {
            return false;
        }
        if *ty == NodeType::Symbol {
            if role == ContextRole::Seed && self.no_symbols {
                return false;
            }
            if role == ContextRole::Neighbor && self.no_symbols && !self.hop_to_symbols {
                return false;
            }
        }
        true
    }
}

/// Intermediate scored candidate before token packing.
#[derive(Debug, Clone)]
struct Candidate {
    node: Node,
    score: f32,
    role: ContextRole,
    hop: u8,
    excerpt: Option<String>,
}

/// Assemble a graph-aware context bundle for an agent prompt.
///
/// `brain_dir` is the `.brain` directory (for loading `graph.mmap`). When the
/// mmap file is missing or the `mmap` feature is disabled, expansion is skipped
/// and only ranked seeds are packed.
///
/// # Errors
///
/// Propagates FTS / database errors from the seed query.
pub fn assemble_context(
    db: &Database,
    brain_dir: &Path,
    prompt: &str,
    opts: &ContextOptions,
) -> Result<ContextBundle> {
    let start = Instant::now();
    let char_budget = opts.max_tokens.saturating_mul(4).max(256);
    let query_tokens = prepare_search_query(prompt)
        .map(|p| p.tokens)
        .unwrap_or_else(|_| tokenize_query(prompt));

    let qopts = opts.seed_query_opts();
    let seeds = db.search_ranked(prompt, &qopts)?;

    let mut candidates: Vec<Candidate> = Vec::new();
    let mut seed_ids: HashSet<String> = HashSet::new();

    for (i, hit) in seeds.into_iter().take(opts.max_seeds).enumerate() {
        if !opts.allows_pack(&hit.node.node_type, ContextRole::Seed) {
            continue;
        }
        seed_ids.insert(hit.node.id.clone());
        // Slight rank-position prior
        let score = hit.score * (1.0 / (1.0 + i as f32 * 0.05));
        let excerpt = if opts.include_excerpts {
            load_excerpt(db, &hit.node, MAX_EXCERPT_CHARS)
        } else {
            None
        };
        candidates.push(Candidate {
            node: hit.node,
            score,
            role: ContextRole::Seed,
            hop: 0,
            excerpt,
        });
    }

    let mut graph_nodes = 0usize;
    let mut graph_edges = 0usize;
    let mut neighbor_ids: Vec<String> = Vec::new();

    #[cfg(feature = "mmap")]
    if opts.hop_depth > 0 {
        let mmap_path = brain_dir.join("graph.mmap");
        if mmap_path.exists() {
            if let Ok(graph) = crate::mmap::CsrMmapGraph::open(&mmap_path) {
                graph_nodes = graph.node_count;
                graph_edges = graph.edge_count;

                let mut best_neighbor: HashMap<String, (f32, u8)> = HashMap::new();

                for cand in candidates.iter().filter(|c| c.role == ContextRole::Seed) {
                    if opts.hop_from_docs_only && cand.node.node_type == NodeType::Symbol {
                        continue;
                    }
                    if let Some(idx) = graph.index_of(&cand.node.id) {
                        for (nidx, edge_w) in
                            graph.k_hop_neighborhood(idx as usize, opts.hop_depth)
                        {
                            if let Some(id) = graph.node_id(nidx as usize) {
                                if seed_ids.contains(id) {
                                    continue;
                                }
                                let hop: u8 = if opts.hop_depth <= 1 || edge_w >= 0.85 {
                                    1
                                } else {
                                    2
                                };
                                let nscore = cand.score * edge_w * opts.hop_decay.powi(hop as i32);
                                best_neighbor
                                    .entry(id.to_string())
                                    .and_modify(|(s, h)| {
                                        if nscore > *s {
                                            *s = nscore;
                                            *h = hop;
                                        }
                                    })
                                    .or_insert((nscore, hop));
                            }
                        }
                    }
                }

                let mut neigh_sorted: Vec<_> = best_neighbor.into_iter().collect();
                neigh_sorted.sort_by(|a, b| {
                    b.1 .0
                        .partial_cmp(&a.1 .0)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });

                for (id, (score, hop)) in neigh_sorted {
                    neighbor_ids.push(id.clone());
                    if let Some(node) = db.get_node(&id)? {
                        if !opts.allows_pack(&node.node_type, ContextRole::Neighbor) {
                            continue;
                        }
                        let mut nscore = score;
                        if node.node_type == NodeType::Symbol {
                            match symbol_neighbor_quality(&node, &query_tokens) {
                                SymbolQuality::Drop => continue,
                                SymbolQuality::Keep { boost } => nscore *= boost,
                            }
                        }
                        let max_ex = if node.node_type == NodeType::Symbol {
                            MAX_SYMBOL_EXCERPT_CHARS
                        } else {
                            MAX_EXCERPT_CHARS
                        };
                        let excerpt = if opts.include_excerpts {
                            load_excerpt(db, &node, max_ex)
                        } else {
                            None
                        };
                        candidates.push(Candidate {
                            node,
                            score: nscore,
                            role: ContextRole::Neighbor,
                            hop,
                            excerpt,
                        });
                    }
                }
            }
        }
    }

    // Sort all candidates by score and pack into token budget.
    candidates.sort_by(|a, b| {
        // Prefer non-symbols slightly when scores are close (stable agent packs).
        let a_bonus = if a.node.node_type == NodeType::Symbol {
            0.0
        } else {
            0.05
        };
        let b_bonus = if b.node.node_type == NodeType::Symbol {
            0.0
        } else {
            0.05
        };
        (b.score + b_bonus)
            .partial_cmp(&(a.score + a_bonus))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut packed: Vec<ContextNode> = Vec::new();
    let mut used_chars = estimate_overhead(prompt);
    let mut seen = HashSet::new();
    let mut symbol_neighbors = 0usize;

    for cand in candidates {
        if packed.len() >= opts.max_nodes {
            break;
        }
        if !seen.insert(cand.node.id.clone()) {
            continue;
        }
        if cand.role == ContextRole::Neighbor
            && cand.node.node_type == NodeType::Symbol
            && symbol_neighbors >= MAX_PACKED_SYMBOL_NEIGHBORS
        {
            continue;
        }
        let ctx_node = to_context_node(&cand);
        let cost = estimate_node_chars(&ctx_node);
        if used_chars + cost > char_budget && !packed.is_empty() {
            // Always try to keep at least one seed
            if cand.role == ContextRole::Neighbor {
                continue;
            }
            if packed.iter().any(|n| n.role == ContextRole::Seed) {
                continue;
            }
        }
        if used_chars + cost > char_budget && !packed.is_empty() {
            break;
        }
        used_chars += cost;
        if cand.role == ContextRole::Neighbor && cand.node.node_type == NodeType::Symbol {
            symbol_neighbors += 1;
        }
        packed.push(ctx_node);
    }

    neighbor_ids.retain(|id| !seed_ids.contains(id));
    neighbor_ids.truncate(48);

    let latency_us = start.elapsed().as_micros() as u64;
    let tokens_used = used_chars.div_ceil(4);

    Ok(ContextBundle {
        prompt: prompt.to_string(),
        max_tokens: opts.max_tokens,
        tokens_used,
        nodes: packed,
        neighbor_ids,
        latency_us,
        graph_nodes,
        graph_edges,
    })
}

enum SymbolQuality {
    Drop,
    Keep { boost: f32 },
}

/// Drop theme consts / short noise; boost query-matching or method-like symbols.
fn symbol_neighbor_quality(node: &Node, query_tokens: &[String]) -> SymbolQuality {
    let title = node.title.to_lowercase();
    let id = node.id.to_lowercase();
    let leaf = id.rsplit('/').next().unwrap_or(&id);

    // Query term appears in symbol name → keep and boost.
    for t in query_tokens {
        if title.contains(t.as_str()) || id.contains(t.as_str()) {
            return SymbolQuality::Keep { boost: 1.8 };
        }
    }

    // Method / path-like symbols (Type::method or multi-segment).
    if title.contains("::") || leaf.contains("::") {
        return SymbolQuality::Keep { boost: 1.15 };
    }

    // Very short or SCREAMING_SNAKE theme constants without query match.
    let name = title.split("::").last().unwrap_or(&title);
    if name.len() <= 2 {
        return SymbolQuality::Drop;
    }
    if name.chars().all(|c| c.is_ascii_uppercase() || c == '_') && name.len() <= 12 {
        // OK, ERR, BG_*, ACCENT_* style noise
        return SymbolQuality::Drop;
    }
    // Generic single-token modules without query match (derive, tree, …)
    if !name.contains('_') && !title.contains("::") && name.len() <= 8 {
        // Keep types that look like CamelCase API entry points.
        let has_upper = name.chars().any(|c| c.is_ascii_uppercase());
        let has_lower = name.chars().any(|c| c.is_ascii_lowercase());
        if has_upper && has_lower {
            return SymbolQuality::Keep { boost: 1.0 };
        }
        return SymbolQuality::Drop;
    }

    SymbolQuality::Keep { boost: 1.0 }
}

fn load_excerpt(db: &Database, node: &Node, max_chars: usize) -> Option<String> {
    let raw = db
        .get_fts_content(&node.id)
        .ok()
        .flatten()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| node.summary.clone().filter(|s| !s.trim().is_empty()))?;
    Some(truncate_excerpt(&raw, max_chars))
}

fn truncate_excerpt(s: &str, max_chars: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i >= max_chars.saturating_sub(1) {
            break;
        }
        out.push(ch);
    }
    out.push('…');
    out
}

fn to_context_node(c: &Candidate) -> ContextNode {
    ContextNode {
        id: c.node.id.clone(),
        node_type: c.node.node_type.clone(),
        title: c.node.title.clone(),
        summary: c.node.summary.clone(),
        excerpt: c.excerpt.clone(),
        file_path: c.node.file_path.clone(),
        score_hint: c.score,
        role: c.role,
        hop: c.hop,
    }
}

fn estimate_overhead(prompt: &str) -> usize {
    120 + prompt.len()
}

fn estimate_node_chars(n: &ContextNode) -> usize {
    let mut c = n.id.len() + n.title.len() + 48;
    if let Some(s) = &n.summary {
        c += s.len();
    }
    if let Some(ex) = &n.excerpt {
        c += ex.len();
    }
    if let Some(p) = &n.file_path {
        c += p.len();
    }
    c
}

/// Convert ranked hits to nodes (helper for CLI / adapters).
pub fn hits_to_nodes(hits: Vec<RankedHit>) -> Vec<Node> {
    hits.into_iter().map(|h| h.node).collect()
}

/// Multi-workspace ranked hit (workspace path + hit).
///
/// Prefer [`crate::GlobalRankedHit`] for the registry API; this type remains for
/// ad-hoc tooling that reuses the same shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceHit {
    /// Absolute workspace path that produced the hit.
    pub workspace: String,
    /// Ranked hit within that workspace.
    pub hit: RankedHit,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brain::Brain;
    use tempfile::tempdir;

    #[test]
    fn context_includes_graph_neighbors_within_budget() {
        let dir = tempdir().unwrap();
        let docs = dir.path().join("docs");
        std::fs::create_dir_all(&docs).unwrap();
        // Only seed A is FTS-matchable for "alphaunique"; B is only reachable via graph.
        std::fs::write(
            docs.join("alpha.md"),
            "---\ntags: [alphaunique]\nnode_type: concept\n---\n# AlphaUnique\nLinks [[betaunique]].\nalphaunique body.\n",
        )
        .unwrap();
        std::fs::write(
            docs.join("beta.md"),
            "---\ntags: [other]\nnode_type: concept\n---\n# BetaUnique\nNo alphaunique here.\n",
        )
        .unwrap();

        let mut brain = Brain::create(dir.path()).unwrap();
        brain.sync().unwrap();

        let opts = ContextOptions {
            max_tokens: 2048,
            hop_depth: 1,
            max_seeds: 4,
            max_nodes: 12,
            hop_decay: 0.7,
            ..ContextOptions::default()
        };
        let ctx = assemble_context(brain.database(), brain.brain_dir(), "alphaunique", &opts)
            .unwrap();
        assert!(!ctx.nodes.is_empty());
        assert!(ctx.tokens_used > 0);
        assert!(ctx.tokens_used <= opts.max_tokens + 64); // soft bound with estimator slack

        // Neighbor should be discovered via graph even if not an FTS seed.
        let has_beta = ctx.nodes.iter().any(|n| n.id.contains("beta"))
            || ctx.neighbor_ids.iter().any(|id| id.contains("beta"));
        assert!(
            has_beta,
            "expected beta neighbor via graph; nodes={:?} neigh={:?}",
            ctx.nodes.iter().map(|n| &n.id).collect::<Vec<_>>(),
            ctx.neighbor_ids
        );
    }

    #[test]
    fn tight_budget_limits_nodes() {
        let dir = tempdir().unwrap();
        let docs = dir.path().join("docs");
        std::fs::create_dir_all(&docs).unwrap();
        for i in 0..8 {
            std::fs::write(
                docs.join(format!("n{i}.md")),
                format!("---\ntags: [topic]\n---\n# Note {i}\nshared topic content {i}\n"),
            )
            .unwrap();
        }
        let mut brain = Brain::create(dir.path()).unwrap();
        brain.sync().unwrap();
        let opts = ContextOptions {
            max_tokens: 40, // very tight
            hop_depth: 0,
            max_seeds: 8,
            max_nodes: 20,
            hop_decay: 0.5,
            ..ContextOptions::default()
        };
        let ctx =
            assemble_context(brain.database(), brain.brain_dir(), "topic", &opts).unwrap();
        assert!(!ctx.nodes.is_empty());
        assert!(ctx.nodes.len() < 8, "budget should clip nodes");
    }

    #[test]
    fn natural_language_prompt_seeds_and_packs_excerpt() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("README.md"),
            "# demotool\n\nLightweight **egui** explorer powered by DuckDB.\n\
             Inspired by Duckling but avoids Tauri/WebView.\n",
        )
        .unwrap();
        let mut brain = Brain::create(dir.path()).unwrap();
        brain.sync().unwrap();

        let ctx = assemble_context(
            brain.database(),
            brain.brain_dir(),
            "why egui not tauri",
            &ContextOptions::default(),
        )
        .unwrap();
        assert!(
            !ctx.nodes.is_empty(),
            "expected seeds for NL prompt; packed=0"
        );
        let has_excerpt = ctx.nodes.iter().any(|n| {
            n.excerpt
                .as_ref()
                .map(|e| e.to_lowercase().contains("egui") || e.to_lowercase().contains("tauri"))
                .unwrap_or(false)
        });
        assert!(
            has_excerpt,
            "expected body excerpt mentioning egui/tauri; nodes={:?}",
            ctx.nodes
                .iter()
                .map(|n| (&n.id, &n.excerpt))
                .collect::<Vec<_>>()
        );
    }
}
