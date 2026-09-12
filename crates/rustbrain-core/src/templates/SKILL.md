---
name: rustbrain
description: >
  Use rustbrain (local Markdown + SQLite second brain) to inject project goals,
  ADRs, concepts, and notes into this session, then persist decisions as
  Markdown so the next agent does not start from zero. Use at session start,
  before claiming history or reversing a decision, after you learn something
  worth keeping, and whenever the user mentions rustbrain, second brain,
  project docs, AGENTS.md, adr, goals, decisions, analysis, project context and concepts, or edge cases.
---

# rustbrain — inject and persist project memory

rustbrain is the project's **durable memory**: Markdown notes + a local SQLite
index (`.brain/`, never commit). Your chat transcript dies; these files do not.
Use it to **orient** (inject the right notes into *this* prompt) and to **write
back** (goals, ADRs, concepts, analyses, edge cases) so later sessions stay
aligned.

This skill is the **agent operating loop and CLI cookbook**. Root `AGENTS.md`
is a short mandate to use rustbrain (not a flag dump). Full flags also live in
`rustbrain <cmd> --help`.

## 0. Detect (once per session)

```bash
rustbrain doctor
```

| Result | Action |
|--------|--------|
| Healthy / info findings (`scaffold_only`, `sparse_readme`, `no_changelog`) | Proceed. Info ≠ broken. |
| Command missing | `cargo install rustbrain`, then ensure `~/.cargo/bin` is on `PATH`. |
| No `.brain` | From repo root: `rustbrain setup --yes`. Do **not** `bootstrap --force` later as a habit (clobbers generated files). Custom `AGENTS.md` is never overwritten. |
| `file_errors` / stale | `rustbrain sync` then retry. |

`query` / `context` / `doctor` walk parent dirs for `.brain` (like git). Prefer
running from repo root, or pass `-w /path/to/repo`.

If `docs/AGENTS.md` exists, it is the **docs-local mandate** (use rustbrain
before claiming decisions). Follow it.

## 1. Session boot — inject context before you reason

Do this **before** answering “why”, “what did we decide”, “what shipped”, or
editing code that implements a documented design. Do **not** invent ADRs,
changelogs, or goal status from the transcript.

**Write a prompt from the user's task**, then pack:

```bash
rustbrain context "<specific prompt>" --type goal,adr,concept -m 1200
```

Prompt like a search, not a novel:

| User task | `context` prompt |
|-----------|------------------|
| Implement / change feature X | `how X works` / `why X` |
| Architecture / design | `summarize architecture` plus the subsystem name |
| Reversing or choosing | `why we chose <X> not <Y>` |
| What already shipped | `what shipped` / `changelog` |
| Roadmap / what next | `roadmap priorities` |
| Bug / trap | `<error or subsystem> edge case` |
| Crate / API | crate name (`serde`, `tokio`, …) — harvested `docs/references/crates/*` after setup |

Then:

1. **Read the pack.** Treat packed bodies as evidence. Follow `[[WikiLinks]]`
   and `symbol:` hits; **open the full note** if you will edit it or the excerpt
   is truncated.
2. **If the pack is thin:** `rustbrain sync`, then `rustbrain query "<topic>" --scores`.
   Empty query: retry `--with-symbols` or `--type adr,goal,concept,plan,analysis`.
3. **If you need neighbors, not prose:** `rustbrain graph docs/adr/<note>.md`
   (or `graph changelog` / `graph roadmap` when those hubs exist).

Do **not** slurp all of `docs/` into the prompt. `context` already budgets
tokens, prefers goals/ADRs, skips ADR `TEMPLATE`, and dedupes README vs
from-readme.

### Token knobs (use them)

| Flag | When |
|------|------|
| default `context "…"` | First pack for the task |
| `-m 800` | Tight window; you already have the files open |
| `-m 1600`+ | Cross-cutting design, several ADRs |
| `--hops 0` | Seeds only — precise, less graph noise |
| `--hops 2` | Rare; noisy. Prefer `graph` on one note instead |
| `--type adr,goal` | Decision alignment; skip symbol junk |
| `--type adr,goal,concept,plan` | Feature work with a roadmap |
| `--with-symbols` | Hunting types/methods; otherwise leave off |
| `--no-hop-symbols` | Notes only even if seeds hop |
| `-F xml` | Only if a tool protocol needs XML-escaped packs |
| `--scope ID` | Multi-brain only — run `rustbrain scopes list` first |

