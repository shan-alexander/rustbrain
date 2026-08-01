# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.20] - 2026-07-31

### Documentation — crate READMEs

- **`rustbrain` CLI README:** full getting-started, benefits table, everyday loop, note types,
  hubs, agent protocol, linking examples, exit codes.
- **`rustbrain-core` README:** library benefits, multiple copy-paste API examples (sync/search/
  context, bootstrap, notes, graph, apply_links), API surface table, plan densify, bootstrap
  options, storage mental model, feature flags.

## [0.3.19] - 2026-07-31

### Changed — plan status `undone` → `blocked`

- Canonical plan status token is now **`blocked`** (FTS `status:blocked`, summary line).
- Parse aliases still accept `undone`, `reopen`, `reopened`, `stuck`, `on_hold`, `paused`, `deferred`.
- Plan scaffold and docs updated; re-`sync` plans to refresh densified tokens.

## [0.3.18] - 2026-07-31

### Added — Cargo.toml → docs.rs reference notes on setup/bootstrap

- **`rustbrain setup` / `bootstrap`** harvests crates.io dependencies from every
  `Cargo.toml` (deps / dev / build / workspace.dependencies), resolves versions
  from `Cargo.lock` when present, and writes:
  - `docs/references/crate-docs.generated.md` (index)
  - `docs/references/crates/{crate}.md` per package with **docs.rs** + crates.io URLs
- Skips `path =` local crates; supports `package =` renames; cap 300 crates.
- Flag: `--no-crate-docs` to skip. Default on when `Cargo.toml` exists.
- Library: `crate_docs` (`collect_crate_deps`, `docs_rs_url`, `write_crate_docs_notes`).
- No network fetch of docs HTML — URL convention only; re-run with `--force` after dep upgrades.

## [0.3.17] - 2026-07-31

### Added — plan status densification (optional, algorithmically dense)

- Index-time **plan status digest** for `plan` notes (and ROADMAP/BACKLOG hubs):
  - Canonical tokens: `backlog`, `in_progress`, `qa`, `done`, `cancelled`, `blocked`
  - Sources: frontmatter `status:`/`state:`, `## Status`, section headings, checkboxes
    (`- [ ]` / `- [/]` / `- [x]` / `- [~]` / `- [?]`), tagged bullets
  - **Summary** line: `plan status=… · open N · done M · cancelled K`
  - **FTS block**: `status:…`, `task:status:slug`, `plan_open:N` for agent query/context
- Plan scaffold includes status sections; docs stress **optional** hubs (no CHANGELOG/plan required)
- Doctor `no_changelog` wording: optional for apps; never blocking

## [0.3.16] - 2026-07-31

### Added — `changelog` + `plan` node types; `docs/AGENTS.md`

- **`NodeType::Changelog`** — root `CHANGELOG.md` / `CHANGES.md` / `HISTORY.md` hub (`changelog`);
  Keep a Changelog summaries + version aliases (was loosely typed as `reference`).
- **`NodeType::Plan`** — roadmaps, backlogs, tasklists, todos (`plan`; parse aliases:
  `roadmap`, `backlog`, `todo`, `tasklist`, …). Root `ROADMAP.md` / `BACKLOG.md` hubs;
  hand-written notes under **`docs/plans/`** with checklist scaffold.
- Bootstrap scaffolds `docs/plans/`, `docs/changelogs/`, and injects **`docs/AGENTS.md`**:
  **every agent turn** must use `rustbrain context` / `query` / `graph` / `sync` (docs-local mandate).
- Ranking boosts for plan/changelog; doctor / AGENTS cookbook updated.

## [0.3.15] - 2026-07-31

### Added — CHANGELOG / planning hubs (Rust community standards)

- Root **`CHANGELOG.md`** (also `CHANGES.md` / `HISTORY.md`) indexes as stable hub **`changelog`**
  (`reference`): Keep a Changelog headings → summary + SemVer aliases; FTS/query boost;
  context inject for release intent (`what shipped`, version tokens, `unreleased`, …).
- Optional **`ROADMAP.md`** / **`BACKLOG.md`** → hubs `roadmap` / `backlog` for HITL prioritization.
- Doctor infos: `no_changelog` (when `Cargo.toml` present), `sparse_changelog`,
  `changelog_no_versions`, `changelog_not_indexed`, `changelog_latest`.
- AGENTS.md cookbook documents CHANGELOG as ship-history ground truth and planning note map.
- Library: `hubs` module (`ProjectHub`, `HUB_CHANGELOG`, `is_release_intent`, …).

