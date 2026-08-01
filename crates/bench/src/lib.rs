//! Shared fixtures for rustbrain criterion benches.
//!
//! - JSON stack shapes (`serde_vs_jshift`)
//! - Synthetic Markdown workspaces (`rustbrain_performance`)
//! - Alternative baselines (walk+contains, SQLite LIKE, re-parse WikiLinks)

use jshift::JsonView;
use regex::Regex;
use rustbrain_core::{extract_wikilinks, Brain, QueryOptions, Result as BrainResult};
// Re-export for benches
pub use rustbrain_core::{densify_plan, NodeType, PlanStatus};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tempfile::TempDir;

// ── JSON stack fixtures (serde vs jshift) ───────────────────────────────────

/// Tiny marker like `.brain/workspace.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkspaceMeta {
    pub version: u32,
    pub workspace: String,
}

#[derive(Debug, Clone, PartialEq, JsonView)]
pub struct WorkspaceMetaView {
    #[json(path = "version")]
    pub version: u32,
    #[json(path = "workspace")]
    pub workspace: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Finding {
    pub severity: String,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DoctorReportLite {
    pub workspace: String,
    pub nodes: usize,
    pub edges: usize,
    pub pending_links: usize,
    pub orphan_notes: usize,
    pub findings: Vec<Finding>,
    pub healthy: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BundleNode {
    pub id: String,
    pub node_type: String,
    pub title: String,
    pub file_path: Option<String>,
    pub summary: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BundleEdge {
    pub source_id: String,
    pub target_id: String,
    pub relation_type: String,
    pub weight: f32,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BrainBundleLite {
    pub version: u32,
    pub name: String,
    pub created_at: i64,
    pub nodes: Vec<BundleNode>,
    pub edges: Vec<BundleEdge>,
}

pub fn sample_workspace() -> WorkspaceMeta {
    WorkspaceMeta {
        version: 1,
        workspace: "/home/farmer/dev-other/parqview".into(),
    }
}

pub fn sample_doctor() -> DoctorReportLite {
    DoctorReportLite {
        workspace: "/home/farmer/dev-other/parqview".into(),
        nodes: 120,
        edges: 95,
        pending_links: 2,
        orphan_notes: 4,
        findings: vec![
            Finding {
                severity: "info".into(),
                code: "orphan_notes".into(),
                message: "4 orphan note(s)".into(),
            },
            Finding {
                severity: "info".into(),
                code: "sparse_readme".into(),
                message: "README is thin".into(),
            },
        ],
        healthy: true,
    }
}

pub fn sample_bundle(n_nodes: usize, n_edges: usize) -> BrainBundleLite {
    let nodes: Vec<BundleNode> = (0..n_nodes)
        .map(|i| BundleNode {
            id: format!("docs/concepts/n{i}"),
            node_type: if i % 5 == 0 {
                "adr".into()
            } else {
                "concept".into()
            },
            title: format!("Note {i}"),
            file_path: Some(format!("docs/concepts/n{i}.md")),
            summary: Some(format!("summary for note {i} with some body text")),
            created_at: 1_700_000_000 + i as i64,
            updated_at: 1_700_000_100 + i as i64,
        })
        .collect();
    let edges: Vec<BundleEdge> = (0..n_edges)
        .map(|i| {
            let a = i % n_nodes.max(1);
            let b = (i * 3 + 1) % n_nodes.max(1);
            BundleEdge {
                source_id: format!("docs/concepts/n{a}"),
                target_id: format!("docs/concepts/n{b}"),
                relation_type: if i % 4 == 0 {
                    "anchors".into()
                } else {
                    "relates_to".into()
                },
                weight: 0.5 + (i % 10) as f32 * 0.05,
                created_at: 1_700_000_000 + i as i64,
            }
        })
        .collect();
    BrainBundleLite {
        version: 1,
        name: "bench-bundle".into(),
        created_at: 1_700_000_000,
        nodes,
        edges,
    }
}

/// Large JSON with noise fields — jshift path-get specialty.
pub fn large_catalog_json(n: usize) -> Vec<u8> {
    let mut s = String::from(r#"{"version":1,"noise":""#);
    s.push_str(&"x".repeat(4096));
    s.push_str(r#"","nodes":["#);
    for i in 0..n {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&format!(
            r#"{{"id":"docs/concepts/n{i}","node_type":"concept","title":"Note {i}","extra":{{"a":{i},"b":"pad-{i}","c":[1,2,3,4,5]}},"file_path":"docs/concepts/n{i}.md"}}"#
        ));
    }
    s.push_str(r#"],"meta":{"workspace":"/tmp/ws","ok":true}}"#);
    s.into_bytes()
}

// ── Synthetic workspace fixtures ────────────────────────────────────────────

/// Keywords planted so search has non-trivial targets.
pub const KEYWORD_SQLITE: &str = "sqlite";
pub const KEYWORD_RAFT: &str = "raft";
pub const KEYWORD_FTS: &str = "fts5";

/// On-disk workspace used by engine performance benches.
pub struct BenchWorkspace {
    /// Keep the temp dir alive for the fixture lifetime.
    pub _dir: TempDir,
    pub root: PathBuf,
    pub n_notes: usize,
    /// Path of a note that is well-linked (good graph root).
    pub hub_note_rel: String,
    /// Absolute path of a single file to touch for incremental-sync benches.
    pub touch_file: PathBuf,
}

impl BenchWorkspace {
    /// Build `n_notes` concept notes with WikiLinks + keyword density, then sync.
    pub fn create(n_notes: usize) -> BrainResult<Self> {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().to_path_buf();
        fs::create_dir_all(root.join("docs/concepts"))?;
        fs::create_dir_all(root.join("docs/adr"))?;
        fs::create_dir_all(root.join("docs/plans"))?;
        fs::create_dir_all(root.join("src"))?;

        // README hub
        fs::write(
            root.join("README.md"),
            "# Bench Project\n\nOffline knowledge graph for agents. Uses sqlite and fts5.\n",
        )?;

        // CHANGELOG hub
        fs::write(
            root.join("CHANGELOG.md"),
            "# Changelog\n\n## 0.3.20\n\n- Performance benches\n- Graph and FTS improvements\n",
        )?;

        for i in 0..n_notes {
            let next = (i + 1) % n_notes;
            let jump = (i * 7 + 3) % n_notes;
            let keyword = match i % 11 {
                0 => KEYWORD_SQLITE,
                1 => KEYWORD_RAFT,
                2 => KEYWORD_FTS,
                _ => "general",
            };
            let tags = match i % 5 {
                0 => "storage,sqlite",
                1 => "consensus,raft",
                2 => "search,fts",
                _ => "general",
            };
            let body = format!(
                r#"---
node_type: concept
tags: [{tags}]
---
# Note {i}

This is synthetic note {i} about {keyword} in a software repository.
Padding text so FTS has something to chew on: Lorem ipsum dolor sit amet,
consectetur adipiscing elit. Performance, indexing, and agent context packing.

See [[docs/concepts/note-{next}]] and [[docs/concepts/note-{jump}]].

Related keyword blob: {keyword} {keyword} note-{i}.
"#
            );
            fs::write(root.join(format!("docs/concepts/note-{i}.md")), body)?;
        }

        // A few ADRs for type-boosted search
        for i in 0..3.min(n_notes) {
            fs::write(
                root.join(format!("docs/adr/decision-{i}.md")),
                format!(
                    r#"---
node_type: adr
tags: [architecture]
---
# Decision {i}

We chose local {KEYWORD_SQLITE} for the knowledge index.

See [[docs/concepts/note-{i}]].
"#
                ),
            )?;
        }

        // One plan note for densify / status search
        fs::write(
            root.join("docs/plans/sprint.md"),
            r#"---
node_type: plan
status: in_progress
---
# Sprint board

## Status
in_progress

- [x] Scaffold
- [/] Index performance
- [ ] Publish benches
- [!] Blocked on CI capacity
"#,
        )?;

        // Minimal Rust so AST path is exercised on sync (optional cost)
        fs::write(
            root.join("src/lib.rs"),
            r#"//! Bench crate surface.
/// Storage facade. See [[docs/adr/decision-0]].
pub struct Database;
impl Database {
    pub fn open() {}
}
"#,
        )?;

        let mut brain = Brain::create(&root)?;
        brain.sync()?;

        let hub_note_rel = "docs/concepts/note-0.md".to_string();
        let touch_file = root.join("docs/concepts/note-1.md");

        Ok(Self {
            _dir: dir,
            root,
            n_notes,
            hub_note_rel,
            touch_file,
        })
    }

    pub fn open_brain(&self) -> BrainResult<Brain> {
        Brain::open_exact(&self.root)
    }
}

// ── Alternative baselines (what rustbrain intentionally did not choose) ─────

/// Hit from a naive baseline search.
#[derive(Debug, Clone)]
pub struct NaiveHit {
    pub path: PathBuf,
    pub score: f32,
}

fn walk_md_files(root: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    fn rec(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if name == ".brain" || name == "target" || name == ".git" {
                continue;
            }
            if path.is_dir() {
                rec(&path, out)?;
            } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
                out.push(path);
            }
        }
        Ok(())
    }
    rec(root, &mut out)?;
    Ok(out)
}

/// **Baseline A — walk + substring:** scan every Markdown file each query.
///
/// This is the “just grep the docs folder” approach agents use without an index.
pub fn naive_walk_search(root: &Path, query: &str, limit: usize) -> std::io::Result<Vec<NaiveHit>> {
    let tokens: Vec<String> = query
        .split_whitespace()
        .filter(|t| t.len() > 2)
        .map(|t| t.to_ascii_lowercase())
        .collect();
    if tokens.is_empty() {
        return Ok(Vec::new());
    }
    let mut hits = Vec::new();
    for path in walk_md_files(root)? {
        let text = fs::read_to_string(&path)?.to_ascii_lowercase();
        let mut score = 0.0f32;
        for t in &tokens {
            let mut start = 0;
            while let Some(pos) = text[start..].find(t.as_str()) {
                score += 1.0;
                start += pos + t.len();
                if start >= text.len() {
                    break;
                }
            }
        }
        if score > 0.0 {
            hits.push(NaiveHit { path, score });
        }
    }
    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    hits.truncate(limit);
    Ok(hits)
}

/// **Baseline B — SQLite LIKE on FTS body table:** uses the same corpus bytes as
/// rustbrain’s index but without FTS5 ranking (full table scan + `LIKE %token%`).
///
/// Models “store docs in SQLite but skip inverted index / BM25.”
pub fn sqlite_like_search(db_path: &Path, query: &str, limit: usize) -> rusqlite::Result<Vec<(String, f32)>> {
    let conn = rusqlite::Connection::open(db_path)?;
    let tokens: Vec<String> = query
        .split_whitespace()
        .filter(|t| t.len() > 2)
        .map(|t| t.to_ascii_lowercase())
        .collect();
    if tokens.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare("SELECT node_id, title, content FROM node_fts")?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    let mut hits = Vec::new();
    for row in rows {
        let (id, title, content) = row?;
        let hay = format!("{title}\n{content}").to_ascii_lowercase();
        let mut score = 0.0f32;
        for t in &tokens {
            if hay.contains(t) {
                score += 1.0 + hay.matches(t.as_str()).count() as f32 * 0.1;
            }
        }
        if score > 0.0 {
            hits.push((id, score));
        }
    }
    hits.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    hits.truncate(limit);
    Ok(hits)
}

/// **Baseline C — context = concat files in path order until char budget.**
///
/// No ranking, no graph hops — common “dump docs into the prompt” approach.
pub fn naive_concat_context(root: &Path, max_chars: usize) -> std::io::Result<(usize, usize)> {
    let mut files = walk_md_files(root)?;
    files.sort();
    let mut used = 0usize;
    let mut n = 0usize;
    for path in files {
        let text = fs::read_to_string(&path)?;
        if used >= max_chars {
            break;
        }
        let take = (max_chars - used).min(text.len());
        used += take;
        n += 1;
    }
    Ok((n, used))
}

/// **Baseline D — context = score by walk-search, then concat top files.**
pub fn grep_ranked_concat_context(
    root: &Path,
    query: &str,
    max_chars: usize,
) -> std::io::Result<(usize, usize)> {
    let hits = naive_walk_search(root, query, 64)?;
    let mut used = 0usize;
    let mut n = 0usize;
    for h in hits {
        let text = fs::read_to_string(&h.path)?;
        if used >= max_chars {
            break;
        }
        let take = (max_chars - used).min(text.len().min(900));
        used += take;
        n += 1;
    }
    Ok((n, used))
}

/// Adjacency list built by re-parsing every Markdown file’s WikiLinks.
pub type WikiAdj = HashMap<String, Vec<String>>;

fn path_to_node_id(root: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    let s = rel.to_string_lossy().replace('\\', "/");
    s.trim_end_matches(".md").to_string()
}

/// **Baseline E — rebuild link graph from disk every time** (no SQLite edges, no CSR).
pub fn rebuild_wikilink_adj(root: &Path) -> std::io::Result<WikiAdj> {
    let mut adj: WikiAdj = HashMap::new();
    for path in walk_md_files(root)? {
        let id = path_to_node_id(root, &path);
        let text = fs::read_to_string(&path)?;
        let targets: Vec<String> = extract_wikilinks(&text)
            .into_iter()
            .map(|w| {
                let t = w.target_node.trim_end_matches(".md").to_string();
                t
            })
            .collect();
        adj.entry(id).or_default().extend(targets);
    }
    Ok(adj)
}

/// BFS over a rebuilt WikiLink adjacency (undirected for fairness with `both`).
pub fn naive_wikilink_bfs(adj: &WikiAdj, root_id: &str, hops: usize) -> usize {
    let mut reverse: HashMap<&str, Vec<&str>> = HashMap::new();
    for (src, tgts) in adj {
        for t in tgts {
            reverse.entry(t.as_str()).or_default().push(src.as_str());
        }
    }
    let mut seen = HashSet::new();
    let mut q = VecDeque::new();
    q.push_back((root_id.to_string(), 0usize));
    seen.insert(root_id.to_string());
    let mut edges = 0usize;
    while let Some((id, d)) = q.pop_front() {
        if d >= hops {
            continue;
        }
        let outs = adj.get(&id).map(|v| v.as_slice()).unwrap_or(&[]);
        let ins = reverse.get(id.as_str()).map(|v| v.as_slice()).unwrap_or(&[]);
        for n in outs.iter().map(|s| s.as_str()).chain(ins.iter().copied()) {
            edges += 1;
            if seen.insert(n.to_string()) {
                q.push_back((n.to_string(), d + 1));
            }
        }
    }
    edges
}

/// Naive `[[...]]` regex — does **not** skip code fences (rustbrain does).
pub fn naive_regex_wikilinks(markdown: &str) -> usize {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"\[\[([^\]]+)\]\]").expect("regex"));
    re.find_iter(markdown).count()
}

/// Multi-paragraph markdown with fences for WikiLink extract benches.
pub fn sample_markdown_with_links(n_links: usize) -> String {
    let mut s = String::from("# Sample\n\n```\n[[inside-fence-should-skip]]\n```\n\n");
    for i in 0..n_links {
        s.push_str(&format!("See [[docs/concepts/note-{i}]] and more text.\n"));
        if i % 10 == 0 {
            s.push_str("Inline `[[also-skipped]]` code.\n");
        }
    }
    s
}

/// Sample plan body for densify microbench.
pub fn sample_plan_markdown() -> &'static str {
    r#"---
node_type: plan
status: in_progress
---
# Sprint

## Status
in_progress

- [ ] backlog item
- [/] doing
- [x] done
- [!] blocked
"#
}

/// Open brain and run ranked query (helper for benches).
pub fn rb_query(brain: &Brain, q: &str, limit: usize) -> BrainResult<usize> {
    let mut opts = QueryOptions::human();
    opts.limit = limit;
    Ok(brain.query_ranked(q, &opts)?.len())
}

/// Touch a file so the next sync has real work.
pub fn touch_append(path: &Path, marker: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut f = fs::OpenOptions::new().append(true).open(path)?;
    writeln!(f, "\n<!-- {marker} -->")?;
    Ok(())
}