Natural-language queries drop stopwords; multi-token is **OR**. Prefer 2–6
content words (`hist pacing lockout`, not `please explain the complete history
of how we download bars`).

## 2. Mid-task — re-inject when the work shifts

Re-run `context` / `query` when you:

- Move to a different subsystem than the boot pack covered
- Are about to contradict an ADR or goal
- Need code symbols the first pack omitted (`query "Name" --with-symbols`)
- Just `sync`'d notes you will now depend on

**Implementation iteration:**

```text
context/query  →  read the 1–3 notes that matter  →  edit code
     ↑                                                ↓
  context again ←  sync (+ links)  ←  capture note if a decision landed
```

| Need | Tool |
|------|------|
| Ranked evidence + why it hit | `query "…" --scores` |
| Packed excerpts under budget | `context "…"` |
| Who links to whom | `graph <path-or-title>` |
| Unresolved `[[WikiLinks]]` / `symbol:` | `links` / `links --json` |
| Index health | `doctor` |

## 3. Write back — memory that survives this session

If you learned a **decision**, **trap**, **timeless what-is**, **dated dig**, or
**goal**, it does not exist for the next agent until it is a note on disk.

**Scaffold, then edit, then sync** (do not pass `--body` unless the full text
is already written — `--body` skips the type template):

```bash
rustbrain note new --type adr --title "short factual title"
# edit the printed path; add [[WikiLinks]] and symbol:Foo
rustbrain sync
```

| Type | Capture |
|------|---------|
| `goal` | What we are building toward (not a chat summary) |
| `adr` | We chose X (short, factual; not a transcript) |
| `concept` | Timeless “what is X” |
| `analysis` | Dated investigation; promote a decision to `adr` |
| `plan` | Roadmap / backlog / tasklist (`docs/plans/`) |
| `edge_case` | A specific trap and how to avoid it |
| `reference` | External docs (vendor pages, pinned docs.rs) |
| `changelog` | Prefer root `CHANGELOG.md` hub; never invent releases |

**Link so future `context` hops work:**

- Notes → code: `symbol:Type` / `symbol:crate::mod::Type::method` / `[[symbol:…]]`
- Notes → notes: `[[docs/adr/…]]` or WikiLinks the scaffold already expects
- Code → notes (rustdoc): `/// See [[docs/adr/my-adr]]` (sync makes `doc_links`)

After Markdown or code that should be indexed:

```bash
rustbrain sync
rustbrain links --auto                  # soft edges, no rewrite
rustbrain links --apply --dry-run       # then --write if the plan is unique
```

`links --apply` never invents notes. Skip ambiguous / generated files unless
`--force`.

**Do not:** invent ADR history; fabricate changelog entries; dump a huge plan
file into the brain; commit `.brain/`; `bootstrap --force` as a habit (only
after a real README rewrite, and expect generated files to refresh. Rustbrain-owned
`AGENTS.md` is also refreshed to the thin mandate; custom `AGENTS.md` is never replaced).

After README improvement: `rustbrain bootstrap --yes --write --force && rustbrain sync`.
After CHANGELOG only: `rustbrain sync`.

## 4. Where files live (generic layout)

| Path | Role |
|------|------|
| `README.md` | Hub `readme` — harvest quality tracks this |
| `CHANGELOG.md` | Hub `changelog` — ground truth for “what shipped” |
| `ROADMAP.md` / `BACKLOG.md` | Optional hubs `roadmap` / `backlog` (`plan`) |
| `docs/goals/` | Goals |
| `docs/adr/` | Decisions |
| `docs/concepts/` | Timeless what-is |
| `docs/analysis/` | Dated digs |
| `docs/plans/` | Hand-written plans |
| `docs/edge_cases/` | Traps |
| `docs/references/` | External / crate maps (some generated) |
| `docs/AGENTS.md` | Docs-local agent protocol when present |
| `AGENTS.md` | Short rustbrain mandate (cookbook is this skill) |
| `.brain/` | Index — gitignore |
| `.rustbrainignore` | Extra skip rules |

Hubs are **features when present**, not required. Goals + ADRs + concepts work
alone. Plan status after sync: `query "status:in_progress" --type plan`.

