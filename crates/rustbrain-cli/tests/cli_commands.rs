//! CLI coverage for bootstrap, doctor, note, query filters.

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;
use std::fs;
use tempfile::tempdir;

#[test]
fn bootstrap_doctor_note_query_filters() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    fs::write(
        root.join("README.md"),
        "# CoolCrate\n\n## Why\n\nBecause local tools.\n\n## Features\n\n- Fast\n",
    )
    .unwrap();
    fs::write(root.join(".gitignore"), "target/\n*.log\n").unwrap();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"coolcrate\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::write(root.join("src/lib.rs"), "/// hi\npub fn greet() {}\n").unwrap();

    cargo_bin_cmd!("rustbrain")
        .current_dir(root)
        .args(["bootstrap", "--yes", "--write", "."])
        .assert()
        .success()
        .stdout(predicate::str::contains("create").or(predicate::str::contains("wrote")));

    assert!(root.join(".rustbrainignore").is_file());
    assert!(root.join("docs/goals/from-readme.md").is_file());
    assert!(root.join("docs/adr/TEMPLATE.md").is_file());
    assert!(root.join("AGENTS.md").is_file());
    let agents = fs::read_to_string(root.join("AGENTS.md")).unwrap();
    assert!(agents.contains("rustbrain"));

    cargo_bin_cmd!("rustbrain")
        .current_dir(root)
        .args(["sync", "."])
        .assert()
        .success()
        .stdout(predicate::str::contains("sync complete"));

    cargo_bin_cmd!("rustbrain")
        .current_dir(root)
        .args(["doctor", "."])
        .assert()
        .success()
        .stdout(predicate::str::contains("nodes="));

    cargo_bin_cmd!("rustbrain")
        .current_dir(root)
        .args([
            "note",
            "new",
            "--type",
            "concept",
            "--title",
            "Agent Note",
            "--note",
            "Body from an AI agent.",
            "--sync",
            "-w",
            ".",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("wrote"));

    cargo_bin_cmd!("rustbrain")
        .current_dir(root)
        .args(["query", "agent", "--no-symbols", "-w", "."])
        .assert()
        .success()
        .stdout(predicate::str::contains("Agent Note"));

    cargo_bin_cmd!("rustbrain")
        .current_dir(root)
        .args(["context", "why local tools", "-F", "markdown", "-w", "."])
        .assert()
        .success()
        .stdout(predicate::str::contains("packed:").or(predicate::str::contains("rustbrain context")));

    cargo_bin_cmd!("rustbrain")
        .current_dir(root)
        .args(["links", "-w", "."])
        .assert()
        .success();

    cargo_bin_cmd!("rustbrain")
        .current_dir(root)
        .args([
            "context",
            "-p",
            "local tools",
            "-F",
            "markdown",
            "--no-symbols",
            "-w",
            ".",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("rustbrain context"));
}
