//! End-to-end CLI smoke tests.

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;
use std::fs;
use tempfile::tempdir;

#[test]
fn init_sync_query_context_export() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    let docs = root.join("docs");
    let src = root.join("src");
    fs::create_dir_all(&docs).unwrap();
    fs::create_dir_all(&src).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::write(
        docs.join("raft.md"),
        "---\ntags: [raft]\nnode_type: concept\n---\n# Raft\nSee [[logcompaction]] and symbol:StorageEngine.\n",
    )
    .unwrap();
    fs::write(
        docs.join("logcompaction.md"),
        "---\ntags: [log]\nnode_type: concept\n---\n# Log Compaction\nSee [[raft]].\n",
    )
    .unwrap();
    fs::write(
        src.join("lib.rs"),
        "pub struct StorageEngine;\nimpl StorageEngine { pub fn open() {} }\n",
    )
    .unwrap();

    cargo_bin_cmd!("rustbrain")
        .current_dir(root)
        .args(["init", "."])
        .assert()
        .success()
        .stdout(predicate::str::contains("initialized"));

    cargo_bin_cmd!("rustbrain")
        .current_dir(root)
        .args(["sync", "."])
        .assert()
        .success()
        .stdout(predicate::str::contains("sync complete"));

    // Second sync resolves pending symbol anchors (note may index before rust).
    cargo_bin_cmd!("rustbrain")
        .current_dir(root)
        .args(["sync", "."])
        .assert()
        .success();

    cargo_bin_cmd!("rustbrain")
        .current_dir(root)
        .args(["query", "raft", "-w", ".", "--scores"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Raft"));

    // Critical: context must not panic on short flags.
    cargo_bin_cmd!("rustbrain")
        .current_dir(root)
        .args(["context", "-p", "raft", "-F", "markdown", "-w", "."])
        .assert()
        .success()
        .stdout(predicate::str::contains("rustbrain context"))
        .stdout(predicate::str::contains("tokens:"));

    cargo_bin_cmd!("rustbrain")
        .current_dir(root)
        .args([
            "context",
            "--for-prompt",
            "raft",
            "--format",
            "xml",
            "--hops",
            "1",
            "-w",
            ".",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("<rustbrain_context"))
        .stdout(predicate::str::contains("tokens_used="));

    let bundle = root.join("out.brainbundle");
    cargo_bin_cmd!("rustbrain")
        .current_dir(root)
        .args([
            "export",
            "--out",
            bundle.to_str().unwrap(),
            "--decouple-ast",
            "-w",
            ".",
        ])
        .assert()
        .success();
    assert!(bundle.exists());

    // Watch command exists (help only — don't block).
    cargo_bin_cmd!("rustbrain")
        .args(["watch", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("debounce"));
}
