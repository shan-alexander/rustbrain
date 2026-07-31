# rustbrain — Principal Engineer Audit (crates.io Readiness)

**Auditor:** Grok 4.3 (xAI)  
**Date:** 2026-07-30  
**Scope:** `/home/farmer/dev-other/rustbrain` only  
**Baseline:** `RUSTBRAIN_ARCHITECTURE_PLAN.md` + live source review + `cargo test --workspace` + end-to-end CLI smoke test  

---

## 0. Executive Verdict

**Not ready for crates.io.** The workspace is a coherent *scaffold* with real wiring (SQLite schema, basic FTS, CSR mmap compile/open, tree-sitter top-level symbols, WikiLink/frontmatter/canvas parsing, CLI skeleton). It is **not** a hardened v0.1 product matching the architecture plan.

Gemini’s roadmap checkboxes (Phases 1–5 marked complete) overstate maturity. Rough readiness score:

| Area | Score | Notes |
| :--- | :---: | :--- |
| Workspace skeleton / compiles | **7/10** | Builds; unit tests pass (8 total) |
| Plan fidelity (claimed features) | **2/10** | Core product APIs & hybrid query missing |
| Correctness / data integrity | **3/10** | FTS dupes, silent FK failures, export data loss |
| Performance claims (SIMD/mmap/sub-ms context) | **2/10** | Mmap CSR real; vectors empty; “SIMD” is unrolled scalar |
| Safety / systems hardening | **3/10** | Unchecked mmap views, no WAL/tx policy, public `conn` |
| Crates.io packaging | **1/10** | No description/readme/licenses/path→version deps |
| Docs / DX | **1/10** | No README, no library facade, CLI panic on `context` |
| Test depth | **2/10** | Happy-path unit tests only; no integration/CLI tests |

**Recommendation:** Treat current code as **0.0.x prototype**. Target a honest **v0.1.0** after the P0/P1 list below, then publish as a multi-crate workspace with a clear feature gate (keyword graph + AST anchors first; embeddings/hybrid search as 0.2).

---

## 1. What Actually Exists (Honest Inventory)

### 1.1 Crates

| Crate | Role today | Publishable as-is? |
| :--- | :--- | :--- |
| `rustbrain-core` | SQLite DB, indexer, CSR mmap, jshift wrappers, registry, exporter | No |
| `rustbrain-ast` | tree-sitter-rust top-level symbol extract + BLAKE3 u64 hash | No |
| `rustbrain-obsidian` | Frontmatter, hand-rolled WikiLinks, Canvas JSON | No |
| `rustbrain` (CLI bin) | `init` / `sync` / `query` / `context` / `export` | No |

~21 Rust source files. No README, LICENSE*, CI, CHANGELOG, examples, benches, or integration tests.

### 1.2 What works in smoke test

On a temp project with two Markdown notes + one `src/lib.rs`:

- `rustbrain init` creates `.brain/db.sqlite`
- `rustbrain sync` indexes 2 concept nodes, 2 edges (when both ends exist), 2 symbol anchors, writes `graph.mmap`
- `rustbrain query "raft"` returns FTS hits
- `rustbrain export` writes JSON `.brainbundle`

### 1.3 Confirmed production bugs (reproduced)

1. **`context` panics in debug builds** — clap short-flag collision: both `--for-prompt` and `--format` claim `-f` (`clap_builder` debug_assert). Command is unusable as written.
2. **FTS table grows duplicates on every re-sync** — after two `sync` runs: 2 nodes, **4** `node_fts` rows. `index_fts` only `INSERT`s; never deletes/updates by `node_id`.
3. **WikiLink edges to missing targets fail silently** — `insert_edge` FK errors swallowed via `let _ = self.db.insert_edge(...)`. Broken links vanish without diagnostics.
4. **AST misses impl methods / nested items** — only direct children of the root node (`function_item`, `struct_item`, …). `impl StorageEngine { fn open() }` not indexed.
5. **`max_tokens` is ignored** — bound as `_max_tokens`; context path is “open mmap + FTS take(5)”, not token-budgeted assembly.
6. **Vector path is dead** — `compile_mmap` always passes `vectors: None, vector_dim: 0`. “SIMD vector search” is never exercised by the product path.
7. **Export corrupts edge semantics** — `get_all_edges` returns only `(src,dst,weight)`; exporter hardcodes `relation_type: "relates_to"` and fabricates `created_at` / `decay_rate`.

