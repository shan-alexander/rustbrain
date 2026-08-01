# rustbrain CLI reference (v0.3.x)

Package: **`rustbrain`** on crates.io · binary: `rustbrain`

```bash
cargo install rustbrain --locked
# or pin: cargo install rustbrain --version 0.3.22 --locked
# ensure: export PATH="$HOME/.cargo/bin:$PATH"
```

All commands accept a workspace path (default `.`). Prefer `-w /path` when the CWD is not the project root.
`query` / `context` / `doctor` / `graph` **walk parent directories** for `.brain/db.sqlite` (git-style).

---

## MainBrain / SubBrain scopes (opt-in multi-crate)

**Default is single-brain** (everything is MainBrain `main`). Multi-crate repos may stay single or enable multi-brain.

### Discovering SubBrain ids

Agents/humans must know the **id** before `import --as`, `query --scope`, or `export --scope`.

| Goal | Command |
|------|---------|
| List ids **here** | `rustbrain scopes list` |
| JSON | `rustbrain scopes list --json` → `manifest.scopes[].id` |
| Suggest id for a path | `rustbrain scopes detect PATH` (before import/attach) |
| List ids **elsewhere** | `rustbrain scopes list -w /path/to/other` |
| After Cargo discover | `scopes enable --cargo` then `scopes list` |
| Brand-new folder / foreign mono | **Choose** id (convention = directory name), then `attach ID --root DIR` or `import --from DIR --as ID` |
| Auto-scope from CWD | `query`/`context`/`graph` when CWD is under a SubBrain (override with `--scope` / `--no-scope-auto`) |

```bash
rustbrain scopes list
rustbrain scopes enable --cargo          # multi + Cargo workspace members as SubBrains
rustbrain scopes enable --empty          # multi, add SubBrains yourself
rustbrain scopes add myapp --root apps/myapp
rustbrain scopes attach legacy --root project-a   # umbrella: existing dir, no copy
rustbrain scopes remove myapp --absorb   # reassign nodes → main, drop SubBrain
rustbrain scopes absorb myapp
rustbrain scopes absorb all              # all SubBrains → main, mode=single
rustbrain scopes reconcile               # fix DB drift after layout changes
rustbrain scopes disable --absorb-all

# Share without merge (SubBrain stays separate) vs merge into MainBrain
rustbrain scopes import --from ../other --as other-lib          # copy under docs/subbrains/
rustbrain scopes import --from ./project-a --as alpha --mount   # path under umbrella
rustbrain scopes import --from ../other --into main             # merge copy into MainBrain
rustbrain export --out share.brainbundle --scope other-lib      # portable SubBrain slice

# Filter retrieval (default: SubBrain + hub nodes only)
rustbrain query "topic" --scope rustbrain-cli
rustbrain query "topic" --scope rustbrain-core --scope-strict
rustbrain query "topic" --scope rustbrain-core --scope-with-main
rustbrain context "task" --scope rustbrain-core
rustbrain note new --type adr --title "Crate decision" --scope rustbrain-cli
```

| Concept | Behavior |
|---------|----------|
| **single** | No `--scope` needed; all nodes `scope=main` |
| **multi** | Flat sibling SubBrains; longest path root wins |
| **`--scope`** | SubBrain + **hubs only** (`readme`/`changelog`/`roadmap`/`backlog`) by default |
| **`--scope-strict`** | SubBrain only |
| **`--scope-with-main`** | SubBrain + all MainBrain-owned nodes |
| **mount / attach** | Former mono MainBrains become SubBrains under a new umbrella MainBrain |
| **One graph** | Cross-scope WikiLinks/symbols still one CSR/SQLite graph |

Design: root `ROADMAP.md` § MainBrain + SubBrain · schema v2 `nodes.scope`.

---

## Lifecycle

```text
setup ──► (or: init ──► bootstrap ──► sync ──► doctor)
                              │
                              ├── note new ──► sync (default)
                              ├── query / context / graph / links
                              └── watch (optional)
```

