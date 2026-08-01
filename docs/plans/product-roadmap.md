---
node_type: plan
status: in_progress
tags: [roadmap, product, hitl, agents]
aliases: [product-roadmap, rustbrain-roadmap, remaining-roadmap, v0.4-plan]
---
# Product roadmap (post-0.3.x)

## Status

in_progress

Overall: **0.3.x is the shippable local Markdown + FTS5 + CSR graph + agent CLI product.**  
This plan tracks **remaining** work — near-term polish and three vision epics.  
Rustbrain densifies status into FTS on `sync` (`status:in_progress`, checkbox tokens).  
It does **not** invent tasks or ship history; edit this file and re-`sync`.

Hub: root [[ROADMAP]] (stable id `roadmap`) points here.

## Intent

Keep a single HITL-visible plan so humans and agents share the same backlog:

1. Harden what agents already use every turn (`context` / `query` / `graph` / `note` / `sync` / `links` / `doctor`).
2. Sequence the three architecture epics without over-claiming them before they exist:
   - Neural embeddings (hybrid retrieval)
   - Full two-way Obsidian write-back
   - Multi-brain `--scope` SubBrain
3. Preserve product laws: **Markdown is truth**, `.brain/` is disposable cache, **no invented ADRs/changelogs**, algorithmic engine.

Related goals (when present): ship offline agent context, code↔docs edges, Git-friendly knowledge.

## Priority / order

1. **Near-term 0.3.x polish** (quality, tests, docs honesty) — current focus for most cycles  
2. **SubBrain `--scope`** — structure only; high monorepo value; no model weights  
3. **Obsidian write-back (narrow first)** — frontmatter-safe + vault path; Canvas later  
4. **Neural embeddings** — only when a real offline-capable backend lands and `graph.mmap` product path uses `D > 0`  
5. Explicit SIMD/ANN marketing — only after measured wins (not a gate for 1–4)

## In Progress

- [/] Maintain first-class roadmap plan + root `ROADMAP.md` hub (this note)
- [/] Keep crate READMEs + [[docs/benchmarks]] + [[docs/cli]] aligned with shipped behavior
- [/] Criterion package `crates/bench` (engine + serde/jshift) as ongoing truth for perf claims

## Backlog

### Near-term polish (0.3.x line)

- [x] Multi-brain UX 0.3.22: detect, cwd auto-scope, graph --scope, import WikiLink rewrite, by_scope sync, setup --multi-cargo, CLI scopes test
- [ ] Integration-test coverage for link apply tiers / plan densify / hubs
- [x] Doctor rules for scopes (orphan/empty/missing roots)
- [x] Sync stats by scope; registry prune on load
- [ ] AST depth: impl/trait/macro coverage; multi-lang only behind explicit features
- [ ] Canvas read completeness (groups, colors) without requiring write-back
- [ ] Bundle schema completeness (further)
- [ ] `links --apply` SUGGEST human/agent review loop UX

### Epic A — Multi-brain `--scope` SubBrain (multi-crate / multi-root)

**Detailed design + phases S0–S4:** root hub [[ROADMAP]] § *MainBrain + SubBrain*.

- [x] Scope manifest in `.brain/workspace.json` (path prefix → flat scope id); **opt-in** multi
- [x] Cargo workspace member discovery + `scopes add --root` multi-root
- [x] Path-stable scope ids; package name as alias when different
- [x] `nodes.scope` set at index time (schema v2)
- [x] CLI: `scopes list|enable|disable|add|remove|absorb|import`; `query`/`context`/`note` `--scope`
- [x] Scoped retrieval = in-scope seeds + MainBrain (default) or `--scope-strict`
- [x] Absorb SubBrain → MainBrain; import copy / mount / merge into MainBrain
- [x] Doctor: empty / orphan / missing roots / mode mismatch
- [x] Umbrella attach+mount; export --scope; SQL scope filter; hubs-only default
- [x] One workspace graph (single mmap v1); no per-crate `.brain/` by default

### Epic B — Full two-way Obsidian write-back