---

## 2. Architecture Plan vs Implementation Gap Matrix

| Plan promise | Status | Gap |
| :--- | :---: | :--- |
| `Brain::open` / `context_for_prompt` library API | ❌ | Does not exist; CLI only |
| Hybrid BM25 + vector + graph query | ❌ | FTS only |
| Sub-ms AI context with graph expansion | ⚠️ | Mmap open is fast; content path still SQLite FTS |
| CSR mmap layout with **symbol-hash index** | ❌ | Offsets/targets/weights only; no hash index section |
| Magic `"RUSTBRAIN"` | ⚠️ | Code uses `"RUSTBRAN"` (8B) — document or fix |
| 64-byte aligned SIMD vector matrix | ❌ | No alignment padding |
| Embeddings generation / storage | ❌ | No model, no embedding table, no offline stub |
| jshift zero-copy JSONL mutation pipeline | ⚠️ | Thin wrappers + unit test; unused by indexer/export/CLI |
| Tree-sitter **incremental** parse | ❌ | Full reparse only; no old tree reuse |
| Location-independent symbol hash (`crate::module::sig`) | ❌ | Hash includes **file path**; module_path = file path |
| Symbol nodes in graph + anchors | ⚠️ | Anchors only; no `NodeType::Symbol` nodes / edges from notes |
| Bidirectional backlinks / hub ranking | ❌ | Forward edges only |
| MainBrain / SubBrain + `workspace.json` | ❌ | Absent |
| `--scope` query | ❌ | Absent |
| `--all-workspaces` | ⚠️ | Present; no stale-path pruning, no ranking merge |
| Obsidian **two-way** sync | ❌ | One-way ingest only; no vault write-back |
| `sync --obsidian-vault` | ❌ | `sync` indexes CWD only |
| pulldown-cmark WikiLink engine | ❌ | Dependency unused; custom byte scanner |
| Decoupled Layer A/B export + AST sidecar | ⚠️ | Nulls path/hash only; no sidecar; symbols not exported |
| Import brainbundle | ❌ | Export only |
| File watcher live reindex + remmap <50ms | ⚠️ | `start_file_watcher` exists, not wired to CLI; no remmap; blocks forever |
| Schema migrations | ❌ | `CREATE IF NOT EXISTS` only |
| FTS5 explicitly enabled | ⚠️ | Works today via bundled sqlite defaults; not declared in features |

**Bottom line:** Phases 1–5 are scaffolded, not delivered. Do not market “fully implemented.”

---

## 3. Critical Correctness & Systems Issues

### 3.1 Storage / SQLite (P0)

| ID | Issue | Severity | Fix direction |
| :--- | :--- | :--- | :--- |
| S1 | FTS insert without delete/replace | **P0** | `DELETE FROM node_fts WHERE node_id=?` then insert, or use FTS content-sync table / external content FTS |
| S2 | Edge insert errors discarded | **P0** | Transactional index pass: create stub nodes for unresolved links, or queue “pending edges” and resolve in second pass; surface counts |
| S3 | No transactions around note index | **P0** | Single `BEGIN IMMEDIATE` … commit per file (or batch); crash mid-file leaves half state |
| S4 | No WAL / busy timeout / foreign_keys re-check | **P1** | `PRAGMA journal_mode=WAL; busy_timeout=5000;` keep `foreign_keys=ON` on every connection |
| S5 | `pub conn: Connection` | **P1** | Encapsulate; expose typed APIs only |
| S6 | No schema version / migrations | **P1** | `schema_version` table + numbered migrations before any public release |
| S7 | FTS query string passed raw to `MATCH` | **P1** | Escape FTS5 query syntax; handle empty/special queries without SQL/FTS errors |
| S8 | `NodeType::from_str` falls back to `Concept` | **P2** | Reject unknown types or preserve raw string |
| S9 | Timestamps always “now” on reindex | **P2** | Preserve `created_at`; only bump `updated_at` when content hash changes |
| S10 | Absolute/relative path inconsistency | **P2** | Store repo-relative paths only; canonicalize against workspace root |
| S11 | `rusqlite` missing explicit `fts5` feature | **P2** | `features = ["bundled", "fts5"]` for explicit contract |
| S12 | Tags never deleted on reindex | **P2** | Replace tag set per node |