## 5. Anti-patterns (these waste the brain)

- Answering “why we did X” from model prior instead of `context "why X"`
- Reading twenty `docs/**` files by hand instead of one `context` pack
- Passing a vague prompt (`help`, `project`, `docs`) — garbage-in, thin hits
- `--hops 2 --with-symbols` on every call — floods the window, drowns ADRs
- Creating notes with `--body` chat paste — skips structure, ranks poorly
- Skipping `sync` after edits — the next session cannot see your work
- Treating `doctor` info (`scaffold_only`) as a blocker
- Enabling multi-brain (`scopes enable`) when a single `.brain` is enough

## 6. Multi-brain (only if `scopes list` says so)

Default is **one brain**. If the repo is already multi-scope:

```bash
rustbrain scopes list          # learn ids first — never guess
rustbrain context "…" --scope <id>
rustbrain query "…" --scope <id> --scope-strict
```

Do not attach/import/absorb unless the user asked.

## 7. Drop this skill into another project

`rustbrain setup --yes` / `bootstrap --yes --write` **installs this file** into
every detected harness dir (`.grok/`, `.claude/`, `.cursor/`, `.agents/`,
`.codex/`, `.gemini/`, `.ai/`) at `.<harness>/skills/rustbrain/SKILL.md`. If
none of those folders exist, it writes repo-root `SKILL.md`. Opt out:
`--no-skill-md`.

Manual copy still works:

```text
<repo>/.grok/skills/rustbrain/SKILL.md
~/.grok/skills/rustbrain/SKILL.md         # all your Grok projects
<repo>/.claude/skills/rustbrain/SKILL.md
<repo>/.cursor/skills/rustbrain/SKILL.md
```

Then in that repo: `rustbrain setup --yes` once (creates `.brain/`, `docs/`
scaffold, thin `AGENTS.md` or an appended rustbrain section). This skill is the
CLI cookbook; `AGENTS.md` is only the short mandate.

---

## First time

| Command | What it does | What to expect |
|---------|----------------|----------------|
| `rustbrain setup --yes` | init + bootstrap + sync + doctor | Creates `.brain/`, `docs/`, `.rustbrainignore`, thin **`AGENTS.md`**, **`SKILL.md`** (harness or root), README harvest, **crate → docs.rs notes**, AST module map, then indexes |
| `rustbrain setup --yes --no-crate-docs` | skip docs.rs harvest | No `docs/references/crates/*` |
| `rustbrain setup --yes --no-agents-md` | skip root AGENTS.md | No `AGENTS.md` write/append |
| `rustbrain setup --yes --no-skill-md` | skip skill install | No harness/root `SKILL.md` write |
| `rustbrain setup --yes --agents-template PATH` | use custom AGENTS body | Your template becomes root `AGENTS.md` (new or rustbrain-owned files only) |
| `rustbrain setup --yes --force` | overwrite generated bootstrap files | Regenerates `from-readme`, module-map, ignore. Custom `AGENTS.md` is **not** overwritten |
| `rustbrain setup --yes --no-bootstrap` | init + sync only | No docs scaffold |
| `rustbrain setup --yes --no-doctor` | skip final health print | Still syncs |

Step-by-step equivalent:

```bash
rustbrain init
rustbrain bootstrap --yes --write
rustbrain sync
rustbrain doctor
```

**Empty / thin README:** bootstrap still succeeds. `from-readme` is skipped (no README) or thin (scrappy README). `doctor` reports `no_readme` / `sparse_readme` / `scaffold_only` as **info** — not failures. Fill knowledge with notes, not invented history.

---

## Everyday loop

```bash
rustbrain context "why <decision> / how does <feature> work"   # orient (content pack)
rustbrain context "what shipped" / "changelog 0.3"             # CHANGELOG hub when present
rustbrain context "roadmap priorities"                         # ROADMAP/BACKLOG hubs if present
rustbrain graph docs/adr/….md                                  # inspect who links where
rustbrain query "topic" --scores                               # search notes
# preferred note creation — see below (scaffold, then edit)
rustbrain note new --type adr --title "…"
# then edit the printed path; sync if you used --no-sync
rustbrain sync && rustbrain doctor && rustbrain links
```

---

## Multi-brain (optional — multi-crate / umbrella workspaces)

