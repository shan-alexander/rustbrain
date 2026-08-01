# rustbrain

**CLI for a Rust-native second brain** — local Markdown + SQLite knowledge graph for humans and AI agents.

[![crates.io](https://img.shields.io/crates/v/rustbrain.svg)](https://crates.io/crates/rustbrain)
[![docs.rs](https://docs.rs/rustbrain-core/badge.svg)](https://docs.rs/rustbrain-core)
[![License](https://img.shields.io/crates/l/rustbrain.svg)](https://github.com/shan-alexander/rustbrain)

Write ordinary notes (`docs/**/*.md`, Obsidian-style WikiLinks). Index Rust code with Tree-Sitter. Search with FTS5. Expand the graph. Pack **agent context** under a token budget. All offline, project-scoped, Git-friendly.

```text
  docs/*.md  +  src/**/*.rs  +  CHANGELOG.md  +  Cargo.toml deps
        │              │              │                │
        ▼              ▼              ▼                ▼
   WikiLinks      tree-sitter    hub: changelog   docs.rs notes
   frontmatter     symbols
        └──────────────┬──────────────┴────────────────┘
                       ▼
              .brain/db.sqlite   ← source of truth
                       │
                       ▼
              .brain/graph.mmap  ← CSR neighborhood cache
                       │
                       ▼
         query · context · graph · doctor · agents
```

**Library API:** [`rustbrain-core`](https://crates.io/crates/rustbrain-core)  
**Full flag book:** [docs/CLI.md](https://github.com/shan-alexander/rustbrain/blob/main/docs/CLI.md)  
**Repo:** [github.com/shan-alexander/rustbrain](https://github.com/shan-alexander/rustbrain)

---

## Why rustbrain?

| Benefit | What you get |
|---------|----------------|
| **Agent-ready** | `context` packs ranked notes + graph neighbors under a token budget (Markdown or XML) |
| **Truth in Git** | Notes are plain Markdown; `.brain/` is a disposable index (gitignore it) |
| **Code ↔ docs** | `symbol:Foo` from notes; `[[docs/adr/…]]` in rustdoc → bidirectional edges |
| **Rust ecosystem** | Indexes `CHANGELOG.md`, harvests **docs.rs** URLs from `Cargo.toml` on setup |
| **No cloud required** | Local SQLite FTS5 + optional CSR mmap; no LLM inventiveness in the engine |
| **HITL planning** | `plan` notes with status densify (`backlog` → `blocked`); optional ROADMAP/BACKLOG hubs |

---

## Install

```bash
cargo install rustbrain --locked
# pin: cargo install rustbrain --version 0.3.20 --locked

# ensure the binary is on PATH
export PATH="$HOME/.cargo/bin:$PATH"
rustbrain --version
```

**Requirements:** Rust **1.80+**, a C toolchain (bundled SQLite + tree-sitter).  
**License:** MIT OR Apache-2.0.

---

## Getting started (5 minutes)

### 1. One-shot setup (recommended)

```bash
cd your-project

rustbrain setup --yes
```

This will:

1. Create `.brain/db.sqlite`
2. Scaffold `docs/` (goals, ADRs, analysis, plans, …)
3. Write **root `AGENTS.md`** + **`docs/AGENTS.md`** (agent protocol)
4. Harvest README → `docs/goals/from-readme.md` (if present)
5. Harvest **Cargo.toml deps → docs.rs notes** under `docs/references/crates/`
6. Optional AST module map
7. **Sync** (index) + **doctor**

Skip pieces when you need to:

```bash
rustbrain setup --yes --no-crate-docs    # no docs.rs harvest
rustbrain setup --yes --no-agents-md     # no AGENTS.md write
rustbrain setup --yes --force            # overwrite generated files
```

### 2. Capture knowledge

```bash
# Preferred: type + title only → scaffold → edit the printed path → sync
rustbrain note new --type adr --title "Use local SQLite"
# edit docs/adr/use-local-sqlite.md  (Status / Context / Decision / Consequences)

rustbrain note new --type plan --title "Q3 platform roadmap"
# edit checkboxes / status: backlog | in_progress | qa | done | cancelled | blocked

rustbrain note new --type analysis --title "query path bench 2026-07-31"
rustbrain note new --type goal --title "Ship offline agent context"

rustbrain sync
```

### 3. Search & orient (agents and humans)

```bash
rustbrain query "sqlite" --scores
rustbrain query "status:in_progress" --type plan --scores
rustbrain query "serde" --scores              # docs.rs note after setup harvest

rustbrain context "why local sqlite"
rustbrain context "what shipped"
rustbrain context "roadmap priorities"

rustbrain graph docs/adr/use-local-sqlite.md
rustbrain graph changelog                     # if CHANGELOG.md exists
rustbrain doctor
rustbrain doctor --orphans
```

### 4. Everyday loop

```bash
# After editing docs or code:
rustbrain sync

# Soft-link orphans / normalize WikiLinks when needed:
rustbrain links --auto
rustbrain links --apply --dry-run
rustbrain links --apply --write

# Live re-index while editing:
rustbrain watch --debounce-ms 300
```

---

## Command reference (overview)

| Command | Purpose |
|---------|---------|
| **`setup`** | One-shot init + bootstrap + sync + doctor |
| **`init`** | Create empty `.brain/db.sqlite` only |
| **`bootstrap`** | Docs tree, AGENTS, ignore, README harvest, crate docs, module map |
| **`sync`** | Index Markdown / Canvas / Rust; bake `graph.mmap` |
| **`doctor`** | Health + optional `--orphans` / `--json` / `--strict` |
| **`note new`** | Typed scaffold under `docs/` (`--type` + `--title`) |
| **`query`** | Ranked FTS (`--scores`, `--type`, `--with-symbols`, `--all-workspaces`) |
| **`context`** | Agent pack: seeds + hops + excerpts (`-m` budget, `-F markdown\|xml`) |
| **`graph`** | Neighborhood ASCII/JSON or workspace stats |
| **`links`** | Pending list; `--auto` soft edges; `--apply` rewrites; `--discover` AC |
| **`watch`** | Debounced live re-index |
| **`export` / `import`** | Portable `.brainbundle` |

Full flags and nuances: **[docs/CLI.md](https://github.com/shan-alexander/rustbrain/blob/main/docs/CLI.md)**.

---

## Note types (quick map)

| Type | Default dir | Use for |
|------|-------------|---------|
| `goal` | `docs/goals/` | Aims / non-goals |
| `adr` | `docs/adr/` | Decisions you committed to |
| `analysis` | `docs/analysis/` | Dated investigations (benches, options) |
| `plan` | `docs/plans/` | Roadmaps, backlogs, tasklists (status densified on sync) |
| `changelog` | root hub / `docs/changelogs/` | Ship history (prefer root **CHANGELOG.md**) |
| `concept` | `docs/concepts/` | Timeless “what is X” |
| `edge_case` | `docs/edge_cases/` | Traps and platform quirks |
| `reference` | `docs/concepts/` or generated crates | External crates / APIs |

**Root hubs (when files exist):**

| File | Node id | Type |
|------|---------|------|
| `README.md` | `readme` | `goal` |
| `CHANGELOG.md` | `changelog` | `changelog` |
| `ROADMAP.md` | `roadmap` | `plan` |
| `BACKLOG.md` | `backlog` | `plan` |

Plan status tokens (after sync): `status:backlog`, `status:in_progress`, `status:qa`, `status:done`, `status:cancelled`, `status:blocked`.

---

## Agent protocol (HITL)

After `setup`, agents should treat **`AGENTS.md`** / **`docs/AGENTS.md`** as the project mandate:

1. **Orient** with `rustbrain context "…"` before large refactors  
2. **Search** with `query` / `graph` instead of inventing history  
3. **Capture** with `note new` scaffolds + WikiLinks / `symbol:`  
4. **`sync`** after doc or code changes  
5. **Never invent** ADR history or changelog entries  

Customize the cookbook:

```bash
rustbrain setup --yes --agents-template ./AGENTS.template.md
# or commit AGENTS.template.md / .rustbrain/AGENTS.template.md
# or export RUSTBRAIN_AGENTS_TEMPLATE=/path/to/template.md
```

---

## Linking notes and code

```markdown
---
node_type: adr
tags: [storage]
---
# Use local SQLite

Decision: keep the brain in-process.

See [[docs/concepts/fts5]] and symbol:Database::open.
```

```rust
/// Primary store. Decision: [[docs/adr/use-local-sqlite]].
pub struct Database { /* … */ }
```

On `sync`:

- Note → code: `anchors`  
- Code rustdoc → note: `doc_links`

---

## Portability

```bash
rustbrain export --out team.brainbundle --decouple-ast
# copy file to another machine / repo
rustbrain import --input team.brainbundle -w /other/project
rustbrain sync -w /other/project
```

---

## Exit codes

| Code | Meaning |
|------|---------|
| `0` | Success |
| `1` | Error, or `doctor --strict` with unhealthy / pending links |

---

## Library usage

Embed the same engine in tools and agents via **[rustbrain-core](https://crates.io/crates/rustbrain-core)**:

```toml
[dependencies]
rustbrain-core = "0.3"
```

See that crate’s README for full Rust API examples (`Brain`, `query_ranked`, `context_for_prompt_with`, …).

---

## License

MIT OR Apache-2.0