### 3.2 Mmap / CSR / “SIMD” (P0–P1)

| ID | Issue | Severity | Fix direction |
| :--- | :--- | :--- | :--- |
| M1 | File size not validated against N/E/D | **P0** | After header parse, compute expected length; reject short/truncated maps |
| M2 | `from_raw_parts` on potentially unaligned mmap slices | **P0** | Use `bytemuck`/`zerocopy` with alignment padding, or copy to aligned buffers; never assume `u32`/`f32` alignment from arbitrary file offset |
| M3 | Endianness is host-LE assumed forever | **P1** | Document LE format version; reject unknown version |
| M4 | Layout diverges from plan (no symbol hash index, magic) | **P1** | Freeze v1 binary format in a `FORMAT.md`; version field must gate readers |
| M5 | No atomic remap publish | **P1** | Write `graph.mmap.tmp` + `rename` for crash-safe bake |
| M6 | “SIMD” is scalar unrolled loop | **P2** | Honest docs, or real `std::simd` / platform intrinsics with scalar fallback; add benches |
| M7 | `top_k_vector_search` full scan + full sort | **P2** | Acceptable for v0.1 if N small; later HNSW/IVF — don’t claim production ANN |
| M8 | No node-id ↔ index side table in mmap | **P1** | Plan’s hash index or companion `ids.json`/string table needed for useful agent context |
| M9 | Product path never writes vectors | **P0** (product) | Either ship embedding pipeline (even deterministic hash-embed stub for tests) or drop vector claims from v0.1 marketing |

### 3.3 Indexer / Graph Semantics (P0–P1)

| ID | Issue | Severity | Fix direction |
| :--- | :--- | :--- | :--- |
| I1 | Node IDs = lowercased file stem only | **P0** | Collisions across dirs (`docs/a/foo.md` vs `docs/b/foo.md`); use stable slug from relative path or UUID + alias index |
| I2 | Title = file stem, not H1 / frontmatter title | **P1** | Prefer `title:` frontmatter, else first H1, else stem |
| I3 | No content hash → always full rewrite | **P1** | BLAKE3 content hash; skip unchanged files |
| I4 | Walk skips only `target` and `.*` | **P1** | Respect `.gitignore` / configurable ignore list (`node_modules`, `vendor`, …) |
| I5 | Symbols not linked into node/edge graph | **P1** | Insert symbol nodes; parse `symbol:…` anchors in notes |
| I6 | No automatic reverse edges / backlinks | **P1** | Materialize `backlink` or query-time invert; plan promised boost |
| I7 | Canvas edges use raw canvas node names as IDs | **P1** | Resolve file nodes to same ID scheme as Markdown nodes |
| I8 | Watcher doesn’t recompile mmap / index `.rs`/`.canvas` | **P1** | Debounced reindex + mmap bake; or drop watcher from v0.1 surface |
| I9 | Watcher API takes `&self` but owns nothing sendable | **P2** | Redesign for long-lived service process |

### 3.4 AST (P1)

| ID | Issue | Severity | Fix direction |
| :--- | :--- | :--- | :--- |
| A1 | Non-recursive walk | **P1** | Tree cursor / query for nested `impl_item` methods, associated types, etc. |
| A2 | Hash includes file path (location-dependent) | **P0** | Hash `crate + module_path + item_kind + signature`; keep path only in anchor row |
| A3 | `module_path` is filesystem path | **P1** | Derive `mod` path from AST + `mod.rs` layout |
| A4 | No `impl` / trait impl / macro / const / type alias | **P1** | Expand grammar coverage intentionally |
| A5 | Doc comments only immediate prev sibling | **P2** | Collect contiguous `///` / `/**` runs |
| A6 | No incremental tree-sitter | **P2** | Optional later; not required for v0.1 honesty |
| A7 | Multi-language claimed by vision, only Rust | **P2** | Feature-gate languages; document Rust-only v0.1 |

