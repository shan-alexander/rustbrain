# rustbrain SQLite Schema (v1)

**File:** `.brain/db.sqlite`  
**Schema version:** `1` (stored in `schema_meta`)

All timestamps are Unix epoch seconds (`INTEGER`).  
Foreign keys are enabled (`PRAGMA foreign_keys = ON`).  
Recommended pragmas: `journal_mode=WAL`, `busy_timeout=5000`.

## schema_meta

| Column | Type | Notes |
|--------|------|-------|
| key | TEXT PK | e.g. `schema_version` |
| value | TEXT | e.g. `1` |

## nodes

| Column | Type | Notes |
|--------|------|-------|
| id | TEXT PK | Stable path slug, e.g. `docs/concepts/raft` |
| node_type | TEXT | `goal` / `adr` / `alternative` / `concept` / `analysis` / `symbol` / `reference` / `edge_case` |
| title | TEXT | Display title |
| file_path | TEXT | Repo-relative path (nullable) |
| symbol_hash | INTEGER | Optional u64 (stored as i64) |
| summary | TEXT | Short summary |
| content_hash | TEXT | BLAKE3 hex of source bytes (change detection) |
| created_at | INTEGER | Preserved across upserts |
| updated_at | INTEGER | Bumped on content change |

## edges

| Column | Type | Notes |
|--------|------|-------|
| source_id | TEXT | FK → nodes.id CASCADE |
| target_id | TEXT | FK → nodes.id CASCADE |
| relation_type | TEXT | e.g. `relates_to`, `implements` |
| weight | REAL | Default 1.0 |
| decay_rate | REAL | Default 0.0 |
| created_at | INTEGER | |
| **PK** | | `(source_id, target_id, relation_type)` |

## symbol_anchors

| Column | Type | Notes |
|--------|------|-------|
| symbol_hash | INTEGER PK | BLAKE3-derived u64 of `crate::module::name` |
| crate_name | TEXT | |
| module_path | TEXT | Logical module path |
| symbol_name | TEXT | |
| file_path | TEXT | Repo-relative |
| start_line / end_line | INTEGER | 1-based |
| doc_comment | TEXT | Nullable |

## node_tags

`(node_id, tag)` PK, FK → nodes CASCADE.

## node_aliases

| Column | Type | Notes |
|--------|------|-------|
| alias | TEXT PK | Lowercased |
| node_id | TEXT | FK → nodes CASCADE |

Used for WikiLink resolution (`[[Raft]]` → node id).

## pending_links

Unresolved WikiLinks kept for later resolution / reporting.

| Column | Type |
|--------|------|
| source_id | TEXT FK |
| raw_target | TEXT |
| relation_type | TEXT |
| created_at | INTEGER |
| **PK** | `(source_id, raw_target, relation_type)` |

## node_fts (FTS5)

```sql
CREATE VIRTUAL TABLE node_fts USING fts5(
    node_id UNINDEXED,
    title,
    content,
    tags
);
```

**Idempotency:** indexers must `DELETE FROM node_fts WHERE node_id = ?` before insert.

## Migrations

Applied by `rustbrain_core::storage::migrations::migrate`.  
Opening a DB with `schema_version > SCHEMA_VERSION` is a hard error.

## Node ID scheme

See code: `rustbrain_core::id::node_id_from_rel_path`.

- Relative to workspace root  
- Lowercase, `/`-separated  
- Extension stripped  
- Whitespace → `-`

### Special hubs

| File | Node id | Default type |
|------|---------|--------------|
| Root `README.md` | `readme` | `goal` (unless frontmatter sets `node_type`) |

### Edge `relation_type` (common)

| Type | Direction | Source |
|------|-----------|--------|
| `relates_to` | note → note | WikiLink in Markdown |
| `anchors` | note → symbol | `symbol:…` / `[[symbol:…]]` in notes |
| `doc_links` | symbol → note | `[[WikiLink]]` in rustdoc (`///`) |
| `auto_filename` / `auto_tag` | soft | `links --auto` (low weight) |

Aliases for the hub include `readme`, `hub`, `home`, and the workspace directory name.

## Ignore

Indexing skips paths matched by built-in patterns and optional **`.rustbrainignore`**.
See [CLI.md](CLI.md) for the dialect and bootstrap import of `.gitignore`.