| Stage | Command | Notes |
|-------|---------|--------|
| One-shot | `setup --yes` | init + bootstrap + sync + doctor (+ `AGENTS.md`) |
| Empty store | `init` | Creates `.brain/db.sqlite` only |
| Mature repo seed | `bootstrap` | Docs tree + ignore + README/AST + **AGENTS.md** |
| Compile notes → graph | `sync` | FTS + edges + `graph.mmap` (includes root `CHANGELOG.md` → hub `changelog`) |
| Inspect structure | `graph` | Neighborhood tree / stats (ASCII or JSON) |
| Health | `doctor` | Pending links, ratios, exit codes |

---

## `setup`

```bash
rustbrain setup --yes
rustbrain setup --yes --force          # overwrite generated bootstrap files
rustbrain setup --yes --no-doctor
rustbrain setup --yes --no-bootstrap   # init + sync only
rustbrain setup --yes --no-agents-md   # skip AGENTS.md
rustbrain setup --yes --agents-template ./AGENTS.template.md
```

Always non-interactive. Preferred entry point for agents and CI.

**Writes by default:** docs scaffold, `.rustbrainignore`, README harvest, module map,
**Cargo.toml → docs.rs notes** under `docs/references/`, root **`AGENTS.md`** + `docs/AGENTS.md`,
then full sync + doctor.

```bash
rustbrain setup --yes --no-crate-docs   # skip docs.rs harvest
```

After bootstrap, agents can:

```bash
rustbrain query "serde" --scores
# → docs/references/crates/serde.md with https://docs.rs/serde/…
```

---

## `init`

```bash
rustbrain init
rustbrain init /path/to/project
```

Creates `.brain/db.sqlite` (schema v1) and registers the workspace in the global registry when possible.

**Does not** create docs, `AGENTS.md`, or index anything. Prefer:

```bash
rustbrain setup --yes
```

---

## `bootstrap`

Deterministic onboarding for **existing** codebases. No LLM. Never invents ADR history.

```bash
# Agents / CI (no prompts)
rustbrain bootstrap --yes --write

# Dry-run plan
rustbrain bootstrap --dry-run

# Humans: interactive prompts (TTY)
rustbrain bootstrap --write

# Overwrite generated files / ignore / AGENTS.md
rustbrain bootstrap --yes --write --force

# Skip or customize agent cookbook
rustbrain bootstrap --yes --write --no-agents-md
rustbrain bootstrap --yes --write --agents-template ./my-agents.md
```

### Flags

| Flag | Meaning |
|------|---------|
| `--write` | Apply changes to disk |
| `--dry-run` | Plan only (implies no write) |
| `-y` / `--yes` | Non-interactive defaults (for agents) |
| `--force` | Overwrite `.rustbrainignore`, `AGENTS.md`, and regenerate `generated: true` files |
| `--no-ignore` | Skip `.rustbrainignore` setup |
| `--import-gitignore` | Force import root `.gitignore` into ignore file |
| `--no-import-gitignore` | Never import `.gitignore` |
| `--no-agents-md` | Do not write root `AGENTS.md` |
| `--agents-template PATH` | Use this file as `AGENTS.md` content |

### What it writes

| Path | Purpose |
|------|---------|
| `docs/goals/`, `docs/adr/`, `docs/concepts/`, `docs/analysis/`, `docs/edge_cases/`, `docs/implementation/`, `docs/experience/` | Directory scaffold |
| `docs/adr/TEMPLATE.md` | ADR template (human fills real ADRs) |
| `docs/goals/README.md` | Goals index stub |
| `docs/BOOTSTRAP_CHECKLIST.md` | Promote drafts → real knowledge |
| `docs/goals/from-readme.md` | Harvested from root `README.md` (`generated: true`) |
| `docs/implementation/module-map.generated.md` | AST symbol list with `symbol:…` refs (`generated: true`) |
| **`AGENTS.md`** | Agent cookbook for this repo (customizable; see below) |
| `.rustbrainignore` | Extra index skips (optional import of `.gitignore`) |
| `.gitignore` | Appends `.brain/` when missing |
| `.brain/db.sqlite` | Created if missing |

### Customizing `AGENTS.md`

Template resolution (**first match wins**):

1. `--agents-template PATH` (CLI) or `BootstrapOptions::agents_template` (library)
2. Environment: `RUSTBRAIN_AGENTS_TEMPLATE=/path/to/template.md`
3. Workspace file: `.rustbrain/AGENTS.template.md` **or** `AGENTS.template.md`
4. Built-in default (`default_agents_md_template()` in `rustbrain-core`)

