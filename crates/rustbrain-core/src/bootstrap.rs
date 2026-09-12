//! Deterministic workspace bootstrap for mature repositories.
//!
//! Creates docs scaffolds, optional `.rustbrainignore`, README-derived goals,
//! a thin `AGENTS.md` mandate, `SKILL.md` for agent harnesses, and an AST
//! module map — **without** inventing ADRs or calling cloud models.

use crate::error::{BrainError, Result};
use crate::ignore::{recommended_ignore_extras, write_rustbrainignore};

use serde::{Deserialize, Serialize};
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

/// How to handle interactive prompts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootstrapMode {
    /// Prompt on a TTY; use defaults when stdin is not a terminal.
    Interactive,
    /// Never prompt; use options as given.
    NonInteractive,
}

/// Options for [`bootstrap_workspace`].
#[derive(Debug, Clone)]
pub struct BootstrapOptions {
    /// Interaction mode.
    pub mode: BootstrapMode,
    /// Write files (false = dry-run report only).
    pub write: bool,
    /// Overwrite generated files that already exist.
    pub force: bool,
    /// Create / update `.rustbrainignore`.
    pub setup_ignore: Option<bool>,
    /// Import root `.gitignore` into `.rustbrainignore`.
    pub import_gitignore: Option<bool>,
    /// Append recommended extra ignore patterns.
    pub ignore_extras: bool,
    /// Harvest README into docs/goals/from-readme.md.
    pub harvest_readme: bool,
    /// Generate AST module map under docs/implementation/.
    pub module_map: bool,
    /// Harvest Cargo.toml deps → docs.rs notes under docs/references/.
    pub crate_docs: bool,
    /// Scaffold docs/ directory tree + templates.
    pub scaffold_docs: bool,
    /// Write/refresh root `AGENTS.md` (short rustbrain mandate). Default true when `None`.
    ///
    /// Custom (non-rustbrain) `AGENTS.md` is never overwritten: a short section is
    /// appended instead.
    pub write_agents_md: Option<bool>,
    /// Optional path to a custom `AGENTS.md` template file (overrides discovery + built-in).
    pub agents_template: Option<PathBuf>,
    /// Install `SKILL.md` into detected agent harnesses (or repo-root `SKILL.md`
    /// if none). Default true when `None`.
    pub write_skill_md: Option<bool>,
}

impl Default for BootstrapOptions {
    fn default() -> Self {
        Self {
            mode: BootstrapMode::Interactive,
            write: true,
            force: false,
            setup_ignore: None,
            import_gitignore: None,
            ignore_extras: true,
            harvest_readme: true,
            module_map: true,
            crate_docs: true,
            scaffold_docs: true,
            write_agents_md: None,
            agents_template: None,
            write_skill_md: None,
        }
    }
}

/// One planned or performed action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapAction {
    /// Short verb (create, skip, would_create).
    pub action: String,
    /// Relative path affected.
    pub path: String,
    /// Detail message.
    pub detail: String,
}

/// Result of bootstrap.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapReport {
    /// Workspace.
    pub workspace: PathBuf,
    /// Whether files were written.
    pub wrote: bool,
    /// Actions taken or planned.
    pub actions: Vec<BootstrapAction>,
}

const DOC_DIRS: &[&str] = &[
    "docs/goals",
    "docs/adr",
    "docs/concepts",
    "docs/analysis",
    "docs/plans",
    "docs/changelogs",
    "docs/edge_cases",
    "docs/implementation",
    "docs/references",
    "docs/experience",
];

/// Run deterministic bootstrap for `workspace`.
pub fn bootstrap_workspace(
    workspace: &Path,
    mut opts: BootstrapOptions,
) -> Result<BootstrapReport> {
    let workspace = if workspace.exists() {
        workspace.canonicalize()?
    } else {
        std::fs::create_dir_all(workspace)?;
        workspace.canonicalize()?
    };

    resolve_interactive(&workspace, &mut opts)?;

    let mut actions = Vec::new();
    let wrote = opts.write;

    if opts.scaffold_docs {
        scaffold_docs(&workspace, opts.write, opts.force, &mut actions)?;
    }

    if opts.setup_ignore.unwrap_or(false) {
        setup_ignore(
            &workspace,
            opts.write,
            opts.force,
            opts.import_gitignore.unwrap_or(false),
            opts.ignore_extras,
            &mut actions,
        )?;
    }

    if opts.harvest_readme {
        harvest_readme(&workspace, opts.write, opts.force, &mut actions)?;
    }

    if opts.module_map {
        #[cfg(feature = "ast")]
        generate_module_map(&workspace, opts.write, opts.force, &mut actions)?;
        #[cfg(not(feature = "ast"))]
        {
            actions.push(BootstrapAction {
                action: "skip".into(),
                path: "docs/implementation/module-map.generated.md".into(),
                detail: "ast feature disabled — module map not generated".into(),
            });
        }
    }

    if opts.crate_docs {
        harvest_crate_docs(&workspace, opts.write, opts.force, &mut actions)?;
    }

    if opts.write_agents_md.unwrap_or(true) {
        write_agents_md(
            &workspace,
            opts.write,
            opts.agents_template.as_deref(),
            &mut actions,
        )?;
    } else {
        actions.push(BootstrapAction {
            action: "skip".into(),
            path: "AGENTS.md".into(),
            detail: "disabled (--no-agents-md / write_agents_md=false)".into(),
        });
    }

    if opts.write_skill_md.unwrap_or(true) {
        write_skill_md(&workspace, opts.write, &mut actions)?;
    } else {
        actions.push(BootstrapAction {
            action: "skip".into(),
            path: "SKILL.md".into(),
            detail: "disabled (--no-skill-md / write_skill_md=false)".into(),
        });
    }

    // Always inject docs/AGENTS.md with every scaffold (or when writing agents), so
    // agents working under docs/ see rustbrain ops on every turn.
    if opts.scaffold_docs || opts.write_agents_md.unwrap_or(true) {
        write_docs_agents_md(&workspace, opts.write, opts.force, &mut actions)?;
    }

    // Ensure brain exists when writing
    if opts.write {
        let brain = workspace.join(".brain");
        if !brain.join("db.sqlite").exists() {
            std::fs::create_dir_all(&brain)?;
            let _ = crate::storage::Database::open(brain.join("db.sqlite"))?;
            actions.push(BootstrapAction {
                action: "create".into(),
                path: ".brain/db.sqlite".into(),
                detail: "initialized empty brain database".into(),
            });
            let marker = brain.join("workspace.json");
            if !marker.exists() {
                let meta = serde_json::json!({
                    "version": 1,
                    "workspace": workspace.to_string_lossy(),
                    "bootstrapped": true,
                });
                std::fs::write(&marker, serde_json::to_string_pretty(&meta)?)?;
            }
        }
        ensure_gitignore_brain(&workspace, true, &mut actions)?;
    }

    actions.push(BootstrapAction {
        action: "next".into(),
        path: ".".into(),
        detail: if wrote {
            "run `rustbrain sync` then `rustbrain doctor` (or `rustbrain setup --yes` next time)"
                .into()
        } else {
            "re-run with --write to apply".into()
        },
    });

    Ok(BootstrapReport {
        workspace,
        wrote,
        actions,
    })
}

