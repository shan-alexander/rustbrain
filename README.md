# rustbrain

**A lightweight, Rust-native second brain** for software repositories — for humans *and* AI agents.

Write ordinary Markdown (Obsidian-compatible WikiLinks and frontmatter). `rustbrain` indexes notes and Rust code into a local SQLite knowledge graph, bakes a CSR graph cache, and serves **ranked search** plus **graph-aware agent context** under a token budget.

```text
  docs/*.md  +  src/**/*.rs
        │              │
        ▼              ▼
   WikiLinks      tree-sitter
   frontmatter     BLAKE3 symbols
        │              │
        └──────┬───────┘
               ▼
        .brain/db.sqlite   ← source of truth (FTS5 + edges)
               │
               ▼
        .brain/graph.mmap  ← CSR cache for neighborhood expansion
               │
               ▼
     rustbrain query / context / agents
```

> **v0.1 honesty:** Markdown + ranked FTS + CSR graph context + Rust AST anchors + note→symbol links + debounced watch.  
> Neural embeddings, multi-brain `--scope`, and full two-way Obsidian write-back are **planned**, not claimed.

---

## Why rustbrain?

| Problem | rustbrain approach |
|--------|---------------------|
| Docs scattered, invisible to agents | Project-scoped graph + FTS in `.brain/` |
| Proprietary note formats | Plain Markdown + YAML + `[[WikiLinks]]` |
| Context dumps are huge / unranked | Token-budgeted packing + graph hops |
| Code and notes disconnected | `symbol:…` anchors + AST symbol nodes |
| Heavy SaaS / cloud lock-in | Local SQLite + optional Obsidian |

---

## Install

### CLI (from source / git)

```bash
# After cloning this repo:
cargo install --path crates/rustbrain-cli --locked

# Or once published:
# cargo install rustbrain --locked
```

### Library

```toml
# Cargo.toml
[dependencies]
rustbrain-core = "0.1"
```

**Build requirements:** a C toolchain (bundled SQLite via `rusqlite`, tree-sitter grammar compile). MSRV: **Rust 1.80**.

---

## Quick start (CLI)

```bash
cd your-project

rustbrain init
# create notes under docs/**/*.md  (optional frontmatter + [[WikiLinks]])

rustbrain sync
rustbrain query "raft consensus" --scores
rustbrain context -p "explain log compaction" -F markdown --hops 1
rustbrain export --out ./my_brain.brainbundle

# while editing:
rustbrain watch --debounce-ms 300
```

### CLI commands

| Command | Purpose |
|---------|---------|
| `init [workspace]` | Create `.brain/db.sqlite` + register workspace |
| `sync [workspace]` | Index Markdown / Canvas / Rust; bake `graph.mmap` |
| `query <q>` | Ranked FTS + tag/alias boosts (`--scores`, `-n`, `--all-workspaces`) |
| `context -p <prompt>` | Graph-aware agent context (`-m` tokens, `--hops`, `-F xml\|markdown`) |
| `watch` | Debounced re-index + remmap on file changes |
| `export --out <path>` | Portable `.brainbundle` JSON (`--decouple-ast`) |
| `import --input <path>` | Import a bundle into the local brain |

Global flag pattern: `-w / --workspace` for non-default roots.

---

## Note format

```markdown
---
tags: [raft, consensus]
node_type: concept
aliases: [Raft Consensus]
title: Raft consensus protocol
---
# Raft

See [[log-compaction]] and [[docs/architecture]].

The storage path is implemented by symbol:StorageEngine
and symbol:demo::crate::StorageEngine::open.
```

### Node types (`node_type`)

| Value | Intent |
|-------|--------|
| `goal` | Goals, non-goals, SLAs, scope |
| `adr` | Architectural decision records |
| `alternative` | Options considered / rejected |
| `concept` | Atomic Zettelkasten-style notes (default) |
| `symbol` | Code entities (usually auto-created from AST) |
| `reference` | External crates, APIs, docs |
| `edge_case` | Bugs, concurrency traps, platform quirks |

### Links

- **WikiLinks:** `[[Note]]`, `[[Note#Section]]`, `[[Note\|Alias]]`
- **Code anchors:** `symbol:Name`, `symbol:mod::Name`, `symbol:crate::mod::Name`, or `[[symbol:…]]`  
  → edges with `relation_type = "anchors"` into the symbol graph  
- WikiLinks and anchors inside fenced code / inline `` `code` `` are ignored

### Node IDs

Stable path slugs relative to the workspace, e.g. `docs/concepts/raft`  
(not bare file stems — avoids collisions across folders).

---

## Library API

