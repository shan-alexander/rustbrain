# rustbrain-core

Core engine for [**rustbrain**](https://github.com/shan-alexander/rustbrain): project-scoped second brain for humans and AI agents.

**crates.io:** only this library and the **`rustbrain`** CLI are published.  
AST (tree-sitter) and Obsidian parsers live **inside** this crate as feature-gated modules.

## Install

```toml
[dependencies]
rustbrain-core = "0.2"

# Lighter compile without tree-sitter:
# rustbrain-core = { version = "0.2", default-features = false, features = ["obsidian", "mmap"] }
```

**MSRV:** 1.80 · **License:** MIT OR Apache-2.0 · C toolchain required for SQLite / optional AST.

## Capabilities

| Area | API |
|------|-----|
| Lifecycle | `Brain::create` / `open` / `open_or_create` / `sync` |
| Bootstrap | `bootstrap_workspace`, `bootstrap_noninteractive` |
| Notes | `create_note` / `NoteNewOptions` |
| Health | `run_doctor` / `DoctorReport` |
| Search | `query_ranked` + `QueryOptions::{human, no_symbols, include_types}` |
| Context | `context_for_prompt_with` + `ContextOptions::{no_symbols, hop_to_symbols}` |
| Ignore | `IgnoreSet`, `.rustbrainignore` |
| Portability | `export` / `import` brainbundles |
| Watch | `watch` (feature `watch`) |

## Quick start

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
            note: Some("Written by a tool.".into()),
            tags: vec!["demo".into()],
            aliases: vec![],
            dir: None,
            force: false,
        },
    )?;
    brain.sync()?;

    let hits = brain.query_ranked("example", &QueryOptions::human())?;
    let report = run_doctor(ws)?;
    assert!(report.db_exists);

    let ctx = brain.context_for_prompt_with(
        "summarize example",
        &ContextOptions {
            max_tokens: 800,
            hop_depth: 1,
            hop_to_symbols: true,
            ..ContextOptions::default()
        },
    )?;
    println!("{}\n{}", report.to_text(), ctx.to_markdown());
    let _ = hits;
    Ok(())
}
```

## Query / context nuances

- **`QueryOptions::human()`** sets `no_symbols = true` so private helpers do not drown note search.
- **`ContextOptions::hop_to_symbols`** (default `true`) still packs graph neighbors that are code symbols when seeds are notes — e.g. ADR → `symbol:…` anchors.
- Root **`README.md`** becomes hub node id `readme` (type `goal` unless frontmatter overrides).

## Ignore files

Built-in skips: `target/`, `.git/`, `.brain/`, `node_modules/`, …

Optional **`.rustbrainignore`** (gitignore-inspired). If it contains:

```text
# rustbrain: import-gitignore
```

…root `.gitignore` is merged when loading. Env `RUSTBRAIN_IMPORT_GITIGNORE=1` forces merge.

## Feature flags

| Feature | Default | Description |
|---------|---------|-------------|
| `ast` | ✓ | Tree-sitter Rust (`src/ast/`) |
| `obsidian` | ✓ | WikiLinks / frontmatter / Canvas (`src/obsidian/`) |
| `mmap` | ✓ | CSR `graph.mmap` |
| `watch` | | Debounced filesystem watcher |
| `jshift` | | In-place JSON helpers |
| `full` | | All optional features |

## On-disk

| Path | Role |
|------|------|
| `.brain/db.sqlite` | Source of truth |
| `.brain/graph.mmap` | CSR cache |
| `docs/**/*.md` | Human notes (source) |

Schema / mmap: repo `docs/SCHEMA.md`, `docs/MMAP_FORMAT.md`.  
CLI detail: repo `docs/CLI.md`.

## Related

- CLI: [`rustbrain`](https://crates.io/crates/rustbrain)

## License

MIT OR Apache-2.0