fn resolve_interactive(workspace: &Path, opts: &mut BootstrapOptions) -> Result<()> {
    if opts.mode != BootstrapMode::Interactive {
        // Non-interactive defaults
        if opts.setup_ignore.is_none() {
            opts.setup_ignore = Some(true);
        }
        if opts.import_gitignore.is_none() {
            opts.import_gitignore = Some(workspace.join(".gitignore").is_file());
        }
        if opts.write_agents_md.is_none() {
            opts.write_agents_md = Some(true);
        }
        if opts.write_skill_md.is_none() {
            opts.write_skill_md = Some(true);
        }
        return Ok(());
    }

    let tty = io::stdin().is_terminal() && io::stdout().is_terminal();
    if !tty {
        if opts.setup_ignore.is_none() {
            opts.setup_ignore = Some(true);
        }
        if opts.import_gitignore.is_none() {
            opts.import_gitignore = Some(workspace.join(".gitignore").is_file());
        }
        if opts.write_agents_md.is_none() {
            opts.write_agents_md = Some(true);
        }
        if opts.write_skill_md.is_none() {
            opts.write_skill_md = Some(true);
        }
        return Ok(());
    }

    println!("rustbrain bootstrap — {}", workspace.display());
    println!("Deterministic setup (no cloud AI). Press Enter to accept [defaults].\n");

    if opts.setup_ignore.is_none() {
        let has = workspace.join(".rustbrainignore").is_file();
        let def = if has { "n" } else { "Y" };
        let ans = prompt(
            &format!("Create/update .rustbrainignore? [Y/n] (default {def})"),
            def,
        )?;
        opts.setup_ignore = Some(ans_yes(&ans, !has));
    }

    if opts.setup_ignore == Some(true) && opts.import_gitignore.is_none() {
        let has_gi = workspace.join(".gitignore").is_file();
        if has_gi {
            let ans = prompt(
                "Import patterns from root .gitignore into .rustbrainignore? [Y/n]",
                "Y",
            )?;
            opts.import_gitignore = Some(ans_yes(&ans, true));
        } else {
            opts.import_gitignore = Some(false);
            println!("  (no .gitignore found — skipping import)");
        }
    }

    if opts.setup_ignore == Some(true) {
        let ans = prompt(
            "Append recommended extras (target/, data/, *.parquet, .env, …)? [Y/n]",
            "Y",
        )?;
        opts.ignore_extras = ans_yes(&ans, true);

        // Offer free-form extra lines
        let ans = prompt(
            "Add extra ignore patterns now? (comma-separated, or empty) []",
            "",
        )?;
        if !ans.trim().is_empty() {
            // Stash extras in a side channel via env-like temporary — use a file write later
            // We'll append them in setup_ignore by reading a thread-local... cleaner: store on opts
            // Extend BootstrapOptions - for simplicity append into recommended via env
            std::env::set_var("RUSTBRAIN_BOOTSTRAP_EXTRA_IGNORES", ans.trim());
        }
    }

    if opts.harvest_readme {
        // already true; allow disable
        if workspace.join("README.md").is_file() {
            let ans = prompt(
                "Harvest README.md into docs/goals/from-readme.md? [Y/n]",
                "Y",
            )?;
            opts.harvest_readme = ans_yes(&ans, true);
        }
    }

    #[cfg(feature = "ast")]
    {
        let ans = prompt(
            "Generate docs/implementation/module-map.generated.md from Rust AST? [Y/n]",
            "Y",
        )?;
        opts.module_map = ans_yes(&ans, true);
    }

    if workspace.join("Cargo.toml").is_file() {
        let ans = prompt(
            "Harvest Cargo.toml deps → docs/references/* with docs.rs URLs? [Y/n]",
            "Y",
        )?;
        opts.crate_docs = ans_yes(&ans, true);
    }

    let ans = prompt("Scaffold docs/ tree + ADR/goal templates? [Y/n]", "Y")?;
    opts.scaffold_docs = ans_yes(&ans, true);

    if opts.write_agents_md.is_none() {
        let ans = prompt(
            "Write/update root AGENTS.md (short rustbrain mandate; custom files get a section appended)? [Y/n]",
            "Y",
        )?;
        opts.write_agents_md = Some(ans_yes(&ans, true));
    }

    if opts.write_agents_md == Some(true) && opts.agents_template.is_none() {
        let ans = prompt(
            "Custom AGENTS.md template path? (empty = built-in or AGENTS.template.md) []",
            "",
        )?;
        if !ans.trim().is_empty() {
            opts.agents_template = Some(PathBuf::from(ans.trim()));
        }
    }

    if opts.write_skill_md.is_none() {
        let ans = prompt(
            "Install rustbrain SKILL.md into detected agent harnesses (or repo-root SKILL.md if none)? [Y/n]",
            "Y",
        )?;
        opts.write_skill_md = Some(ans_yes(&ans, true));
    }

    if !opts.write {
        let ans = prompt("Write files to disk? [Y/n]", "Y")?;
        opts.write = ans_yes(&ans, true);
    }

    Ok(())
}

fn prompt(msg: &str, default: &str) -> Result<String> {
    print!("{msg} ");
    io::stdout().flush()?;
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line)?;
    let t = line.trim();
    if t.is_empty() {
        Ok(default.to_string())
    } else {
        Ok(t.to_string())
    }
}

fn ans_yes(ans: &str, default_yes: bool) -> bool {
    match ans.trim().to_ascii_lowercase().as_str() {
        "y" | "yes" => true,
        "n" | "no" => false,
        "" => default_yes,
        _ => default_yes,
    }
}

