# rustbrain-core

**Core engine** for [rustbrain](https://github.com/shan-alexander/rustbrain): a project-scoped knowledge graph for software repositories — humans and AI agents.

[![crates.io](https://img.shields.io/crates/v/rustbrain-core.svg)](https://crates.io/crates/rustbrain-core)
[![docs.rs](https://docs.rs/rustbrain-core/badge.svg)](https://docs.rs/rustbrain-core)
[![License](https://img.shields.io/crates/l/rustbrain-core.svg)](https://github.com/shan-alexander/rustbrain)

Index Markdown notes and Rust symbols into **SQLite (FTS5 + edges)**, bake an optional **CSR `graph.mmap`**, then serve ranked search and **graph-aware context packs** under a token budget. No cloud, no invented ADRs — algorithmic and Git-friendly.

```text
  Markdown notes          Rust sources
  WikiLinks / YAML        tree-sitter symbols
         │                        │
         └───────────┬────────────┘
                     ▼
              Brain::sync()
                     │
         ┌───────────┴───────────┐
         ▼                       ▼
   .brain/db.sqlite        .brain/graph.mmap
   nodes · FTS · edges     CSR neighborhoods
         │                       │
         └───────────┬───────────┘
                     ▼
    query_ranked · context_for_prompt · graph · doctor
```

**CLI:** [`rustbrain`](https://crates.io/crates/rustbrain) (same engine, agent-friendly commands)  
**Repo / architecture:** [github.com/shan-alexander/rustbrain](https://github.com/shan-alexander/rustbrain)

---

## Why embed rustbrain-core?

| Benefit | For your tool / agent |
|---------|------------------------|
| **Structured retrieval** | Ranked FTS + type filters + tag/alias boosts — not bag-of-files grep alone |
| **Graph hops** | From a note, expand CSR neighbors (ADRs → symbols, plans → concepts) |
| **Token-budget packs** | `context_for_prompt*` returns Markdown or XML sized for LLM prompts |
| **Scaffolded capture** | `create_note` writes type-aware templates agents can fill |
| **Project hubs** | `README`, `CHANGELOG`, optional `ROADMAP`/`BACKLOG` as stable node ids |
| **Deps → docs.rs** | Bootstrap harvests crates.io deps into reference notes with docs.rs URLs |
| **Disposable index** | Rebuild `.brain/` anytime; truth stays in Markdown |

---

## Install

```toml
[dependencies]
rustbrain-core = "0.3"

# Smaller compile (no tree-sitter):
# rustbrain-core = { version = "0.3", default-features = false, features = ["obsidian", "mmap"] }

# Live re-index API:
# rustbrain-core = { version = "0.3", features = ["watch"] }
```

**MSRV:** 1.80 · **License:** MIT OR Apache-2.0 · C toolchain required (bundled SQLite; optional AST).

---

## Getting started

### Minimal: open, sync, search, pack

```rust
use rustbrain_core::{Brain, ContextOptions, QueryOptions, Result};

fn main() -> Result<()> {
    // Walks parents for .brain/ (like git); creates if missing at this path.
    let mut brain = Brain::open_or_create(".")?;

    // Index docs/**/*.md + Rust (when `ast` feature is on) + bake graph.mmap
    let stats = brain.sync()?;
    println!(
        "synced: upserted={} pending={} mmap={}",
        stats.nodes_upserted, stats.edges_pending, stats.mmap_written
    );

    // Note-first search (excludes pure code symbols)
    let hits = brain.query_ranked("sqlite fts", &QueryOptions::human())?;
    for h in hits.iter().take(5) {
        println!(
            "[{:.2}] {}  ({})  {}",
            h.score, h.node.title, h.node.node_type, h.node.id
        );
    }

    // Agent context under ~token budget (chars/4 heuristic)
    let ctx = brain.context_for_prompt("why local sqlite", 1024)?;
    println!("{}", ctx.to_markdown());
    // or: ctx.to_xml()

    Ok(())
}
```

### Bootstrap a mature repo (docs tree + AGENTS + crate docs)

```rust
use rustbrain_core::{bootstrap_noninteractive, run_doctor, Brain, Result};

fn main() -> Result<()> {
    let ws = std::path::Path::new(".");

    // Scaffold docs/, AGENTS.md, docs/AGENTS.md, .rustbrainignore,
    // README harvest, Cargo.toml → docs.rs notes, optional module map
    bootstrap_noninteractive(ws, /* write */ true, /* force */ false)?;

    let mut brain = Brain::open_or_create(ws)?;
    brain.sync()?;

    let report = run_doctor(ws)?;
    print!("{}", report.to_text());
    // report.healthy, report.findings, report.pending_links, …

    Ok(())
}
```

### Capture a note (scaffold), then re-index

```rust
use rustbrain_core::{create_note, Brain, NoteNewOptions, NodeType, Result};

fn main() -> Result<()> {
    let ws = std::path::Path::new(".");
    let mut brain = Brain::open_or_create(ws)?;

    // Prefer empty body → type-specific scaffold (ADR / plan / analysis / …)
    let created = create_note(
        ws,
        &NoteNewOptions {
            node_type: NodeType::Adr,
            title: "Use local SQLite".into(),
            note: None, // scaffold Status / Context / Decision / Consequences
            tags: vec!["storage".into()],
            aliases: vec![],
            dir: None, // default docs/adr/
            force: false,
        },
    )?;
    println!("wrote {} (id {})", created.rel_path.display(), created.node_id);

    // Agent edits the file on disk, then:
    brain.sync()?;
    Ok(())
}
```

### Graph neighborhood & plan status search

```rust
use rustbrain_core::{Brain, GraphOptions, QueryOptions, NodeType, Result};

fn main() -> Result<()> {
    let brain = Brain::open(".")?;

    // Structure around a note (ASCII or use GraphNeighborhood as data)
    let nb = brain.graph_neighborhood("docs/adr/use-local-sqlite", &GraphOptions {
        hops: 1,
        include_auto: false,
        ..GraphOptions::default()
    })?;
    print!("{}", nb.to_ascii());

    // Plans densify status tokens on sync (status:in_progress, status:blocked, …)
    let mut opts = QueryOptions::human();
    opts.include_types = vec![NodeType::Plan];
    let open = brain.query_ranked("status:in_progress", &opts)?;
    for h in open {
        println!("{} — {:?}", h.node.id, h.node.summary);
    }
    Ok(())
}
```

### Apply safe WikiLink rewrites (optional)

```rust
use rustbrain_core::{ApplyOptions, ApplyStyle, Brain, Result};

fn main() -> Result<()> {
    let mut brain = Brain::open(".")?;

    // Dry-run unique pending normalizations (+ optional AC discover)
    let plan = brain.apply_links(&ApplyOptions {
        write: false,
        discover: true,
        style: ApplyStyle::Wrap,
        graph_priors: true,
        ..ApplyOptions::default()
    })?;
    print!("{}", plan.to_text());

    // Apply AUTO tier only
    let applied = brain.apply_links(&ApplyOptions {
        write: true,
        dry_run: false,
        discover: false,
        sync_after: true,
        ..ApplyOptions::default()
    })?;
    if applied.recommend_sync {
        brain.sync()?;
    }
    Ok(())
}
```

---

## Core API surface

| Area | Entry points |
|------|----------------|
| **Lifecycle** | `Brain::create` · `open` · `open_exact` · `open_or_create` · `sync` |
| **Search** | `query` · `query_ranked` + `QueryOptions::{human, no_symbols, include_types, type_boosts}` |
| **Context** | `context_for_prompt` · `context_for_prompt_with` + `ContextOptions` · `ContextBundle::{to_markdown, to_xml}` |
| **Graph** | `graph_neighborhood` · `graph_stats` + `GraphOptions` / `GraphDirection` |
| **Notes** | `create_note` / `Brain::note_new` · `NoteNewOptions` · `NodeType` |
| **Bootstrap** | `bootstrap_workspace` · `bootstrap_noninteractive` · `BootstrapOptions` · agents templates |
| **Crate docs** | `collect_crate_deps` · `write_crate_docs_notes` · `docs_rs_url` |
| **Health** | `run_doctor` / `run_doctor_with` · `DoctorReport` |
| **Links** | `list_orphan_notes` · `run_auto_link` · `apply_links` · `ApplyOptions` |
| **Portability** | `export` / `import` brainbundles |
| **Watch** | `Brain::watch` (feature `watch`) |
| **Hubs** | `detect_project_hub` · `HUB_CHANGELOG` · `is_release_intent` · `is_planning_intent` |
| **Plan status** | `densify_plan` · `PlanStatus` · `enrich_plan_index_fields` |

---

## Node types

| `NodeType` | String | Typical path / hub |
|------------|--------|---------------------|
| `Goal` | `goal` | `docs/goals/`, hub `readme` |
| `Adr` | `adr` | `docs/adr/` |
| `Alternative` | `alternative` | `docs/adr/` |
| `Concept` | `concept` | `docs/concepts/` |
| `Analysis` | `analysis` | `docs/analysis/` (dated digs) |
| `Plan` | `plan` | `docs/plans/`, hubs `roadmap` / `backlog` |
| `Changelog` | `changelog` | hub `changelog` ← root `CHANGELOG.md` |
| `Reference` | `reference` | crate docs notes, external refs |
| `EdgeCase` | `edge_case` | `docs/edge_cases/` |
| `Symbol` | `symbol` | AST-extracted code entities |

`NodeType::parse` accepts aliases such as `roadmap` / `todo` → `Plan`, `analyses` → `Analysis`.

### Plan status densification (on sync)

For `plan` notes, Markdown status is projected into summary + FTS:

| Canonical | Surfaces (examples) |
|-----------|---------------------|
| `backlog` | todo, open, pending |
| `in_progress` | wip, doing, active |
| `qa` | review, testing |
| `done` | complete, finished |
| `cancelled` | canceled, wontfix |
| `blocked` | stuck, on_hold, **undone** (legacy alias) |

Sources: frontmatter `status:` / `state:`, `## Status`, section headings, checkboxes (`[ ]` `[/]` `[x]` `[~]` `[?]` `[!]`).

---

## Query & context nuances

- **`QueryOptions::human()`** — `no_symbols = true` so code noise does not drown notes.  
- **Natural-language FTS** — stopwords stripped; multi-token OR for recall (`why X not Y`).  
- **`ContextOptions`** — note-first seeds, body excerpts, ADR-first pack rank; `hop_to_symbols` (default true) still allows ADR → code.  
- **Hub inject** — empty/generic prompts pull README / harvest; release intent pulls `changelog`; planning intent pulls `roadmap` / `backlog` when indexed.  
- **`Brain::open`** — walks parent directories for `.brain` (use `open_exact` for a fixed path).

```rust
use rustbrain_core::{ContextOptions, NodeType, QueryOptions};

// Only ADRs and goals
let mut q = QueryOptions::human();
q.include_types = vec![NodeType::Adr, NodeType::Goal];
q.limit = 10;

// Larger pack, deeper hops, symbols as seeds too
let ctx_opts = ContextOptions {
    max_tokens: 2048,
    hop_depth: 2,
    no_symbols: false,
    hop_to_symbols: true,
    ..ContextOptions::default()
};
```

---

## Bootstrap options

```rust
use rustbrain_core::{bootstrap_workspace, BootstrapMode, BootstrapOptions};

let report = bootstrap_workspace(
    ".",
    BootstrapOptions {
        mode: BootstrapMode::NonInteractive,
        write: true,
        force: false,
        setup_ignore: Some(true),
        import_gitignore: Some(true),
        ignore_extras: true,
        harvest_readme: true,
        module_map: true,
        crate_docs: true,       // Cargo.toml → docs/references/crates/*
        scaffold_docs: true,
        write_agents_md: Some(true),
        agents_template: None,
    },
)?;
for a in report.actions {
    println!("[{}] {} — {}", a.action, a.path, a.detail);
}
```

**Generated layout (typical):**

| Path | Role |
|------|------|
| `docs/goals/`, `docs/adr/`, `docs/analysis/`, `docs/plans/`, … | Note trees |
| `AGENTS.md`, `docs/AGENTS.md` | Agent cookbooks |
| `docs/goals/from-readme.md` | Algorithmic README harvest |
| `docs/references/crates/*.md` | docs.rs URLs per crates.io dep |
| `docs/implementation/module-map.generated.md` | AST map (`ast` feature) |
| `.rustbrainignore` | Extra index skips |

---

## Feature flags

| Feature | Default | Description |
|---------|---------|-------------|
| `ast` | ✓ | Tree-sitter Rust symbol extraction |
| `obsidian` | ✓ | WikiLinks, YAML frontmatter, Canvas |
| `mmap` | ✓ | CSR `graph.mmap` reader/compiler |
| `watch` | | Debounced filesystem watcher (`notify`) |
| `jshift` | | Optional sparse JSON path helpers (not a serde_json replacement) |
| `full` | | All optional features |

JSON stack policy (when to use serde vs jshift): repo [`docs/JSON_STACK.md`](https://github.com/shan-alexander/rustbrain/blob/main/docs/JSON_STACK.md).

---

## On-disk layout

| Path | Role |
|------|------|
| `.brain/db.sqlite` | Source of truth (nodes, edges, FTS5, pending links) |
| `.brain/graph.mmap` | CSR adjacency cache for hops |
| `.brain/link_lexicon.json` | Optional AC lexicon cache (apply --discover) |
| `docs/**/*.md` | Human/agent-authored knowledge (source of truth for prose) |
| `CHANGELOG.md` | Optional hub `changelog` |
| `Cargo.toml` / `Cargo.lock` | Input to docs.rs harvest on bootstrap |

**Ignore:** built-in `target/`, `.git/`, `.brain/`, … plus optional `.rustbrainignore`.  
Line `# rustbrain: import-gitignore` merges root `.gitignore`. Env `RUSTBRAIN_IMPORT_GITIGNORE=1` forces merge.

Schema / mmap details: [`docs/SCHEMA.md`](https://github.com/shan-alexander/rustbrain/blob/main/docs/SCHEMA.md), [`docs/MMAP_FORMAT.md`](https://github.com/shan-alexander/rustbrain/blob/main/docs/MMAP_FORMAT.md).

---

## Mental model (storage)

```text
Markdown / Rust on disk     = source of truth (Git)
.brain/*                    = disposable index + densified projections
  nodes.summary             = skimmable (e.g. plan status line, latest changelog heading)
  node_fts.content          = full body + dense tokens (status:…, versions, …)
  edges                     = WikiLinks, anchors, doc_links, auto_*
  graph.mmap                = hop topology only
```

Status checkboxes and changelog headings are **not** separate SQL task tables — they are densified into FTS/summary on `sync` so agents can query without a kanban product.

---

## CLI companion

Install the same engine as a binary:

```bash
cargo install rustbrain --locked
rustbrain setup --yes
rustbrain context "topic"
```

See **[rustbrain](https://crates.io/crates/rustbrain)** for the full command reference.

---

## Related crates / docs

| Resource | Link |
|----------|------|
| CLI crate | [rustbrain](https://crates.io/crates/rustbrain) |
| Source | [shan-alexander/rustbrain](https://github.com/shan-alexander/rustbrain) |
| CLI book | [docs/CLI.md](https://github.com/shan-alexander/rustbrain/blob/main/docs/CLI.md) |
| API docs | [docs.rs/rustbrain-core](https://docs.rs/rustbrain-core) |

---

## License

MIT OR Apache-2.0
