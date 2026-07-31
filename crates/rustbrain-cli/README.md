# rustbrain (CLI)

Command-line interface for [rustbrain](https://github.com/shan-alexander/rustbrain).

**crates.io:** `rustbrain` · **binary:** `rustbrain` · **library:** use [`rustbrain-core`](https://crates.io/crates/rustbrain-core)

## Install

```bash
cargo install rustbrain --locked
# pin: cargo install rustbrain --version 0.3.7 --locked
```

Requires a C toolchain (bundled SQLite + tree-sitter).

## Typical workflow

```bash
cd your-project
rustbrain setup --yes          # docs + AGENTS.md + sync + doctor

# Point coding agents at the generated AGENTS.md (edit or template it org-wide)

rustbrain note new --type concept --title "Topic" --note "Body for agents."
rustbrain query "topic" --scores
rustbrain context "explain topic"
rustbrain links
```

### `AGENTS.md`

Written by `setup` / `bootstrap` unless `--no-agents-md`. Customize with:

- `--agents-template PATH`
- `RUSTBRAIN_AGENTS_TEMPLATE`
- committed `AGENTS.template.md` or `.rustbrain/AGENTS.template.md`

## Commands

| Command | Description |
|---------|-------------|
| `setup` | One-shot init + bootstrap + sync + doctor (`--no-agents-md`, `--agents-template`) |
| `init` | Create `.brain/db.sqlite` |
| `bootstrap` | Scaffold docs, `AGENTS.md`, ignore, README harvest, AST map (`--no-agents-md`, `--agents-template`) |
| `sync` | Index notes + Rust AST; bake CSR mmap |
| `doctor` | Health + orphans (`--orphans`, `--json`, `--strict`) |
| `note new` | Create note (`--type`, `--title`, `--body`/`--note`) |
| `links` / `link` | Pending links, or `--auto` soft links (`--auto path.md`) |
| `query` | Ranked search (`--no-symbols`, `--type`, `--scores`, `--all-workspaces`) |
| `context` | Agent context (positional/`-p`, excerpts, `--with-symbols`, `--no-hop-symbols`, `-F`) |
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