**Default is single-brain.** Only enable multi when you need SubBrains.

### Discovering ids (agents: run these *before* import/attach)

| What you need | Command | What it prints |
|---------------|---------|----------------|
| **SubBrain ids in *this* workspace** | `rustbrain scopes list` | `mode`, `main`, each SubBrain **id**, roots, node counts |
| Machine-readable ids | `rustbrain scopes list --json` | `manifest.scopes[].id` + `counts` |
| Ids in **another** path | `rustbrain scopes list -w /path/to/other` | Same, for that tree (if it already has multi-brain) |
| Cargo package → candidate SubBrain id | `rustbrain scopes enable --cargo` then `scopes list` | Path-stable ids (e.g. `rustbrain-cli`); package name may appear as **alias** |
| **Pick an id for a folder you will import** | You **choose** it: use the directory name | e.g. `./project-a` → `--as project-a` or `attach project-a --root project-a` (sanitize: lowercase, `-` not `_`) |
| Node ids (notes/symbols) | `rustbrain query "…" --scores` / `graph <path>` | Node `id:` lines; hubs: `readme`, `changelog`, `roadmap`, `backlog` |

**Rule:** SubBrain **id** is not auto-discovered from a foreign single-brain until you **name** it (`--as` / `attach <id>`). Prefer the folder name. Confirm with `scopes list` after attach/import.

```bash
# 1) See current mode + ids
rustbrain scopes list
rustbrain scopes list --json

# 2) Enable multi (Cargo monorepo)
rustbrain scopes enable --cargo && rustbrain scopes list

# 3) Umbrella: three former mono-repos under one folder
rustbrain scopes enable --empty
rustbrain scopes attach project-a --root project-a          # id = project-a (you chose it)
rustbrain scopes import --from ./project-b --as project-b --mount
rustbrain scopes import --from ./project-c --as project-c --mount
rustbrain scopes reconcile
rustbrain scopes list                                        # confirm ids + node counts

# 4) Query / share using the id from `scopes list`
rustbrain query "topic" --scope project-a                    # hubs-only MainBrain mix
rustbrain query "topic" --scope project-a --scope-strict
rustbrain query "topic" --scope project-a --scope-with-main
rustbrain export --out a.brainbundle --scope project-a       # share SubBrain without merge
# Keep separate: import --as id · Merge into MainBrain: import --into main
# Fold SubBrain into main: scopes absorb project-a
```

Nested `project-a/.brain` may still exist for working inside that tree alone. The umbrella MainBrain owns path scopes after attach/mount.

---

## CLI reference (variations)

### `setup` / `bootstrap` / `init` / `sync`

| Command | Use when | Expect |
|---------|----------|--------|
| `setup --yes` | Cold start / CI / agents | Full scaffold + index; prefer this over multi-step |
| `bootstrap --yes --write` | Scaffold only (already have `.brain`) | Files under `docs/`, optional harvest; **no** full re-think of ADRs |
| `bootstrap --dry-run` | See plan | Prints actions; writes nothing |
| `bootstrap --yes --write --no-agents-md` | Scaffold without AGENTS.md | No `AGENTS.md` write/append |
| `bootstrap --yes --write --no-skill-md` | Scaffold without skill install | No harness/root `SKILL.md` |
| `bootstrap --yes --write --agents-template ./AGENTS.template.md` | Org template | Used for new or rustbrain-owned `AGENTS.md` only |
| `init` | Empty store only | `.brain/db.sqlite`; does **not** create docs or index |
| `sync` | After Markdown/code changes | Re-index; content-hash skips unchanged files; `file_errors=N` if some files fail |
| `sync` from a subdirectory | CWD anywhere under the repo | Prefer `rustbrain sync -w /repo` or run from root; open walks parents for query/context/doctor |

### `doctor`

| Command | Expect |
|---------|--------|
| `rustbrain doctor` | Text health: db/mmap/counts + **info** findings (sparse README, scaffold-only, template ADR, pending links, …) |
| `rustbrain doctor --json` | Same as JSON for tools |
| `rustbrain doctor --strict` | Exit **1** if unhealthy **or** any pending links |

Doctor walks parent dirs for `.brain` (like git). **Info ≠ broken** — e.g. `scaffold_only` means “few real notes yet”, not corrupt DB.

### `query` (search)

