# rustbrain performance benches

Criterion harness: package **`bench`** (`publish = false`) at [`crates/bench`](../crates/bench).

```bash
# Engine vs intentional alternatives (search, context, graph, sync, …)
cargo bench -p bench --bench rustbrain_performance

# JSON stack decision (serde_json vs jshift) — see also JSON_STACK.md
cargo bench -p bench --bench serde_vs_jshift

# Fast smoke (fewer samples)
cargo bench -p bench --bench rustbrain_performance -- --quick
```

HTML reports: `target/criterion/report/index.html` after a run.

## Fairness

Baselines are **not** peer products. They model approaches rustbrain deliberately avoided as the *primary* path:

| Workload | rustbrain | Baseline |
|----------|-----------|----------|
| Search | FTS5 BM25 + title/tag/type boosts | Walk every `.md` + substring; or SQLite `LIKE` full scan of the same FTS body rows |
| Context | Ranked seeds + CSR hops + token budget | Path-order file concat; or walk-grep rank then concat |
| Neighborhood | SQLite edge BFS **or** CSR `graph.mmap` k-hop | Re-parse all WikiLinks from disk, then BFS |
| WikiLinks | Fence / inline-code aware scanner | Naive `[[…]]` regex (counts fenced false positives) |

Quality differs. Faster baseline times (e.g. path-order concat, naive regex) often mean **worse agent outcomes**. Prefer ratios on **comparable quality** rows (search vs walk, graph vs re-parse).

Fixture: synthetic workspace with *N* concept notes, WikiLinks, ADRs, a plan, README/CHANGELOG hubs, one Rust file. Built once per size; query/context/graph measure the hot path after `sync`.

## Results (release, x86_64 Linux, Criterion `--quick`, 2026-08)

Times are approximate medians from one machine. **Re-run the harness on yours** before citing absolute numbers.

### Search (`01_search`) — query `"sqlite storage"`, limit 25

| Notes | rustbrain `query_ranked` | Walk `.md` + contains | SQLite `LIKE` scan |
|------:|-------------------------:|----------------------:|-------------------:|
| 100 | ~1.4 ms | ~1.6 ms | ~0.40 ms |
| 200 | ~2.8 ms | ~3.2 ms | ~0.53 ms |
| 500 | ~3.5 ms | ~7.6 ms | ~0.90 ms |

**How to read:** FTS + ranking is **not** the cheapest scan — it pays for BM25, boosts, and type filters. Vs **walk** it pulls ahead as *N* grows (~2.2× at 500). **`LIKE`** is a full table scan of already-indexed bodies without ranking quality; useful lower bound, not a product replacement.

### Context pack (`02_context_pack`) — ~1024 token budget

| Notes | rustbrain `context` | Path-order concat | Grep-rank then concat |
|------:|--------------------:|------------------:|----------------------:|
| 100 | ~2.5 ms | ~0.75 ms | ~1.9 ms |
| 200 | ~3.2 ms | ~1.3 ms | ~3.4 ms |
| 500 | ~5.7 ms | ~2.7 ms | ~8.1 ms |

**How to read:** Dumping files in path order is faster and **unranked**. Grep-then-concat approaches rustbrain cost at small *N* and loses at 500 while still lacking graph hops and type-aware packing.

### Graph neighborhood (`03_graph_neighborhood`) — 1 hop from `docs/concepts/note-0`

| Notes | SQL neighborhood | CSR k-hop | Re-parse WikiLinks + BFS |
|------:|-----------------:|----------:|-------------------------:|
| 100 | ~345 µs | ~156 ns | ~1.9 ms |
| 200 | ~649 µs | ~159 ns | ~3.7 ms |
| 500 | ~1.5 ms | ~156 ns | ~8.9 ms |

**How to read:** This is the headline structural win. CSR k-hop stays **~nanoseconds** as *N* grows; re-parsing every note each query grows linearly (~**50 000×** slower at 500 notes). SQL neighborhood (relation types preserved for `graph` CLI) sits in between.

### WikiLink extract (`04_wikilink_extract`)

| Links in fixture | Fence-aware (rustbrain) | Naive regex |
|-----------------:|------------------------:|------------:|
| 50 | ~11 µs | ~5.3 µs |
| 200 | ~45 µs | ~21 µs |
| 1000 | ~215 µs | ~107 µs |

**How to read:** Regex is ~2× faster and **wrong** on fenced/inline code (fixture plants `[[inside-fence-should-skip]]`). Correctness is the product choice.

### Sync / open (micro, *N* = 200 unless noted)

| Op | Time |
|----|-----:|
| `sync` no-op (hashes match), 100 notes | ~7.5 ms |
| `sync` one dirty file, 100 notes | ~8.3 ms |
| `sync` no-op, 200 notes | ~12 ms |
| `sync` one dirty file, 200 notes | ~13 ms |
| `Brain::open_exact` (200) | ~0.48 ms |
| Open `graph.mmap` (200) | ~52 µs |
| `doctor` (200) | ~12 ms |
| `densify_plan` micro | ~1.7 µs |

## Reproduce tables

```bash
cargo bench -p bench --bench rustbrain_performance -- --quick
# or full:
cargo bench -p bench --bench rustbrain_performance
```

Update this file and crate READMEs when medians move materially across releases.
