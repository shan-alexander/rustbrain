# rustbrain CLI reference (v0.3.2)

Package: **`rustbrain`** on crates.io · binary: `rustbrain`

```bash
cargo install rustbrain --locked
# or pin: cargo install rustbrain --version 0.3.2 --locked
# ensure: export PATH="$HOME/.cargo/bin:$PATH"
```

All commands accept a workspace path (default `.`). Prefer `-w /path` when the CWD is not the project root.
`query` / `context` / `doctor` **walk parent directories** for `.brain/db.sqlite` (git-style).

---

## Lifecycle

```text
setup ──► (or: init ──► bootstrap ──► sync ──► doctor)
                              │
                              ├── note new ──► sync
                              ├── query / context / links
                              └── watch (optional)
```

| Stage | Command | Notes |
|-------|---------|--------|
| One-shot | `setup --yes` | init + bootstrap + sync + doctor |
| Empty store | `init` | Creates `.brain/db.sqlite` only |
| Mature repo seed | `bootstrap` | Docs tree + ignore + README/AST harvest |
| Compile notes → graph | `sync` | FTS + edges + `graph.mmap` |
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

Always non-interactive. Preferred entry point for agents and CI. Writes `AGENTS.md` by default.

---

## `init`

```bash
rustbrain init
rustbrain init /path/to/project
```

Creates `.brain/db.sqlite` (schema v1) and registers the workspace in the global registry when possible.

**Does not** create docs or index anything. Prefer:

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

# Overwrite generated files / ignore file
rustbrain bootstrap --yes --write --force
```

### Flags

| Flag | Meaning |
|------|---------|
| `--write` | Apply changes to disk |
| `--dry-run` | Plan only (implies no write) |
| `-y` / `--yes` | Non-interactive defaults (for agents) |
| `--force` | Overwrite `.rustbrainignore` and regenerate `generated: true` files |
| `--no-ignore` | Skip `.rustbrainignore` setup |
| `--import-gitignore` | Force import root `.gitignore` into ignore file |
| `--no-import-gitignore` | Never import `.gitignore` |
| `--no-agents-md` | Do not write root `AGENTS.md` |
| `--agents-template PATH` | Use this file as `AGENTS.md` content |

### What it writes

| Path | Purpose |
|------|---------|
| `docs/goals/`, `docs/adr/`, `docs/concepts/`, `docs/edge_cases/`, `docs/implementation/`, `docs/experience/` | Directory scaffold |
| `docs/adr/TEMPLATE.md` | ADR template (human fills real ADRs) |
| `docs/goals/README.md` | Goals index stub |
| `docs/BOOTSTRAP_CHECKLIST.md` | Promote drafts → real knowledge |
| `docs/goals/from-readme.md` | Harvested from root `README.md` (`generated: true`) |
| `docs/implementation/module-map.generated.md` | AST symbol list with `symbol:…` refs (`generated: true`) |
| `AGENTS.md` | Agent cookbook for this repo (customizable; see below) |
| `.rustbrainignore` | Extra index skips (optional import of `.gitignore`) |
| `.brain/db.sqlite` | Created if missing |

### Customizing `AGENTS.md`

1. `rustbrain bootstrap --agents-template ./my-agents.md`
2. `export RUSTBRAIN_AGENTS_TEMPLATE=/path/to/template.md`
3. Commit `AGENTS.template.md` or `.rustbrain/AGENTS.template.md` in the repo
4. Built-in default from rustbrain (agent loop + conventions)

Skip with `--no-agents-md`. Re-write with `--force` if `AGENTS.md` already exists.

### Interactive prompts (TTY, without `--yes`)

1. Create/update `.rustbrainignore`?
2. Import root `.gitignore`?
3. Append recommended extras (`target/`, `data/`, `*.parquet`, `.env`, …)?
4. Extra comma-separated patterns?
5. Harvest README?
6. Generate AST module map?
7. Scaffold docs tree?
8. Write to disk?

### Nuances

- Files marked `generated: true` in frontmatter are refreshed on re-bootstrap even without `--force`.
- Human-edited files without that marker are **skipped** unless `--force`.
- Bootstrap does **not** run `sync`. Always `rustbrain sync` after.
- ADRs are **not** auto-authored — only a template + checklist (avoids fictional history).

---

## `sync`

```bash
rustbrain sync
rustbrain sync /path/to/project
```

Walks the workspace (respecting ignore rules), indexes Markdown / Canvas / Rust, resolves pending links, bakes `.brain/graph.mmap`.

Example output:

```text
sync complete: md=6 canvas=0 rs=7 nodes_upserted=97 skipped=0 edges=91 pending=0 symbols=91 mmap=true file_errors=0
```

| Field | Meaning |
|-------|---------|
| `md` / `rs` / `canvas` | Files processed this run |
| `nodes_upserted` | Inserts/updates |
| `skipped` | Unchanged (`content_hash` match) |
| `pending` | Unresolved WikiLinks / `symbol:` targets remaining |
| `file_errors` | Per-file failures skipped (sync continues) |

### Ignore rules

Always skipped (built-in): `target/`, `.git/`, `.brain/`, `node_modules/`, …

Plus patterns from **`.rustbrainignore`**. If that file contains:

```text
# rustbrain: import-gitignore
```

…or you set `RUSTBRAIN_IMPORT_GITIGNORE=1`, root `.gitignore` is merged at load time.

### README hub

Root `README.md` is indexed as node id **`readme`**, default type **`goal`**, with aliases `hub` / `home` / crate folder name, and a ranking boost.

---

## `doctor`

```bash
rustbrain doctor
rustbrain doctor --json
rustbrain doctor --strict    # exit 1 if unhealthy or pending_links > 0
```

Reports:

- db / mmap presence, schema version  
- node / edge / FTS / pending / symbol counts  
- breakdown by `node_type`  
- findings (`empty_brain`, `pending_links`, `symbol_flood`, `no_ignore`, …)  
- pending link samples (`source -[rel]-> target`)

**Exit codes:** `0` healthy (and, without `--strict`, pending is only a WARN). `--strict` fails on any pending links.

---

## `note new`

Write a structured Markdown note without opening the editor. Designed for **AI agents**.

```bash
rustbrain note new \
  --type adr \
  --title "Use DuckDB CLI" \
  --note "Spawn duckdb -json; no libduckdb link." \
  --tags "duckdb,architecture" \
  --aliases "ADR-duckdb" \
  --sync