```bash
# Use a committed org-wide template
rustbrain bootstrap --yes --write --agents-template ./AGENTS.template.md

# Or env for CI
export RUSTBRAIN_AGENTS_TEMPLATE=$HOME/templates/rustbrain-AGENTS.md
rustbrain setup --yes
```

- **Skip:** `--no-agents-md` on `bootstrap` or `setup`.
- **Overwrite existing:** `--force` (by default an existing `AGENTS.md` is left alone).
- **Edit freely** after write; the built-in header documents how to regenerate.

### Interactive prompts (TTY, without `--yes`)

1. Create/update `.rustbrainignore`?
2. Import root `.gitignore`?
3. Append recommended extras (`target/`, `data/`, `*.parquet`, `.env`, …)?
4. Extra comma-separated patterns?
5. Harvest README?
6. Generate AST module map?
7. Scaffold docs/ tree + templates?
8. Write root `AGENTS.md`?
9. Custom `AGENTS.md` template path? (empty = discovery / built-in)
10. Write files to disk?

---

## `sync`

```bash
rustbrain sync
rustbrain sync /path/to/project
```

Indexes Markdown, Canvas (if present), Rust AST symbols; resolves WikiLinks / `symbol:` refs; bakes `graph.mmap`.

Also indexes **rustdoc WikiLinks**: `/// See [[docs/adr/…]]` creates `doc_links` edges
from the symbol node to the note (bidirectional graph with note-side `symbol:…` anchors).

Reports `file_errors=N` when individual files fail (does not abort the whole walk).

---

## `doctor`

```bash
rustbrain doctor
rustbrain doctor --json
rustbrain doctor --strict     # exit 1 if unhealthy or pending links
rustbrain doctor --orphans    # detailed orphan analysis (alias: --orphan)
```

Walks parents for `.brain`. Summary line includes `orphans=N` when **N > 0**
(notes with no **explicit** WikiLink/`symbol:` edges; soft `auto_*` links do not count).

| Code | Meaning |
|------|---------|
| `orphan_notes` | Count of orphans — see `doctor --orphans` or `links --auto` |
| `no_readme` / `sparse_readme` | Missing or thin root README (harvest will be empty/thin) |
| `thin_from_readme` / `no_from_readme` | Harvest quality / presence |
| `scaffold_only` | Only bootstrap stubs — write real notes for better context |
| `knowledge_thin` | Few substantial notes vs many symbols |
| `no_agents_md` | No root agent cookbook |

`status: OK` still means the DB/index is usable; infos guide enrichment, they do not invent docs.

### Soft auto-links

```bash
rustbrain links --auto                      # all notes: filename stem + shared tags
rustbrain link --auto                       # synonym of links
rustbrain links --auto docs/goals/foo.md    # one note
rustbrain links --auto --json
```

Creates low-weight edges (`auto_filename` ~0.4, `auto_tag` ~0.25). Same basename under
different folders (e.g. `goals/rust-fluency.md` and `concepts/rust-fluency.md`) is linked.
Re-run rebuilds auto edges. Explicit Markdown links remain preferred for hops.

### `links --apply` (rewrite Markdown carefully)

Closes **pending** WikiLinks when the target now uniquely exists, and optionally **discovers**
unmarked entity mentions (Aho–Corasick over a closed-world lexicon). Never invents notes.

```bash
# Phase 0 — plan only (default)
rustbrain links --apply --dry-run
rustbrain links --apply                 # same: no --write ⇒ dry-run

# Phase 0 — write unique pending normalizations, then sync
rustbrain links --apply --write

# Phase 1 — also plan unmarked title/alias/symbol mentions
rustbrain links --apply --discover --dry-run
rustbrain links --apply --discover --write
rustbrain links --apply --discover --write --style related   # ## Related section
rustbrain links --apply --discover --write --no-graph-priors

# Focus one source note; JSON for agents
rustbrain links --apply --dry-run docs/concepts/raft.md
rustbrain links --apply --write --json
rustbrain links --apply --write --force   # allow generated: true files
rustbrain links --apply --write --limit 50
rustbrain links --apply --write --no-sync
```