```rust
use rustbrain_core::{Brain, ContextOptions, QueryOptions, Result};

fn main() -> Result<()> {
    let mut brain = Brain::open_or_create(".")?;
    let stats = brain.sync()?;
    eprintln!("indexed {} markdown files", stats.markdown_files);

    // Ranked search (BM25 + tags/aliases + type priors)
    let ranked = brain.query_ranked("raft", &QueryOptions::default())?;
    for hit in ranked.iter().take(5) {
        println!("{:.2}  [{}] {}", hit.score, hit.node.node_type, hit.node.title);
    }

    // Agent context: seeds + CSR neighbors, packed under a token budget
    let ctx = brain.context_for_prompt_with(
        "how does raft elect a leader?",
        &ContextOptions {
            max_tokens: 1024,
            hop_depth: 1,
            ..ContextOptions::default()
        },
    )?;
    print!("{}", ctx.to_xml());   // or ctx.to_markdown()
    Ok(())
}
```

Primary entry point: [`Brain`](https://docs.rs/rustbrain-core/latest/rustbrain_core/struct.Brain.html) in **`rustbrain-core`**.

### Published crates (crates.io)

| Package | Role | Who installs it |
|---------|------|-----------------|
| **`rustbrain-core`** | Library engine (SQLite, FTS, mmap, AST, Obsidian parsers) | Apps, agents, `cargo add rustbrain-core` |
| **`rustbrain`** | CLI binary | Humans, `cargo install rustbrain` |

AST (`tree-sitter`) and Obsidian (WikiLink / frontmatter / Canvas) parsing are **modules inside** `rustbrain-core`, gated by features `ast` and `obsidian` — not separate published crates.

---

## On-disk layout (`.brain/`)

| Path | Purpose |
|------|---------|
| `db.sqlite` | Source of truth — nodes, edges, FTS5, aliases, symbols, schema version |
| `graph.mmap` | Compiled CSR adjacency + node id table (atomic replace on sync) |
| `workspace.json` | Workspace marker |

Formats: [docs/SCHEMA.md](docs/SCHEMA.md), [docs/MMAP_FORMAT.md](docs/MMAP_FORMAT.md).

Registry of workspaces (for `--all-workspaces`):  
`$XDG_CONFIG_HOME/rustbrain/registry.json` (or platform equivalent via the `dirs` crate).

---

## Architecture (v0.1)

```text
Tier 1  Human notes          Markdown + frontmatter + WikiLinks
Tier 2  Master store         SQLite + FTS5 BM25 + migrations
Tier 3  AST overlay          tree-sitter-rust → symbol nodes + anchors
Tier 4  Agent read path      CSR mmap neighbors + ranked seeds + token pack
```

**Sync pipeline:** walk workspace → transactional Markdown upsert (content-hash skip) → AST symbols → resolve pending links → compile `graph.mmap`.

**Query pipeline:** escape FTS query → BM25 → boost title/id/tag/alias/type → optional cross-workspace merge.

**Context pipeline:** ranked seeds → k-hop CSR expansion → score fusion → pack until `max_tokens` (~4 chars/token heuristic).

---

## Feature flags (`rustbrain-core`)

| Feature | Default | Description |
|---------|---------|-------------|
| `ast` | ✓ | Tree-sitter Rust indexing |
| `obsidian` | ✓ | WikiLinks / frontmatter / Canvas |
| `mmap` | ✓ | CSR `graph.mmap` |
| `watch` | | Debounced filesystem watcher |
| `jshift` | | Optional in-place JSON helpers |
| `full` | | All optional features (CI / docs.rs) |

The CLI enables `ast`, `obsidian`, `mmap`, and `watch`.

---

## What v0.1 claims / does not claim

**Does:**

- Project-scoped Markdown second brain (Obsidian-compatible *input*)
- Ranked keyword search (BM25 + tags/aliases)
- Rust AST symbol anchors + note→symbol edges
- Graph-aware agent context under a token budget
- Portable `.brainbundle` export/import
- Local-only operation (no network required at runtime)

**Does not (yet):**

- Learned / neural embeddings (`vector_dim = 0` in the product path)
- Explicit AVX-512 / NEON kernels (portable unrolled dot product only)
- Full bidirectional Obsidian vault *write-back*
- MainBrain / SubBrain `--scope` hierarchy

---

## Development

```bash
git clone https://github.com/shan-alexander/rustbrain
cd rustbrain

cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
cargo doc -p rustbrain-core --no-deps --all-features --open
```

CI: [`.github/workflows/ci.yml`](.github/workflows/ci.yml).

### Publishing (maintainers)

See [docs/PUBLISHING.md](docs/PUBLISHING.md). Only **two** crates:

1. `rustbrain-core` (library)
2. `rustbrain` (CLI)

---

## Contributing

Bug reports and PRs welcome. Please:

1. Keep claims honest (no marketing for unfinished features).
2. Add tests for correctness fixes (FTS idempotency, edge FKs, mmap validation).
3. Run `cargo test --workspace --all-features` and clippy before pushing.

More detail: [CONTRIBUTING.md](CONTRIBUTING.md).

---

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you shall be dual-licensed as above, without any
additional terms or conditions.