### 3.5 Obsidian (P1)

| ID | Issue | Severity | Fix direction |
| :--- | :--- | :--- | :--- |
| O1 | `pulldown-cmark` unused | **P1** | Use it for Markdown structure **or** remove dependency |
| O2 | WikiLink scanner ignores code spans/blocks | **P1** | Don’t extract `[[x]]` inside fences/inline code |
| O3 | No aliases → ID resolution | **P1** | Frontmatter `aliases` must resolve links |
| O4 | No write-back / frontmatter round-trip emitter | **P1** | If “two-way” is a goal, implement emit; else document one-way import |
| O5 | `serde_yaml` is **deprecated** | **P1** | Migrate to `serde_yml` / `saphyr` / similar maintained crate |
| O6 | Frontmatter parse fails open (returns full doc as body) | **P2** | Surface parse errors |
| O7 | Canvas incomplete vs Obsidian schema | **P2** | Support color, groups, fromEnd/toEnd as needed |

### 3.6 Exporter / Registry / Portability (P1)

| ID | Issue | Severity | Fix direction |
| :--- | :--- | :--- | :--- |
| E1 | Edge metadata loss on export | **P0** | Select full edge rows |
| E2 | No import path | **P1** | `BrainImporter` symmetric to export |
| E3 | No tags / FTS / symbol_anchors in bundle | **P1** | Define `.brainbundle` schema v1 (versioned) |
| E4 | `decouple_ast` only nulls fields; keeps symbol *nodes* if any | **P1** | Filter `NodeType::Symbol` + edge types; optional sidecar |
| E5 | Registry only uses `$HOME` | **P1** | XDG on Linux, known folders on macOS/Windows (`dirs` crate) |
| E6 | Registry never GC’s dead workspaces | **P2** | Prune missing paths on load |
| E7 | jshift not on critical path | **P2** | Use for JSONL stream export **or** drop from v0.1 deps |

### 3.7 CLI (P0)

| ID | Issue | Severity | Fix direction |
| :--- | :--- | :--- | :--- |
| C1 | `-f` short option collision on `context` | **P0** | Unique shorts; add clap tests / `debug_assert` CI on release too via trycmd |
| C2 | `context` does not use graph neighborhood | **P0** | Rank FTS seeds → k-hop expand via mmap → format with token budget |
| C3 | No `--scope`, no vault flag | **P1** | Match plan or cut from docs |
| C4 | Errors printed as success-ish paths | **P1** | Non-zero exit codes; structured errors |
| C5 | Emoji logs in CLI | **P3** | Fine; offer `--quiet` / NO_COLOR |
| C6 | No `watch` subcommand | **P2** | Wire or remove dead code |
| C7 | Binary package is only bin — no lib re-export | **P1** | Publish `rustbrain` as lib+bin **or** document `rustbrain-core` as the lib crate |

---

## 4. API & Product Design Gaps (Library Consumers)

The plan’s flagship API:

```rust
let brain = Brain::open("./.brain")?;
let context = brain.context_for_prompt(prompt, max_tokens)?;
```

**Does not exist.** External crates cannot depend on a clean façade.

### Recommendations for a real v0.1 library surface

```text
rustbrain-core (or re-export crate `rustbrain`)
├── Brain::open / create
├── Brain::sync_workspace
├── Brain::query(Query) -> QueryResult   // FTS (+ optional graph expand)
├── Brain::context_for_prompt(...) -> ContextBundle
├── Brain::export / import
├── typed errors: BrainError (thiserror)
└── feature flags: `ast`, `obsidian`, `mmap`, `watch`
```

Requirements:

1. **Stable error type** — `thiserror` is already a dependency but **never used**; replace `anyhow` at the library boundary (keep `anyhow` in CLI only).
2. **No public `rusqlite::Connection`.**
3. **Documented on-disk format** for `.brain/` (sqlite schema version + mmap version).
4. **Doctests** for the happy path.
5. **MSRV** policy (e.g. 1.80+) in workspace `rust-version`.

---

