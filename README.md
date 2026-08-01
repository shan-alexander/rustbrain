# rustbrain

**A Rust-native second brain for software repositories** — Markdown knowledge graph + SQLite FTS5 + CSR mmap, for humans *and* AI coding agents.

[![crates.io](https://img.shields.io/crates/v/rustbrain.svg)](https://crates.io/crates/rustbrain)
[![docs.rs](https://docs.rs/rustbrain-core/badge.svg)](https://docs.rs/rustbrain-core)
[![License: MIT OR Apache-2.0](https://img.shields.io/crates/l/rustbrain.svg)](https://github.com/shan-alexander/rustbrain)

```bash
cargo install rustbrain --locked
export PATH="$HOME/.cargo/bin:$PATH"
cd your-project && rustbrain setup --yes
rustbrain context "why did we choose this architecture?"
```

| Package on crates.io | Role |
|---|---|
| **[`rustbrain`](https://crates.io/crates/rustbrain)** | **CLI (primary)** — install this; binary name `rustbrain` |
| **[`rustbrain-core`](https://crates.io/crates/rustbrain-core)** | **Library** — same engine for embedders / agent runtimes |

> **Primary path:** use the **CLI** in your repo (`setup` → `note` / `sync` → `context` / `query` / `graph`).  
> **Library path:** engineers who want rustbrain *inside* a tool depend on **`rustbrain-core`** only — the CLI is a thin, agent-friendly front end over that crate. They are **separated intentionally** so app authors do not pull a binary package just to call `Brain::sync()`.

Full published CLI guide (commands, sequences, flags): **[crates.io/crates/rustbrain](https://crates.io/crates/rustbrain)** · in-repo book: **[docs/CLI.md](docs/CLI.md)**.

---

## What is rustbrain?

Write ordinary notes (`docs/**/*.md`, Obsidian-style WikiLinks + frontmatter). Index Rust with Tree-Sitter. Search with **FTS5**. Expand a **CSR graph**. Pack **agent context** under a token budget.

All **offline**, **project-scoped**, **Git-friendly**. The engine is algorithmic — it does **not** invent ADRs, changelogs, or decision history.

```text
  docs/*.md   src/**/*.rs   CHANGELOG.md   Cargo.toml deps
       │            │             │               │
       ▼            ▼             ▼               ▼
  WikiLinks    tree-sitter   hub:changelog   docs.rs notes
  frontmatter   symbols
       └────────────┬─────────────┴───────────────┘
                    ▼
           .brain/db.sqlite     ← disposable index (gitignore)
                    │
                    ▼
           .brain/graph.mmap    ← CSR neighborhood cache
                    │
                    ▼
     setup · note · sync · query · context · graph · links · doctor
```

**Analogy:** plain “dump docs into the prompt” is a pile of papers. rustbrain is a **catalog + map of the building** — find the right rooms, see who links to whom, then pack only what fits the token budget.

### Four jobs

| Job | Phrase | CLI |
|---|---|---|
| **Orient** | What does this repo already know? | `context`, `query`, `graph` |
| **Capture** | Write a real note, not chat residue | `note new` → edit → `sync` |
| **Connect** | Who links to whom? Fix pending links | `graph`, `links`, `links --apply` |
| **Health** | Is the brain usable for agents? | `doctor`, `sync` |

Markdown on disk is the **source of truth**. `.brain/` is a **rebuildable cache** (like `target/` for knowledge).

---

## Why rustbrain?

| Benefit | What you get |
|---|---|
| **Agent-ready** | `setup --yes` writes `AGENTS.md`; agents use `context` / `query` every turn |
| **Truth in Git** | Notes are plain Markdown; `.brain/` is disposable |
| **Code ↔ docs** | `symbol:Foo` from notes; `[[docs/adr/…]]` in rustdoc → bidirectional edges |
| **Graph-aware packs** | `context` = ranked seeds + hops under a token budget (Markdown or XML) |
| **Rust ecosystem** | Root `CHANGELOG.md` hub; Cargo.toml → **docs.rs** reference notes on setup |
| **HITL planning** | `plan` notes with densified status (`backlog` … `blocked`); optional ROADMAP/BACKLOG |
| **No cloud** | Local SQLite + optional CSR mmap; no LLM inventiveness in the engine |
| **Careful rewrites** | `links --apply` closes unique pending WikiLinks; AC discover is opt-in, dry-run default |

### When to use it

| Prefer **rustbrain** when… | Prefer something else when… |
|---|---|
| Structured notes (goals, ADRs, plans) must survive chat sessions | One-off grep in a throwaway folder |
| Agents must **not invent** decision history | You want a SaaS “memory” product only |
| You need **code↔doc graph** hops | Pure vector RAG over opaque blobs is enough |
| Knowledge should be PR-reviewable in Git | Docs must live only outside the repo |

---

## Install

### CLI (recommended for almost everyone)

```bash
cargo install rustbrain --locked
# pin: cargo install rustbrain --version 0.3.21 --locked
export PATH="$HOME/.cargo/bin:$PATH"
rustbrain --version
```

**Requirements:** Rust **1.80+**, a C toolchain (bundled SQLite + tree-sitter).  
**License:** MIT OR Apache-2.0.

### Library (embedders only)

```toml
[dependencies]
rustbrain-core = "0.3"
```

See **[rustbrain-core](https://crates.io/crates/rustbrain-core)** for API examples, feature flags, and bootstrap options.

---

## Quick start

### One-shot (greenfield or mature repo)

```bash
cd your-project
rustbrain setup --yes
```

This creates `.brain/`, scaffolds `docs/`, writes **`AGENTS.md`** + **`docs/AGENTS.md`**, harvests README + Cargo.toml → docs.rs notes, runs **sync** + **doctor**.

```bash
# optional knobs
rustbrain setup --yes --no-crate-docs
rustbrain setup --yes --no-agents-md
rustbrain setup --yes --force
rustbrain setup --yes --agents-template ./AGENTS.template.md
```

### Capture → search → pack

```bash
# Preferred: type + title only → scaffold → edit printed path → sync
rustbrain note new --type adr --title "Use local SQLite"
# edit docs/adr/use-local-sqlite.md

rustbrain note new --type plan --title "Q3 platform roadmap"
rustbrain note new --type analysis --title "query path bench 2026-07-31"

rustbrain sync

rustbrain query "sqlite" --scores
rustbrain query "status:in_progress" --type plan --scores
rustbrain context "why local sqlite"
rustbrain graph docs/adr/use-local-sqlite.md
rustbrain doctor
```

### Everyday loop

```text
edit docs/code → sync → query | context | graph
                 ├── links --auto          (soft edges)
                 ├── links --apply --dry-run / --write
                 └── watch --debounce-ms 300
```

---

## Recommended sequences

### Agent turn (HITL)

```bash
rustbrain context "task keywords"
rustbrain query "related" --scores
rustbrain graph docs/adr/relevant.md
# … implement …
rustbrain note new --type adr --title "Decision title"   # then edit file
rustbrain sync
rustbrain doctor
```

### Planning

```bash
rustbrain note new --type plan --title "Sprint board"
rustbrain sync
rustbrain query "status:in_progress" --type plan --scores
rustbrain query "status:blocked" --type plan --scores
rustbrain context "roadmap priorities"
```

### Link hygiene

```bash
rustbrain links
rustbrain links --auto
rustbrain links --apply --dry-run
rustbrain links --apply --write
rustbrain links --apply --discover --dry-run
```

### Portability

```bash
rustbrain export --out team.brainbundle --decouple-ast
rustbrain import --input team.brainbundle -w /other/project
rustbrain sync -w /other/project
```

More playbooks and every flag: **[docs/CLI.md](docs/CLI.md)** · published CLI README on **[crates.io/crates/rustbrain](https://crates.io/crates/rustbrain)**.

---

## CLI overview

| Command | Purpose |
|---|---|
| `setup` | One-shot init + bootstrap + sync + doctor |
| `init` | Create `.brain/db.sqlite` only |
| `bootstrap` | Docs tree, AGENTS, ignore, README/crate harvest, module map |
| `sync` | Index Markdown / Canvas / Rust; bake `graph.mmap` |
| `doctor` | Health (`--strict`, `--json`, `--orphans`; multi-brain scope checks) |
| `note new` | Typed scaffold (`--type` + `--title`; optional `--scope`) |
| `query` | Ranked FTS (`--scores`, `--type`, `--with-symbols`, `--scope`) |
| `context` | Agent pack (`-m`, `-F markdown\|xml`, `--scope`) |
| `graph` | Neighborhood tree / workspace stats |
| `scopes` | MainBrain / SubBrain: **`list` (ids)**, enable, add, attach, import, absorb, reconcile |
| `links` | Pending; `--auto`; `--apply` (+ optional `--discover`) |
| `watch` | Debounced live re-index |
| `export` / `import` | Portable `.brainbundle` (`export --scope ID` = SubBrain slice) |

Most commands accept `-w /path`. `query` / `context` / `doctor` / `graph` walk parents for `.brain/` (git-style).

### Discover SubBrain ids (before import / scoped query)

```bash
rustbrain scopes list                 # ids + roots + node counts (this workspace)
rustbrain scopes list --json          # tools / agents
rustbrain scopes list -w /other/path  # inspect another tree
# New SubBrain id is chosen by you: directory name is the convention
rustbrain scopes attach project-a --root project-a
rustbrain scopes import --from ./project-b --as project-b --mount
```

Full command book: **[docs/CLI.md](docs/CLI.md)** · generated **AGENTS.md** after `setup` has agent-oriented tables.

---

## Note format & types

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

```rust
/// App shell. See [[docs/adr/0001-use-egui]] for the GUI choice.
pub struct ParqApp { /* … */ }
```

| `node_type` | Intent |
|---|---|
| `goal` | Goals, non-goals, SLAs |
| `adr` | Architectural decisions (committed) |
| `alternative` | Options considered |
| `concept` | Timeless “what is X” |
| `analysis` | Dated investigation (benches, compares) |
| `plan` | Roadmaps / backlogs / todos (status densified on sync) |
| `changelog` | Ship history (prefer root `CHANGELOG.md`) |
| `symbol` | Code entities (usually from AST) |
| `reference` | External crates / APIs |
| `edge_case` | Traps, bugs, platform quirks |

**Root hubs when files exist:** `README.md` → `readme` (goal); `CHANGELOG.md` → `changelog`; `ROADMAP.md` / `BACKLOG.md` → plan hubs.

**Plan status tokens (after sync):** `status:backlog`, `status:in_progress`, `status:qa`, `status:done`, `status:cancelled`, `status:blocked` (`undone` is a legacy alias for blocked).

- **WikiLinks:** `[[Note]]`, `[[Note#H]]`, `[[Note\|Alias]]` (skipped in fences)  
- **Code anchors:** `symbol:Name` → `anchors`  
- **Rustdoc:** `[[docs/…]]` → `doc_links`

---

## Library API (embedders)

Use **`rustbrain-core`** when you are building a tool, agent runtime, or IDE integration — not when you only need the CLI in a project.

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
            note: None, // type-specific scaffold when None
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
            no_symbols: true,
            hop_to_symbols: true,
            ..ContextOptions::default()
        },
    )?;
    println!("{}", run_doctor(ws)?.to_text());
    println!("{}", ctx.to_markdown());
    let _ = hits;
    Ok(())
}
```

AST and Obsidian parsers live **inside** `rustbrain-core` (`ast`, `obsidian` features), not as separate crates.

More: [crates.io/crates/rustbrain-core](https://crates.io/crates/rustbrain-core) · [docs.rs/rustbrain-core](https://docs.rs/rustbrain-core).

---

## On-disk layout

| Path | Purpose |
|---|---|
| `docs/**/*.md` | Human source of truth |
| `CHANGELOG.md` / `ROADMAP.md` / `BACKLOG.md` | Optional hubs |
| `.rustbrainignore` | Extra index skips |
| `.brain/db.sqlite` | Derived SQLite + FTS5 |
| `.brain/graph.mmap` | CSR adjacency cache |
| `.brain/link_lexicon.json` | Optional AC lexicon |
| `AGENTS.md`, `docs/AGENTS.md` | Agent cookbooks |

Formats: [docs/SCHEMA.md](docs/SCHEMA.md), [docs/MMAP_FORMAT.md](docs/MMAP_FORMAT.md).  
Ignore dialect + CLI details: [docs/CLI.md](docs/CLI.md).

---

## Feature flags (`rustbrain-core`)

| Feature | Default | Description |
|---|---|---|
| `ast` | ✓ | Tree-sitter Rust indexing |
| `obsidian` | ✓ | WikiLinks / frontmatter / Canvas |
| `mmap` | ✓ | CSR `graph.mmap` |
| `watch` | | Debounced watcher |
| `jshift` | | Sparse JSON path helpers (optional) |
| `full` | | All of the above |

JSON policy: [docs/JSON_STACK.md](docs/JSON_STACK.md).

---

## Performance

Criterion package: [`crates/bench`](crates/bench) (`publish = false`).

```bash
cargo bench -p bench --bench rustbrain_performance   # engine vs baselines
cargo bench -p bench --bench serde_vs_jshift         # JSON stack (see docs/JSON_STACK.md)
```

Full tables + fairness: **[docs/BENCHMARKS.md](docs/BENCHMARKS.md)**.

Headline (release, ~500 synthetic notes, one Linux machine, Criterion `--quick` — re-run locally):

| Workload | rustbrain | Alternative | ~Ratio |
|---|---|---|---|
| Search (`query_ranked` vs walk `.md`) | ~3.5 ms | ~7.6 ms | **~2.2×** |
| Graph hop (CSR vs re-parse WikiLinks) | ~156 ns | ~8.9 ms | **~50 000×** |
| Context vs grep-rank concat | ~5.7 ms | ~8.1 ms | **~1.4×** + graph/type packing |

Baselines model “no FTS / no CSR / dump docs into the prompt” — not peer SaaS products. Cheaper dumps exist; they are unranked and agent-hostile.

---

## What v0.3.x claims / does not

**Does:** local Markdown second brain; bootstrap for mature repos; doctor; agent `note new`; ranked FTS with type filters; graph-aware context (including note→symbol hops); first-class **`changelog`** + **`plan`**; docs.rs harvest; portable bundles; careful `links --apply`.

**Does not (yet):** neural embeddings; explicit AVX-512 kernels; full two-way Obsidian write-back; multi-brain `--scope` SubBrain product.

---

## Testimonials

Interestingly, AI agents can give their testimonial of their experience using rustbrain. Below is a testimonial from Gemini 3.6 Flash Medium:

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

| Doc | Path |
|---|---|
| CLI book | [docs/CLI.md](docs/CLI.md) |
| Performance | [docs/BENCHMARKS.md](docs/BENCHMARKS.md) |
| Schema | [docs/SCHEMA.md](docs/SCHEMA.md) |
| Publish | [docs/PUBLISHING.md](docs/PUBLISHING.md) |
| Contributing | [CONTRIBUTING.md](CONTRIBUTING.md) |
| Changelog | [CHANGELOG.md](CHANGELOG.md) |

Published crate READMEs (what crates.io shows):

- CLI: [crates/rustbrain-cli/README.md](crates/rustbrain-cli/README.md) → package **`rustbrain`**
- Library: [crates/rustbrain-core/README.md](crates/rustbrain-core/README.md) → package **`rustbrain-core`**

---

## License

MIT OR Apache-2.0 — see [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE).