```

| Flag | Required | Description |
|------|----------|-------------|
| `--type` | yes | `goal` \| `adr` \| `alternative` \| `concept` \| `reference` \| `edge_case` |
| `--title` | yes | H1 + filename slug |
| `--note` | no | Body after the title (agent payload) |
| `--tags` | no | Comma-separated |
| `--aliases` | no | Comma-separated |
| `--dir` | no | Override path (default: `docs/goals`, `docs/adr`, …) |
| `--force` | no | Overwrite existing file |
| `--sync` | no | Run `sync` immediately after write |
| `-w` | no | Workspace root |

**Default directories:**

| Type | Directory |
|------|-----------|
| `goal` | `docs/goals/` |
| `adr` / `alternative` | `docs/adr/` |
| `concept` / `reference` / `symbol` | `docs/concepts/` |
| `edge_case` | `docs/edge_cases/` |

Without `--note`, ADR/goal templates include stub sections (Status/Context/Decision, Goals/Non-goals).

Prints the path and the **node id after sync** (path slug).

---

## `links`

```bash
rustbrain links
rustbrain links --json
```

Lists unresolved WikiLink / `symbol:` targets from `pending_links`. Use after sync when `pending>0`.

---

## `query`

```bash
rustbrain query "authentication" --scores
rustbrain query "duckdb" --no-symbols -n 20
rustbrain query "consensus" --type goal,adr,concept
rustbrain query "greet" --all-types          # include symbols
rustbrain query "raft" --all-workspaces
```

| Flag | Description |
|------|-------------|
| `--scores` | Show rank score |
| `-n` / `--limit` | Max hits (default 25) |
| `--no-symbols` | Exclude `symbol` nodes (human-friendly) |
| `--type a,b` | Only these types |
| `--all-types` | Clear type filters / include symbols |
| `--all-workspaces` | Merge ranked hits across registry |
| `-w` | Workspace |

Ranking = FTS5 BM25 + stopword strip + multi-token OR MATCH + title/id/tag/alias/coverage boosts + type priors + README hub boost. Not neural search.

Natural-language prompts work: `why egui not tauri` becomes FTS `"egui" OR "tauri"` after dropping stopwords.

---

## `context`

Build an agent-oriented pack: ranked seeds + optional CSR k-hop neighbors, under a token budget.
Packs **body excerpts** (from FTS content), not titles alone.

```bash
rustbrain context "why duckdb cli" -F markdown --hops 1 -m 1200
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

**Defaults (v0.3):** note-first seeds; hops still expand to useful symbols (noise consts filtered). Empty packs print a short recovery hint.

---

## `watch` / `export` / `import`

```bash
rustbrain watch --debounce-ms 300
rustbrain export --out ./brain.brainbundle --decouple-ast
rustbrain import --input ./brain.brainbundle
```

`export --decouple-ast` strips symbol nodes and file paths for portable concept transfer.

---

## Exit codes

| Code | Meaning |
|------|---------|
| `0` | Success (`doctor` healthy, or non-strict with warnings only) |
| `1` | Error, or `doctor --strict` with pending/unhealthy |

---

## Global registry

Workspaces are registered under the user config dir (e.g. `~/.config/rustbrain/registry.json`) on `init`/`sync` for `--all-workspaces` queries.
