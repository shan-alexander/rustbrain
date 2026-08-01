# rustbrain

**The CLI for a Rust-native second brain** — project-scoped Markdown knowledge graph for humans and AI coding agents.

[![crates.io](https://img.shields.io/crates/v/rustbrain.svg)](https://crates.io/crates/rustbrain)
[![docs.rs](https://docs.rs/rustbrain-core/badge.svg)](https://docs.rs/rustbrain-core)
[![License: MIT OR Apache-2.0](https://img.shields.io/crates/l/rustbrain.svg)](https://github.com/shan-alexander/rustbrain)

```bash
cargo install rustbrain --locked
export PATH="$HOME/.cargo/bin:$PATH"
cd your-project && rustbrain setup --yes
rustbrain context "why did we choose this architecture?"
```

| You install… | You get… |
|---|---|
| **`rustbrain`** (this crate) | The **`rustbrain` binary** — agent-friendly commands |
| [`rustbrain-core`](https://crates.io/crates/rustbrain-core) | The same engine as a **library** (embed in tools / agents) |

> **Intended primary use:** the CLI in a repo.  
> **Library path** is separate for engineers who want `Brain` / `query_ranked` / `context_for_prompt` in-process — same algorithms, no second product.

---

## What is rustbrain?

Write ordinary notes (`docs/**/*.md`, Obsidian-style WikiLinks + frontmatter). Index Rust with Tree-Sitter. Search with SQLite **FTS5**. Expand a **CSR graph** (`graph.mmap`). Pack **agent context** under a token budget.

All **offline**, **project-scoped**, **Git-friendly**. The engine does not invent ADRs, changelogs, or history.

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

**A simple analogy:** most “docs for agents” dump a folder of Markdown into the prompt and hope. rustbrain is a **filing system with an index and a map of the building** — search the catalog, walk who links to whom, then pack only the rooms that fit the token budget.

### Mental model (four jobs)

| Job | Everyday phrase | Command |
|---|---|---|
| **Orient** | “What does this repo already know?” | `context`, `query`, `graph` |
| **Capture** | “Write a real note, not chat residue.” | `note new` → edit file → `sync` |
| **Connect** | “Who links to whom? Fix pending WikiLinks.” | `graph`, `links`, `links --apply` |
| **Health** | “Is the brain usable for agents?” | `doctor`, `sync` |

Markdown on disk is the **source of truth**. `.brain/` is a **rebuildable cache** (like `target/` for knowledge).

---

## Why rustbrain?

| Benefit | What you get |
|---|---|
| **Agent-ready in one install** | `setup --yes` → `AGENTS.md` + index + doctor; then `context` / `query` every turn |
| **Truth in Git** | Notes are plain Markdown; `.brain/` is disposable — gitignore it |
| **Code ↔ docs** | `symbol:Foo` from notes; `[[docs/adr/…]]` in rustdoc → bidirectional edges |
| **Graph-aware packs** | `context` ranks seeds + hops neighbors under `-m` token budget (Markdown or XML) |
| **Rust ecosystem hubs** | Indexes root `CHANGELOG.md`; harvests **docs.rs** URLs from `Cargo.toml` on setup |
| **HITL planning** | `plan` notes with densified statuses (`backlog` → `blocked`); optional ROADMAP/BACKLOG |
| **No cloud required** | Local SQLite FTS5 + optional CSR mmap; algorithmic ranking — not neural inventiveness |
| **Safe link rewrites** | `links --apply` closes unique pending WikiLinks; optional Aho–Corasick discover (dry-run default) |

### When to use rustbrain vs plain grep / RAG

| Prefer **rustbrain** when… | Prefer something else when… |
|---|---|
| You want **structured notes** (goals, ADRs, plans, analyses) that agents must respect | You only need one-off file search in a throwaway folder |
| Agents must **not invent** decision history | You want a chat product that “remembers” without files |
| You need **code↔doc edges** and neighborhood expansion | Pure vector RAG over opaque blobs is enough |
| Docs live **in the repo** and should be reviewable in PRs | Knowledge must live only in a SaaS second-brain app |

**Rule of thumb:** if the answer should still be true after the chat ends, put it in Markdown and index it with rustbrain.

---

## Install

```bash
cargo install rustbrain --locked
# pin a release:
# cargo install rustbrain --version 0.3.22 --locked

export PATH="$HOME/.cargo/bin:$PATH"
rustbrain --version
rustbrain --help
```

| Requirement | Notes |
|---|---|
| **Rust** | MSRV **1.80+** |
| **C toolchain** | Bundled SQLite + tree-sitter need a system C compiler |
| **License** | MIT OR Apache-2.0 |

**Workspace flag:** almost every command accepts `-w /path/to/project` (default `.`).  
`query` / `context` / `doctor` / `graph` **walk parent directories** for `.brain/` (git-style).

---

## Getting started (5 minutes)

### 1. One-shot setup (recommended)

```bash
cd your-project
rustbrain setup --yes
```

What `setup --yes` does:

1. Creates `.brain/db.sqlite`
2. Scaffolds `docs/` (goals, ADRs, analysis, plans, …)
3. Writes root **`AGENTS.md`** + **`docs/AGENTS.md`** (agent protocol for this repo)
4. Harvests README → `docs/goals/from-readme.md` (if present)
5. Harvests **Cargo.toml deps → docs.rs notes** under `docs/references/crates/`
6. Optional AST module map (`docs/implementation/module-map.generated.md`)
7. **`sync`** (full index) + **`doctor`**

Common variants:

```bash
rustbrain setup --yes --no-crate-docs     # skip docs.rs harvest
rustbrain setup --yes --no-agents-md      # do not write AGENTS.md
rustbrain setup --yes --force             # overwrite generated bootstrap files
rustbrain setup --yes --no-doctor         # skip doctor at the end
rustbrain setup --yes --no-bootstrap      # init + sync only
rustbrain setup --yes --agents-template ./AGENTS.template.md
```

### 2. Capture knowledge (scaffold → edit → sync)

```bash
# Preferred: type + title only → scaffold → edit the printed path → sync
rustbrain note new --type adr --title "Use local SQLite"
# edit docs/adr/use-local-sqlite.md  (Status / Context / Decision / Consequences)

rustbrain note new --type plan --title "Q3 platform roadmap"
# status: backlog | in_progress | qa | done | cancelled | blocked

rustbrain note new --type analysis --title "query path bench 2026-07-31"
rustbrain note new --type goal --title "Ship offline agent context"

rustbrain sync
```

**Agent tip:** omit `--body` / `--note` so scaffolds appear; fill the file; then `sync`.  
Pass `--body` only when the full text is already finished (skips scaffold).

### 3. Orient (search, pack, structure)

```bash
rustbrain query "sqlite" --scores
rustbrain query "status:in_progress" --type plan --scores
rustbrain query "serde" --scores                 # docs.rs note after setup harvest

rustbrain context "why local sqlite"
rustbrain context "what shipped"
rustbrain context "roadmap priorities"
rustbrain context "why local sqlite" -m 2048 -F xml

rustbrain graph docs/adr/use-local-sqlite.md
rustbrain graph changelog                        # if CHANGELOG.md exists
rustbrain graph                                  # workspace stats

rustbrain doctor
rustbrain doctor --orphans
rustbrain doctor --strict                        # exit 1 if unhealthy / pending links
```

### 4. Everyday loop

```text
edit docs or code  →  rustbrain sync  →  query / context / graph
       │
       ├── pending WikiLinks?  →  rustbrain links
       ├── soft connect?       →  rustbrain links --auto
       ├── normalize links?    →  rustbrain links --apply --dry-run
       │                         rustbrain links --apply --write
       └── live editing?       →  rustbrain watch --debounce-ms 300
```

```bash
rustbrain sync
rustbrain links --auto
rustbrain links --apply --dry-run
rustbrain links --apply --write
rustbrain watch --debounce-ms 300
```

---

## Recommended sequences

Copy-paste playbooks. Prefer these over inventing your own flag soup.

### A. Greenfield / first day on a repo

```bash
cd repo
rustbrain setup --yes
# read AGENTS.md (and docs/AGENTS.md) — agents should follow it every turn
rustbrain context "project overview"
rustbrain doctor
```

### B. Agent turn (HITL coding session)

```bash
# 1. Orient before large refactors
rustbrain context "task keywords here"
rustbrain query "related concept" --scores
rustbrain graph docs/adr/relevant-decision.md

# 2. Do the work in the codebase…

# 3. Capture decisions / findings (scaffold, then edit)
rustbrain note new --type adr --title "Short decision title"
# edit the file — do not invent history that did not happen
rustbrain note new --type analysis --title "bench or dig YYYY-MM-DD"

# 4. Re-index
rustbrain sync
rustbrain doctor
```

### C. Release / “what shipped?”

```bash
# Prefer a real root CHANGELOG.md (Keep a Changelog style works well)
rustbrain sync
rustbrain context "what shipped"
rustbrain query "0.3" --type changelog --scores
rustbrain graph changelog
```

### D. Planning / backlog hygiene

```bash
rustbrain note new --type plan --title "Sprint board"
# edit checkboxes + status sections; optional root ROADMAP.md / BACKLOG.md hubs
rustbrain sync
rustbrain query "status:in_progress" --type plan --scores
rustbrain query "status:blocked" --type plan --scores
rustbrain context "roadmap priorities"
```

### E. Link hygiene (careful rewrites)

```bash
rustbrain links                              # list pending
rustbrain links --auto                       # soft edges only (no file rewrite)
rustbrain links --apply --dry-run            # plan unique pending normalizations
rustbrain links --apply --write              # apply AUTO tier, then sync
rustbrain links --apply --discover --dry-run # also scan unmarked mentions (AC)
rustbrain links --apply --discover --write --style wrap
# style: wrap (inline) | related (## Related list)
```

### F. Portability (hand a brain to another machine)

```bash
rustbrain export --out team.brainbundle --decouple-ast
# copy team.brainbundle …
rustbrain import --input team.brainbundle -w /other/project
rustbrain sync -w /other/project
```

### G. Multi-brain / umbrella (optional)

```bash
# Learn ids first
rustbrain scopes list
rustbrain scopes list --json

# Cargo monorepo SubBrains
rustbrain scopes enable --cargo
rustbrain scopes list                    # note the ids

# Umbrella: three former mono MainBrains under one folder
rustbrain scopes enable --empty
rustbrain scopes attach project-a --root project-a
rustbrain scopes import --from ./project-b --as project-b --mount
rustbrain scopes reconcile
rustbrain query "topic" --scope project-a
# Share without merge:
rustbrain export --out a.brainbundle --scope project-a
# Merge a SubBrain into MainBrain:
rustbrain scopes absorb project-a
```

### G. CI / agents (non-interactive only)

```bash
rustbrain setup --yes
rustbrain doctor --strict
# or stepwise:
rustbrain init
rustbrain bootstrap --yes --write
rustbrain sync
rustbrain doctor --strict --json
```

---

## Command reference

Full flag book (always authoritative for edge cases):  
**[docs/CLI.md](https://github.com/shan-alexander/rustbrain/blob/main/docs/CLI.md)** on GitHub.

### Lifecycle map

```text
setup ──► (or: init ──► bootstrap ──► sync ──► doctor)
                 │
                 ├── note new ──► sync
                 ├── query / context / graph / links / scopes
                 └── watch (optional live re-index)
```

### Command index

| Command | Purpose |
|---|---|
| **`setup`** | One-shot: init + bootstrap + sync + doctor |
| **`init`** | Create empty `.brain/db.sqlite` only |
| **`bootstrap`** | Docs tree, AGENTS, ignore, README harvest, crate docs, module map |
| **`sync`** | Index Markdown / Canvas / Rust; bake `graph.mmap` |
| **`doctor`** | Health (`--orphans`, `--json`, `--strict`; multi-brain scope checks) |
| **`note new`** | Typed scaffold (`--type` + `--title`; optional `--scope`) |
| **`query`** | Ranked FTS (`--scores`, `--type`, `--with-symbols`, `--scope`) |
| **`context`** | Agent pack (`-m`, `-F markdown\|xml`, `--scope`) |
| **`graph`** | Neighborhood ASCII/JSON or workspace stats |
| **`scopes`** | Multi-brain: **`list` (discover ids)**, enable, add, attach, import, absorb, reconcile |
| **`links`** | Pending; `--auto`; `--apply` (+ `--discover`) |
| **`watch`** | Debounced live re-index |
| **`export` / `import`** | `.brainbundle` (`export --scope ID` shares one SubBrain) |

### Discover SubBrain ids (before import / `--scope`)

| Goal | Command |
|------|---------|
| List ids **here** | `rustbrain scopes list` |
| JSON for agents | `rustbrain scopes list --json` |
| List ids elsewhere | `rustbrain scopes list -w /path/to/other` |
| After Cargo discover | `scopes enable --cargo` then `scopes list` |
| New folder / foreign mono | **Choose** id (convention = directory name) → `attach ID --root DIR` or `import --from DIR --as ID` |

```bash
rustbrain scopes list
# → SubBrain ids, roots, node counts
rustbrain query "auth" --scope project-a
rustbrain export --out a.brainbundle --scope project-a
```

After `setup`, root **AGENTS.md** includes the same command tables for agents.

---

### `setup`

```bash
rustbrain setup --yes
rustbrain setup --yes --force
rustbrain setup --yes --no-crate-docs
rustbrain setup --yes --no-agents-md
rustbrain setup --yes --agents-template ./AGENTS.template.md
rustbrain setup --yes --no-bootstrap
rustbrain setup --yes --no-doctor
```

Always non-interactive. **Preferred entry** for agents and CI.

| Flag | Meaning |
|---|---|
| `--yes` | Required non-interactive mode |
| `--force` | Overwrite generated bootstrap files / ignore / AGENTS when regenerating |
| `--no-crate-docs` | Skip Cargo.toml → docs.rs notes |
| `--no-agents-md` | Skip writing `AGENTS.md` |
| `--agents-template PATH` | Custom cookbook content |
| `--no-bootstrap` | init + sync only |
| `--no-doctor` | Skip final doctor |

---

### `init`

```bash
rustbrain init
rustbrain init /path/to/project
```

Creates `.brain/db.sqlite` and registers the workspace when possible.  
**Does not** scaffold docs or index. Prefer `setup --yes` unless you are composing a custom pipeline.

---

### `bootstrap`

Deterministic onboarding for **existing** codebases. No LLM. Never invents ADR history.

```bash
rustbrain bootstrap --yes --write          # agents / CI
rustbrain bootstrap --dry-run              # plan only
rustbrain bootstrap --write                # interactive (TTY prompts)
rustbrain bootstrap --yes --write --force
rustbrain bootstrap --yes --write --no-agents-md
rustbrain bootstrap --yes --write --agents-template ./my-agents.md
```

| Flag | Meaning |
|---|---|
| `--write` | Apply changes to disk |
| `--dry-run` | Plan only |
| `-y` / `--yes` | Non-interactive defaults |
| `--force` | Overwrite ignore, AGENTS, `generated: true` files |
| `--no-ignore` | Skip `.rustbrainignore` |
| `--import-gitignore` / `--no-import-gitignore` | Force / forbid `.gitignore` merge |
| `--no-agents-md` | Do not write root `AGENTS.md` |
| `--agents-template PATH` | Cookbook source |

**Typical writes:** `docs/**` scaffolds, `docs/goals/from-readme.md`, crate docs notes, module map, `AGENTS.md`, `docs/AGENTS.md`, `.rustbrainignore`, `.brain/` if missing, append `.brain/` to `.gitignore`.

**`AGENTS.md` template order (first match wins):**

1. `--agents-template`
2. `RUSTBRAIN_AGENTS_TEMPLATE`
3. `.rustbrain/AGENTS.template.md` or `AGENTS.template.md`
4. Built-in default

---

### `sync`

```bash
rustbrain sync
rustbrain sync /path/to/project
```

Indexes Markdown, Canvas (if present), Rust AST symbols; resolves WikiLinks / `symbol:` / rustdoc `[[…]]`; densifies plan status + changelog summaries; bakes `graph.mmap`.

Reports `file_errors=N` when individual files fail (does not abort the whole walk).  
**Run after** editing docs or code that should appear in search/context.

---

### `doctor`

```bash
rustbrain doctor
rustbrain doctor --json
rustbrain doctor --strict
rustbrain doctor --orphans
```

| Flag | Meaning |
|---|---|
| `--json` | Machine-readable report |
| `--strict` | Exit **1** if unhealthy or pending links |
| `--orphans` | Detail notes with no explicit WikiLink / `symbol:` edges |

`status: OK` means the index is usable; infos guide enrichment — they do not invent docs.

---

### `note new`

Designed for **AI agents** and humans who want consistent structure.

```bash
# Preferred: scaffold, then edit the printed path
rustbrain note new --type adr --title "Use local SQLite"
rustbrain note new --type plan --title "Q3 platform"
rustbrain note new --type analysis --title "criterion 2026-07-31"
rustbrain note new --type goal --title "Ship offline agent context"
rustbrain note new --type concept --title "FTS5"
rustbrain note new --type edge_case --title "NixOS Wayland WebKitGTK"

# Full body ready (skips scaffold)
rustbrain note new --type goal --title "X" --body "Prefer context before large refactors."

# Options
rustbrain note new --type adr --title "Y" --tags "storage,sqlite" --aliases "local-db"
rustbrain note new --type concept --title "Z" --no-sync --force
```

| Flag | Meaning |
|---|---|
| `--type` | `goal`, `adr`, `concept`, `analysis`, `plan`, `edge_case`, `reference`, … |
| `--title` | H1 + filename slug |
| `--note` / `--body` | Body after H1 (aliases; skip scaffold when set) |
| `--tags` / `--aliases` | Comma-separated |
| `--no-sync` | Do not index after write |
| `--force` | Overwrite existing file |
| `--dir` | Override default folder |

---

### `query`

Ranked FTS — not neural embeddings.

```bash
rustbrain query "authentication" --scores
rustbrain query "consensus" --type goal,adr,concept
rustbrain query "status:in_progress" --type plan --scores
rustbrain query "greet" --with-symbols
rustbrain query "raft" --all-workspaces
rustbrain query "why egui not tauri" --scores
```

| Flag | Meaning |
|---|---|
| `--scores` | Show rank score |
| `-n` / `--limit` | Max hits (default 25) |
| `--with-symbols` / `--all-types` | Include `symbol` nodes (default: notes only) |
| `--type a,b` | Only these types |
| `--all-workspaces` | Merge hits across the global registry |
| `-w` | Workspace root |

Natural-language prompts work: stopwords drop; multi-token OR for recall.  
Ranking = BM25 + title/id/tag/alias boosts + type priors + hub boosts.

---

### `context`

Build an **agent-oriented pack**: ranked seeds + optional CSR k-hop neighbors, under a token budget. Packs **body excerpts** (frontmatter stripped), not titles alone.

```bash
rustbrain context "why duckdb cli"
rustbrain context -p "overview" -F xml
rustbrain context "strict notes only" --no-hop-symbols
rustbrain context "open" --with-symbols
rustbrain context "topic" -m 2048 --hops 2
```

| Flag | Meaning |
|---|---|
| positional / `-p` / `--for-prompt` | Topic string |
| `-m` / `--max-tokens` | Soft budget (~4 chars/token) |
| `--hops` | Graph depth (`0` = seeds only) |
| `--with-symbols` | Include symbols as **seeds** (default note-first) |
| `--no-hop-symbols` | Also exclude symbols from **neighbors** |
| `--type a,b` | Seed type filter |
| `-F markdown\|xml` | Output format (default **markdown**) |

**Defaults:** note-first seeds; hops may still reach useful symbols (noise consts filtered).  
Empty/generic prompts fall back to README hub; release-ish prompts pull `changelog`; planning prompts pull `roadmap` / `backlog` when indexed.

---

### `graph`

Inspect **structure** (who links to whom). Complements `context` (which packs **content**).

```bash
rustbrain graph                              # workspace stats
rustbrain graph --json
rustbrain graph docs/concepts/raft.md
rustbrain graph docs/adr/0001-use-egui --hops 2
rustbrain graph "Raft" --no-auto
rustbrain graph symbol:StorageEngine --direction out
rustbrain graph docs/raft.md --no-symbols --type adr,concept,goal
rustbrain graph docs/raft.md --direction in
rustbrain graph docs/raft.md --stats
```

| Flag | Meaning |
|---|---|
| `TARGET` | Node id, path, unique title, or `symbol:Name` (omit = stats) |
| `--hops` | BFS depth (default `1`) |
| `--direction` | `both` (default), `out`, `in` |
| `--no-auto` | Hide soft `auto_*` edges |
| `--no-symbols` | Hide symbol neighbors |
| `--type` | Neighbor type filter |
| `--limit` | Max edges shown |
| `--json` | Machine-readable |
| `--stats` | With TARGET: stats header + neighborhood |

```text
graph: docs/concepts/raft  (concept)  "Raft"
├──[→ relates_to w=1.00] docs/concepts/logcompaction
├──[→ anchors w=1.00] symbol/…/storageengine
└──[← relates_to w=0.90] docs/adr/0001-use-raft
```

---

### `links`

Three modes: **list pending**, **soft auto-edges**, **apply rewrites**.

```bash
rustbrain links                              # pending WikiLinks / symbol: refs
rustbrain links --auto                       # soft edges (filename stem + tags)
rustbrain links --auto docs/goals/foo.md
rustbrain links --auto --json

# Apply (default = dry-run; --write required to mutate files)
rustbrain links --apply --dry-run
rustbrain links --apply --write
rustbrain links --apply --discover --dry-run
rustbrain links --apply --discover --write --style related
rustbrain links --apply --write --json --limit 50
rustbrain links --apply --write --force      # allow generated: true files
rustbrain links --apply --write --no-sync
```

| Mode | Mutates Markdown? | What it does |
|---|---|---|
| default list | No | Shows unresolved refs |
| `--auto` | No | Inserts low-weight `auto_*` edges in the DB |
| `--apply` | Only with `--write` | Closes unique pending links; optional AC discover |

**Apply flags:** `--discover`, `--style wrap|related`, `--no-graph-priors`, `--force`, `--limit N`, `--no-sync`, `--json`, optional `TARGET` filter.

**Tiers:** `AUTO` may write; `SUGGEST` report-only; `SKIP` never writes (ambiguous, generated, limit, …).  
Discover uses `.brain/link_lexicon.json` (invalidated when nodes/aliases change). Never invents notes.

---

### `watch`

```bash
rustbrain watch
rustbrain watch --debounce-ms 300
```

Debounced re-index while you edit. Requires the CLI build with watch support (default install includes it when enabled in the package).

---

### `export` / `import`

```bash
rustbrain export --out project.brainbundle
rustbrain export --out project.brainbundle --decouple-ast
rustbrain import --input project.brainbundle
rustbrain import --input project.brainbundle -w /other/project
```

Portable bundles for sharing an index snapshot. Re-`sync` after import if sources differ.

---

## Note types & hubs

| Type | Default dir | Use for |
|---|---|---|
| `goal` | `docs/goals/` | Aims / non-goals |
| `adr` | `docs/adr/` | Decisions you committed to |
| `alternative` | `docs/adr/` | Options considered |
| `analysis` | `docs/analysis/` | Dated investigations (benches, options) |
| `plan` | `docs/plans/` | Roadmaps, backlogs, tasklists (status densified on sync) |
| `changelog` | root hub / `docs/changelogs/` | Ship history (prefer root **CHANGELOG.md**) |
| `concept` | `docs/concepts/` | Timeless “what is X” |
| `edge_case` | `docs/edge_cases/` | Traps and platform quirks |
| `reference` | `docs/concepts/` or generated crates | External crates / APIs |
| `symbol` | (from AST) | Code entities |

**Root hubs (when files exist):**

| File | Node id | Type |
|---|---|---|
| `README.md` | `readme` | `goal` |
| `CHANGELOG.md` | `changelog` | `changelog` |
| `ROADMAP.md` | `roadmap` | `plan` |
| `BACKLOG.md` | `backlog` | `plan` |

### Plan status densification

After `sync`, plan notes project status into summary + FTS:

| Canonical | Surfaces (examples) |
|---|---|
| `backlog` | todo, open, pending |
| `in_progress` | wip, doing, active |
| `qa` | review, testing |
| `done` | complete, finished |
| `cancelled` | canceled, wontfix |
| `blocked` | stuck, on_hold, **undone** (legacy alias) |

Query tokens: `status:backlog`, `status:in_progress`, `status:qa`, `status:done`, `status:cancelled`, `status:blocked`.

Sources: frontmatter `status:` / `state:`, `## Status`, section headings, checkboxes (`[ ]` `[/]` `[x]` `[~]` `[?]` `[!]`).  
There is **no** separate tasks SQL table — Markdown is truth; the brain densifies for search.

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

| From → To | Edge |
|---|---|
| Note `symbol:…` → code | `anchors` |
| Rustdoc `[[note]]` → note | `doc_links` |
| Note WikiLinks | `relates_to` (and friends) |
| Soft connect | `auto_filename`, `auto_tag` |

---

## Agent protocol (HITL)

After `setup`, treat **`AGENTS.md`** / **`docs/AGENTS.md`** as the project mandate:

1. **Orient** with `rustbrain context "…"` before large refactors  
2. **Search** with `query` / `graph` instead of inventing history  
3. **Capture** with `note new` scaffolds + WikiLinks / `symbol:`  
4. **`sync`** after doc or code changes  
5. **Never invent** ADR history or changelog entries  

```bash
rustbrain setup --yes --agents-template ./AGENTS.template.md
# or commit AGENTS.template.md / .rustbrain/AGENTS.template.md
# or export RUSTBRAIN_AGENTS_TEMPLATE=/path/to/template.md
```

---

## On-disk layout

| Path | Role |
|---|---|
| `docs/**/*.md` | Human/agent-authored knowledge (source of truth) |
| `CHANGELOG.md` | Optional hub `changelog` |
| `ROADMAP.md` / `BACKLOG.md` | Optional plan hubs |
| `.rustbrainignore` | Extra index skips (optional) |
| `.brain/db.sqlite` | Derived SQLite + FTS5 + edges |
| `.brain/graph.mmap` | CSR adjacency cache |
| `.brain/link_lexicon.json` | Optional AC lexicon cache |
| `.brain/workspace.json` | Marker |
| `AGENTS.md`, `docs/AGENTS.md` | Agent cookbooks |

**Ignore dialect:** built-in `target/`, `.git/`, `.brain/`, … plus optional `.rustbrainignore`.  
Line `# rustbrain: import-gitignore` merges root `.gitignore`. Env `RUSTBRAIN_IMPORT_GITIGNORE=1` forces merge.

Formats: [SCHEMA.md](https://github.com/shan-alexander/rustbrain/blob/main/docs/SCHEMA.md) · [MMAP_FORMAT.md](https://github.com/shan-alexander/rustbrain/blob/main/docs/MMAP_FORMAT.md).

---

## Exit codes

| Code | Meaning |
|---|---|
| `0` | Success |
| `1` | Error, or `doctor --strict` with unhealthy / pending links |

---

## Library usage (optional)

Embed the **same engine** in tools and agents via **[rustbrain-core](https://crates.io/crates/rustbrain-core)**:

```toml
[dependencies]
rustbrain-core = "0.3"
```

```rust
use rustbrain_core::{Brain, ContextOptions, QueryOptions, Result};

fn main() -> Result<()> {
    let mut brain = Brain::open_or_create(".")?;
    brain.sync()?;
    let hits = brain.query_ranked("sqlite", &QueryOptions::human())?;
    let ctx = brain.context_for_prompt("why local sqlite", 1024)?;
    println!("{}", ctx.to_markdown());
    let _ = hits;
    Ok(())
}
```

Full API surface, feature flags, and more examples: the [rustbrain-core README](https://crates.io/crates/rustbrain-core) and [docs.rs/rustbrain-core](https://docs.rs/rustbrain-core).

---

## Performance

Criterion harness: `cargo bench -p bench --bench rustbrain_performance`  
Full write-up and fairness notes: **[docs/BENCHMARKS.md](https://github.com/shan-alexander/rustbrain/blob/main/docs/BENCHMARKS.md)**.

Numbers below are **approximate medians** (release, x86_64 Linux, Criterion `--quick`). Re-run on your machine.

### Search — 500 notes, query `sqlite storage`

| Approach | ~Time | Notes |
|---|---:|---|
| **rustbrain `query_ranked`** | **~3.5 ms** | FTS5 BM25 + title/tag/type boosts |
| Walk all `.md` + substring | ~7.6 ms | No index; scales with corpus |
| SQLite `LIKE` full scan | ~0.90 ms | Same bodies, **no** ranking quality |

### Graph neighborhood — 500 notes, 1 hop

| Approach | ~Time | Notes |
|---|---:|---|
| **CSR `graph.mmap` k-hop** | **~156 ns** | Topology cache used by agent context hops |
| SQL neighborhood (`graph` CLI) | ~1.5 ms | Preserves relation types |
| Re-parse all WikiLinks + BFS | ~8.9 ms | “No edge index” baseline (~**50 000×** vs CSR) |

### Context pack — 500 notes, ~1024 token budget

| Approach | ~Time | Notes |
|---|---:|---|
| **rustbrain `context`** | **~5.7 ms** | Ranked seeds + CSR hops + excerpts |
| Path-order file concat | ~2.7 ms | Faster dump, **unranked** |
| Grep-rank then concat | ~8.1 ms | No graph hops / type packing |

**Takeaway:** CSR graph hops and FTS-ranked retrieval beat “walk the docs every time” as the corpus grows. Cheaper dumps (concat, naive regex) trade away ranking and correctness — see the benchmark doc for framing.

---

## What v0.3.x claims / does not

**Does:** local Markdown second brain; bootstrap for mature repos; doctor; agent `note new`; ranked FTS with type filters; graph-aware context; first-class `changelog` + `plan` types; docs.rs harvest; portable bundles; careful `links --apply`.

**Does (0.3.22+):** MainBrain / SubBrain multi-brain (`scopes list|detect|…`, `--scope`, attach/mount).

**Does not (yet):** neural embeddings; full two-way Obsidian write-back; inventing history for you.

---

## Related resources

| Resource | Link |
|---|---|
| Full CLI book | [docs/CLI.md](https://github.com/shan-alexander/rustbrain/blob/main/docs/CLI.md) |
| Performance benches | [docs/BENCHMARKS.md](https://github.com/shan-alexander/rustbrain/blob/main/docs/BENCHMARKS.md) |
| Library crate | [rustbrain-core](https://crates.io/crates/rustbrain-core) |
| Source monorepo | [shan-alexander/rustbrain](https://github.com/shan-alexander/rustbrain) |
| API docs | [docs.rs/rustbrain-core](https://docs.rs/rustbrain-core) |

---

## License

MIT OR Apache-2.0
