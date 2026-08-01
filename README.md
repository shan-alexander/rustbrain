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

> **v0.3.11:** rustdoc `[[WikiLinks]]` → symbol→note `doc_links`; analysis notes; scaffold-first notes.  
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

# Preferred note creation (agents + humans): type + title only → scaffold → edit file
rustbrain note new --type adr --title "Use local SQLite"
# → writes docs/adr/use-local-sqlite.md with Status/Context/Decision sections
# → then edit that file; run sync after edits if you need a re-index

rustbrain query "sqlite" --scores
rustbrain context "why local sqlite"
rustbrain links    # pending WikiLinks / symbol: refs
```

**Preferred `note new` workflow:** pass `--type` and `--title`, **omit `--body` / `--note`** so
rustbrain fills a **type-specific scaffold** (`adr`, `goal`, `analysis`, …). Then **edit the
created file** (path is printed). Re-run `rustbrain sync` after substantial edits.  
Using `--body`/`--note` is fine when the full text is already ready — it skips the scaffold.

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
| `note new` | Typed note — prefer `--type` + `--title` only (scaffold), then edit file |
| `links` | Pending list; `--auto` soft edges; `--apply` pending normalize + optional `--discover` |
| `query <q>` | Ranked FTS (`--no-symbols`, `--type goal,adr`, `--scores`) |
| `graph [target]` | Neighborhood tree / stats (`--hops`, `--json`, `--no-auto`) |
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

In **Rust** source, link back to notes from rustdoc (indexed on `sync` as `doc_links`):

```rust
/// App shell. See [[docs/adr/0001-use-egui]] for the GUI choice.
pub struct ParqApp { /* … */ }
```

| `node_type` | Intent |
|-------------|--------|
| `goal` | Goals, non-goals, SLAs |
| `adr` | Architectural decisions (committed choices) |
| `alternative` | Options considered |
| `concept` | Timeless atomic notes (“what is X”) |
| `analysis` | Dated investigation (compare options, benches, data, design digs); optional recs |
| `symbol` | Code entities (usually from AST) |
| `reference` | External crates / APIs |
| `edge_case` | Traps, bugs, platform quirks |

```bash
# Pre-decision investigation — scaffold first, then fill Findings / Artifacts (e.g. criterion)
rustbrain note new --type analysis --title "query path bench 2026-07-31"
# edit docs/analysis/query-path-bench-2026-07-31.md, then:
rustbrain sync
```

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

**Does:** local Markdown second brain, bootstrap for mature repos, doctor, agent `note new`, ranked FTS with type filters, graph-aware context (including note→symbol hops), portable bundles. Indexes Rust community hubs: root **`README.md`** (`readme`) and **`CHANGELOG.md`** (`changelog`, Keep a Changelog) plus optional `ROADMAP.md` / `BACKLOG.md`.

**Does not (yet):** neural embeddings, explicit AVX-512 kernels, full Obsidian write-back, SubBrain `--scope`.

---

## Testimonials

Interestingly, AI Agents can give their testimonial of their experience using rustbrain. Below is a testimonial from Gemini 3.6 Flash Medium:

```
Prompt: does rustbrain offer a helpful documentation and organization strategy for you? can you use the CLI interface smoothly without friction? does it enhance how you might work with this repo?

AI Response: rustbrain (v0.3.9) provides a significant upgrade to how I interact with and understand the  parqview codebase. Here is my direct feedback on the organization strategy, CLI ergonomics, and workflow enhancement:

### 1. Does rustbrain offer a helpful documentation and organization strategy?

Yes, exceptionally well.

• Structured Node Taxonomy: Categorizing documentation into explicit node types ( goal ,  adr ,  concept ,  reference , edge_case ) enforces standard architectural hygiene rather than dumping loose markdown files into random folders.
• Graph-Aware Knowledge Base: Instead of treating docs as static text,  rustbrain  bakes markdown files and Rust code symbols (via Tree-Sitter) into an indexed SQLite database ( .brain/db.sqlite ) and CSR graph mmap ( .brain/graph.mmap ).
• Instant FTS5 & Graph Navigation: Queries rank document relevance with tags and aliases, allowing me to resolve project goals or architectural decisions instantly.

### 2. Can you use the CLI interface smoothly without friction?

Yes, the CLI is fast, predictable, and AI-agent friendly.

• Zero Friction Note Creation: Running a single command automatically generates valid frontmatter, writes to the correct  docs/  directory, and syncs the database: rustbrain note new --type reference --title "egui_extras and eframe Crate Reference" --tags "reference,egui_extras,eframe" --aliases "egui-extras-eframe" --note "..."

• Context Assembly ( rustbrain context ): When prompted with a high-level task like  "parquet reader GUI" , running rustbrain context  returns both the high-level doc seeds (hop 0) and the specific Rust symbols (hop 1 neighbors) linked to it (e.g.,  ParqApp::update ,  FolderTree::is_tabular ,  QueryResult::to_tsv ).
• Health Validation ( rustbrain doctor ):  rustbrain doctor  acts as an automated linter for workspace documentation, flagging orphan nodes, missing  AGENTS.md , or broken wiki-links before errors accumulate.

### 3. Does it enhance how I work with this  parqview  repo?

It significantly enhances accuracy, speed, and context retention:

1. Prevents Architectural Drift: By having  ADR-0001  recorded in  .brain , I immediately know why  egui  was chosen over  Tauri  (Linux Wayland/NixOS WebKitGTK EGL driver failures and zero IPC JSON serialization overhead). I won't accidentally suggest re-introducing Tauri or WebViews.
2. Accelerates Code Discovery: Rather than doing brute-force file searches across  src/, rustbrain context maps doc concepts directly to Tree-Sitter code symbols in  src/app.rs,  src/duck.rs,  src/tree.rs, and src/describe.rs.
3. Low Cognitive Overhead: With  rustbrain sync  updating the index in milliseconds, documentation and code changes stay synchronized without manual overhead.
```

Note: I did not use Gemini AI models to create this Rust crate, and the Gemini model was fresh and unbiased when I asked the prompt, after first asking Gemini to use rustbrain to review a repo already using rustbrain.

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