Default is **note-first** (goals/ADRs/concepts; symbols excluded).

| Command | Expect |
|---------|--------|
| `query "duckdb"` | Ranked notes; may hit README hub / from-readme / ADRs |
| `query "duckdb" --scores` | Same + numeric scores + reasons |
| `query "open" --with-symbols` | Include code symbols (methods, types) |
| `query "x" --all-types` | All node types (alias of with-symbols for type filters cleared) |
| `query "x" --type goal,adr,concept` | Only those types |
| `query "x" -n 10` | Cap results |
| `query "x" --all-workspaces` | Merge across registered local workspaces |
| `query "x" -w /path/to/repo` | Explicit workspace |
| `query "x" --scope ID` | Multi-brain: SubBrain + hub nodes only (default) |
| `query "x" --scope ID --scope-strict` | SubBrain only |
| `query "x" --scope ID --scope-with-main` | SubBrain + all MainBrain nodes |
| `query "status:in_progress" --type plan` | Plan densify tokens after sync |

Natural language: stopwords dropped; multi-token uses OR (`why egui not tauri` → egui OR tauri). **Garbage-in:** thin README → thin hits. Empty results print a hint (`--with-symbols` or sync). Learn SubBrain **ID** via `scopes list` first.

### `context` (agent pack)

Builds FTS seeds + optional graph hops under a token budget. Default format: **markdown**.

| Command | Expect |
|---------|--------|
| `context "why egui not tauri"` | Seeds notes; packs **body excerpts** (not titles only); stopword-aware |
| `context "summarize architecture"` | If FTS is weak/generic, **hub fallback** (README / harvest / module map) |
| `context "topic" -F xml` | XML-escaped for tool protocols |
| `context "topic" -m 800` | Smaller token budget |
| `context "topic" --hops 0` | Seeds only (no graph neighbors) |
| `context "topic" --hops 2` | Deeper graph (noisier) |
| `context "topic" --with-symbols` | Allow symbols as FTS seeds |
| `context "topic" --no-hop-symbols` | Never pack symbol neighbors |
| `context "topic" --type adr,goal` | Seed type filter |
| `context "topic" -p "…"` | Same as positional prompt |
| `context "topic" --scope ID` | Scoped seeds (hubs-only Main mix); neighbors may hop out |
| `context "topic" --scope ID --scope-strict` | Strict SubBrain seeds |
| `context` from `src/` | Finds parent `.brain` automatically |

Packing prefers **seeds and ADRs/goals** over symbol noise; skips ADR `TEMPLATE`; dedupes README vs from-readme; strips YAML frontmatter from excerpts.

### `scopes` (MainBrain / SubBrain)

| Command | Expect |
|---------|--------|
| `scopes list` | **Primary way to learn ids** — mode, main, SubBrain ids, roots, node counts |
| `scopes list --json` | Same for tools (`manifest.scopes[].id`) |
| `scopes detect PATH` | Suggest id + mount tips **before** import/attach |
| `scopes list -w /other` | Inspect another workspace path |
| `scopes enable --cargo` | multi + Cargo members as SubBrains; then `sync` |
| `scopes enable --empty` | multi with no SubBrains yet |
| `scopes add ID --root PATH` | Add/update SubBrain root(s) |
| `scopes attach ID --root PATH` | Umbrella: existing dir as SubBrain (no copy) |
| `scopes import --from PATH --as ID` | Copy notes → separate SubBrain |
| `scopes import --from PATH --as ID --mount` | Source under this tree → attach path, no copy |
| `scopes import --from PATH --into main` | **Merge** copy into MainBrain |
| `scopes absorb ID` | SubBrain nodes → main; drop SubBrain def |
| `scopes absorb all` | Everything → main; mode=single |
| `scopes remove ID [--absorb]` | Drop def (prefer `--absorb`) |
| `scopes reconcile` | Recompute all node scopes from manifest |
| `scopes disable [--absorb-all]` | Back to single mode |

### `graph` (structure inspect)

Shows **who links to whom** (ASCII tree or JSON). Use when you need edge types/weights, not a full content pack.

