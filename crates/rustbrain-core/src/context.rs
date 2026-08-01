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
use crate::fts::{is_generic_topic, prepare_search_query, tokenize_query};
use crate::hubs::{is_planning_intent, is_release_intent, HUB_BACKLOG, HUB_CHANGELOG, HUB_README, HUB_ROADMAP};
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
    let mut seeds = db.search_ranked(prompt, &qopts)?;
    let generic = is_generic_topic(&query_tokens);
    let release_intent = is_release_intent(&query_tokens);
    let planning_intent = is_planning_intent(&query_tokens);
    let cold_start = seeds.is_empty() || generic;
    // Soft / empty retrieval → inject project hubs (README, CHANGELOG, …).
    if cold_start || release_intent || planning_intent {
        inject_hub_seeds(
            db,
            &mut seeds,
            opts,
            release_intent,
            planning_intent,
            cold_start,
        )?;
    }

    let mut candidates: Vec<Candidate> = Vec::new();
    let mut seed_ids: HashSet<String> = HashSet::new();

    for (i, hit) in seeds.into_iter().take(opts.max_seeds).enumerate() {
        if !opts.allows_pack(&hit.node.node_type, ContextRole::Seed) {
            continue;
        }
        // Skip pure ADR templates — they burn budget without knowledge.
        if is_template_stub(&hit.node) {
            continue;
        }
        seed_ids.insert(hit.node.id.clone());
        // Slight rank-position prior
        let mut score = hit.score * (1.0 / (1.0 + i as f32 * 0.05));
        // Prefer hand-written / ADR over generated harvest when scores are close.
        if is_generated_path(&hit.node) {
            score *= 0.85;
        }
        if hit.node.node_type == NodeType::Adr {
            score *= 1.08;
        }
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

    // If filters wiped everything (e.g. only template matched), hub-inject again.
    if candidates.is_empty() {
        let mut hub_hits = Vec::new();
        inject_hub_seeds(db, &mut hub_hits, opts, true, true, true)?;
        for hit in hub_hits.into_iter().take(6) {
            if !opts.allows_pack(&hit.node.node_type, ContextRole::Seed) {
                continue;
            }
            if seed_ids.contains(&hit.node.id) {
                continue;
            }
            seed_ids.insert(hit.node.id.clone());
            let excerpt = if opts.include_excerpts {
                load_excerpt(db, &hit.node, MAX_EXCERPT_CHARS)
            } else {
                None
            };
            candidates.push(Candidate {
                node: hit.node,
                score: hit.score,
                role: ContextRole::Seed,
                hop: 0,
                excerpt,
            });
        }
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

    // Sort for agent-useful packs: seeds before neighbors, decisions before symbols,
    // then raw score. Keeps ADR + README above opportunistic symbol hops.
    candidates.sort_by(|a, b| {
        pack_rank(b)
            .partial_cmp(&pack_rank(a))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut packed: Vec<ContextNode> = Vec::new();
    let mut used_chars = estimate_overhead(prompt);
    let mut seen = HashSet::new();
    let mut symbol_neighbors = 0usize;
    let mut packed_excerpts: Vec<String> = Vec::new();

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
        // Prefer a single README-family hub (root readme XOR from-readme harvest).
        if is_readme_family(&cand.node.id)
            && packed.iter().any(|n| is_readme_family(&n.id))
        {
            continue;
        }
        // Don't double-pack near-identical changelog paths if aliases collide.
        if cand.node.id == HUB_CHANGELOG
            && packed.iter().any(|n| n.id == HUB_CHANGELOG)
        {
            continue;
        }
        // Drop near-duplicate body text (harvested clones of the same prose).
        if let Some(ex) = &cand.excerpt {
            if packed_excerpts
                .iter()
                .any(|prev| excerpt_jaccard(prev, ex) > 0.55)
            {
                continue;
            }
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
        if let Some(ex) = &ctx_node.excerpt {
            packed_excerpts.push(ex.clone());
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

/// Soft-inject project hubs when FTS is empty, generic, or intent-matched.
fn inject_hub_seeds(
    db: &Database,
    seeds: &mut Vec<RankedHit>,
    opts: &ContextOptions,
    release_intent: bool,
    planning_intent: bool,
    cold_start: bool,
) -> Result<()> {
    let existing: HashSet<String> = seeds.iter().map(|h| h.node.id.clone()).collect();
    let mut hubs: Vec<(&str, f32)> = Vec::new();
    if cold_start {
        hubs.extend([
            (HUB_README, 4.5_f32),
            ("docs/goals/from-readme", 3.8),
            ("docs/implementation/module-map.generated", 2.2),
            ("docs/goals/readme", 1.5),
        ]);
    }
    if release_intent || cold_start {
        // Keep a Changelog is ground truth for "what shipped".
        hubs.push((HUB_CHANGELOG, if release_intent { 5.0 } else { 3.2 }));
    }
    if planning_intent || cold_start {
        hubs.push((HUB_ROADMAP, if planning_intent { 4.6 } else { 2.4 }));
        hubs.push((HUB_BACKLOG, if planning_intent { 4.4 } else { 2.2 }));
    }
    let mut seen = HashSet::new();
    hubs.retain(|(id, _)| seen.insert(*id));

    for (id, score) in hubs {
        if existing.contains(id) {
            continue;
        }
        if let Some(node) = db.get_node(id)? {
            if !opts.allows_pack(&node.node_type, ContextRole::Seed) {
                continue;
            }
            if is_template_stub(&node) {
                continue;
            }
            seeds.push(RankedHit {
                node,
                score,
                reasons: vec![format!("hub-fallback:{id}")],
            });
        }
    }
    // Keep highest first for take(max_seeds).
    seeds.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Ok(())
}

fn is_template_stub(node: &Node) -> bool {
    let id = node.id.to_lowercase();
    let path = node.file_path.as_deref().unwrap_or("").to_lowercase();
    let title = node.title.to_lowercase();
    id.contains("template")
        || path.ends_with("template.md")
        || title.contains("template")
        || title == "adr template"
}

fn is_generated_path(node: &Node) -> bool {
    let path = node.file_path.as_deref().unwrap_or("");
    path.contains("from-readme")
        || path.contains(".generated.")
        || path.ends_with("module-map.generated.md")
}

fn is_readme_family(id: &str) -> bool {
    id == "readme" || id.contains("from-readme")
}

fn excerpt_jaccard(a: &str, b: &str) -> f32 {
    let ta: HashSet<&str> = a
        .split_whitespace()
        .filter(|w| w.len() > 2)
        .collect();
    let tb: HashSet<&str> = b
        .split_whitespace()
        .filter(|w| w.len() > 2)
        .collect();
    if ta.is_empty() || tb.is_empty() {
        return 0.0;
    }
    let inter = ta.intersection(&tb).count() as f32;
    let uni = ta.union(&tb).count() as f32;
    if uni == 0.0 {
        0.0
    } else {
        inter / uni
    }
}

fn pack_rank(c: &Candidate) -> f32 {
    let role = match c.role {
        ContextRole::Seed => 3.0,
        ContextRole::Neighbor => 0.0,
    };
    let ty = match c.node.node_type {
        NodeType::Adr => 2.5,
        NodeType::Goal => 2.0,
        NodeType::EdgeCase => 1.8,
        NodeType::Analysis => 1.65,
        NodeType::Concept => 1.4,
        NodeType::Reference => 1.2,
        NodeType::Alternative => 1.1,
        NodeType::Symbol => 0.0,
    };
    // Score still dominates large gaps; bonuses break near-ties for agent packs.
    c.score + role + ty
}

fn load_excerpt(db: &Database, node: &Node, max_chars: usize) -> Option<String> {
    let raw = db
        .get_fts_content(&node.id)
        .ok()
        .flatten()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| node.summary.clone().filter(|s| !s.trim().is_empty()))?;
    let cleaned = strip_yaml_frontmatter(&raw);
    Some(truncate_excerpt(cleaned, max_chars))
}

/// Drop leading `---` YAML frontmatter so agent packs show body prose first.
fn strip_yaml_frontmatter(s: &str) -> &str {
    let t = s.trim_start();
    if !t.starts_with("---") {
        return s.trim();
    }
    let after_open = &t[3..];
    // Allow optional newline after opening fence.
    let body = after_open.strip_prefix('\n').unwrap_or(after_open);
    if let Some(idx) = body.find("\n---") {
        let rest = &body[idx + 4..];
        return rest.trim_start_matches(['\r', '\n']).trim();
    }
    s.trim()
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
    use crate::indexer::WorkspaceIndexer;
    use crate::storage::Database;
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

    #[test]
    fn release_prompt_prefers_changelog_hub() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("README.md"),
            "# Demo\n\nShip notes live in the changelog.\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("CHANGELOG.md"),
            "# Changelog\n\n## [1.2.3] - 2026-01-01\n\n### Added\n- important feature alpha\n",
        )
        .unwrap();
        let brain = dir.path().join(".brain");
        std::fs::create_dir_all(&brain).unwrap();
        let db = Database::open(brain.join("db.sqlite")).unwrap();
        let indexer = WorkspaceIndexer::new(db, dir.path());
        indexer.index_workspace().unwrap();
        let db = Database::open(brain.join("db.sqlite")).unwrap();
        let ctx = assemble_context(
            &db,
            &brain,
            "what shipped in the changelog release",
            &ContextOptions {
                max_tokens: 800,
                hop_depth: 0,
                ..ContextOptions::default()
            },
        )
        .unwrap();
        assert!(
            ctx.nodes.iter().any(|n| n.id == "changelog"),
            "expected changelog hub packed; nodes={:?}",
            ctx.nodes.iter().map(|n| &n.id).collect::<Vec<_>>()
        );
    }

    #[test]
    fn generic_overview_prompt_falls_back_to_hub() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("README.md"),
            "# demotool\n\nNative egui + DuckDB CLI. Avoids Tauri.\n",
        )
        .unwrap();
        let mut brain = Brain::create(dir.path()).unwrap();
        brain.sync().unwrap();

        for prompt in [
            "summarize architecture",
            "what is this project about",
            "give an overview",
        ] {
            let ctx = assemble_context(
                brain.database(),
                brain.brain_dir(),
                prompt,
                &ContextOptions::default(),
            )
            .unwrap();
            assert!(
                !ctx.nodes.is_empty(),
                "expected hub fallback for `{prompt}`; packed=0"
            );
            let has_hub = ctx.nodes.iter().any(|n| {
                n.id == "readme"
                    || n.id.contains("from-readme")
                    || n.excerpt
                        .as_ref()
                        .map(|e| e.to_lowercase().contains("egui") || e.to_lowercase().contains("duckdb"))
                        .unwrap_or(false)
            });
            assert!(
                has_hub,
                "expected README hub content for `{prompt}`; nodes={:?}",
                ctx.nodes.iter().map(|n| &n.id).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn strips_frontmatter_from_excerpts() {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("docs")).unwrap();
        std::fs::write(
            dir.path().join("docs/a.md"),
            "---\ntags: [x]\nnode_type: concept\n---\n# Alpha\n\nBody about egui.\n",
        )
        .unwrap();
        let mut brain = Brain::create(dir.path()).unwrap();
        brain.sync().unwrap();
        let ctx = assemble_context(
            brain.database(),
            brain.brain_dir(),
            "egui",
            &ContextOptions {
                hop_depth: 0,
                ..ContextOptions::default()
            },
        )
        .unwrap();
        let ex = ctx
            .nodes
            .iter()
            .find_map(|n| n.excerpt.as_ref())
            .expect("excerpt");
        assert!(
            !ex.trim_start().starts_with("---"),
            "frontmatter leaked into excerpt: {ex}"
        );
        assert!(ex.to_lowercase().contains("egui") || ex.contains("Alpha"));
    }

    #[test]
    fn prefers_adr_seed_over_symbol_neighbor() {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("docs/adr")).unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "/// Run SQL via duckdb CLI.\npub fn run_sql() {}\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("docs/adr/sql.md"),
            "---\nnode_type: adr\n---\n# SQL via duckdb\n\nUse symbol:run_sql for SQL execution via duckdb CLI.\n",
        )
        .unwrap();
        let mut brain = Brain::create(dir.path()).unwrap();
        brain.sync().unwrap();
        let _ = brain.sync().unwrap(); // resolve anchors
        let ctx = assemble_context(
            brain.database(),
            brain.brain_dir(),
            "how does SQL execution work",
            &ContextOptions::default(),
        )
        .unwrap();
        assert!(!ctx.nodes.is_empty());
        let first = &ctx.nodes[0];
        assert_eq!(
            first.node_type,
            NodeType::Adr,
            "expected ADR first, got {:?} id={}",
            first.node_type,
            first.id
        );
    }

    #[test]
    fn dedups_near_identical_excerpts() {
        let dir = tempdir().unwrap();
        let body = "Lightweight egui explorer with DuckDB CLI. Avoids Tauri completely.\n".repeat(3);
        std::fs::write(
            dir.path().join("README.md"),
            format!("# demotool\n\n{body}"),
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("docs/goals")).unwrap();
        std::fs::write(
            dir.path().join("docs/goals/from-readme.md"),
            format!(
                "---\nnode_type: goal\n---\n# Goals harvested from README\n\n{body}"
            ),
        )
        .unwrap();
        let mut brain = Brain::create(dir.path()).unwrap();
        brain.sync().unwrap();
        let ctx = assemble_context(
            brain.database(),
            brain.brain_dir(),
            "egui duckdb tauri",
            &ContextOptions {
                max_tokens: 900,
                hop_depth: 0,
                ..ContextOptions::default()
            },
        )
        .unwrap();
        // Should not pack both near-identical README + harvest when budget is modest.
        let ids: Vec<_> = ctx.nodes.iter().map(|n| n.id.as_str()).collect();
        let both = ids.contains(&"readme") && ids.iter().any(|i| i.contains("from-readme"));
        assert!(
            !both || ctx.nodes.len() == 1,
            "expected dedup of near-identical hubs; packed={ids:?}"
        );
    }
}