## 5. crates.io Publishing Checklist

### 5.1 Hard blockers (must fix before any publish)

- [ ] **Valid package metadata on every published crate**
  - `description` (required for good listing)
  - `readme = "README.md"`
  - `license` + actual **LICENSE-MIT** and **LICENSE-APACHE** files in repo
  - `repository`, `homepage`, `documentation` (docs.rs)
  - `keywords` (≤5), `categories` (e.g. `command-line-utilities`, `text-processing`, `database`)
  - `authors` (real maintainers, not “Antigravity & Open Source Community”)
- [ ] **Replace `path` dependencies with versioned deps for publish**
  - e.g. `rustbrain-core` depends on `rustbrain-ast = { version = "0.1.0", path = "..." }`
  - Publish order: `rustbrain-ast` → `rustbrain-obsidian` → `rustbrain-core` → `rustbrain` (CLI)
- [ ] **Name availability** — confirm `rustbrain`, `rustbrain-core`, etc. are free on crates.io; have a fallback (`rbk`, `rust-brain`, …)
- [ ] **Repository URL must exist** — current `https://github.com/rustbrain/rustbrain` looks placeholder
- [ ] **Fix CLI panic** (`context` short flags) before anyone `cargo install`s
- [ ] **Fix FTS reindex corruption** (duplicate rows)
- [ ] **Minimal README** with install, quickstart, feature honesty, non-goals
- [ ] **`cargo publish --dry-run` for each crate** clean

### 5.2 Strongly recommended before 0.1

- [ ] CI: `fmt`, `clippy -D warnings`, `test`, `doc`, `cargo deny` (licenses/advisories)
- [ ] `CHANGELOG.md` (Keep a Changelog)
- [ ] `CONTRIBUTING.md` + issue templates
- [ ] Examples: `examples/basic_query.rs`, `examples/agent_context.rs`
- [ ] Integration tests under `tests/` with `assert_cmd` / `predicates` for CLI
- [ ] `include` / `exclude` in manifests (don’t ship `target/`, `test_brain/`, huge plan drafts if undesired)
- [ ] Feature flags to keep default dep tree lean:
  - default = sqlite + markdown
  - `ast` → tree-sitter (heavy)
  - `watch` → notify
  - `mmap` → memmap2
- [ ] Pin policy: workspace deps OK; avoid `serde_yaml` deprecated; upgrade `jshift` from `0.1` → current if API stable (`0.7` exists on crates.io — currently locked to ancient `0.1.0`)
- [ ] Binary install story: `cargo install rustbrain --locked`
- [ ] docs.rs metadata: `cargo-args`, feature docs, `#![deny(missing_docs)]` on public API gradually

### 5.3 Workspace structure recommendation for publish

```text
Option A (preferred):
  crates/rustbrain-ast
  crates/rustbrain-obsidian
  crates/rustbrain-core      # main lib
  crates/rustbrain-cli       # package name `rustbrain`, bin only
  + root README that documents all

Option B:
  Single crate `rustbrain` with modules + optional features
  (simpler publish, worse compile times for consumers who only want Markdown)
```

Path deps today make **Option A require coordinated multi-crate release**. Automate with `cargo-release` or `cargo workspaces`.

### 5.4 Legal / supply chain

- Dual MIT/Apache-2.0 is fine; add both license texts.
- Audit transitive licenses (`cargo deny`).
- `tree-sitter-rust` / bundled sqlite increase build time — document `build-dependencies` / network-free builds.
- Do not commit `.brain/` or local DB artifacts; add `.gitignore` entries if missing (`target/` already via cargo; verify `.brain/`).

---

## 6. Testing Gap Analysis

Current: **8 unit tests**, all happy-path, no failure modes.

### Must-have test matrix for v0.1

