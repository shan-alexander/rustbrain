---
node_type: plan
status: in_progress
tags: [roadmap, hub]
aliases: [roadmap, milestones, future, product-plan]
---
# Roadmap

Project planning hub for **rustbrain**. Indexed as stable node id **`roadmap`**.

## Status

in_progress

## Where the full plan lives

**Canonical plan note (checkboxes + epics + design notes):**

→ **[[docs/plans/product-roadmap]]**

Edit that file for backlog / in_progress / blocked / done. Then:

```bash
rustbrain sync
rustbrain query "status:in_progress" --type plan --scores
rustbrain context "roadmap priorities"
```

## Priority (summary)

1. **0.3.x polish** — tests, doctor, docs honesty, link apply UX, monorepo ergonomics  
2. **MainBrain + SubBrain (`--scope`)** — multi-crate / multi-root workspaces (detailed below)  
3. **Obsidian write-back (narrow)** — frontmatter-safe + vault path; Canvas later  
4. **Neural embeddings** — hybrid FTS + vectors; offline pluggable backend; mmap `D > 0` only when real  

## Does not claim (yet)

- Neural embeddings / hybrid search in the product path  
- Full two-way Obsidian vault write-back  
- Multi-brain `--scope` SubBrain CLI  
- Production ANN or AVX-512 as a marketed feature  

Ship history belongs in **[[changelog]]**, not here.

---

## MainBrain + SubBrain — multi-root / multi-crate workspaces

**Status:** in_progress (core shipped + hardening; umbrella mount/attach supported)

**Problem today:** one workspace root → one `.brain/`. A Cargo workspace like:

```text
rustbrain/                 ← git root, workspace Cargo.toml
  crates/rustbrain-core/
  crates/rustbrain-cli/
  crates/bench/
  docs/
```

…indexes fine as a single bag of notes + symbols. Agents working only on `rustbrain-cli` still get flooded by `core` symbols and unrelated crate docs. Larger monorepos (many crates, apps, services) make **unscoped** `context` / `query` noisy.

**Goal:** keep **one Git project = one MainBrain index** (one `.brain/db.sqlite`, one graph), but attach every node a **flat owner scope** so agents can focus without splitting into many disconnected databases.

### Topology

```text
                         MainBrain  (workspace root)
                         id: main
              goals, ADRs, ROADMAP, CHANGELOG, cross-cutting docs/
                                  │
         ┌────────────────────────┼────────────────────────┐
         ▼                        ▼                        ▼
   SubBrain                   SubBrain                  SubBrain
   id: rustbrain-core         id: rustbrain-cli         id: bench
   crates/rustbrain-core/**   crates/rustbrain-cli/**   crates/bench/**
```

| Concept | Meaning |
|---------|---------|
| **MainBrain** | Workspace root scope (`main`). Owns root hubs (`readme`, `changelog`, `roadmap`, …), root `docs/**` by default, and cross-crate ADRs/goals. |
| **SubBrain** | Flat sibling scope under MainBrain — **not** nested SubBrains of SubBrains. One id per logical package/root. |
| **Owner scope** | Every node has exactly one owner (`main` or a subbrain id). |
| **Cross-scope edges** | WikiLinks / `symbol:` / rustdoc links may span scopes; graph stays workspace-global. |
| **Not this feature** | `--all-workspaces` = different **projects** on the machine (global registry). SubBrain = scopes **inside one project**. |

### Multi-crate (Cargo / polyglot monorepo)

**Default discovery (algorithmic, no LLM):**

1. If root `Cargo.toml` has `[workspace].members`, each member path becomes a candidate SubBrain.  
2. Scope **id** = package `name` from that member’s `Cargo.toml` when available; else last path segment (`rustbrain-core`).  
3. Index roots for that scope: member dir tree (`src/**`, crate `README.md`, crate-local `docs/**`).  
4. Root workspace `docs/**`, root `README` / `CHANGELOG` / `ROADMAP` / `BACKLOG` → **MainBrain**.  
5. Symbols from `crates/foo/src/**` → SubBrain `foo` (package name).  
6. Optional `[workspace.metadata.rustbrain.subbrains]` (or `.brain/workspace.json` scopes) to **override** ids, exclude members, or add non-Cargo roots.

**Example mapping for this repo:**

