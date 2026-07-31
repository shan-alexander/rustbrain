# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
