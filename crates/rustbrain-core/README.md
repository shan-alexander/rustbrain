# rustbrain-core

Core engine for [**rustbrain**](https://github.com/shan-alexander/rustbrain): a project-scoped, Rust-native second brain for humans and AI agents.

This is the **library** you depend on from application code. The `rustbrain` CLI is a thin wrapper around the same APIs.

> **crates.io:** only `rustbrain-core` (this crate) and `rustbrain` (CLI) are published.  
> Tree-sitter AST and Obsidian parsers ship **inside** this crate as feature-gated modules.

## What it provides

- **`Brain` façade** — open/create a `.brain/` store, sync a workspace, query, build agent context, export/import
- **SQLite master store** — nodes, weighted edges, FTS5 BM25, aliases, tags, pending links, schema migrations
- **Ranked search** — BM25 + title/id/tag/alias boosts + node-type priors
- **Graph-aware context** — ranked seeds → CSR k-hop expansion → token-budget packing (XML or Markdown)
- **Optional AST indexing** — tree-sitter Rust symbols (feature `ast`)
- **Optional Obsidian parsing** — WikiLinks / frontmatter / Canvas (feature `obsidian`)
- **Optional CSR mmap cache** — neighborhood walks (feature `mmap`)
- **Optional file watch** — debounced re-index (feature `watch`)

## Install

```toml
[dependencies]
rustbrain-core = "0.1"

# Skip tree-sitter (lighter compile) — Markdown + FTS + mmap only:
# rustbrain-core = { version = "0.1", default-features = false, features = ["obsidian", "mmap"] }
```

**MSRV:** 1.80 · **License:** MIT OR Apache-2.0

Requires a C toolchain when `ast` is enabled (tree-sitter) and for bundled SQLite.

## Quick start

```rust
use rustbrain_core::{Brain, ContextOptions, QueryOptions, Result};

fn main() -> Result<()> {
    let mut brain = Brain::open_or_create(".")?;
    brain.sync()?;

    let hits = brain.query_ranked("authentication", &QueryOptions::default())?;
    for h in hits.iter().take(5) {
        println!("{:.2} {}", h.score, h.node.title);
    }

    let ctx = brain.context_for_prompt("how do we store sessions?", 1500)?;
    println!("{}", ctx.to_markdown());
    Ok(())
}
```

## Feature flags

| Feature | Default | Description |
|---------|---------|-------------|
| `ast` | yes | Tree-sitter Rust symbol extraction (`src/ast/`) |
| `obsidian` | yes | WikiLinks, YAML frontmatter, Canvas (`src/obsidian/`) |
| `mmap` | yes | Compile/open `.brain/graph.mmap` CSR cache |
| `watch` | no | Debounced filesystem watcher |
| `jshift` | no | In-place JSON field mutation helpers |
| `full` | no | Enables every optional feature |

## On-disk data

Created under `<workspace>/.brain/`:

| File | Role |
|------|------|
| `db.sqlite` | Source of truth |
| `graph.mmap` | CSR adjacency + id table |
| `workspace.json` | Marker / metadata |

See the workspace docs: [SCHEMA.md](../../docs/SCHEMA.md), [MMAP_FORMAT.md](../../docs/MMAP_FORMAT.md).

## Module map

| Module | Responsibility |
|--------|----------------|
| `brain` | High-level `Brain` API |
| `query` | Ranked FTS search |
| `context` | Agent context assembly |
| `storage` | SQLite + migrations |
| `mmap` | CSR compiler/reader |
| `indexer` | Workspace walk / sync |
| `ast` | Tree-sitter symbols (feature) |
| `obsidian` | WikiLinks / frontmatter / Canvas (feature) |
| `symbols` | `symbol:…` ref parsing |
| `exporter` | `.brainbundle` I/O |
| `registry` | Global multi-workspace registry |

## Related package

- CLI: [`rustbrain`](https://crates.io/crates/rustbrain) (`cargo install rustbrain`)

## Non-goals (v0.1)

- Neural embeddings / ANN indexes (vector dim is 0 in the product path)
- Cloud sync or multi-user servers
- Full Obsidian vault write-back

## License

MIT OR Apache-2.0
