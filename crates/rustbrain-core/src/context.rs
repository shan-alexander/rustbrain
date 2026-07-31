//! Graph-aware AI context assembly with token budgeting.
//!
//! Pipeline used by [`crate::Brain::context_for_prompt`]:
//!
//! 1. Ranked FTS seeds ([`crate::query`])
//! 2. Optional CSR k-hop expansion from `.brain/graph.mmap`
//! 3. Score fusion (seed score × edge weight × hop decay)
//! 4. Pack nodes until the approximate character/token budget is exhausted
//!
//! Token accounting is intentionally simple (`chars / 4`). It is good enough for
//! agent prompt packing, not for billing-grade tokenizer parity.

use crate::error::Result;
use crate::query::{QueryOptions, RankedHit};
use crate::storage::Database;
use crate::types::{ContextBundle, ContextNode, ContextRole, Node};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Instant;

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
    /// Exclude pure code symbols from seeds and packed neighbors.
    pub no_symbols: bool,
    /// Only include these node types when non-empty.
    pub include_types: Vec<crate::types::NodeType>,
    /// Always exclude these node types.
    pub exclude_types: Vec<crate::types::NodeType>,
    /// When true, allow graph hops *to* symbols even if `no_symbols` filters seeds.
    /// Default true so ADR → `symbol:foo` remains useful for agents.
    pub hop_to_symbols: bool,
}

impl Default for ContextOptions {
    fn default() -> Self {
        Self {
            max_tokens: 2048,
            hop_depth: 1,
            max_seeds: 8,
            max_nodes: 24,
            hop_decay: 0.65,
            no_symbols: false,
            include_types: Vec::new(),
            exclude_types: Vec::new(),
            hop_to_symbols: true,
        }
    }
}

impl ContextOptions {
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
        candidates.push(Candidate {
            node: hit.node,
            score,
            role: ContextRole::Seed,
            hop: 0,
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
                    if let Some(idx) = graph.index_of(&cand.node.id) {
                        for (nidx, edge_w) in
                            graph.k_hop_neighborhood(idx as usize, opts.hop_depth)
                        {
                            if let Some(id) = graph.node_id(nidx as usize) {
                                if seed_ids.contains(id) {
                                    continue;
                                }
                                // k_hop returns first-visit neighbors; for depth>1 we
                                // approximate hop from cumulative path weight.
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
                        candidates.push(Candidate {
                            node,
                            score,
                            role: ContextRole::Neighbor,
                            hop,
                        });
                    }
                }
            }
        }
    }

    // Sort all candidates by score and pack into token budget.
    candidates.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut packed: Vec<ContextNode> = Vec::new();
    let mut used_chars = estimate_overhead(prompt);
    let mut seen = HashSet::new();

    for cand in candidates {
        if packed.len() >= opts.max_nodes {
            break;
        }
        if !seen.insert(cand.node.id.clone()) {
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
        packed.push(ctx_node);
    }

    // Ensure neighbor_ids only lists those not fully expanded into packed as seeds
    // (keep as list of graph-discovered ids for transparency).
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

fn to_context_node(c: &Candidate) -> ContextNode {
    ContextNode {
        id: c.node.id.clone(),
        node_type: c.node.node_type.clone(),
        title: c.node.title.clone(),
        summary: c.node.summary.clone(),
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
}
