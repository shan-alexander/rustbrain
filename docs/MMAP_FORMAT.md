# rustbrain CSR mmap format (v1)

**File:** `.brain/graph.mmap`  
**Endianness:** little-endian  
**Magic:** `RBRNMAP1` (8 bytes)  
**Version field:** `u32 = 1`

Compiled from SQLite via `CsrCompiler` and published with **write-temp + `rename`** for crash safety.

## Header (64 bytes)

| Offset | Size | Field |
|--------|------|-------|
| 0 | 8 | Magic `RBRNMAP1` |
| 8 | 4 | Version (`1`) |
| 12 | 4 | Node count `N` |
| 16 | 4 | Edge count `E` |
| 20 | 4 | Vector dim `D` (0 if none) |
| 24 | 4 | Flags (`bit0 = HAS_IDS`) |
| 28 | 36 | Reserved (zero) |

## Sections (in order)

1. **Row offsets** — `(N + 1) × u32`  
   CSR offsets into the target/weight arrays. Monotonic; final value equals `E`.

2. **Edge targets** — `E × u32`  
   Destination node indices in `[0, N)`.

3. **Edge weights** — `E × f32`

4. **Vector matrix** — `N × D × f32`  
   Present only when `D > 0`. **v0.1 product path always uses `D = 0`.**

5. **ID string table** (when `HAS_IDS`) — for each of `N` nodes:  
   `u16 length` + `length` UTF-8 bytes (node id string).  
   Order matches CSR node index order (same as SQLite `ORDER BY id ASC` at compile time).

## Validation rules (reader must enforce)

- File length ≥ end of vector section  
- Magic + version match  
- Row offsets monotonic and `offsets[N] == E`  
- Every target index `< N`  
- ID table fully readable when `HAS_IDS`  
- No unaligned `from_raw_parts` casts — use explicit `from_le_bytes` loads

## Atomic publish

```
write graph.mmap.tmp.<pid>
fsync
rename → graph.mmap
```

## Non-goals of v1

- 64-byte SIMD alignment padding (not required for correctness; portable loads used)
- ANN indexes (HNSW, etc.)
- Big-endian hosts (format is LE-only)