| Path prefix | Owner scope |
|-------------|-------------|
| `docs/**`, `README.md`, `CHANGELOG.md`, `ROADMAP.md`, `AGENTS.md` | `main` |
| `crates/rustbrain-core/**` | `rustbrain-core` |
| `crates/rustbrain-cli/**` | `rustbrain` (package name of the CLI crate) or `rustbrain-cli` (path-stable id — pick one rule and document it) |
| `crates/bench/**` | `bench` |

**Id rule (recommended):** prefer **path-stable** ids (`rustbrain-core`, `rustbrain-cli`, `bench`) so renaming the Cargo package does not thrash graph ids; store Cargo package name as an alias/tag when different.

### Multi-root workspaces (beyond Cargo)

Same MainBrain, additional roots that are not workspace members:

```text
repo/
  apps/web/
  apps/api/
  packages/ui/
  services/ingest/
  docs/                 → main
```

| Mechanism | Use |
|-----------|-----|
| Auto | Detect `*/docs/` one level down; or directories with their own `README.md` + source tree |
| Explicit | `.brain/workspace.json` → `scopes: [{ "id": "web", "roots": ["apps/web"] }, …]` |
| Multi-VCS roots | Optional later: additional path roots still under one MainBrain (not separate registry entries) |

**Rules:**

- Flat sibling scopes only (no `web/admin` nested brain id; deep folders stay path prefixes under one id).  
- Overlapping roots are a **doctor error** (ambiguous owner).  
- Longest-prefix match assigns owner scope.  
- Unmatched paths → `main` or `unscoped` policy (prefer assign to `main` with doctor info, not silent drop).

### Manifest shape (target)

`.brain/workspace.json` grows from a tiny marker to a versioned scope map (illustrative):

```json
{
  "version": 2,
  "workspace": "/abs/path/to/repo",
  "main": { "id": "main" },
  "scopes": [
    {
      "id": "rustbrain-core",
      "roots": ["crates/rustbrain-core"],
      "aliases": ["rustbrain_core"],
      "source": "cargo-workspace"
    },
    {
      "id": "rustbrain-cli",
      "roots": ["crates/rustbrain-cli"],
      "aliases": ["rustbrain"],
      "source": "cargo-workspace"
    },
    {
      "id": "bench",
      "roots": ["crates/bench"],
      "source": "cargo-workspace"
    }
  ],
  "discovery": {
    "cargo_workspace": true,
    "extra_roots": []
  }
}
```

- `setup` / `bootstrap` / first `sync` writes or refreshes scopes (idempotent; `--force` rebuilds).  
- Humans may edit roots/ids; rustbrain must not invent scope narrative — only path→id rules.

### CLI surface (target)

```bash
# Whole workspace (MainBrain + all SubBrains) — default, same as today
rustbrain query "sqlite"
rustbrain context "why local index"

# Focus one SubBrain (+ MainBrain hubs + 1-hop cross-scope neighbors)
rustbrain query "clap derive" --scope rustbrain-cli
rustbrain context "CLI flags" --scope rustbrain-cli
rustbrain graph crates/rustbrain-cli/src/main.rs --scope rustbrain-cli

# Capture into a SubBrain tree
rustbrain note new --type adr --title "Split CLI package" --scope rustbrain-cli
# → docs path under that scope’s docs root, or main docs with frontmatter scope:

# List scopes
rustbrain scopes
rustbrain scopes --json

# Doctor
rustbrain doctor              # includes scope health when scopes configured
```

**Scoped retrieval semantics:**

1. FTS / ranked seeds filtered to `owner_scope IN (selected, main_hubs?)`  
2. Always allow **MainBrain hub** inject (readme / changelog / roadmap) when intent matches  
3. Graph hops may leave the scope (cross-crate `symbol:` / WikiLink) so agents still see real edges  
4. Soft option later: `--scope-strict` = no outbound hops (rare; document as power user)

### Storage / graph (target)

| Piece | Approach |
|-------|----------|
| SQLite | `nodes.scope TEXT NOT NULL DEFAULT 'main'` (+ index); migrate schema v2 |
| FTS | Optional densify token `scope:rustbrain-cli` for agent queries |
| CSR mmap | Keep **one** workspace graph (topology global); scope filter is query-time, not a second mmap (v1) |
| Pending links | Resolve across scopes by global id / path (today’s resolver + scope metadata) |
| Export | Bundle includes scope field; import restores it |

