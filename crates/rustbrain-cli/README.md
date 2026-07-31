# rustbrain (CLI)

Command-line interface for [rustbrain](https://github.com/shan-alexander/rustbrain).

**crates.io:** `rustbrain` · **binary:** `rustbrain` · **library:** use [`rustbrain-core`](https://crates.io/crates/rustbrain-core)

## Install

```bash
cargo install rustbrain --locked
# pin: cargo install rustbrain --version 0.2.0 --locked
```

Requires a C toolchain (bundled SQLite + tree-sitter).

## Typical workflow

```bash
cd your-project
rustbrain init
rustbrain bootstrap --yes --write   # mature repos / agents
rustbrain sync
rustbrain doctor

rustbrain note new --type concept --title "Topic" --note "Body for agents." --sync
rustbrain query "topic" --no-symbols --scores
rustbrain context -p "explain topic" -F markdown --hops 1
rustbrain links
```

## Commands

| Command | Description |
|---------|-------------|
| `init` | Create `.brain/db.sqlite` |
| `bootstrap` | Scaffold docs, ignore file, README harvest, AST map (`--yes`, `--write`, `--force`) |
| `sync` | Index notes + Rust AST; bake CSR mmap |
| `doctor` | Health + pending links (`--json`, `--strict`) |
| `note new` | Create note (`--type`, `--title`, `--note`, `--sync`) |
| `links` | List pending unresolved links |
| `query` | Ranked search (`--no-symbols`, `--type`, `--scores`, `--all-workspaces`) |
| `context` | Agent context (`-p`, `-m`, `--hops`, `--no-symbols`, `--no-hop-symbols`, `-F`) |
| `watch` | Debounced live re-index |
| `export` / `import` | `.brainbundle` portable graph |

Full flag reference and nuances: **[docs/CLI.md](../../docs/CLI.md)** in the repo.

### Agent tip: `note new`

```bash
rustbrain note new \
  --type adr \
  --title "Prefer path deps in workspace" \
  --note "Keeps publish graph simple; only core + CLI on crates.io." \
  --tags "packaging" \
  --sync
```

### Bootstrap tip: interactive ignore

On a TTY without `--yes`, bootstrap asks whether to create `.rustbrainignore`, import `.gitignore`, and add recommended extras (`data/`, `*.parquet`, `.env`, …).

## Exit codes

- `0` — success  
- `1` — error, or `doctor --strict` with pending/unhealthy state  

## License

MIT OR Apache-2.0