fn scaffold_docs(
    workspace: &Path,
    write: bool,
    force: bool,
    actions: &mut Vec<BootstrapAction>,
) -> Result<()> {
    for d in DOC_DIRS {
        let path = workspace.join(d);
        if path.is_dir() {
            actions.push(BootstrapAction {
                action: "exists".into(),
                path: d.to_string(),
                detail: "directory already present".into(),
            });
        } else if write {
            std::fs::create_dir_all(&path)?;
            actions.push(BootstrapAction {
                action: "create".into(),
                path: d.to_string(),
                detail: "created directory".into(),
            });
        } else {
            actions.push(BootstrapAction {
                action: "would_create".into(),
                path: d.to_string(),
                detail: "directory".into(),
            });
        }
    }

    // ADR template
    let adr_tpl = workspace.join("docs/adr/TEMPLATE.md");
    write_if_allowed(
        &adr_tpl,
        "docs/adr/TEMPLATE.md",
        ADR_TEMPLATE,
        write,
        force,
        actions,
    )?;

    // Goals placeholder if empty
    let goals_readme = workspace.join("docs/goals/README.md");
    write_if_allowed(
        &goals_readme,
        "docs/goals/README.md",
        GOALS_DIR_README,
        write,
        force,
        actions,
    )?;

    // Checklist
    let checklist = workspace.join("docs/BOOTSTRAP_CHECKLIST.md");
    write_if_allowed(
        &checklist,
        "docs/BOOTSTRAP_CHECKLIST.md",
        BOOTSTRAP_CHECKLIST,
        write,
        force,
        actions,
    )?;

    let plans_readme = workspace.join("docs/plans/README.md");
    write_if_allowed(
        &plans_readme,
        "docs/plans/README.md",
        PLANS_DIR_README,
        write,
        force,
        actions,
    )?;

    Ok(())
}

/// Inject `docs/AGENTS.md` — per-docs cookbook: use rustbrain every agent turn.
fn write_docs_agents_md(
    workspace: &Path,
    write: bool,
    force: bool,
    actions: &mut Vec<BootstrapAction>,
) -> Result<()> {
    let path = workspace.join("docs/AGENTS.md");
    write_if_allowed(
        &path,
        "docs/AGENTS.md",
        DOCS_AGENTS_MD,
        write,
        force,
        actions,
    )
}

fn setup_ignore(
    workspace: &Path,
    write: bool,
    force: bool,
    import_gitignore: bool,
    extras: bool,
    actions: &mut Vec<BootstrapAction>,
) -> Result<()> {
    let path = workspace.join(".rustbrainignore");
    let rel = ".rustbrainignore";
    if path.exists() && !force {
        actions.push(BootstrapAction {
            action: "skip".into(),
            path: rel.into(),
            detail: "already exists (use --force to overwrite)".into(),
        });
        return Ok(());
    }

    let mut extra_lines: Vec<String> = Vec::new();
    extra_lines.push("# rustbrain: import-gitignore".into());
    if !import_gitignore {
        // comment marker only for documentation; runtime only imports when present
        // If user declined import, remove the directive
        extra_lines.clear();
    }

    if extras {
        for l in recommended_ignore_extras() {
            extra_lines.push(l.to_string());
        }
    }

    if let Ok(more) = std::env::var("RUSTBRAIN_BOOTSTRAP_EXTRA_IGNORES") {
        for part in more.split(',') {
            let p = part.trim();
            if !p.is_empty() {
                extra_lines.push(p.to_string());
            }
        }
    }

    let extras_ref: Vec<&str> = extra_lines.iter().map(|s| s.as_str()).collect();

    if write {
        write_rustbrainignore(workspace, import_gitignore, &extras_ref)?;
        actions.push(BootstrapAction {
            action: "create".into(),
            path: rel.into(),
            detail: format!("ignore file (import_gitignore={import_gitignore}, extras={extras})"),
        });
    } else {
        actions.push(BootstrapAction {
            action: "would_create".into(),
            path: rel.into(),
            detail: format!("ignore file (import_gitignore={import_gitignore}, extras={extras})"),
        });
    }
    Ok(())
}

/// Write docs.rs reference notes from Cargo manifests (setup/bootstrap).
fn harvest_crate_docs(
    workspace: &Path,
    write: bool,
    force: bool,
    actions: &mut Vec<BootstrapAction>,
) -> Result<()> {
    if !workspace.join("Cargo.toml").is_file() {
        actions.push(BootstrapAction {
            action: "skip".into(),
            path: "docs/references/".into(),
            detail: "no Cargo.toml — crate docs harvest skipped".into(),
        });
        return Ok(());
    }
    let deps = crate::crate_docs::collect_crate_deps(workspace)?;
    let (n, details) = crate::crate_docs::write_crate_docs_notes(workspace, &deps, write, force)?;
    for d in details {
        let (action, path) = if let Some(rest) = d.strip_prefix("write ") {
            ("create", rest)
        } else if let Some(rest) = d.strip_prefix("would_write ") {
            ("would_create", rest)
        } else if let Some(rest) = d.strip_prefix("skip ") {
            ("skip", rest)
        } else {
            ("info", d.as_str())
        };
        actions.push(BootstrapAction {
            action: action.into(),
            path: path.to_string(),
            detail: if action == "info" {
                d.clone()
            } else {
                format!("{n} crate note(s) total; {d}")
            },
        });
    }
    if deps.is_empty() {
        actions.push(BootstrapAction {
            action: "info".into(),
            path: "docs/references/".into(),
            detail: "no crates.io dependencies found".into(),
        });
    } else {
        actions.push(BootstrapAction {
            action: "info".into(),
            path: "docs/references/crate-docs.generated.md".into(),
            detail: format!(
                "harvested {n} crates.io package(s) with docs.rs links (from Cargo.toml/lock)"
            ),
        });
    }
    Ok(())
}

fn harvest_readme(
    workspace: &Path,
    write: bool,
    force: bool,
    actions: &mut Vec<BootstrapAction>,
) -> Result<()> {
    let readme = workspace.join("README.md");
    let out_rel = "docs/goals/from-readme.md";
    let out = workspace.join(out_rel);
    if !readme.is_file() {
        actions.push(BootstrapAction {
            action: "skip".into(),
            path: out_rel.into(),
            detail: "no README.md at workspace root".into(),
        });
        return Ok(());
    }

    let text = std::fs::read_to_string(&readme)?;
    let body = extract_readme_sections(&text);
    let title = first_h1(&text).unwrap_or_else(|| {
        workspace
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Project")
            .to_string()
    });

    let content = format!(
        "---\n\
         tags: [goal, readme, generated]\n\
         node_type: goal\n\
         aliases: [from-readme, {title}]\n\
         generated: true\n\
         source: README.md\n\
         ---\n\
         # Goals harvested from README\n\n\
         > Generated by `rustbrain bootstrap`. Edit freely; re-run with `--force` to regenerate.\n\n\
         Project title: **{title}**\n\n\
         {body}\n"
    );

    write_if_allowed(&out, out_rel, &content, write, force, actions)?;
    Ok(())
}

