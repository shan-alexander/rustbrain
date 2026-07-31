# Publishing rustbrain to crates.io and GitHub

## Published packages (only two)

| Crate path | crates.io name | Contents |
|------------|----------------|----------|
| `crates/rustbrain-core` | **`rustbrain-core`** | Library (AST + Obsidian modules) |
| `crates/rustbrain-cli` | **`rustbrain`** | CLI binary |

## Pre-flight

```bash
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo doc -p rustbrain-core --no-deps --all-features
```

- CHANGELOG has the version section  
- README install lines match the version  
- `repository` / `homepage` → `https://github.com/shan-alexander/rustbrain`  
- `cargo login` done  

## crates.io order

```bash
cargo publish -p rustbrain-core
# wait until searchable (~30–60s)
cargo publish -p rustbrain
```

## GitHub

```bash
git tag -a v0.3.8 -m "v0.3.8"
git push origin main --tags
```

## Consumer install

```toml
rustbrain-core = "0.3"
```

```bash
cargo install rustbrain --locked
# pin: cargo install rustbrain --version 0.3.8 --locked
```