| Flag | Meaning |
|------|---------|
| `--apply` | Enter apply mode (mutually exclusive with `--auto`) |
| `--write` | Required to mutate files |
| `--dry-run` | Plan only (default without `--write`) |
| `--discover` | AC scan for unmarked mentions (suggest + strong auto) |
| `--style` | `wrap` (default, inline) or `related` (`## Related` list) |
| `--no-graph-priors` | Disable 1-hop neighbor boost for discover scoring |
| `--force` | Rewrite generated bootstrap files |
| `--limit N` | Cap auto-tier edits (default 200) |
| `--no-sync` | Skip automatic sync after write |
| `TARGET` | Optional source path/id filter |
| `--json` | Full `ApplyReport` |

Discover uses a **LinkLexicon** cache at `.brain/link_lexicon.json` (invalidated when nodes/aliases change).

**Tiers:** `AUTO` may write; `SUGGEST` is report-only; `SKIP` never writes (ambiguous, unresolved,
generated, missing file, limit). Edits are atomic (temp + rename) with UTF-8-safe spans.

---

## `graph`

Inspect the **structure** of the knowledge graph (who links to whom). Complements
`context` (which packs **content** under a token budget).

```bash
# Workspace stats: counts by type/relation + high-degree hubs
rustbrain graph
rustbrain graph --json

# Neighborhood of a note (path, id, or exact title)
rustbrain graph docs/concepts/raft.md
rustbrain graph docs/adr/0001-use-egui --hops 2
rustbrain graph "Raft" --no-auto
rustbrain graph symbol:StorageEngine --direction out

# Filters
rustbrain graph docs/raft.md --no-symbols --type adr,concept,goal
rustbrain graph docs/raft.md --direction in   # only reverse edges
rustbrain graph docs/raft.md --json           # agents/tools
rustbrain graph docs/raft.md --stats          # stats header + neighborhood
```

| Flag | Meaning |
|------|---------|
| `TARGET` | Node id, path, unique title, or `symbol:Name` (omit for stats) |
| `--hops` | BFS depth (default `1`) |
| `--direction` | `both` (default), `out`, or `in` |
| `--no-auto` | Hide soft `auto_*` edges |
| `--no-symbols` | Hide symbol neighbors |
| `--type` | Neighbor type filter (comma-separated) |
| `--limit` | Max edges shown (default 200) |
| `--json` | Machine-readable report |
| `--stats` | With TARGET: print stats above the tree |
| `-w` | Workspace root |

**ASCII sample:**

```text
graph: docs/concepts/raft  (concept)  "Raft"
  path: docs/concepts/raft.md
  hops=1  edges_shown=3  nodes_in_subgraph=4  db=120/95 n/e
├──[→ relates_to w=1.00] docs/concepts/logcompaction  (concept)  "Log Compaction"
├──[→ anchors w=1.00] symbol/demo/lib/storageengine  (symbol)  "StorageEngine"
└──[← relates_to w=0.90] docs/adr/0001-use-raft  (adr)  "Use Raft"
```

Edges come from SQLite (relation types preserved). Run `sync` after note/code edits.

---

## `note new`

Write a structured Markdown note without opening the editor. Designed for **AI agents**.

### Preferred workflow (scaffold, then edit)

**Better agentic outcomes:** create with **type + title only** (no `--body` / `--note`) so
rustbrain writes a **type-specific boilerplate**, then **edit the file** that was created,
then `sync` if needed.

```bash
rustbrain note new --type adr --title "Use local SQLite"
# → docs/adr/use-local-sqlite.md with Status / Context / Decision / Consequences
# edit the file, then:
rustbrain sync
```

Passing `--body` or `--note` fills the body immediately and **skips** the scaffold — use that
when the full text is already finished.

| Flag | Meaning |
|------|---------|
| `--type` | `goal`, `adr`, `concept`, `analysis`, `edge_case`, … |
| `--title` | Becomes the Markdown **H1** + filename slug |
| `--note` / **`--body`** | Body text **after** the H1 (aliases of each other) |
| `--tags` / `--aliases` | Comma-separated |
| `--no-sync` | Do not index after write |
| `--force` | Overwrite existing file |

