//! CLI coverage for multi-brain scopes (0.3.22).

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;
use std::fs;
use tempfile::tempdir;

#[test]
fn scopes_detect_enable_list_and_query_scope() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("crates/core/src")).unwrap();
    fs::create_dir_all(root.join("crates/cli/src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        r#"[workspace]
members = ["crates/*"]
"#,
    )
    .unwrap();
    fs::write(
        root.join("crates/core/Cargo.toml"),
        "[package]\nname = \"core\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::write(
        root.join("crates/cli/Cargo.toml"),
        "[package]\nname = \"cli\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::write(
        root.join("crates/core/src/lib.rs"),
        "/// core secret alpha\npub fn core_fn() {}\n",
    )
    .unwrap();
    fs::write(
        root.join("crates/cli/src/main.rs"),
        "fn main() { /* cli secret beta */ }\n",
    )
    .unwrap();
    fs::write(root.join("README.md"), "# Workspace\n").unwrap();

    cargo_bin_cmd!("rustbrain")
        .current_dir(root)
        .args(["init", "."])
        .assert()
        .success();

    cargo_bin_cmd!("rustbrain")
        .current_dir(root)
        .args(["scopes", "detect", "crates/core", "-w", "."])
        .assert()
        .success()
        .stdout(predicate::str::contains("suggested_id: core"))
        .stdout(predicate::str::contains("mountable: true"));

    cargo_bin_cmd!("rustbrain")
        .current_dir(root)
        .args(["scopes", "enable", "--cargo", "-w", "."])
        .assert()
        .success()
        .stdout(predicate::str::contains("multi-brain enabled"));

    cargo_bin_cmd!("rustbrain")
        .current_dir(root)
        .args(["scopes", "list", "-w", "."])
        .assert()
        .success()
        .stdout(predicate::str::contains("core"))
        .stdout(predicate::str::contains("cli"));

    cargo_bin_cmd!("rustbrain")
        .current_dir(root)
        .args(["sync", "."])
        .assert()
        .success()
        .stdout(predicate::str::contains("by_scope"));

    cargo_bin_cmd!("rustbrain")
        .current_dir(root)
        .args([
            "query",
            "alpha",
            "--scope",
            "core",
            "--scope-strict",
            "--with-symbols",
            "-w",
            ".",
        ])
        .assert()
        .success();
}