fn extract_readme_sections(text: &str) -> String {
    // Pull sections whose headings look goal-related, plus first paragraphs.
    let mut out = String::new();
    let mut capture = true; // preamble
    let mut current = String::new();
    let mut current_title = String::from("Overview");

    let flush = |title: &str, body: &str, out: &mut String| {
        let body = body.trim();
        if body.is_empty() {
            return;
        }
        out.push_str(&format!("## {title}\n\n{body}\n\n"));
    };

    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("# ") {
            // top title — skip as section
            let _ = rest;
            continue;
        }
        if let Some(rest) = line.strip_prefix("## ") {
            flush(&current_title, &current, &mut out);
            current_title = rest.trim().to_string();
            current.clear();
            let lower = current_title.to_ascii_lowercase();
            capture = lower.contains("goal")
                || lower.contains("why")
                || lower.contains("feature")
                || lower.contains("non-goal")
                || lower.contains("non goal")
                || lower.contains("about")
                || lower.contains("overview")
                || lower.contains("require")
                || lower.contains("architect");
            continue;
        }
        if capture {
            current.push_str(line);
            current.push('\n');
        }
    }
    flush(&current_title, &current, &mut out);

    if out.trim().is_empty() {
        // Fallback: first 40 non-empty lines
        let mut n = 0;
        out.push_str("## Overview\n\n");
        for line in text.lines() {
            if line.trim().is_empty() {
                continue;
            }
            if line.starts_with('#') {
                continue;
            }
            out.push_str(line);
            out.push('\n');
            n += 1;
            if n >= 40 {
                break;
            }
        }
    }
    out
}

fn first_h1(text: &str) -> Option<String> {
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("# ") {
            let t = rest.trim();
            if !t.is_empty() {
                return Some(t.to_string());
            }
        }
    }
    None
}

#[cfg(feature = "ast")]
fn generate_module_map(
    workspace: &Path,
    write: bool,
    force: bool,
    actions: &mut Vec<BootstrapAction>,
) -> Result<()> {
    use crate::ast::CodeAstParser;
    use crate::id::rel_path_from_workspace;

    let out_rel = "docs/implementation/module-map.generated.md";
    let out = workspace.join(out_rel);
    let mut parser = CodeAstParser::new_rust().map_err(|e| BrainError::Ast(e.to_string()))?;

    let crate_name = workspace
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("crate")
        .to_string();

    // Prefer package name from Cargo.toml
    let crate_name = read_package_name(workspace).unwrap_or(crate_name);

    let mut sections: Vec<(String, Vec<String>)> = Vec::new();
    walk_rs(workspace, &mut |path| {
        let rel = rel_path_from_workspace(workspace, path);
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        if rel_str.starts_with("target/") {
            return;
        }
        let Ok(src) = std::fs::read_to_string(path) else {
            return;
        };
        let Ok(anchors) = parser.parse_symbols(&crate_name, &rel_str, &src) else {
            return;
        };
        if anchors.is_empty() {
            return;
        }
        let mut lines = Vec::new();
        for a in anchors {
            // Prefer public-looking / type-level items first in display
            lines.push(format!(
                "- `{}` — symbol:{}::{}::{} (`{}` L{}-{})",
                a.symbol_name,
                a.crate_name,
                a.module_path,
                a.symbol_name,
                a.file_path,
                a.start_line,
                a.end_line
            ));
        }
        sections.push((rel_str, lines));
    })?;

    sections.sort_by(|a, b| a.0.cmp(&b.0));

    let mut body = String::from(
        "---\n\
         tags: [implementation, generated, ast]\n\
         node_type: concept\n\
         aliases: [module-map, generated-module-map]\n\
         generated: true\n\
         ---\n\
         # Module map (generated)\n\n\
         > Generated by `rustbrain bootstrap` from Tree-Sitter. Do not hand-edit;\n\
         > re-run bootstrap with `--force` to refresh.\n\n",
    );

    if sections.is_empty() {
        body.push_str("_No Rust symbols found._\n");
    } else {
        for (file, lines) in &sections {
            body.push_str(&format!("## `{file}`\n\n"));
            for l in lines {
                body.push_str(l);
                body.push('\n');
            }
            body.push('\n');
        }
    }

    write_if_allowed(&out, out_rel, &body, write, force, actions)?;
    Ok(())
}

#[cfg(feature = "ast")]
fn walk_rs(dir: &Path, f: &mut dyn FnMut(&Path)) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(
                    name,
                    "target" | ".git" | ".brain" | "node_modules" | "vendor"
                ) || name.starts_with('.')
                {
                    continue;
                }
            }
            walk_rs(&path, f)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            f(&path);
        }
    }
    Ok(())
}

fn read_package_name(workspace: &Path) -> Option<String> {
    let text = std::fs::read_to_string(workspace.join("Cargo.toml")).ok()?;
    let mut in_package = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            in_package = t == "[package]";
            continue;
        }
        if in_package {
            if let Some(rest) = t.strip_prefix("name") {
                let rest = rest.trim().trim_start_matches('=').trim();
                let name = rest.trim_matches('"').trim_matches('\'').to_string();
                if !name.is_empty() {
                    return Some(name);
                }
            }
        }
    }
    None
}

fn write_if_allowed(
    abs: &Path,
    rel: &str,
    content: &str,
    write: bool,
    force: bool,
    actions: &mut Vec<BootstrapAction>,
) -> Result<()> {
    if abs.exists() && !force {
        // Allow overwrite of generated files marked generated: true
        if let Ok(existing) = std::fs::read_to_string(abs) {
            if existing.contains("generated: true") && write {
                std::fs::write(abs, content)?;
                actions.push(BootstrapAction {
                    action: "update".into(),
                    path: rel.into(),
                    detail: "regenerated (generated: true)".into(),
                });
                return Ok(());
            }
        }
        actions.push(BootstrapAction {
            action: "skip".into(),
            path: rel.into(),
            detail: "exists (use --force to overwrite)".into(),
        });
        return Ok(());
    }
    if write {
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(abs, content)?;
        actions.push(BootstrapAction {
            action: "create".into(),
            path: rel.into(),
            detail: "wrote file".into(),
        });
    } else {
        actions.push(BootstrapAction {
            action: "would_create".into(),
            path: rel.into(),
            detail: "file".into(),
        });
    }
    Ok(())
}

const ADR_TEMPLATE: &str = r#"---
tags: [adr]
node_type: adr
---
# ADR-XXXX: Title

## Status

Proposed

## Context

<!-- Why is this decision needed? -->

## Decision

<!-- What did we decide? -->

## Consequences

<!-- Trade-offs, follow-ups -->

<!-- After writing, rename to docs/adr/000N-slug.md and link from goals/concepts. -->
"#;

const GOALS_DIR_README: &str = r#"---
tags: [goal, index]
node_type: goal
---
# Goals index

Place project goals and non-goals here.

- `from-readme.md` — harvested by `rustbrain bootstrap` (when README exists)
- Hand-written: `rustbrain note new --type goal --title "…"`
- Add ADRs under `docs/adr/` for decisions that achieve these goals
"#;