## [0.3.14] - 2026-07-31

### Improved — apply discover + watch polish (try-harder)

- **LinkLexicon disk cache** at `.brain/link_lexicon.json` (fingerprint of nodes/aliases);
  rebuild only when the brain identity changes; AC automaton rebuilt from cached surfaces.
- **Graph priors** (default on): 1-hop SQLite adjacency boosts discover hits that already
  neighbor the source (promotes weak → AUTO; reason tag `[graph-neighbor boost]`).
  Disable with `--no-graph-priors`.
- **`--style wrap|related`**: wrap still inlines WikiLinks; `related` leaves prose alone and
  appends under `## Related` (creates the section if missing; idempotent).
- **Watch**: clean `cfg(feature = "watch")` implementation (no dead imports); respects
  `.rustbrainignore`; skips `.brain`/`target`/…; reindex failures log and continue;
  unit tests for path filters. CLI already enables the `watch` feature.
- **Integration test**: pending WikiLink + target node → apply write → reindex clears ledger.
- Library: `ApplyStyle`, `is_indexable`, `is_under_skipped`.

## [0.3.13] - 2026-07-31

### Added — `links --apply` (Phase 0 pending + Phase 1 AC discover)

- **`rustbrain links --apply`** plans Markdown rewrites for the knowledge graph:
  - **Phase 0:** normalize pending WikiLinks that now uniquely resolve (`[[LogCompaction]]` →
    `[[docs/concepts/logcompaction|LogCompaction]]`). Skips ambiguous/unresolved, missing files,
    and generated notes (unless `--force`).
  - **Phase 1:** `--discover` builds a closed-world **LinkLexicon** (titles/aliases/stems/symbols)
    and scans note bodies with **Aho–Corasick** (leftmost-longest, case-insensitive) for unmarked
    mentions. Auto-tier wraps strong unique hits; suggest-tier is report-only.
- Safety: dry-run by default (needs **`--write`** to mutate); atomic temp+rename; span edits from
  end; overlap rejection; `--limit`; optional source `TARGET`; auto-`sync` after write.
- Library: `apply_links`, `ApplyOptions`, `ApplyReport`, `Brain::apply_links`
- WikiLink byte spans: `extract_wikilink_spans` / `WikiLinkSpan`
- Dependency: `aho-corasick`

## [0.3.12] - 2026-07-31

### Added — `rustbrain graph` (P1 neighborhood CLI)

- **`rustbrain graph [TARGET]`** — inspect the knowledge graph:
  - **No target:** workspace stats (counts by type/relation, high-degree hubs)
  - **With target:** k-hop neighborhood as ASCII tree (relation, weight, direction)
  - Target resolution: node id, path (`docs/…md`), exact/unique title, `symbol:Name`
- Flags: `--hops`, `--direction both|out|in`, `--no-auto`, `--no-symbols`, `--type`,
  `--limit`, `--json`, `--stats`, `-w`
- Library: `graph` module — `neighborhood`, `graph_stats`, `GraphOptions`,
  `Brain::graph_neighborhood` / `Brain::graph_stats`
- Complements `context` (content pack) with **structure** inspection for agents/humans

### Documentation / decision — JSON stack (serde vs jshift)

- Inventory of all `serde_json` / serde-derive / optional jshift usage.
- Criterion harness [`crates/json-stack-bench`](crates/json-stack-bench): workspace markers,
  doctor pretty JSON, brainbundle full I/O, large sparse path get, in-place field patch.
- **Result:** keep **serde_json** for production full encode/decode/pretty CLI and bundles;
  keep **jshift** optional for sparse path / in-place mutate only. Not a wholesale replace.
  Write-up: [`docs/JSON_STACK.md`](docs/JSON_STACK.md).
- Workspace pin `jshift` **0.1 → 0.7** (mutator API unchanged).

## [0.3.11] - 2026-07-31

### Documentation

- Built-in **AGENTS.md** documents code→note rustdoc WikiLinks (`doc_links`) alongside note→code `symbol:` anchors.

## [0.3.10] - 2026-07-31

### Added — rustdoc → brain WikiLinks (`doc_links`)

- During `sync`, Tree-Sitter-extracted **rustdoc** (`///` / `//!` / `/**`) is scanned for
  Obsidian-style `[[WikiLinks]]`.
- Creates explicit edges **symbol → note** with `relation_type = doc_links` (weight 1.0).
- Unresolved targets become **pending** and resolve on later syncs (same as note WikiLinks).
- Doc text is included in the symbol content hash so comment edits re-link.
- Completes bidirectional graph: notes already use `symbol:…` / `[[symbol:…]]` (`anchors`).