- [ ] Vault path flag / config (`sync --obsidian-vault` or `obsidian push|pull`)
- [ ] Frontmatter round-trip preserving unknown keys (plugin-safe)
- [ ] WikiLink normalize write-back (extend `links --apply` toward vault target)
- [ ] Dry-run + content-hash conflict policy (Git-first recommended)
- [ ] Canvas write-back (optional later; read-only Canvas may remain default)
- [ ] Document mobile Obsidian → Git → `watch`/`sync` loop

### Epic C — Neural embeddings (hybrid search)

- [ ] Pluggable embed backend: deterministic stub (tests) + offline local model path
- [ ] Embed on `sync` only when content hash changes
- [ ] Store vectors; bake `graph.mmap` with `D > 0` on product path when enabled
- [ ] Hybrid `query`: fuse FTS BM25 + vector top-k + existing type/hub boosts
- [ ] `context` uses hybrid seeds then CSR hops (embeddings do not replace graph)
- [ ] Feature-gate; never require cloud for core
- [ ] Re-embed versioning when model changes
- [ ] ANN (HNSW etc.) only if N justifies it; brute-force mmap top-k fine for small brains

### Explicit non-goals (until decided otherwise)

- [~] Inventing ADR/changelog history for agents
- [~] Replacing Markdown with a proprietary note DB
- [~] Claiming hybrid/SIMD/ANN before product path ships them
- [~] Deep nested SubBrain trees (flat siblings only — architecture rule)

## QA

- [?] After any epic lands: update README “Does / Does not”, [[docs/cli]], [[changelog]], and this plan’s Done section
- [?] Perf claims re-run via `cargo bench -p bench --bench rustbrain_performance`

## Done

- [x] 0.3.x core product: setup/bootstrap, FTS ranked query, graph-aware context, CSR mmap (`D=0`), AST symbols, hubs, plans/changelog densify, links apply, docs.rs harvest, export/import, watch, published CLI + core
- [x] Honest “does not yet” messaging for embeddings / two-way Obsidian / SubBrain scope
- [x] JSON stack decision (serde primary; jshift optional sparse) — [[docs/json_stack]]
- [x] Engine performance baselines documented — [[docs/benchmarks]]

## Blocked

- [!] Cloud-only embedding default — blocked by offline-first product law (need local backend first)
- [!] Full Canvas layout fidelity write-back — blocked on narrow Markdown/frontmatter write-back shipping first

## Cancelled

- [~] Separate published `rustbrain-ast` / `rustbrain-obsidian` crates (folded into `rustbrain-core` features)
- [~] `.kg` as primary storage (rejected; Markdown + WikiLinks)

## Epic design notes (for implementers)

### A. SubBrain `--scope`

```text
MainBrain (workspace root goals / cross-cutting ADRs)
    ├── SubBrain "core"
    ├── SubBrain "biology"
    └── SubBrain "math"     # flat siblings only
```

- One canonical owner scope per node; WikiLinks may cross scopes.
- `query --scope X` filters seeds but may hop to neighbors outside X.
- Distinct from `query --all-workspaces` (machine-wide registry).

### B. Obsidian two-way

- **Today:** one-way compatible ingest (frontmatter, WikiLinks, Canvas read).
- **Write-back:** update human Markdown (and optionally vault mirror), never invent notes.
- Prefer Git-first conflict policy; dry-run required for agents.

### C. Embeddings

- **Today:** FTS5 + boosts; mmap vector dim `D = 0`.
- **Future:** embed changed nodes on sync → optional vector matrix → hybrid fusion for seeds; CSR hops unchanged.
- Stub embeddings only for tests — do not market as neural search.

## Related

- [[ROADMAP]] — root hub (`roadmap`)
- [[changelog]] — ship history (not this plan)
- [[docs/benchmarks]] — measured performance
- [[docs/cli]] — command book
- [[docs/json_stack]] — serde vs jshift
- [[docs/schema]] · [[docs/mmap_format]] — storage / CSR
- Architecture vision (repo root): `RUSTBRAIN_ARCHITECTURE_PLAN.md` (aspirational; this plan is the live backlog)
- Agent cookbooks: `AGENTS.md`, `docs/AGENTS.md` when present

## Out of scope

- Building a SaaS second-brain product
- Neural *generation* of ADRs or release notes
- Replacing Git as the collaboration surface for notes