const PLANS_DIR_README: &str = r#"---
node_type: plan
tags: [plan, index]
status: backlog
---
# Plans / roadmaps / tasklists

Use **`node_type: plan`** (aliases: roadmap, backlog, todo, tasklist) for prioritization
and work queues — not ship history (that is root **`CHANGELOG.md`** → hub `changelog`).

**All of this is optional.** If you only use concepts/ADRs, rustbrain still works.

### Status (indexed densely on `sync`)

| Token | Meaning |
|-------|---------|
| `backlog` | not started |
| `in_progress` | active work |
| `qa` | review / testing |
| `done` | finished |
| `cancelled` | abandoned |
| `blocked` | stuck / on hold / reopened (`undone` still accepted as alias) |

Set overall with frontmatter `status: in_progress` and/or checkboxes:

- `- [ ]` backlog · `- [/]` in progress · `- [x]` done · `- [~]` cancelled · `- [?]` qa · `- [!]` blocked

```bash
rustbrain note new --type plan --title "Q3 platform roadmap"
# edit checkboxes / status, then:
rustbrain sync
rustbrain query "status:in_progress" --type plan --scores
rustbrain context "open plan tasks"
```

Root hubs (if present): `ROADMAP.md` → id `roadmap`, `BACKLOG.md` → id `backlog`.
"#;

/// Injected under `docs/AGENTS.md` so agents in the docs tree always see rustbrain ops.
const DOCS_AGENTS_MD: &str = r#"<!-- rustbrain-docs-agents: generated/maintained by rustbrain bootstrap/setup.
     Re-run with --force to refresh. Root AGENTS.md has full cookbook; this file is the docs-local mandate. -->
# AGENTS.md — docs/ knowledge protocol

This repository uses **[rustbrain](https://github.com/shan-alexander/rustbrain)** as the project second brain.

## Every agent turn (mandatory)

Before large edits, refactors, or claims about decisions/status/history, **run rustbrain** so context is graph-backed, not invented:

```bash
export PATH="$HOME/.cargo/bin:$PATH"   # after cargo install rustbrain

# Orient (content pack under token budget)
rustbrain context "why <decision> / how <feature> works / what shipped"

# Search notes (and --with-symbols when hunting code)
rustbrain query "<topic>" --scores
rustbrain query "serde" --scores   # crates → docs.rs notes after setup harvest

# Structure: who links to whom
rustbrain graph docs/<path>.md
rustbrain graph changelog          # if CHANGELOG.md exists
rustbrain graph roadmap            # if ROADMAP.md exists

# Health + orphans
rustbrain doctor
rustbrain doctor --orphans
```

After you change docs, ADRs, goals, plans, or code:

```bash
rustbrain sync
# optional: soft-link orphans, normalize pending WikiLinks
rustbrain links --auto
rustbrain links --apply --dry-run
rustbrain links --apply --write
```

## Where knowledge lives

| Artifact | Path / hub | Type |
|----------|------------|------|
| Ship history (Keep a Changelog) | root **`CHANGELOG.md`** → hub **`changelog`** | `changelog` |
| Goals | `docs/goals/` | `goal` |
| Decisions | `docs/adr/` | `adr` |
| Plans / roadmaps / todos | `docs/plans/`, root `ROADMAP.md` / `BACKLOG.md` | `plan` (optional) |
| Investigations | `docs/analysis/` | `analysis` |
| Concepts | `docs/concepts/` | `concept` |
| Edge cases | `docs/edge_cases/` | `edge_case` |
| Crate docs (docs.rs) | `docs/references/crates/` | `reference` (generated by setup/bootstrap) |

**Optional hubs:** CHANGELOG / ROADMAP / BACKLOG are *features when present*, never required.
Core workflow (goals, ADRs, concepts, analysis, symbols) works without them.

**Plan status tokens** (after sync): query `status:in_progress`, `status:done`, `plan_open:…`
or read the plan note summary line `plan status=… · open N · done M`.

**Do not invent ADR history or changelog entries.** If `CHANGELOG.md` exists, treat it as ground truth for releases; update it when you ship, then `sync`.

## Capture work

```bash
# Prefer scaffold, then edit the file, then sync
rustbrain note new --type adr --title "…"
rustbrain note new --type plan --title "…"
rustbrain note new --type analysis --title "…"
rustbrain note new --type goal --title "…"
rustbrain sync
```

Link notes→code with `symbol:Name` and code→notes with `[[docs/…]]` in rustdoc.

## Full repo cookbook

See root **`AGENTS.md`** for complete CLI variations (`setup`, bootstrap flags, export/import, …).

```bash
rustbrain --help
```
"#;

const BOOTSTRAP_CHECKLIST: &str = r#"# Bootstrap checklist

Generated by `rustbrain bootstrap`. Tick items as you promote drafts into real knowledge.

- [ ] Review `docs/goals/from-readme.md` (edit for accuracy)
- [ ] Promote real architectural decisions into `docs/adr/0001-….md` (do **not** invent history)
- [ ] Skim `docs/implementation/module-map.generated.md` and link key symbols from concepts
- [ ] Add `edge_case` notes for known traps
- [ ] Capture investigations as `analysis` notes under `docs/analysis/` (dated; optional recs → later ADR)
- [ ] Capture roadmaps/tasklists under `docs/plans/` (`note new --type plan`)
- [ ] Keep root `CHANGELOG.md` truthful when shipping (hub `changelog`)
- [ ] Read / customize root `AGENTS.md`, `docs/AGENTS.md`, and `SKILL.md` (or `.<harness>/skills/rustbrain/SKILL.md`) for AI coding agents
- [ ] Run `rustbrain sync`
- [ ] Run `rustbrain doctor` and clear pending links
- [ ] Optional: `rustbrain note new --type concept --title "…"` (scaffold, then edit the file)
"#;

/// Built-in root `AGENTS.md` body written by bootstrap/setup.
///
/// Override with `--agents-template`, `RUSTBRAIN_AGENTS_TEMPLATE`, or
/// workspace `AGENTS.template.md` / `.rustbrain/AGENTS.template.md`.
pub fn default_agents_md_template() -> &'static str {
    DEFAULT_AGENTS_MD
}

/// Resolve template content: explicit path → env → workspace templates → built-in.
pub fn resolve_agents_md_template(
    workspace: &Path,
    explicit: Option<&Path>,
) -> Result<(String, String)> {
    if let Some(p) = explicit {
        let text = std::fs::read_to_string(p).map_err(|e| {
            BrainError::Indexer(format!(
                "failed to read AGENTS template {}: {e}",
                p.display()
            ))
        })?;
        return Ok((text, format!("file:{}", p.display())));
    }
    if let Ok(env_path) = std::env::var("RUSTBRAIN_AGENTS_TEMPLATE") {
        let p = PathBuf::from(env_path.trim());
        if !p.as_os_str().is_empty() && p.is_file() {
            let text = std::fs::read_to_string(&p)?;
            return Ok((text, format!("env:{}", p.display())));
        }
    }
    for rel in [".rustbrain/AGENTS.template.md", "AGENTS.template.md"] {
        let p = workspace.join(rel);
        if p.is_file() {
            let text = std::fs::read_to_string(&p)?;
            return Ok((text, format!("workspace:{rel}")));
        }
    }
    Ok((DEFAULT_AGENTS_MD.to_string(), "builtin".into()))
}