| Command | Expect |
|---------|--------|
| `graph` | Workspace stats: by type, by relation, hubs |
| `graph docs/concepts/raft.md` | 1-hop neighborhood of that note |
| `graph "Raft" --hops 2` | Deeper tree (title resolve when unique) |
| `graph symbol:StorageEngine` | Symbol-centered neighborhood |
| `graph docs/x.md --no-auto --no-symbols` | Explicit note links only |
| `graph docs/x.md --direction out` | Outgoing edges only |
| `graph docs/x.md --json` | Machine-readable for tools |

### `links --apply` (safe Markdown rewrites)

| Command | Expect |
|---------|--------|
| `links --apply --dry-run` | Plan unique pending WikiLink normalizations (no writes) |
| `links --apply --write` | Apply AUTO edits atomically; auto-sync refreshes pending/edges |
| `links --apply --discover --dry-run` | + Aho–Corasick unmarked mentions (suggest/auto tiers) |
| `links --apply --discover --write --style related` | Append under `## Related` instead of wrapping prose |
| `links --apply --write --json` | Full report for agents |

Never invents notes. Ambiguous / unresolved / generated files are skipped unless `--force`.


### `note new` (preferred agent workflow)

**Preferred usage (better agentic outcomes):** create with **type + title only**, leave the
body empty so rustbrain writes a **type-specific scaffold**, then **edit that file**.

```bash
# 1) Scaffold (omit --body / --note)
rustbrain note new --type analysis --title "criterion query-path 2026-07-31"
#    → writes docs/analysis/….md with Question / Findings / Artifacts / Recommendations / …
#    → prints path; auto-syncs so the empty scaffold is indexed

# 2) Edit the file on disk (fill sections, add symbol:… and [[WikiLinks]])
#    e.g. open the path printed by note new

# 3) Re-index after the edit
rustbrain sync
```

Same pattern for other types:

```bash
rustbrain note new --type goal --title "Use rustbrain well"
rustbrain note new --type adr --title "Use duckdb CLI not libduckdb"
rustbrain note new --type concept --title "CSR mmap cache"
rustbrain note new --type edge_case --title "NixOS EGL_BAD_PARAMETER"
```

| Command | Expect |
|---------|--------|
| `note new --type T --title "T"` | **Preferred** — scaffold body for `adr` / `goal` / `analysis`; path printed; syncs |
| `note new --type T --title "T" --body "…"` | Fills body immediately (skips scaffold). Use when the whole text is ready |
| `note new --type T --title "T" --note "…"` | Same as `--body` (synonym) |
| `note new … --tags a,b --aliases x` | Frontmatter tags/aliases |
| `note new … --no-sync` | Write only; `sync` after you edit |
| `note new … --force` | Overwrite existing path |
| `note new … --scope ID` | Multi-brain: write under that SubBrain’s tree |
| `note new … -w /repo` | Explicit workspace |

Types: `goal`, `adr`, `alternative`, `concept`, `analysis`, `plan`, `changelog`, `reference`, `edge_case`.
- **concept** — timeless “what is X”
- **analysis** — dated investigation (crate compare, design options, `cargo bench` / criterion review, data digests); recommendations optional; promote decisions to **adr**
- **plan** — roadmaps, backlogs, tasklists, todos (`docs/plans/`; aliases: roadmap, backlog, todo)
- **changelog** — release notes (prefer root **CHANGELOG.md** hub; type `changelog`)
- **adr** — we chose X
- **edge_case** — a specific trap

Link **notes → code** with `symbol:Type::method` (or `[[symbol:…]]`).
Link **code → notes** in rustdoc: `/// See [[docs/adr/my-adr]]` (sync creates `doc_links` edges).

### `links` / `watch` / `export` / `import`

| Command | Expect |
|---------|--------|
| `links` | Unresolved WikiLinks / `symbol:` targets |
| `links --json` | Machine-readable |
| `links --auto` | Soft `auto_*` edges (no Markdown rewrite) |
| `links --apply --dry-run` / `--write` | Pending WikiLink normalize (see above) |
| `watch` | Debounced re-sync on file changes (Ctrl-C to stop) |
| `watch --debounce-ms 500` | Slower debounce |
| `export --out x.brainbundle` | Portable JSON graph (AST optionally decoupled) |
| `export --out x.brainbundle --scope ID` | Share **one SubBrain** (+ hubs) without full merge |
| `import --input x.brainbundle` | Merge bundle into this brain + remmap |

Full flag book: repo `docs/CLI.md` or `rustbrain <cmd> --help`.

