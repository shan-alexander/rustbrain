# Contributing to rustbrain

Thanks for helping. This project values **correctness**, **honest documentation**, and **small, testable changes**.

## Development setup

```bash
git clone https://github.com/shan-alexander/rustbrain
cd rustbrain
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all
```

MSRV is **1.80** (see `rust-version` in the workspace `Cargo.toml`).

## Before you open a PR

1. **Tests** — add or update unit/integration tests for behavioral changes.
2. **Clippy** — `cargo clippy --workspace --all-targets --all-features -- -D warnings` must pass.
3. **Docs** — update `///` comments and README claims if user-visible behavior changes. Do not mark unfinished features as complete.
4. **Formats** — if you change SQLite or mmap layout, bump the version and update `docs/SCHEMA.md` / `docs/MMAP_FORMAT.md`.

## Design principles

- **Markdown is the source of truth for humans**; SQLite/mmap are derived caches.
- **No silent data loss** — do not swallow FK / FTS failures without reporting them.
- **Library boundary uses `BrainError`** — keep `anyhow` in the CLI only.
- **Feature flags** stay lean: default features should remain useful offline for a typical Rust repo.

## Commit / PR hygiene

- Prefer focused PRs over large multi-topic dumps.
- Reference issues when applicable.
- Dual-license contributions under MIT OR Apache-2.0 (see root README).

## Reporting bugs

Include:

- OS and `rustc` version  
- Minimal repro (Markdown / Rust fixtures when possible)  
- Output of `rustbrain sync` / `query` if CLI-related  

## Security

If you believe you found a security issue in mmap parsing, SQL usage, or path handling, please open a private security advisory on GitHub (or contact maintainers) rather than a public issue with exploit detail.
