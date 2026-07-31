# Publishing rustbrain to crates.io and GitHub

Checklist for maintainers releasing **v0.1.0** (and later).

## Published packages (only two)

| Crate | crates.io name | Contents |
|-------|----------------|----------|
| `crates/rustbrain-core` | **`rustbrain-core`** | Library (AST + Obsidian parsers included as features) |
| `crates/rustbrain-cli` | **`rustbrain`** | CLI binary |

AST and Obsidian code live **inside** `rustbrain-core` (`src/ast/`, `src/obsidian/`), not as separate crates. That avoids a multi-crate publish graph and matches how users depend on the product.

## Pre-flight

- [ ] `cargo test --workspace --all-features`
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- [ ] `cargo fmt --all -- --check`
- [ ] `cargo doc -p rustbrain-core --no-deps --all-features`
- [ ] CHANGELOG.md entry is accurate
- [ ] README install snippets match the version you are about to publish
- [ ] `repository` / `homepage` URLs point at the real GitHub org/repo
- [ ] Names free on crates.io: `rustbrain`, `rustbrain-core`
- [ ] `cargo login` done

## Dry-run

```bash
cargo publish -p rustbrain-core --dry-run
cargo publish -p rustbrain --dry-run
```

## GitHub

```bash
git tag -a v0.1.0 -m "rustbrain v0.1.0"
git push origin main --tags
# Open a GitHub Release; paste the CHANGELOG section
```

## crates.io

```bash
# 1. Library first
cargo publish -p rustbrain-core

# Wait until crates.io indexes (~1 min), then:

# 2. CLI (depends on rustbrain-core from the registry)
cargo publish -p rustbrain
```

## After publish

- [ ] https://crates.io/crates/rustbrain-core and docs.rs build
- [ ] https://crates.io/crates/rustbrain
- [ ] `cargo install rustbrain --version 0.1.0` on a clean machine
- [ ] Smoke: `rustbrain init && rustbrain sync && rustbrain query test`

## Consumer install

```toml
# Library / agents
[dependencies]
rustbrain-core = "0.1"
```

```bash
# Humans
cargo install rustbrain --locked
```

## Yanking

Prefer a patch release (`0.1.1`) over yanking unless the release is unusable.

```bash
cargo yank --vers 0.1.0 rustbrain-core   # only if truly broken
cargo yank --vers 0.1.0 rustbrain
```