/// True when `AGENTS.md` was written by rustbrain (builtin or refreshed template).
pub fn is_rustbrain_owned_agents_md(text: &str) -> bool {
    text.contains("<!-- rustbrain-agents-md:")
}

/// True when a custom `AGENTS.md` already has the appended rustbrain section.
pub fn has_rustbrain_agents_section(text: &str) -> bool {
    text.contains("<!-- rustbrain-agents-section:")
}

/// True when a `SKILL.md` looks like the rustbrain skill (frontmatter `name: rustbrain`).
pub fn is_rustbrain_skill_md(text: &str) -> bool {
    text.lines().take(40).any(|l| l.trim() == "name: rustbrain")
}

/// Agent harness directories that load `skills/<name>/SKILL.md`.
pub const AGENT_HARNESS_DIRS: &[&str] = &[
    ".grok", ".claude", ".cursor", ".agents", ".codex", ".gemini", ".ai",
];

fn rel_display(workspace: &Path, abs: &Path) -> String {
    abs.strip_prefix(workspace)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| abs.display().to_string())
}

fn write_text_file(
    abs: &Path,
    rel: &str,
    content: &str,
    write: bool,
    action: &str,
    detail: &str,
    actions: &mut Vec<BootstrapAction>,
) -> Result<()> {
    if write {
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(abs, content)?;
        actions.push(BootstrapAction {
            action: action.into(),
            path: rel.into(),
            detail: detail.into(),
        });
    } else {
        actions.push(BootstrapAction {
            action: format!("would_{action}"),
            path: rel.into(),
            detail: detail.into(),
        });
    }
    Ok(())
}

fn write_agents_md(
    workspace: &Path,
    write: bool,
    explicit_template: Option<&Path>,
    actions: &mut Vec<BootstrapAction>,
) -> Result<()> {
    let (content, source) = resolve_agents_md_template(workspace, explicit_template)?;
    let out = workspace.join("AGENTS.md");
    if out.is_file() {
        let existing = std::fs::read_to_string(&out)?;
        if is_rustbrain_owned_agents_md(&existing) {
            if existing == content {
                actions.push(BootstrapAction {
                    action: "skip".into(),
                    path: "AGENTS.md".into(),
                    detail: format!("rustbrain-owned, already current (template={source})"),
                });
                return Ok(());
            }
            write_text_file(
                &out,
                "AGENTS.md",
                &content,
                write,
                "update",
                &format!("refreshed rustbrain-owned mandate (template={source})"),
                actions,
            )?;
            return Ok(());
        }
        if has_rustbrain_agents_section(&existing) {
            actions.push(BootstrapAction {
                action: "skip".into(),
                path: "AGENTS.md".into(),
                detail: "custom file already has rustbrain section".into(),
            });
            return Ok(());
        }
        let mut appended = existing;
        if !appended.ends_with('\n') && !appended.is_empty() {
            appended.push('\n');
        }
        appended.push('\n');
        appended.push_str(AGENTS_MD_APPEND_SECTION);
        if !appended.ends_with('\n') {
            appended.push('\n');
        }
        write_text_file(
            &out,
            "AGENTS.md",
            &appended,
            write,
            "update",
            "appended rustbrain section (custom AGENTS.md never overwritten)",
            actions,
        )?;
        return Ok(());
    }
    write_text_file(
        &out,
        "AGENTS.md",
        &content,
        write,
        "create",
        &format!("wrote file (template={source})"),
        actions,
    )
}

fn skill_md_destinations(workspace: &Path) -> Vec<PathBuf> {
    let mut dests = Vec::new();
    for dir in AGENT_HARNESS_DIRS {
        let harness = workspace.join(dir);
        if harness.is_dir() {
            dests.push(harness.join("skills").join("rustbrain").join("SKILL.md"));
        }
    }
    if dests.is_empty() {
        dests.push(workspace.join("SKILL.md"));
    }
    dests
}

fn write_skill_md(workspace: &Path, write: bool, actions: &mut Vec<BootstrapAction>) -> Result<()> {
    let content = crate::skill_template::default_skill_md_template();
    for dest in skill_md_destinations(workspace) {
        let rel = rel_display(workspace, &dest);
        if dest.is_file() {
            let existing = std::fs::read_to_string(&dest)?;
            if !is_rustbrain_skill_md(&existing) {
                actions.push(BootstrapAction {
                    action: "skip".into(),
                    path: rel,
                    detail: "exists and is not the rustbrain skill (name: rustbrain)".into(),
                });
                continue;
            }
            if existing == content {
                actions.push(BootstrapAction {
                    action: "skip".into(),
                    path: rel,
                    detail: "rustbrain skill already current".into(),
                });
                continue;
            }
            write_text_file(
                &dest,
                &rel,
                content,
                write,
                "update",
                "refreshed rustbrain skill",
                actions,
            )?;
            continue;
        }
        write_text_file(
            &dest,
            &rel,
            content,
            write,
            "create",
            "wrote rustbrain skill",
            actions,
        )?;
    }
    Ok(())
}

/// Convenience used by CLI tests / agents: non-interactive write bootstrap.
pub fn bootstrap_noninteractive(
    workspace: &Path,
    write: bool,
    force: bool,
) -> Result<BootstrapReport> {
    bootstrap_workspace(
        workspace,
        BootstrapOptions {
            mode: BootstrapMode::NonInteractive,
            write,
            force,
            setup_ignore: Some(true),
            import_gitignore: Some(workspace.join(".gitignore").is_file()),
            ignore_extras: true,
            harvest_readme: true,
            module_map: true,
            crate_docs: true,
            scaffold_docs: true,
            write_agents_md: Some(true),
            agents_template: None,
            write_skill_md: Some(true),
        },
    )
}

const DEFAULT_AGENTS_MD: &str = r#"<!-- rustbrain-agents-md: generated by `rustbrain bootstrap` / `rustbrain setup`.
     This file is rustbrain-owned and is refreshed on bootstrap.
     Hand-written AGENTS.md: remove this header so bootstrap appends a section
     instead of replacing the file. Custom files are never overwritten (`--force`
     included). Customize new/owned files: AGENTS.template.md |
     .rustbrain/AGENTS.template.md | --agents-template PATH |
     RUSTBRAIN_AGENTS_TEMPLATE=PATH
     Skip: rustbrain bootstrap --no-agents-md  /  setup --no-agents-md
