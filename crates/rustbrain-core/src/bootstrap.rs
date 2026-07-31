//! Deterministic workspace bootstrap for mature repositories.
//!
//! Creates docs scaffolds, optional `.rustbrainignore`, README-derived goals,
//! and an AST module map — **without** inventing ADRs or calling cloud models.

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
    /// Scaffold docs/ directory tree + templates.
    pub scaffold_docs: bool,
    /// Write root `AGENTS.md` (agent cookbook for this repo). Default true when `None`.
    pub write_agents_md: Option<bool>,
    /// Optional path to a custom `AGENTS.md` template file (overrides discovery + built-in).
    pub agents_template: Option<PathBuf>,
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
            scaffold_docs: true,
            write_agents_md: None,
            agents_template: None,
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
    "docs/edge_cases",
    "docs/implementation",
    "docs/experience",
];

/// Run deterministic bootstrap for `workspace`.
pub fn bootstrap_workspace(workspace: &Path, mut opts: BootstrapOptions) -> Result<BootstrapReport> {
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

    if opts.write_agents_md.unwrap_or(true) {
        write_agents_md(
            &workspace,
            opts.write,
            opts.force,
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
            "run `rustbrain sync` then `rustbrain doctor` (or `rustbrain setup --yes` next time)".into()
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
        return Ok(());
    }

    println!("rustbrain bootstrap — {}", workspace.display());
    println!("Deterministic setup (no cloud AI). Press Enter to accept [defaults].\n");

    if opts.setup_ignore.is_none() {
        let has = workspace.join(".rustbrainignore").is_file();
        let def = if has { "n" } else { "Y" };
        let ans = prompt(
            &format!(
                "Create/update .rustbrainignore? [Y/n] (default {def})"
            ),
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
            let ans = prompt("Harvest README.md into docs/goals/from-readme.md? [Y/n]", "Y")?;
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

    let ans = prompt("Scaffold docs/ tree + ADR/goal templates? [Y/n]", "Y")?;
    opts.scaffold_docs = ans_yes(&ans, true);

    if opts.write_agents_md.is_none() {
        let has = workspace.join("AGENTS.md").is_file();
        let def = if has { "n" } else { "Y" };
        let ans = prompt(
            &format!(
                "Write root AGENTS.md (agent cookbook for rustbrain)? [Y/n] (default {def})"
            ),
            def,
        )?;
        opts.write_agents_md = Some(ans_yes(&ans, !has));
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

    Ok(())
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
            detail: format!(
                "ignore file (import_gitignore={import_gitignore}, extras={extras})"
            ),
        });
    } else {
        actions.push(BootstrapAction {
            action: "would_create".into(),
            path: rel.into(),
            detail: format!(
                "ignore file (import_gitignore={import_gitignore}, extras={extras})"
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
- Add ADRs under `docs/adr/` for decisions that achieve these goals
"#;

const BOOTSTRAP_CHECKLIST: &str = r#"# Bootstrap checklist

Generated by `rustbrain bootstrap`. Tick items as you promote drafts into real knowledge.

- [ ] Review `docs/goals/from-readme.md` (edit for accuracy)
- [ ] Promote real architectural decisions into `docs/adr/0001-….md` (do **not** invent history)
- [ ] Skim `docs/implementation/module-map.generated.md` and link key symbols from concepts
- [ ] Add `edge_case` notes for known traps
- [ ] Capture investigations as `analysis` notes under `docs/analysis/` (dated; optional recs → later ADR)
- [ ] Read / customize root `AGENTS.md` for AI coding agents
- [ ] Run `rustbrain sync`
- [ ] Run `rustbrain doctor` and clear pending links
- [ ] Optional: `rustbrain note new --type concept --title "…" --body "…"` for atomic notes
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
    for rel in [
        ".rustbrain/AGENTS.template.md",
        "AGENTS.template.md",
    ] {
        let p = workspace.join(rel);
        if p.is_file() {
            let text = std::fs::read_to_string(&p)?;
            return Ok((text, format!("workspace:{rel}")));
        }
    }
    Ok((DEFAULT_AGENTS_MD.to_string(), "builtin".into()))
}

fn write_agents_md(
    workspace: &Path,
    write: bool,
    force: bool,
    explicit_template: Option<&Path>,
    actions: &mut Vec<BootstrapAction>,
) -> Result<()> {
    let (content, source) = resolve_agents_md_template(workspace, explicit_template)?;
    let out = workspace.join("AGENTS.md");
    // Prefer not clobbering a hand-edited AGENTS.md unless --force.
    // If content is still the rustbrain-generated header, allow --force refresh.
    if out.is_file() && !force {
        actions.push(BootstrapAction {
            action: "skip".into(),
            path: "AGENTS.md".into(),
            detail: format!("exists (use --force to overwrite; template source was {source})"),
        });
        return Ok(());
    }
    write_if_allowed(
        &out,
        "AGENTS.md",
        &content,
        write,
        force,
        actions,
    )?;
    if let Some(last) = actions.last_mut() {
        if last.path == "AGENTS.md" && (last.action == "create" || last.action == "would_create" || last.action == "update") {
            last.detail = format!("{} (template={source})", last.detail);
        }
    }
    Ok(())
}

/// Convenience used by CLI tests / agents: non-interactive write bootstrap.
pub fn bootstrap_noninteractive(workspace: &Path, write: bool, force: bool) -> Result<BootstrapReport> {
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
            scaffold_docs: true,
            write_agents_md: Some(true),
            agents_template: None,
        },
    )
}

const DEFAULT_AGENTS_MD: &str = r#"<!-- rustbrain-agents-md: generated by `rustbrain bootstrap` / `rustbrain setup`.
     Edit freely. Re-run with --force to replace from the template.
     Customize: AGENTS.template.md | .rustbrain/AGENTS.template.md
     | --agents-template PATH | RUSTBRAIN_AGENTS_TEMPLATE=PATH
     Skip: rustbrain bootstrap --no-agents-md  /  setup --no-agents-md
-->
# AGENTS.md — working in this repository

This project uses **[rustbrain](https://github.com/shan-alexander/rustbrain)**: a local Markdown + SQLite second brain (no cloud required for search/index).

Ensure the CLI is on `PATH` (`export PATH="$HOME/.cargo/bin:$PATH"` after `cargo install rustbrain`).

---

## First time

| Command | What it does | What to expect |
|---------|----------------|----------------|
| `rustbrain setup --yes` | init + bootstrap + sync + doctor | Creates `.brain/`, `docs/`, `.rustbrainignore`, **`AGENTS.md`**, harvests README → `docs/goals/from-readme.md` if present, AST module map, then indexes |
| `rustbrain setup --yes --no-agents-md` | same, skip this file | No `AGENTS.md` write |
| `rustbrain setup --yes --agents-template PATH` | use custom AGENTS body | Your template becomes root `AGENTS.md` |
| `rustbrain setup --yes --force` | overwrite generated bootstrap files | Regenerates `from-readme`, module-map, ignore, **and** `AGENTS.md` if present |
| `rustbrain setup --yes --no-bootstrap` | init + sync only | No docs scaffold |
| `rustbrain setup --yes --no-doctor` | skip final health print | Still syncs |

Step-by-step equivalent:

```bash
rustbrain init
rustbrain bootstrap --yes --write
rustbrain sync
rustbrain doctor
```

**Empty / thin README:** bootstrap still succeeds. `from-readme` is skipped (no README) or thin (scrappy README). `doctor` reports `no_readme` / `sparse_readme` / `scaffold_only` as **info** — not failures. Fill knowledge with notes, not invented history.

---

## Everyday loop

```bash
rustbrain context "why <decision> / how does <feature> work"   # orient
rustbrain query "topic" --scores                               # search notes
rustbrain note new --type adr --title "…" --note "…"           # capture decision
rustbrain sync && rustbrain doctor && rustbrain links          # after edits
```

---

## CLI reference (variations)

### `setup` / `bootstrap` / `init` / `sync`

| Command | Use when | Expect |
|---------|----------|--------|
| `setup --yes` | Cold start / CI / agents | Full scaffold + index; prefer this over multi-step |
| `bootstrap --yes --write` | Scaffold only (already have `.brain`) | Files under `docs/`, optional harvest; **no** full re-think of ADRs |
| `bootstrap --dry-run` | See plan | Prints actions; writes nothing |
| `bootstrap --yes --write --no-agents-md` | Scaffold without cookbook | No `AGENTS.md` |
| `bootstrap --yes --write --agents-template ./AGENTS.template.md` | Org template | Copies that file to `AGENTS.md` |
| `init` | Empty store only | `.brain/db.sqlite`; does **not** create docs or index |
| `sync` | After Markdown/code changes | Re-index; content-hash skips unchanged files; `file_errors=N` if some files fail |
| `sync` from a subdirectory | CWD anywhere under the repo | Prefer `rustbrain sync -w /repo` or run from root; open walks parents for query/context/doctor |

### `doctor`

| Command | Expect |
|---------|--------|
| `rustbrain doctor` | Text health: db/mmap/counts + **info** findings (sparse README, scaffold-only, template ADR, pending links, …) |
| `rustbrain doctor --json` | Same as JSON for tools |
| `rustbrain doctor --strict` | Exit **1** if unhealthy **or** any pending links |

Doctor walks parent dirs for `.brain` (like git). **Info ≠ broken** — e.g. `scaffold_only` means “few real notes yet”, not corrupt DB.

### `query` (search)

Default is **note-first** (goals/ADRs/concepts; symbols excluded).

| Command | Expect |
|---------|--------|
| `query "duckdb"` | Ranked notes; may hit README hub / from-readme / ADRs |
| `query "duckdb" --scores` | Same + numeric scores + reasons |
| `query "open" --with-symbols` | Include code symbols (methods, types) |
| `query "x" --all-types` | All node types (alias of with-symbols for type filters cleared) |
| `query "x" --type goal,adr,concept` | Only those types |
| `query "x" -n 10` | Cap results |
| `query "x" --all-workspaces` | Merge across registered local workspaces |
| `query "x" -w /path/to/repo` | Explicit workspace |

Natural language: stopwords dropped; multi-token uses OR (`why egui not tauri` → egui OR tauri). **Garbage-in:** thin README → thin hits. Empty results print a hint (`--with-symbols` or sync).

### `context` (agent pack)

Builds FTS seeds + optional graph hops under a token budget. Default format: **markdown**.

| Command | Expect |
|---------|--------|
| `context "why egui not tauri"` | Seeds notes; packs **body excerpts** (not titles only); stopword-aware |
| `context "summarize architecture"` | If FTS is weak/generic, **hub fallback** (README / harvest / module map) |
| `context "topic" -F xml` | XML-escaped for tool protocols |
| `context "topic" -m 800` | Smaller token budget |
| `context "topic" --hops 0` | Seeds only (no graph neighbors) |
| `context "topic" --hops 2` | Deeper graph (noisier) |
| `context "topic" --with-symbols` | Allow symbols as FTS seeds |
| `context "topic" --no-hop-symbols` | Never pack symbol neighbors |
| `context "topic" --type adr,goal` | Seed type filter |
| `context "topic" -p "…"` | Same as positional prompt |
| `context` from `src/` | Finds parent `.brain` automatically |

Packing prefers **seeds and ADRs/goals** over symbol noise; skips ADR `TEMPLATE`; dedupes README vs from-readme; strips YAML frontmatter from excerpts.

### `note new`

| Command | Expect |
|---------|--------|
| `note new --type adr --title "T" --note "body"` | Writes `docs/adr/….md` **and syncs** (searchable immediately) |
| `note new --type concept --title "T" --note "…"` | Under `docs/concepts/` |
| `note new --type goal --title "T" --note "…"` | Under `docs/goals/` |
| `note new … --tags a,b --aliases x` | Frontmatter tags/aliases |
| `note new … --no-sync` | Write file only; run `sync` later |
| `note new … --force` | Overwrite existing path |
| `note new … -w /repo` | Explicit workspace |

Types: `goal`, `adr`, `alternative`, `concept`, `analysis`, `reference`, `edge_case`.
- **concept** — timeless “what is X”
- **analysis** — dated investigation (crate compare, design options, `cargo bench` / criterion review, data digests); recommendations optional; promote decisions to **adr**
- **adr** — we chose X
- **edge_case** — a specific trap

Link code with `symbol:Type::method`.

### `links` / `watch` / `export` / `import`

| Command | Expect |
|---------|--------|
| `links` | Unresolved WikiLinks / `symbol:` targets |
| `links --json` | Machine-readable |
| `watch` | Debounced re-sync on file changes (Ctrl-C to stop) |
| `watch --debounce-ms 500` | Slower debounce |
| `export --out x.brainbundle` | Portable JSON graph (AST optionally decoupled) |
| `import --input x.brainbundle` | Merge bundle into this brain + remmap |

---

## Where knowledge lives

| Path | Purpose |
|------|---------|
| `README.md` | Hub node `readme` (quality of harvest depends on this) |
| `docs/goals/from-readme.md` | **Algorithmic** harvest of README sections (not an LLM) |
| `docs/goals/`, `docs/adr/`, `docs/analysis/`, … | Hand-written project knowledge |
| `docs/analysis/` | Dated investigations (`note new --type analysis`) |
| `docs/implementation/module-map.generated.md` | AST symbol list |
| `AGENTS.md` | This file — agent ops for *this* repo |
| `.brain/` | Local index — **never commit** |
| `.rustbrainignore` | Extra index skips |

---

## Conventions

- Prefer short factual ADRs over chat logs.
- Link code: `symbol:Name` / `symbol:crate::mod::Name` / `[[symbol:…]]`.
- Frontmatter when useful:

  ```yaml
  ---
  tags: [topic]
  node_type: adr
  aliases: [short-name]
  ---
  ```

- Do **not** invent ADR history. Do **not** commit `.brain/`.
- After improving README: `rustbrain bootstrap --yes --write --force && rustbrain sync`.

---

## Full help

```bash
rustbrain --help
rustbrain <command> --help
```

Upstream CLI book: rustbrain repo `docs/CLI.md`.
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
        std::fs::write(
            &gi,
            "# rustbrain local index\n.brain/\n",
        )?;
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
        assert!(dir.path().join("docs/adr/TEMPLATE.md").is_file());
        assert!(dir.path().join(".rustbrainignore").is_file());
        assert!(dir.path().join("docs/goals/from-readme.md").is_file());
        let agents = std::fs::read_to_string(dir.path().join("AGENTS.md")).unwrap();
        assert!(
            agents.contains("rustbrain"),
            "AGENTS.md should mention rustbrain"
        );
        assert!(agents.contains("rustbrain context") || agents.contains("setup --yes"));
        let gi = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert!(gi.contains(".brain/"), "expected .brain/ in gitignore: {gi}");
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
                scaffold_docs: true,
                write_agents_md: Some(false),
                agents_template: None,
            },
        )
        .unwrap();
        assert!(!dir.path().join("AGENTS.md").exists());
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
                scaffold_docs: false,
                write_agents_md: Some(true),
                agents_template: Some(tpl),
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
                scaffold_docs: false,
                write_agents_md: Some(true),
                agents_template: None,
            },
        )
        .unwrap();
        let agents = std::fs::read_to_string(dir.path().join("AGENTS.md")).unwrap();
        assert!(agents.contains("From workspace template"));
    }
}
