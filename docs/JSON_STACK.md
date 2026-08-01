# JSON stack: serde_json vs jshift

Decision record after inventory + criterion benches (2026-07-31).

## Short answer

**Keep `serde` / `serde_json` for all production JSON in rustbrain.**  
**Keep `jshift` optional** (`feature = "jshift"`) for sparse path get / in-place field patch only.

jshift is **not** a full replacement for rustbrain’s workloads. On the tasks we actually do (full typed encode/decode, pretty CLI JSON, full brainbundle round-trip, YAML frontmatter), **serde is faster or required**. jshift wins on sparse path access and in-place mutate of large buffers — paths we do not take on the hot index/query/CLI path today.

## Inventory (production call sites)

| Site | Operation | Size / shape | Tool |
|------|-----------|--------------|------|
| CLI `--json` (doctor, links, autolink) | `to_string_pretty` full struct | tens–hundreds of KB max | **serde_json** |
| `.brain` workspace marker | encode tiny object | ~50–100 B | **serde_json** |
| Global `registry.json` | full load/save | tiny | **serde_json** |
| `.brainbundle` export/import | full encode/decode of nodes+edges | 50–2000+ nodes | **serde_json** |
| Obsidian canvas parse | full `Deserialize` | small–medium | **serde_json** |
| Note frontmatter | YAML | — | **serde_yaml_ng** (forces serde derive) |
| `mutator` (optional feature) | path find + in-place mutate | raw bytes | **jshift** |

Widespread `#[derive(Serialize, Deserialize)]` is also required for:

- YAML frontmatter (`NodeType`, `Frontmatter`, note types)
- CLI JSON responses (`DoctorReport`, autolink reports, query hits)
- Bundle / canvas / registry types

Removing serde derive from those types is not free: YAML has no jshift equivalent, and full CLI/bundle I/O is the product surface.

## Bench harness

```bash
cargo bench -p json-stack-bench --bench serde_vs_jshift
```

Crate: [`crates/json-stack-bench`](../crates/json-stack-bench) (`publish = false`), jshift **0.7**, shapes modeled on workspace markers, doctor reports, brainbundles, and fat catalogs.

## Results (release, x86_64 Linux, sample-size 50)

Times are approximate medians from one run; re-run the harness on your machine.

### 01 — Tiny workspace marker (~56 B)

| Op | Time |
|----|------|
| serde encode | ~93 ns |
| serde encode pretty | ~89 ns |
| serde decode | ~135 ns |
| jshift `to_schema_bytes` | ~263 ns (**~2.8× slower**) |
| jshift `JsonView::read_from` | ~199 ns (**~1.5× slower**) |
| jshift `TypedDoc` get one field | ~88 ns (beats full serde decode for *one* field only) |

**Winner for full mark/load: serde.**

### 02 — Doctor CLI pretty JSON

| Op | Time |
|----|------|
| serde `to_string_pretty` (full report) | ~698 ns |
| serde full decode | ~845 ns |
| jshift TypedDoc get 2 fields | ~513 ns |

CLI needs the **full** pretty document. Sparse TypedDoc is not a substitute for `--json` output. **serde for emit.**

### 03 — Brainbundle full encode/decode

| Nodes | serde encode | serde decode | jshift sparse 2 fields |
|------:|-------------:|-------------:|-----------------------:|
| 50 | ~35 µs | ~83 µs | ~0.89 µs |
| 500 | ~360 µs | ~825 µs | ~7.3 µs |
| 2000 | ~1.9 ms | ~3.2 ms | ~37 µs |

Sparse is ~**90×** cheaper than full decode — but **import/export must materialize every node and edge**. jshift nested `JsonView` over `Vec` of complex records is not a drop-in for `PortableBrainBundle`. **serde for bundle I/O.**

### 04 — Large JSON, sparse path only

| Catalog size | serde `Value` + index | jshift TypedDoc 2 fields | jshift `find_value` |
|-------------:|----------------------:|-------------------------:|--------------------:|
| 500 nodes | ~833 µs | ~88 µs (**~9×**) | ~1.8 µs (**~450×**) |
| 5000 nodes | ~9.4 ms | ~1.1 ms (**~8×**) | ~1.8 µs (**~5k×**) |

**jshift clearly wins** when you only need a few paths out of a large buffer and never build a typed tree. **No current rustbrain production path does this.**

### 05 — In-place field patch

| Op | Time |
|----|------|
| serde `Value` set + `to_vec` | ~1.3 µs |
| jshift `mutate_value` | ~226 ns (**~5.7×**) |

**jshift wins** for byte-level patch without full re-serialize. Already exposed as optional helpers in `rustbrain_core::mutator`.

## Decision matrix (when to use which)

| Workload | Choice | Why |
|----------|--------|-----|
| Pretty / full encode of a Rust struct | **serde_json** | Faster closed emit; matches CLI/export |
| Full typed decode | **serde_json** | Required for import/canvas/registry |
| YAML frontmatter | **serde_yaml_ng** | No alternative; keeps serde derives |
| Sparse path get on large/open JSON | **jshift** | Path scan, no `Value` DOM |
| In-place field mutate of a buffer | **jshift** | Avoids re-serialize |
| Hot index / FTS / CSR / query | **neither** | Not on that path |

jshift’s own docs agree: closed emit of small cards and full small JSON favor serde; fat docs with selective paths favor jshift.

## What we did / did not change

**Did:**

- Added reproducible benches under `crates/json-stack-bench`
- Documented this decision
- Kept / documented optional `jshift` feature for mutate/path helpers
- Workspace pin **jshift 0.7** (API used by mutator: `parse_path`, `find_value`, `mutate_value` stable)

**Did not:**

- Replace CLI / bundle / registry / canvas / markers with jshift (would be slower or incomplete)
- Drop serde derives from domain types (YAML + full JSON still need them)
- Make jshift a default dependency (optional remains correct)

## Future switch triggers

Revisit only if rustbrain gains a real workload like:

1. Streaming multi‑MB agent JSONL and reading a few fields per line
2. Patching large on-disk JSON manifests in place without re-encoding
3. Open schemas with large unknown payloads where full `Deserialize` is wasteful

Then enable `jshift` at those call sites and re-run `json-stack-bench` (extend it with the new shape). Do **not** migrate full-document CLI/export “for consistency.”

## Re-run

```bash
export PATH="$HOME/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin:$PATH"
cargo bench -p json-stack-bench --bench serde_vs_jshift
```
