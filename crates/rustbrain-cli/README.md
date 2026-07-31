# rustbrain (CLI)

Command-line interface for the [rustbrain](https://github.com/shan-alexander/rustbrain) second-brain engine.

Package name on crates.io: **`rustbrain`**  
Binary name: **`rustbrain`**

## Install

```bash
cargo install rustbrain --locked

# From a local checkout:
cargo install --path crates/rustbrain-cli --locked
```

Requires a C toolchain (bundled SQLite + tree-sitter).

## Usage

```bash
cd your-project
rustbrain init
rustbrain sync
rustbrain query "your topic" --scores
rustbrain context -p "explain the auth flow" -F markdown --hops 1
rustbrain export --out ./brain.brainbundle
rustbrain watch --debounce-ms 300
```

### Commands

| Command | Description |
|---------|-------------|
| `init [path]` | Create `.brain/` and empty SQLite DB |
| `sync [path]` | Index notes + Rust AST; bake CSR mmap |
| `query <q>` | Ranked search (`--scores`, `-n`, `--all-workspaces`, `-w`) |
| `context` | Agent context (`-p` prompt, `-m` tokens, `--hops`, `-F xml\|md`) |
| `watch` | Debounced live re-index (`--debounce-ms`) |
| `export` | Write `.brainbundle` (`--out`, `--decouple-ast`) |
| `import` | Load a `.brainbundle` (`--input`) |

Run `rustbrain <command> --help` for full flags.

## Library

This package is **CLI-only**. For embedding rustbrain in your own tools, depend on
**`rustbrain-core`** (the only other published crate):

```toml
[dependencies]
rustbrain-core = "0.1"
```

## Exit codes

- `0` — success  
- `1` — error (message on stderr)

## License

MIT OR Apache-2.0
