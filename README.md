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

> **v0.3.7:** orphan detection in doctor; soft auto-links via `links --auto` (filename + tags).  
> Neural embeddings, multi-brain `--scope`, and full two-way Obsidian write-back are **planned**, not claimed.

---

## Install

```bash
cargo install rustbrain --locked          # CLI
# ensure cargo bin is on PATH:
#   export PATH="$HOME/.cargo/bin:$PATH"
```

```toml
# Library / agents
[dependencies]
rustbrain-core = "0.3"
```

**Build requirements:** C toolchain (bundled SQLite, tree-sitter). **MSRV:** 1.80.

---

## Quick start

### Greenfield or mature repo

```bash
cd your-project

# Agents / one-shot (recommended) — also writes AGENTS.md
rustbrain setup --yes

# Optional: skip or customize the agent cookbook
# rustbrain setup --yes --no-agents-md
# rustbrain setup --yes --agents-template ./AGENTS.template.md

# Or step-by-step:
# rustbrain init && rustbrain bootstrap --yes --write && rustbrain sync && rustbrain doctor

# Agent-friendly note creation (auto-syncs by default)
rustbrain note new \
  --type adr \
  --title "Use local SQLite" \
  --note "Embedded store; no network at runtime."

rustbrain query "sqlite" --scores
rustbrain context "why local sqlite"
rustbrain links    # pending WikiLinks / symbol: refs
```

`setup` / `bootstrap` write root **`AGENTS.md`** (how agents should use rustbrain in this repo).
Template order: `--agents-template` → `RUSTBRAIN_AGENTS_TEMPLATE` → `AGENTS.template.md` /
`.rustbrain/AGENTS.template.md` → built-in default. Opt out: `--no-agents-md`.

Interactive humans can run `rustbrain bootstrap --write` **without** `--yes` to answer prompts about `.rustbrainignore`, `.gitignore` import, and `AGENTS.md`.

### Full CLI reference

See **[docs/CLI.md](docs/CLI.md)** for every flag, ignore dialect, bootstrap outputs, and agent nuances.

---

## CLI overview

| Command | Purpose |
|---------|---------|
| `setup` | One-shot init + bootstrap + sync + doctor |
| `init` | Create `.brain/db.sqlite` |
| `bootstrap` | Docs tree, `AGENTS.md`, `.rustbrainignore`, README harvest, AST module map |
| `sync` | Index Markdown / Canvas / Rust; bake `graph.mmap` |
| `doctor` | Health: pending links, type ratios (`--strict`, `--json`) |
| `note new` | Typed note (`--type`, `--title`, `--note`, `--sync`) |
| `links` | List pending unresolved links |
| `query <q>` | Ranked FTS (`--no-symbols`, `--type goal,adr`, `--scores`) |
| `context …` | Agent context (positional or `-p`; note-first; `--with-symbols`) |
| `watch` | Debounced re-index |
| `export` / `import` | Portable `.brainbundle` |

---

## Note format

```markdown
---
tags: [raft, consensus]
node_type: concept
aliases: [Raft Consensus]
---
# Raft

See [[log-compaction]] and [[docs/architecture]].

Implemented by symbol:StorageEngine and symbol:demo::crate::StorageEngine::open.
```

| `node_type` | Intent |
|-------------|--------|
| `goal` | Goals, non-goals, SLAs |
| `adr` | Architectural decisions |
| `alternative` | Options considered |
| `concept` | Atomic notes (default for most docs) |
| `symbol` | Code entities (usually from AST) |
| `reference` | External crates / APIs |
| `edge_case` | Traps, bugs, platform quirks |

- **WikiLinks:** `[[Note]]`, `[[Note#H]]`, `[[Note\|Alias]]` (skipped inside code fences)  
- **Code anchors:** `symbol:Name` / `symbol:crate::mod::Name` → edges `anchors`  
- **IDs:** path slugs (`docs/concepts/raft`); root **README.md** → hub id `readme` (type `goal` by default)

---

## Library API

```rust
use rustbrain_core::{
    bootstrap_noninteractive, create_note, run_doctor, Brain, ContextOptions,
    NoteNewOptions, NodeType, QueryOptions, Result,
};

fn main() -> Result<()> {
    let ws = std::path::Path::new(".");
    bootstrap_noninteractive(ws, true, false)?;

    let mut brain = Brain::open_or_create(ws)?;
    brain.sync()?;

    create_note(
        ws,
        &NoteNewOptions {
            node_type: NodeType::Concept,
            title: "Example".into(),
            note: Some("Body from an agent.".into()),
            tags: vec![],
            aliases: vec![],
            dir: None,
            force: false,
        },
    )?;
    brain.sync()?;

    let hits = brain.query_ranked("example", &QueryOptions::human())?;
    let ctx = brain.context_for_prompt_with(
        "summarize",
        &ContextOptions {
            max_tokens: 1024,
            hop_depth: 1,
            no_symbols: true,       // note-first seeds (default in 0.3)
            hop_to_symbols: true,   // still allow ADR → code neighbors
            ..ContextOptions::default()
        },
    )?;
    println!("{}", run_doctor(ws)?.to_text());
    println!("{}", ctx.to_xml());
    let _ = hits;
    Ok(())
}
```

### Published crates

| Package | Role |
|---------|------|
| **`rustbrain-core`** | Library (apps / agents) |
| **`rustbrain`** | CLI binary |

AST and Obsidian parsers are **modules inside** `rustbrain-core` (`ast`, `obsidian` features), not separate crates.

---

## On-disk layout

| Path | Purpose |
|------|---------|
| `docs/**/*.md` | Human source of truth |
| `.rustbrainignore` | Extra index skips (optional) |
| `.brain/db.sqlite` | Derived SQLite + FTS5 |
| `.brain/graph.mmap` | CSR adjacency + id table |
| `.brain/workspace.json` | Marker |

Formats: [docs/SCHEMA.md](docs/SCHEMA.md), [docs/MMAP_FORMAT.md](docs/MMAP_FORMAT.md).  
Ignore dialect + CLI details: [docs/CLI.md](docs/CLI.md).

---

## Feature flags (`rustbrain-core`)

| Feature | Default | Description |
|---------|---------|-------------|
| `ast` | ✓ | Tree-sitter Rust indexing |
| `obsidian` | ✓ | WikiLinks / frontmatter / Canvas |
| `mmap` | ✓ | CSR `graph.mmap` |
| `watch` | | Debounced watcher |
| `jshift` | | In-place JSON helpers |
| `full` | | All of the above |

---

## What v0.3.x claims / does not

**Does:** local Markdown second brain, bootstrap for mature repos, doctor, agent `note new`, ranked FTS with type filters, graph-aware context (including note→symbol hops), portable bundles.

**Does not (yet):** neural embeddings, explicit AVX-512 kernels, full Obsidian write-back, SubBrain `--scope`.

---

## Development

```bash
git clone https://github.com/shan-alexander/rustbrain
cd rustbrain
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Maintainers: [docs/PUBLISHING.md](docs/PUBLISHING.md) · [CONTRIBUTING.md](CONTRIBUTING.md)

---

## License

MIT OR Apache-2.0 — see [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE).