| Layer | Tests |
| :--- | :--- |
| DB | upsert node, FK cascade, FTS replace idempotent reindex, FTS special chars, concurrent open |
| Mmap | roundtrip CSR, reject truncated file, reject bad magic/version, alignment-safe reads, empty graph |
| Indexer | collision paths, missing wikilink targets, code-fence wikilinks ignored, canvas→edges, ignore dirs |
| AST | impl methods, nested mods, stable hash across file move (after A2 fix), doc comments |
| Obsidian | alias resolution, frontmatter round-trip, malformed YAML |
| Export/Import | relation_type preserved, decouple_ast filters symbols, import reopens searchable |
| CLI | trycmd/assert_cmd for init/sync/query/context/export; exit codes |
| Registry | XDG paths, missing workspace prune |
| Benches | mmap neighbor lookup; FTS query; full sync on fixture repo (criterion) |

**Do not claim sub-millisecond** without a bench harness and published numbers on defined hardware.

---

## 7. Performance Honesty Check

| Claim | Reality |
| :--- | :--- |
| Sub-ms context generation | Opening a tiny mmap is sub-ms; **building useful context still hits SQLite FTS** and ignores graph expansion / token budgets |
| SIMD AVX-512 / NEON | Manual 8-wide scalar FMA-ish sum; may auto-vectorize sometimes — **not** explicit SIMD |
| Zero-copy | Mmap neighbor views yes (if alignment fixed); JSON path uses `Vec` mutation; Markdown fully owned `String`s |
| Incremental AST | Full reparse every file every sync |
| Live remmap <50ms | Not implemented end-to-end |

**v0.1 messaging should be:**  
“Fast local SQLite + optional CSR graph cache for neighborhood expansion. Embeddings/hybrid search planned.”

---

## 8. Security & Robustness Notes

1. **Mmap trust model:** `.brain/graph.mmap` is a local cache; still validate bounds to avoid UB on corruption (UB is a security issue in multi-agent environments).
2. **Path traversal:** Indexer walks user workspace; ensure it never follows symlinks outside root if used on untrusted trees (`symlink_metadata` policy).
3. **SQL:** Parameterized queries are good; FTS `MATCH` still needs query sanitization.
4. **XML/Markdown context emission:** No escaping — agent context can break XML if titles contain `<>&`. Escape for XML format.
5. **Registry writes under HOME:** Use secure create (`0o755` dirs), avoid following symlinks for config replace (write temp + rename).

---

## 9. Dependency Hygiene

| Dep | Issue | Action |
| :--- | :--- | :--- |
| `uuid` | Declared, **never used** in code | Remove or use for node IDs |
| `thiserror` | Declared, **never used** | Implement `BrainError` |
| `pulldown-cmark` | Declared, **never used** | Use or remove |
| `jshift` | Locked to **0.1.0**; crates.io has **0.7.x** | Upgrade or drop until needed |
| `serde_yaml` | Marked **deprecated** | Replace |
| `notify` | Pulled into core always | Feature-gate |
| `tree-sitter*` | Heavy native build | Feature-gate `ast` |
| `rusqlite` bundled | Good for portability | Document build tools (C compiler) |

---

## 10. Recommended Path to crates.io (Phased)

### Phase A — “Honest Prototype Hardening” (pre-0.1, ~1–2 weeks focused)

1. Fix **C1** clap panic, **S1** FTS idempotency, **E1** export edges, **S2/S3** transactional indexing with resolved/pending links.  
2. Add `Brain` façade + `BrainError`.  
3. Freeze on-disk formats (`SCHEMA.md`, `MMAP_FORMAT.md`) with versions.  
4. Fix mmap bounds + alignment (**M1/M2/M5**).  
5. Node ID scheme based on relative path slug.  
6. README + licenses + package metadata.  
7. Integration + CLI tests; `cargo clippy -D warnings`.  
8. Strip or gate unused deps; feature flags.

**Exit criteria:** `cargo test`, `cargo publish --dry-run` all crates, CLI smoke without panic, re-sync FTS row count stable.

### Phase B — “v0.1.0 Useful Engineer Tool”

1. Graph-aware `context_for_prompt` with token budget + XML escape.  
2. Symbol nodes + note→symbol links; improved AST coverage (impl methods).  
3. Location-independent symbol hashes.  
4. Import bundle; tags/aliases in query.  
5. Optional `watch` subcommand with debounced remmap.  
6. XDG registry; `--all-workspaces` merge ranking.  
7. CI + docs.rs.