Example:

```rust
/// Primary UI shell. Decision: [[docs/adr/0001-use-egui]].
pub struct ParqApp { /* … */ }
```

## [0.3.9] - 2026-07-31

### Documentation — preferred `note new` workflow

- **AGENTS.md** (built-in template) and **README** / **CLI.md** recommend:
  1. `rustbrain note new --type "…" --title "…"` **without** `--body`/`--note`
  2. Edit the scaffolded file on disk
  3. `rustbrain sync` after edits  
  Passing a full body is still supported but skips type-specific boilerplate; scaffold-first
  improves agentic structure for `adr` / `goal` / `analysis`.

## [0.3.8] - 2026-07-31

### Added — `analysis` node type

- **`NodeType::Analysis`** (`analysis`; parse also accepts `analyses`)
- Default dir: **`docs/analysis/`** (bootstrap scaffolds it)
- **`note new --type analysis`** with optional light scaffold:
  Question/scope, When, Findings, Artifacts (e.g. criterion / `cargo bench` evidence),
  Recommendations (not a decision), Open questions / edge cases, Related
- Ranking / context pack mild boost (between concept and edge_case/ADR)
- Docs + AGENTS.md type guidance: concept vs analysis vs ADR

Analysis notes are **time-bound investigations** (crate compare, perf/bench review, design
options, data digests, …). They may recommend; **decisions still belong in ADRs**.

## [0.3.7] - 2026-07-31

### Added — orphans + soft auto-links (P0/P1)

- **`doctor` orphan count** — `orphans=N` in summary and finding `orphan_notes` when
  notes have **no explicit** edges (`auto_*` ignored). Hidden when count is 0.
- **`doctor --orphans` / `--orphan`** — detailed list with soft-link **suggestions**
  (filename stem + shared tags) without writing edges.
- **`links --auto` / `link --auto`** — create low-weight soft edges:
  - `auto_filename` (e.g. `docs/goals/rust-fluency.md` ↔ `docs/concepts/rust-fluency.md`)
  - `auto_tag` (shared non-trivial tags)
- **Targeted auto-link:** `rustbrain links --auto docs/goals/foo.md` (or node id).
- Soft edges are re-built on full `--auto`; do not count as explicit for orphan status.
- Library: `list_orphan_notes`, `run_auto_link`, `Brain::list_orphans` / `auto_link`.

## [0.3.6] - 2026-07-31

### Added / UX

- **`note new --body`** — visible alias of `--note` for the body after the H1 title
  (same field; either flag works).
- **Tip after `sync`, `doctor`, `setup`, and `rustbrain --help`** — recommended first
  goal example:

  ```bash
  rustbrain note new --type goal --title "Use rustbrain well" \
    --body "Prefer rustbrain context/query before large refactors. …"
  ```

## [0.3.5] - 2026-07-31

### Improved — doctor (knowledge density, non-presumptuous)

Algorithmic **info** findings when the brain is healthy but thin on prose:

| Code | When |
|------|------|
| `no_readme` | No root `README.md` (harvest cannot run) |
| `sparse_readme` | README very short |
| `thin_from_readme` | Harvest file exists but little body mass |
| `no_from_readme` | Expected harvest missing or README absent |
| `scaffold_only` | Only stubs/templates/thin harvest — no substantial notes |
| `knowledge_thin` | Few substantial notes vs many symbols |
| `no_agents_md` | Missing root `AGENTS.md` |

Does **not** invent content or fail the brain for sparse docs. Rich README harvest still counts as useful knowledge.

### Improved — `AGENTS.md` template

Built-in agent cookbook expanded with a **CLI variations table**: what each command does, flags, and expected behavior (`setup`/`bootstrap`/`doctor`/`query`/`context`/`note`/`links`/`watch`/`export`), plus cold-start notes for empty/thin README.

## [0.3.4] - 2026-07-31

### Documentation

- CLI reference and READMEs fully document **`AGENTS.md` bootstrap** (template
  resolution order, `--no-agents-md`, `--agents-template`, env var, interactive
  prompts, library APIs).
- Version pins and CLI doc header aligned to **0.3.4**.
- Root README quick start notes that `setup` writes `AGENTS.md` and shows opt-out /
  custom-template flags.

### Notes

- No functional changes vs 0.3.3 (same AGENTS.md feature set). Prefer `0.3.4` for
  correct published docs; `0.3.3` already ships the feature.

## [0.3.3] - 2026-07-31

### Added