| Type | Folder | Use for |
|------|--------|---------|
| `concept` | `docs/concepts/` | Timeless “what is X” |
| `analysis` | `docs/analysis/` | Dated investigation (crate compare, design dig, **bench/criterion review**, data digest, …). Recs optional; not a decision |
| `adr` | `docs/adr/` | We chose X (often after one or more analyses) |
| `edge_case` | `docs/edge_cases/` | A specific trap (often *surfaced by* an analysis) |
| `goal` | `docs/goals/` | Aims / non-goals |

Example goal (also shown after `sync` / `doctor` / `--help`):

```bash
rustbrain note new --type goal --title "Use rustbrain well" \
  --body "Prefer rustbrain context/query before large refactors. Capture decisions with note new --type adr. Run sync after doc/code changes. Keep docs truthful — do not invent ADR history."
```

Example analysis (empty body → scaffold sections; or pass `--body`):

```bash
rustbrain note new --type analysis --title "criterion query-path 2026-07-31" \
  --body "Compared main vs branch. p50 -12% cold cache. Artifacts: target/criterion/…. Recommendation: merge patch; not an ADR until we accept the tradeoff."

# Filter later:
rustbrain query "criterion" --type analysis --scores
```

---

## `query`

```bash
rustbrain query "authentication" --scores
rustbrain query "duckdb"
rustbrain query "consensus" --type goal,adr,concept
rustbrain query "greet" --with-symbols
rustbrain query "raft" --all-workspaces
```

| Flag | Description |
|------|-------------|
| `--scores` | Show rank score |
| `-n` / `--limit` | Max hits (default 25) |
| `--with-symbols` / `--all-types` | Include `symbol` nodes (default is notes only) |
| `--type a,b` | Only these types |
| `--all-workspaces` | Merge ranked hits across registry |
| `-w` | Workspace |

Ranking = FTS5 BM25 + stopword strip + multi-token OR MATCH + title/id/tag/alias/coverage boosts + type priors + README hub boost. Not neural search.

Natural-language prompts work: `why egui not tauri` becomes FTS `"egui" OR "tauri"` after dropping stopwords.

---

## `context`

Build an agent-oriented pack: ranked seeds + optional CSR k-hop neighbors, under a token budget.
Packs **body excerpts** (from FTS content, frontmatter stripped), not titles alone.

```bash
rustbrain context "why duckdb cli"
rustbrain context -p "overview" -F xml
rustbrain context "strict notes only" --no-hop-symbols
rustbrain context "open" --with-symbols     # include symbols as seeds
```

| Flag | Description |
|------|-------------|
| positional / `-p` / `--for-prompt` | Topic string |
| `-m` / `--max-tokens` | Soft budget (~4 chars/token) |
| `--hops` | Graph depth (`0` = seeds only) |
| `--with-symbols` / `--all-types` | Include symbols as **seeds** (default is note-first) |
| `--no-hop-symbols` | Also exclude symbols from **neighbors** (default allows ADR → code hops) |
| `--type a,b` | Seed type filter |
| `-F markdown\|xml` | Output format (default **markdown**; XML entity-escaped) |

**Defaults:** note-first seeds; hops expand to useful symbols (noise consts filtered). Empty packs print a short recovery hint. Generic overview prompts fall back to README hub.

---

## `links` / `watch` / `export` / `import`

```bash
rustbrain links
rustbrain watch --debounce-ms 300
rustbrain export --out project.brainbundle
rustbrain import --input project.brainbundle
```

See `rustbrain <cmd> --help` for full flags.

---

## Ignore files

- Built-in skips: `target/`, `.git/`, `.brain/`, `node_modules/`, …
- Optional **`.rustbrainignore`** (gitignore-inspired dialect)
- Directive `# rustbrain: import-gitignore` merges root `.gitignore` patterns
- Env `RUSTBRAIN_IMPORT_GITIGNORE=1` forces import at index time

---

## Agent tip

After `setup --yes`, point coding agents at the generated **`AGENTS.md`** in the project
root — it encodes the rustbrain loop for that repo. Customize the template org-wide via
`AGENTS.template.md` so every new bootstrap is on-brand.