**Ship claim:** project-scoped Markdown 2nd brain + Rust AST anchors + FTS + CSR neighborhood context. **Do not claim** hybrid embeddings yet.

### Phase C — “v0.2.0 Plan Fidelity”

1. Embeddings (pluggable backend: hash stub / candle / external API).  
2. Hybrid retrieval fusion (BM25 + vector + graph).  
3. MainBrain/SubBrain + `--scope`.  
4. True Obsidian vault sync.  
5. Multi-language AST features.

---

## 11. Priority Backlog (Actionable)

### P0 — Do before any public release

1. Fix CLI `context` flag collision (reproduced panic).  
2. Make FTS reindex idempotent.  
3. Transactional markdown index + non-silent edge failures.  
4. Validate mmap length/magic/version; eliminate unsound unaligned casts.  
5. Preserve full edge records on export.  
6. Introduce `Brain` API matching the README you will publish.  
7. crates.io metadata, LICENSE files, real repository URL, versioned path deps.  
8. README with accurate feature list and limitations.

### P1 — Required for a respectable v0.1

9. Schema migrations + WAL.  
10. Stable relative-path node IDs + title resolution.  
11. Graph expansion in context assembly; honor `max_tokens`.  
12. AST: recursive items, hash without file path, symbol nodes.  
13. Feature flags; remove/replace dead & deprecated deps.  
14. Import + versioned brainbundle schema.  
15. Integration/CLI tests + clippy CI.  
16. Escape agent output formats; non-zero exit codes.  
17. Document binary formats; atomic mmap replace.

### P2 — Soon after 0.1

18. Backlinks / hub nodes.  
19. SubBrain scopes.  
20. Watcher productization.  
21. Real SIMD + benches.  
22. Embedding pipeline.  
23. Obsidian two-way sync.  
24. Multi-language parsers.

---

## 12. Code Quality Observations (Principal-Level)

**Positives**

- Clean workspace split matching the plan’s crate boundaries.  
- SQLite schema largely matches the design doc.  
- CSR compile/load + unit test is a solid kernel to build on.  
- WikiLink/frontmatter/canvas parsers are small and understandable.  
- Smoke path `init → sync → query → export` basically works.

**Anti-patterns to retire**

- `let _ =` on fallible index operations (error swallowing).  
- Plan checkboxes marked complete without acceptance tests.  
- Marketing-grade comments (“AVX-512”, “zero-copy”, “sub-millisecond”) adjacent to scalar/unfinished code.  
- Library using `anyhow::Result` everywhere — no typed errors for consumers.  
- Giant God-objects avoided (good) but also no orchestration type (`Brain`) so CLI reimplements policy.  
- Re-index strategy is “blind upsert + append FTS” rather than content-addressed sync.

**Suggested internal standard for this repo**

> Every plan bullet that remains checked must have: (1) a public API or CLI flag, (2) a test that fails if removed, (3) a README mention that is literally true.

---

## 13. Suggested Immediate Patch List (Small, High Leverage)

If implementing before the next review, order is:

1. **clap** — rename shorts (`-p` for prompt, `-F` for format) or drop shorts.  
2. **`index_fts`** — delete-by-node_id then insert; add test `sync_twice_fts_count_stable`.  
3. **`insert_edge` path** — two-phase link resolve; count `edges_created` / `edges_pending`.  
4. **`get_all_edges`** — return full `Edge`.  
5. **mmap open** — expected_len check before slice ops.  
6. **Package metadata + README + LICENSE***.  
7. **`Brain` wrapper** with `open`, `sync`, `query`, `context_for_prompt` (even if context is FTS-only at first — name it honestly).

---

## 14. Final Recommendation

Publish **only after Phase A**. Current code is a promising architecture demo, not a crates.io-grade knowledge engine. Reframe v0.1 around **reliable local Markdown+FTS+graph cache for Rust repos**, then earn the hybrid/SIMD/Obsidian claims in 0.2+.

The architecture plan is ambitious and largely sound. The implementation needs principal-level finishing: **data integrity, format freezes, typed APIs, tests, packaging, and honest docs** — not more surface-area scaffolds.

---

*End of audit. Artifact path: `rustbrain/GROK_AUDIT.md`.*