-->
# AGENTS.md — working in this repository

This project uses **[rustbrain](https://github.com/shan-alexander/rustbrain)**: a local Markdown + SQLite second brain (no cloud required for search/index).

Ensure the CLI is on `PATH` (`export PATH="$HOME/.cargo/bin:$PATH"` after `cargo install rustbrain`).

Before claiming decisions, status, or history, and before large edits, **orient with rustbrain** (do not invent ADRs or changelog entries):

```bash
rustbrain context "why <decision> / how <feature> works / what shipped"
rustbrain query "<topic>" --scores
rustbrain sync    # after you edit docs, notes, or indexed code
```

Create notes with `rustbrain note new --type adr|goal|concept|analysis|plan --title "…"` (scaffold, then edit the file, then `sync`).

The full agent cookbook (CLI flags, bootstrap, multi-brain) lives in **SKILL.md** — either at the repo root or at `.<harness>/skills/rustbrain/SKILL.md` after bootstrap detects `.grok/`, `.claude/`, `.cursor/`, `.agents/`, `.codex/`, `.gemini/`, or `.ai/`.

```bash
rustbrain --help
```
"#;

/// Section appended to a *custom* root `AGENTS.md` (idempotent via the HTML markers).
const AGENTS_MD_APPEND_SECTION: &str = r#"<!-- rustbrain-agents-section: start -->
## rustbrain

This project uses **[rustbrain](https://github.com/shan-alexander/rustbrain)**. Before claiming decisions, status, or history:

```bash
rustbrain context "why <decision> / how <feature> works"
rustbrain query "<topic>" --scores
rustbrain sync
```

Cookbook: `SKILL.md` or `.<harness>/skills/rustbrain/SKILL.md` (installed by `rustbrain bootstrap` when `.grok/`, `.claude/`, `.cursor/`, `.agents/`, `.codex/`, `.gemini/`, or `.ai/` exists).
<!-- rustbrain-agents-section: end -->
"#;

/// Ensure `.brain/` is listed in the workspace `.gitignore` (create file if needed).
fn ensure_gitignore_brain(
    workspace: &Path,
    write: bool,
    actions: &mut Vec<BootstrapAction>,
) -> Result<()> {
    let gi = workspace.join(".gitignore");
    if gi.is_file() {
        let text = std::fs::read_to_string(&gi)?;
        let already = text.lines().any(|l| {
            let t = l.trim();
            t == ".brain/" || t == ".brain" || t == "**/.brain/" || t == "/.brain/"
        });
        if already {
            actions.push(BootstrapAction {
                action: "skip".into(),
                path: ".gitignore".into(),
                detail: ".brain/ already ignored".into(),
            });
            return Ok(());
        }
        if write {
            let mut out = text;
            if !out.ends_with('\n') && !out.is_empty() {
                out.push('\n');
            }
            out.push_str("\n# rustbrain local index\n.brain/\n");
            std::fs::write(&gi, out)?;
            actions.push(BootstrapAction {
                action: "update".into(),
                path: ".gitignore".into(),
                detail: "appended .brain/".into(),
            });
        } else {
            actions.push(BootstrapAction {
                action: "would_update".into(),
                path: ".gitignore".into(),
                detail: "append .brain/".into(),
            });
        }
    } else if write {
        std::fs::write(&gi, "# rustbrain local index\n.brain/\n")?;
        actions.push(BootstrapAction {
            action: "create".into(),
            path: ".gitignore".into(),
            detail: "created with .brain/".into(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn bootstrap_writes_scaffold() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("README.md"),
            "# Demo\n\n## Why\n\nFast local tools.\n\n## Features\n\n- A\n- B\n",
        )
        .unwrap();
        std::fs::write(dir.path().join(".gitignore"), "target/\n*.log\n").unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "pub fn hello() {}\n").unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();

        let report = bootstrap_noninteractive(dir.path(), true, false).unwrap();
        assert!(report.wrote);
        assert!(dir.path().join("docs/goals").is_dir());
        assert!(dir.path().join("docs/plans").is_dir());
        assert!(dir.path().join("docs/adr/TEMPLATE.md").is_file());
        assert!(dir.path().join(".rustbrainignore").is_file());
        assert!(dir.path().join("docs/goals/from-readme.md").is_file());
        assert!(
            dir.path()
                .join("docs/references/crate-docs.generated.md")
                .is_file(),
            "expected crate docs harvest"
        );
        assert!(dir.path().join("docs/references/crates").is_dir());
        let agents = std::fs::read_to_string(dir.path().join("AGENTS.md")).unwrap();
        assert!(
            agents.contains("rustbrain"),
            "AGENTS.md should mention rustbrain"
        );
        assert!(
            is_rustbrain_owned_agents_md(&agents),
            "fresh AGENTS.md should be rustbrain-owned"
        );
        assert!(
            agents.contains("rustbrain context") && agents.contains("SKILL.md"),
            "thin AGENTS.md should mandate context and point at SKILL.md"
        );
        assert!(
            !agents.contains("## First time"),
            "thin AGENTS.md must not contain the old CLI cookbook"
        );
        let skill = std::fs::read_to_string(dir.path().join("SKILL.md")).unwrap();
        assert!(
            is_rustbrain_skill_md(&skill),
            "no harness dir → root SKILL.md"
        );
        let docs_agents = std::fs::read_to_string(dir.path().join("docs/AGENTS.md")).unwrap();
        assert!(
            docs_agents.contains("Every agent turn") && docs_agents.contains("rustbrain context"),
            "docs/AGENTS.md must mandate rustbrain every turn"
        );
        let gi = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert!(
            gi.contains(".brain/"),
            "expected .brain/ in gitignore: {gi}"
        );
        #[cfg(feature = "ast")]
        assert!(dir
            .path()
            .join("docs/implementation/module-map.generated.md")
            .is_file());
    }

    #[test]
    fn bootstrap_can_skip_agents_md() {
        let dir = tempdir().unwrap();
        bootstrap_workspace(
            dir.path(),
            BootstrapOptions {
                mode: BootstrapMode::NonInteractive,
                write: true,
                force: false,
                setup_ignore: Some(false),
                import_gitignore: Some(false),
                ignore_extras: false,
                harvest_readme: false,
                module_map: false,
                crate_docs: false,
                scaffold_docs: true,
                write_agents_md: Some(false),
                agents_template: None,
                write_skill_md: Some(false),
            },
        )
        .unwrap();
        assert!(!dir.path().join("AGENTS.md").exists());
        assert!(!dir.path().join("SKILL.md").exists());
    }

    #[test]
    fn bootstrap_uses_custom_agents_template() {
        let dir = tempdir().unwrap();
        let tpl = dir.path().join("my-agents.tpl");
        std::fs::write(&tpl, "# Custom agents file\n\nUse the force.\n").unwrap();
        bootstrap_workspace(
            dir.path(),
            BootstrapOptions {
                mode: BootstrapMode::NonInteractive,
                write: true,
                force: false,
                setup_ignore: Some(false),
                import_gitignore: Some(false),
                ignore_extras: false,
                harvest_readme: false,
                module_map: false,
                crate_docs: false,
                scaffold_docs: false,
                write_agents_md: Some(true),
                agents_template: Some(tpl),
                write_skill_md: Some(false),
            },
        )
        .unwrap();
        let agents = std::fs::read_to_string(dir.path().join("AGENTS.md")).unwrap();
        assert!(agents.contains("Use the force"));
    }

    #[test]
    fn bootstrap_uses_workspace_agents_template_file() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("AGENTS.template.md"),
            "# From workspace template\n",
        )
        .unwrap();
        bootstrap_workspace(
            dir.path(),
            BootstrapOptions {
                mode: BootstrapMode::NonInteractive,
                write: true,
                force: false,
                setup_ignore: Some(false),
                import_gitignore: Some(false),
                ignore_extras: false,
                harvest_readme: false,
                module_map: false,
                crate_docs: false,
                scaffold_docs: false,
                write_agents_md: Some(true),
                agents_template: None,
                write_skill_md: Some(false),
            },
        )
        .unwrap();
        let agents = std::fs::read_to_string(dir.path().join("AGENTS.md")).unwrap();
        assert!(agents.contains("From workspace template"));
    }

    fn agents_skill_opts(
        write_agents: bool,
        write_skill: bool,
        force: bool,
        template: Option<PathBuf>,
    ) -> BootstrapOptions {
        BootstrapOptions {
            mode: BootstrapMode::NonInteractive,
            write: true,
            force,
            setup_ignore: Some(false),
            import_gitignore: Some(false),
            ignore_extras: false,
            harvest_readme: false,
            module_map: false,
            crate_docs: false,
            scaffold_docs: false,
            write_agents_md: Some(write_agents),
            agents_template: template,
            write_skill_md: Some(write_skill),
        }
    }

    #[test]
    fn bootstrap_appends_section_to_custom_agents_md_even_with_force() {
        let dir = tempdir().unwrap();
        let custom = "# Host rules\n\nDo not clobber me.\n";
        std::fs::write(dir.path().join("AGENTS.md"), custom).unwrap();
        let report =
            bootstrap_workspace(dir.path(), agents_skill_opts(true, false, true, None)).unwrap();
        let agents = std::fs::read_to_string(dir.path().join("AGENTS.md")).unwrap();
        assert!(
            agents.contains("Do not clobber me"),
            "custom body kept: {agents}"
        );
        assert!(
            has_rustbrain_agents_section(&agents),
            "section appended: {agents}"
        );
        assert!(!is_rustbrain_owned_agents_md(&agents));
        assert!(
            report
                .actions
                .iter()
                .any(|a| a.path == "AGENTS.md" && a.action == "update"),
            "expected append update: {:?}",
            report.actions
        );

        bootstrap_workspace(dir.path(), agents_skill_opts(true, false, true, None)).unwrap();
        let again = std::fs::read_to_string(dir.path().join("AGENTS.md")).unwrap();
        assert_eq!(
            again
                .matches("<!-- rustbrain-agents-section: start -->")
                .count(),
            1,
            "append is idempotent: {again}"
        );
    }

    #[test]
    fn bootstrap_refreshes_rustbrain_owned_agents_md() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("AGENTS.md"),
            "<!-- rustbrain-agents-md: generated by old fat cookbook -->\n# stale\n",
        )
        .unwrap();
        bootstrap_workspace(dir.path(), agents_skill_opts(true, false, false, None)).unwrap();
        let agents = std::fs::read_to_string(dir.path().join("AGENTS.md")).unwrap();
        assert!(is_rustbrain_owned_agents_md(&agents));
        assert!(agents.contains("SKILL.md"));
        assert!(!agents.contains("stale"));
    }

    #[test]
    fn bootstrap_installs_skill_into_every_detected_harness() {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".grok")).unwrap();
        std::fs::create_dir_all(dir.path().join(".claude")).unwrap();
        bootstrap_workspace(dir.path(), agents_skill_opts(false, true, false, None)).unwrap();
        let grok = dir.path().join(".grok/skills/rustbrain/SKILL.md");
        let claude = dir.path().join(".claude/skills/rustbrain/SKILL.md");
        assert!(grok.is_file(), "missing {grok:?}");
        assert!(claude.is_file(), "missing {claude:?}");
        assert!(
            !dir.path().join("SKILL.md").exists(),
            "root SKILL.md only when no harness"
        );
        assert!(is_rustbrain_skill_md(
            &std::fs::read_to_string(&grok).unwrap()
        ));
    }

    #[test]
    fn bootstrap_skips_foreign_skill_md() {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".cursor/skills/rustbrain")).unwrap();
        let dest = dir.path().join(".cursor/skills/rustbrain/SKILL.md");
        std::fs::write(&dest, "---\nname: something-else\n---\n# Hands off\n").unwrap();
        bootstrap_workspace(dir.path(), agents_skill_opts(false, true, true, None)).unwrap();
        let got = std::fs::read_to_string(&dest).unwrap();
        assert!(
            got.contains("Hands off"),
            "foreign skill must not be replaced: {got}"
        );
        assert!(!is_rustbrain_skill_md(&got));
    }

    #[test]
    fn bootstrap_refreshes_rustbrain_skill_md() {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".grok/skills/rustbrain")).unwrap();
        let dest = dir.path().join(".grok/skills/rustbrain/SKILL.md");
        std::fs::write(&dest, "---\nname: rustbrain\n---\n# stale skill\n").unwrap();
        bootstrap_workspace(dir.path(), agents_skill_opts(false, true, false, None)).unwrap();
        let got = std::fs::read_to_string(&dest).unwrap();
        assert!(is_rustbrain_skill_md(&got));
        assert!(!got.contains("stale skill"));
        assert!(got.contains("inject and persist project memory") || got.contains("rustbrain"));
    }

    #[test]
    fn embedded_skill_matches_repo_root_when_present() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../SKILL.md");
        if root.is_file() {
            let disk = std::fs::read_to_string(&root)
                .unwrap()
                .replace("\r\n", "\n");
            let embedded = crate::default_skill_md_template().replace("\r\n", "\n");
            assert_eq!(
                disk, embedded,
                "repo-root SKILL.md must match crates/rustbrain-core/src/templates/SKILL.md"
            );
        }
    }
}