---

## Where knowledge lives

| Path | Purpose |
|------|---------|
| `README.md` | Hub node `readme` (quality of harvest depends on this) |
| **`CHANGELOG.md`** | Hub **`changelog`**, type **`changelog`** — Keep a Changelog + SemVer. Ground truth for "what shipped" |
| `ROADMAP.md` / `BACKLOG.md` (optional) | Hubs `roadmap` / `backlog`, type **`plan`** |
| `docs/plans/` | Hand-written plans / roadmaps / tasklists (`note new --type plan`) |
| `docs/AGENTS.md` | **Mandatory** docs-local agent protocol: use rustbrain every turn |
| `docs/goals/from-readme.md` | **Algorithmic** harvest of README sections (not an LLM) |
| `docs/goals/`, `docs/adr/`, `docs/analysis/`, … | Hand-written project knowledge |
| `docs/analysis/` | Dated investigations (`note new --type analysis`) — good for epic digests |
| `docs/implementation/module-map.generated.md` | AST symbol list |
| `docs/references/crates/*.md` | **docs.rs** URLs for Cargo.toml deps (generated on setup/bootstrap) |
| `docs/references/crate-docs.generated.md` | Index of all harvested crate docs |
| `AGENTS.md` | Short rustbrain mandate (not the CLI cookbook) |
| `SKILL.md` or `.<harness>/skills/rustbrain/SKILL.md` | Agent loop + CLI cookbook |
| `.brain/` | Local index — **never commit** |
| `.rustbrainignore` | Extra index skips |

### CHANGELOG (Rust community standard)

If this repo publishes a crate (or you want ship history for agents):

1. Keep a root **`CHANGELOG.md`** in [Keep a Changelog](https://keepachangelog.com/) form (`## [x.y.z] - YYYY-MM-DD`, `## [Unreleased]`).
2. Run **`rustbrain sync`** after edits — maps to stable id **`changelog`**, type **`changelog`**, aliases versions / "releases" / "unreleased"; boosts release-oriented `query` / `context`.
3. Prefer **truthful ship notes** over inventing history. Agents: `rustbrain context "what changed in 0.3"` / `query changelog --scores`.

Doctor reports `no_changelog` (info) when a `Cargo.toml` exists but no CHANGELOG; `changelog_latest` when the hub is healthy.

Also read **`docs/AGENTS.md`** — mandates rustbrain tooling on every agent turn when working in docs/.

### HITL planning (roadmaps, epics, status)

| Need | Where it lives | rustbrain type / hub |
|------|----------------|----------------------|
| Shipped / versioned history | `CHANGELOG.md` | type `changelog`, hub `changelog` |
| Future direction | `ROADMAP.md` or `docs/plans/` | type `plan`, hub `roadmap` |
| Unordered work queue | `BACKLOG.md` or `docs/plans/` | type `plan`, hub `backlog` |
| Time-bound dig / epic write-up | `docs/analysis/` | `analysis` |
| Decision | `docs/adr/` | `adr` |
| Status of a slice | plan checklist or analysis + WikiLinks | do **not** invent kanban columns |

Query: `context "roadmap priorities"`, `context "what shipped"`, `graph changelog`.

---

## Conventions

- **Create notes with scaffold, then edit:** prefer  
  `rustbrain note new --type "…" --title "…"` **without** `--body`/`--note`,  
  then edit the created file, then `rustbrain sync`. Passing a full body skips the
  type template and often produces thinner structure.
- Prefer short factual ADRs over chat logs.
- **Changelog is ground truth for releases** — update it when you ship; never invent entries.
- Link notes→code: `symbol:Name` / `symbol:crate::mod::Name` / `[[symbol:…]]`.
- Link code→notes in rustdoc: `/// See [[docs/adr/…]]` (becomes `doc_links` on sync).
- Frontmatter when useful:

  ```yaml
  ---
  tags: [topic]
  node_type: adr
  aliases: [short-name]
  ---
  ```

- Do **not** invent ADR history. Do **not** commit `.brain/`.
- After improving README: `rustbrain bootstrap --yes --write --force && rustbrain sync`.
- After updating CHANGELOG: `rustbrain sync` (no harvest needed).

---

## Full help

```bash
rustbrain --help
rustbrain <command> --help
```