### Phased delivery

#### Phase S0 — Spec & discovery only (no query filter yet)

- [x] Freeze id rules (path-stable; package name as alias) in [[docs/schema]] / scopes module  
- [x] Implement Cargo workspace member discovery + write `workspace.json` scopes  
- [x] `rustbrain scopes` list / enable / add  
- [x] Tests (unit + multi-scope index/query + import)  

#### Phase S1 — Index ownership

- [x] Schema: `nodes.scope` + migration v2  
- [x] Indexer assigns scope by longest root prefix (+ frontmatter override when valid)  
- [x] `doctor`: orphan scopes, empty SubBrains, missing roots, mode/DB mismatch  
- [x] `scopes reconcile` reassigns all nodes from manifest  
- [x] `scopes list` counts per scope  

#### Phase S2 — Query / context / graph `--scope`

- [x] `query --scope` / `context --scope` with **SQL-side** scope filter  
- [x] Default Main mix = **hubs only**; `--scope-strict` / `--scope-with-main`  
- [ ] `graph --scope` filter (optional; neighbors still global)  
- [x] AGENTS.md + CLI cookbook snippet  
- [x] Integration tests: strict scope + umbrella mount  

#### Phase S3 — Capture & multi-root / umbrella

- [x] `note new --scope` (+ frontmatter `scope:`)  
- [x] Explicit roots via `scopes add --root` / `scopes attach`  
- [x] Import: copy SubBrain, **mount** under umbrella, or **merge** `--into main`  
- [x] Absorb SubBrain → MainBrain (`scopes absorb`) with FTS densify  
- [x] `export --scope` share SubBrain without merging whole brain  
- [ ] Optional `[workspace.metadata.rustbrain]` in root Cargo.toml  
- [ ] Bootstrap creates per-crate `docs/` stubs only when asked  

#### Phase S4 — Polish

- [x] `scope:` FTS densify on set_node_scope / reconcile / absorb  
- [x] Bundle export filtered by scope; import stamp scope on nodes  
- [ ] Bench: scoped query vs unscoped on synthetic multi-crate fixture  
- [ ] Consider multiple CSR mmaps only if measured need (not default)  
- [ ] Auto default `--scope` from cwd package path (agent convenience)  
- [ ] WikiLink rewrite on copy-import (ids change under `docs/subbrains/…`)  

### Umbrella recipe (three mono MainBrains → one MainBrain)

```text
umbrella/                 # new git superproject (or folder)
  .brain/                 # NEW MainBrain
  project-a/              # former mono-repo (+ optional nested .brain)
  project-b/
  project-c/
```

```bash
cd umbrella && rustbrain setup --yes --no-bootstrap   # or init + sync
rustbrain scopes enable --empty
rustbrain scopes attach project-a --root project-a
rustbrain scopes import --from ./project-b --as project-b --mount
rustbrain scopes import --from ./project-c --as project-c --mount
rustbrain scopes reconcile
rustbrain query "topic" --scope project-a
# Nested project-a/.brain still works when CWD is inside project-a
```

### Agent protocol (when shipped)

1. Detect cwd package → default `--scope` when under a known root (optional convenience).  
2. Prefer `context "…" --scope <crate>` before large refactors in that crate.  
3. Put cross-cutting decisions in **MainBrain** (`docs/adr/`, root hubs), not duplicated per crate.  
4. Never invent SubBrain lists — run `rustbrain scopes` / read `workspace.json`.  

### Out of scope for this epic

- Nested SubBrain trees (`main/foo/bar` as separate brain hierarchy)  
- One `.brain/` per crate by default (multiple DBs) — rejected for v1 (hurts cross-links; use scopes instead)  
- Automatic merge of unrelated git repos into one MainBrain  
- Replacing `--all-workspaces` (stays machine-global registry)  

### Related

- Full product backlog: [[docs/plans/product-roadmap]] (Epic A)  
- Architecture sketch: repo `RUSTBRAIN_ARCHITECTURE_PLAN.md` §10  
- Schema / mmap: [[docs/schema]], [[docs/mmap_format]]  

---

## Related

- [[docs/plans/product-roadmap]]
- [[changelog]]
- [[docs/benchmarks]]
- [[docs/cli]]