- **`AGENTS.md` on bootstrap/setup** — writes a root agent cookbook (how to use
  rustbrain in this repo: setup, context, query, note new, conventions).
- **Configurable template** (first match wins):
  1. `--agents-template PATH` / `BootstrapOptions::agents_template`
  2. env `RUSTBRAIN_AGENTS_TEMPLATE`
  3. workspace `.rustbrain/AGENTS.template.md` or `AGENTS.template.md`
  4. built-in default ([`default_agents_md_template`](https://docs.rs/rustbrain-core))
- **Opt out:** `--no-agents-md` on `bootstrap` and `setup` (interactive bootstrap
  can also decline). Existing `AGENTS.md` is not overwritten without `--force`.
- Library: `default_agents_md_template()`, `resolve_agents_md_template()`,
  `BootstrapOptions::{write_agents_md, agents_template}`.

## [0.3.2] - 2026-07-31

### Polish — third dogfood pass

- **`doctor` walks parents** for `.brain` (same as `query`/`context`) so CWD inside `src/` works.
- **Context packing rank**: seeds before neighbors; ADR/goal before symbols — decisions surface first.
- **Strip YAML frontmatter** from packed excerpts (body-first for agents).
- **`context` default format is markdown** (use `-F xml` for tool protocols).
- **Empty query hints** suggest `--with-symbols` or `sync`/`doctor`.
- Tests: parent-walk doctor, frontmatter strip, ADR-over-symbol pack order.

## [0.3.1] - 2026-07-31

### Fixed / Improved — second dogfood round

Exercise-driven follow-ups after cold bootstrap on a parqview-like repo:

- **Hub fallback** for empty or *generic* prompts (`summarize architecture`,
  `what is this project about`) so agents still get README + module map.
- **README-family dedup** — pack either root `readme` or `from-readme`, not both
  near-identical bodies.
- **Skip ADR TEMPLATE** stubs in context packing.
- **`note new` auto-syncs** by default (`--no-sync` to skip) so agent-written notes
  are immediately queryable.
- **`query` is note-first** by default (`--with-symbols` / `--all-types` to include code).
- **Doctor** warns `adr_template_only` / `no_adrs` when decisions are missing.
- Stopword list includes `summarize`; generic-topic detection (`is_generic_topic`).

Regression tests cover NL `why egui not tauri`, generic overview fallback, and excerpt dedup.

## [0.3.0] - 2026-07-31

### Added

- **`rustbrain setup --yes`** — one-shot init + bootstrap + sync (+ doctor) for agents/CI.
- **Natural-language FTS rewrite** — stopword stripping + multi-token `OR` MATCH so prompts
  like `why egui not tauri` seed README/goal notes instead of returning zero hits.
- **Context body excerpts** — packs FTS content (truncated) into seeds/neighbors; Markdown/XML
  render `excerpt` / fenced body blocks (not title-only).
- **`Brain::open` parent walk** — finds `.brain/db.sqlite` in parents (git-style); `open_exact`
  for strict path open.
- **Bootstrap appends `.brain/` to `.gitignore`** when writing.
- **Empty-context hints** when packing yields zero nodes.
- CLI `context` accepts a **positional** prompt (`rustbrain context "topic"`) in addition to `-p`.

### Changed

- **Context defaults are note-first** (`ContextOptions::no_symbols = true`); symbols still hop
  in via anchors unless `--no-hop-symbols`. Use `--with-symbols` / `--all-types` for symbol seeds.
- Graph hops prefer **doc seeds**; low-signal symbol neighbors (theme consts, short noise) are
  filtered; max packed symbol neighbors capped.
- Multi-token coverage boosts ranking when several significant terms hit the same note.
- Init / error messages point at `rustbrain setup --yes`.

### Library

- `prepare_search_query`, `tokenize_query`, `PreparedQuery`, `find_brain_dir`
- `ContextNode.excerpt`, `ContextOptions::{hop_from_docs_only, include_excerpts, agent, with_symbols}`
- `Database::get_fts_content`, `Brain::open_exact`

## [0.2.0] - 2026-07-31

### Added

- **`rustbrain bootstrap`** — deterministic mature-repo setup:
  docs tree, ADR template, checklist, README harvest → `docs/goals/from-readme.md`,
  AST module map (generated), interactive or `--yes` non-interactive mode.
- **`.rustbrainignore`** — gitignore-inspired index filters; bootstrap can import
  `.gitignore` and recommended extras (interactive prompts on TTY).
- **`rustbrain doctor`** — schema/db/mmap health, type breakdown, pending links,
  symbol/note ratio; `--json`, `--strict`.
- **`rustbrain note new`** — `--type`, `--title`, `--note` (agent-friendly body),
  `--tags`, `--aliases`, `--force`, `--sync`.
- **`rustbrain links`** — list pending WikiLink / symbol refs (`--json`).
- **Query filters** — `--no-symbols`, `--type goal,adr,concept`, `--all-types`.
- **Context filters** — `--no-symbols`, `--no-hop-symbols`, `--type …`.
- **README hub** — root `README.md` indexes as node id `readme` (default `goal`)
  with hub aliases and ranking boost.

### Library

- Modules: `bootstrap`, `doctor`, `note`, `ignore`
- `QueryOptions::{human, no_symbols, include_types, exclude_types}`
- `ContextOptions::{no_symbols, hop_to_symbols, include_types, exclude_types}`
- `PendingLink`, `DoctorReport`, `BootstrapReport`, `NoteNewOptions`

## [0.1.1] - 2026-07-31

### Fixed

- **UTF-8 safe `symbol:` scanning** — indexing no longer panics when notes contain
  multi-byte characters (e.g. `→`) near symbol refs. Regression test included.
- **Per-file error isolation on sync** — a single failing Markdown/Rust/Canvas file
  is skipped and counted in `SyncStats.file_errors` instead of aborting the walk.

### Changed

- CLI `sync` output reports `file_errors=N`.

## [0.1.0] - 2026-07-30

### Changed — crates.io packaging

- **Only two published crates:** `rustbrain-core` (library) and `rustbrain` (CLI).
- Inlined former `rustbrain-ast` / `rustbrain-obsidian` as feature-gated modules
  inside `rustbrain-core` (`src/ast/`, `src/obsidian/`) so path-only deps never
  block publishing.

### Added — Docs / publish prep

- Expanded root and per-crate READMEs (install, architecture, honesty matrix).
- Crate-level and public-API `///` / `//!` rustdoc for `rustbrain-core` and the CLI.
- `CONTRIBUTING.md` and `docs/PUBLISHING.md` (two-crate publish order).
- `#![warn(missing_docs)]` on the library crate.

### Added — Phase B (useful engineer tool)

- Graph-aware `context_for_prompt` with hop expansion, score fusion, and token packing
  (`tokens_used`, seed vs neighbor roles, configurable `--hops`).
- Ranked query: FTS5 BM25 + title/id/tag/alias boosts + type priors (`query_ranked`,
  CLI `--scores`).
- Cross-workspace ranked merge (`--all-workspaces` + `GlobalRegistry::search_all_ranked`).
- Note → symbol anchors: `symbol:Name` / `symbol:crate::mod::Name` / `[[symbol:…]]`
  create `anchors` edges into the graph.
- AST: impl methods recorded as `Type::method`; symbol nodes get FTS + aliases;
  content-hash skip for unchanged symbols.
- CLI `watch` with debounced re-index + remmap (`--debounce-ms`).
- GitHub Actions CI (fmt, clippy `-D warnings`, test, doc, package leaves).
- docs.rs metadata (`all-features`) on library crates.

### Added — Phase A (honest prototype)

- `Brain` library façade: `create` / `open` / `open_or_create` / `sync` / `query` /
  `context_for_prompt` / `export` / `import` / `watch`.
- Typed `BrainError` (`thiserror`) at the library boundary.
- SQLite schema v1 with migrations (`schema_meta`), WAL + foreign keys, `content_hash`,
  `node_aliases`, `pending_links`.
- Idempotent FTS5 indexing (delete-then-insert by `node_id`) and FTS query escaping.
- Transactional Markdown indexing with two-phase WikiLink resolution.
- Stable path-slug node IDs (`docs/concepts/raft`).
- CSR `graph.mmap` format v1 (`RBRNMAP1`): bounds-checked reader, ID table, atomic replace.
- Portable `.brainbundle` v1 export/import preserving full edge metadata.
- CLI: `init`, `sync`, `query`, `context`, `export`, `import`, `watch` with non-zero exit codes.
- Feature flags: `ast`, `obsidian`, `mmap` (default), `watch`, `jshift`, `full`.
- Docs: README, SCHEMA.md, MMAP_FORMAT.md, dual MIT/Apache-2.0 licenses.

### Honest non-goals (still planned)

- Neural embeddings / hybrid vector search (vector dim is 0 in product path).
- Explicit AVX-512/NEON kernels.
- Full two-way Obsidian vault write-back.
- MainBrain / SubBrain `--scope` hierarchy.
