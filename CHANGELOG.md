# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
