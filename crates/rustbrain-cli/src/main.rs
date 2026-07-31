//! # rustbrain CLI
//!
//! Command-line interface for [rustbrain](https://github.com/shan-alexander/rustbrain).
//!
//! ```bash
//! rustbrain setup --yes          # init + bootstrap + sync (+ doctor)
//! rustbrain note new --type concept --title "X" --note "body for agents"
//! rustbrain query "topic" --scores
//! rustbrain context "why egui not tauri"
//! ```

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use rustbrain_core::{
    bootstrap_workspace, create_note, run_doctor, BootstrapMode, BootstrapOptions, Brain,
    GlobalRegistry, NoteNewOptions, NodeType, QueryOptions,
};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "rustbrain")]
#[command(
    about = "Project-scoped, Rust-first second-brain knowledge engine for engineers and AI agents",
    long_about = None,
    version,
    after_help = AFTER_HELP
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

/// Shown under `rustbrain --help` and after common commands.
const AFTER_HELP: &str = "\
Examples:
  rustbrain setup --yes
  rustbrain sync
  rustbrain doctor
  rustbrain context \"why <decision>\"
  rustbrain query \"topic\" --scores

Recommended first goal (body goes after the H1 title; --body and --note are the same):
  rustbrain note new --type goal --title \"Use rustbrain well\" \\
    --body \"Prefer rustbrain context/query before large refactors. Capture decisions with note new --type adr. Run sync after doc/code changes. Keep docs truthful — do not invent ADR history.\"
";

fn print_note_tip() {
    eprintln!(
        "\
tip: write a goal into the brain (title = H1; --body/--note = body after it):
  rustbrain note new --type goal --title \"Use rustbrain well\" \\
    --body \"Prefer rustbrain context/query before large refactors. Capture decisions with note new --type adr. Run sync after doc/code changes. Keep docs truthful — do not invent ADR history.\""
    );
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a `.brain` directory in the workspace
    Init {
        /// Workspace directory (defaults to current directory)
        #[arg(default_value = ".")]
        workspace: PathBuf,
    },
    /// One-shot: init + bootstrap + sync (+ optional doctor) for agents/CI
    Setup {
        /// Workspace root
        #[arg(default_value = ".")]
        workspace: PathBuf,
        /// Non-interactive (accepted for symmetry with bootstrap; setup is always non-interactive)
        #[arg(long, short = 'y', default_value_t = true)]
        yes: bool,
        /// Overwrite generated bootstrap files
        #[arg(long, default_value_t = false)]
        force: bool,
        /// Skip doctor at the end
        #[arg(long, default_value_t = false)]
        no_doctor: bool,
        /// Skip bootstrap (only init + sync)
        #[arg(long, default_value_t = false)]
        no_bootstrap: bool,
        /// Do not write root AGENTS.md during bootstrap
        #[arg(long, default_value_t = false)]
        no_agents_md: bool,
        /// Custom AGENTS.md template file (overrides AGENTS.template.md / built-in)
        #[arg(long, value_name = "PATH")]
        agents_template: Option<PathBuf>,
    },
    /// Deterministic docs/ignore bootstrap for mature repositories
    Bootstrap {
        /// Workspace root
        #[arg(default_value = ".")]
        workspace: PathBuf,
        /// Write files (default: true when --yes; otherwise interactive may ask)
        #[arg(long, default_value_t = false)]
        write: bool,
        /// Dry-run: print plan only
        #[arg(long, default_value_t = false)]
        dry_run: bool,
        /// Non-interactive (agents/CI): sensible defaults, no prompts
        #[arg(long, short = 'y', default_value_t = false)]
        yes: bool,
        /// Overwrite existing generated files / ignore file
        #[arg(long, default_value_t = false)]
        force: bool,
        /// Skip .rustbrainignore setup
        #[arg(long, default_value_t = false)]
        no_ignore: bool,
        /// Force import of root .gitignore into .rustbrainignore
        #[arg(long, default_value_t = false)]
        import_gitignore: bool,
        /// Do not import .gitignore
        #[arg(long, default_value_t = false)]
        no_import_gitignore: bool,
        /// Do not write root AGENTS.md
        #[arg(long, default_value_t = false)]
        no_agents_md: bool,
        /// Custom AGENTS.md template file (overrides AGENTS.template.md / built-in)
        #[arg(long, value_name = "PATH")]
        agents_template: Option<PathBuf>,
    },
    /// Health check: pending links, ratios, schema
    Doctor {
        /// Workspace root
        #[arg(default_value = ".")]
        workspace: PathBuf,
        /// Emit JSON instead of text
        #[arg(long, default_value_t = false)]
        json: bool,
        /// Exit 1 when unhealthy or when pending links exist
        #[arg(long, default_value_t = false)]
        strict: bool,
    },
    /// Create structured Markdown notes
    Note {
        #[command(subcommand)]
        cmd: NoteCmd,
    },
    /// List unresolved WikiLink / symbol targets
    Links {
        /// Workspace root
        #[arg(short = 'w', long, default_value = ".")]
        workspace: PathBuf,
        /// Only show pending (default true)
        #[arg(long, default_value_t = true)]
        pending: bool,
        /// JSON output
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Index notes and code symbols; bake the CSR mmap cache
    Sync {
        /// Target workspace directory
        #[arg(default_value = ".")]
        workspace: PathBuf,
    },
    /// Query notes via ranked FTS5 + tag/alias boosts
    Query {
        /// Search query terms
        query: String,
        /// Query across all registered workspaces on this machine
        #[arg(long, default_value_t = false)]
        all_workspaces: bool,
        /// Max results
        #[arg(short = 'n', long, default_value_t = 25)]
        limit: usize,
        /// Show ranking scores
        #[arg(long, default_value_t = false)]
        scores: bool,
        /// Include code symbols (default: notes only — human/agent friendly)
        #[arg(long, default_value_t = false)]
        with_symbols: bool,
        /// Exclude symbol nodes (default behavior; kept for scripts)
        #[arg(long, default_value_t = false)]
        no_symbols: bool,
        /// Only these node types (comma-separated: goal,adr,concept,…)
        #[arg(long, value_name = "TYPES")]
        r#type: Option<String>,
        /// Include all types including symbols
        #[arg(long, default_value_t = false)]
        all_types: bool,
        /// Workspace root containing `.brain/`
        #[arg(short = 'w', long, default_value = ".")]
        workspace: PathBuf,
    },
    /// Build graph-aware prompt context (FTS seeds + CSR neighbors)
    Context {
        /// Topic or prompt (positional; same as `-p`)
        #[arg(value_name = "PROMPT")]
        prompt: Option<String>,
        /// Topic or prompt requirement (`-p` / `--for-prompt`)
        #[arg(short = 'p', long = "for-prompt")]
        for_prompt: Option<String>,
        /// Approximate max tokens for context output
        #[arg(short = 'm', long, default_value = "2048")]
        max_tokens: usize,
        /// Graph expansion hop depth (0 = seeds only)
        #[arg(long, default_value_t = 1)]
        hops: usize,
        /// Include symbols as FTS seeds (default: notes-first; hops to symbols still allowed)
        #[arg(long, default_value_t = false)]
        with_symbols: bool,
        /// Alias for `--with-symbols` (include every node type as seeds)
        #[arg(long, default_value_t = false)]
        all_types: bool,
        /// Exclude symbols from graph-hop packing (as well as seeds)
        #[arg(long, default_value_t = false)]
        no_hop_symbols: bool,
        /// Legacy alias: exclude symbol seeds (default behavior; kept for scripts)
        #[arg(long, default_value_t = false, hide = true)]
        no_symbols: bool,
        /// Only these seed types (comma-separated)
        #[arg(long, value_name = "TYPES")]
        r#type: Option<String>,
        /// Output format: `markdown` (default) or `xml`
        #[arg(short = 'F', long, default_value = "markdown")]
        format: String,
        /// Workspace root (walks parents for `.brain` like git)
        #[arg(short = 'w', long, default_value = ".")]
        workspace: PathBuf,
    },
    /// Watch workspace for changes and re-index (debounced)
    Watch {
        /// Workspace root
        #[arg(default_value = ".")]
        workspace: PathBuf,
        /// Debounce window in milliseconds
        #[arg(long, default_value_t = 300)]
        debounce_ms: u64,
    },
    /// Export brain into a portable `.brainbundle` file
    Export {
        /// Output path for export bundle
        #[arg(short, long)]
        out: PathBuf,
        /// Strip repo-local AST symbol nodes and file paths
        #[arg(long, default_value_t = true)]
        decouple_ast: bool,
        /// Workspace root
        #[arg(short = 'w', long, default_value = ".")]
        workspace: PathBuf,
    },
    /// Import a `.brainbundle` into the workspace brain
    Import {
        /// Input bundle path
        #[arg(short, long)]
        input: PathBuf,
        /// Workspace root
        #[arg(short = 'w', long, default_value = ".")]
        workspace: PathBuf,
    },
}

#[derive(Subcommand)]
enum NoteCmd {
    /// Create a new Markdown note under docs/
    New {
        /// Node type: goal, adr, alternative, concept, reference, edge_case
        #[arg(long, value_name = "TYPE")]
        r#type: String,
        /// Title (becomes the Markdown H1 + filename slug)
        #[arg(long)]
        title: String,
        /// Body text after the H1 title (alias: `--body`)
        #[arg(long = "note", visible_alias = "body", value_name = "TEXT")]
        note: Option<String>,
        /// Comma-separated tags
        #[arg(long)]
        tags: Option<String>,
        /// Comma-separated aliases
        #[arg(long)]
        aliases: Option<String>,
        /// Override directory (default from type)
        #[arg(long)]
        dir: Option<PathBuf>,
        /// Overwrite if the file exists
        #[arg(long, default_value_t = false)]
        force: bool,
        /// Workspace root
        #[arg(short = 'w', long, default_value = ".")]
        workspace: PathBuf,
        /// Skip indexing after write (default is to sync so notes are searchable immediately)
        #[arg(long, default_value_t = false)]
        no_sync: bool,
        /// Sync after write (default true; use `--no-sync` to skip)
        #[arg(long, default_value_t = true, hide = true)]
        sync: bool,
    },
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<ExitCode> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Init { workspace } => {
            let brain = Brain::create(&workspace)
                .with_context(|| format!("failed to init brain in {}", workspace.display()))?;
            println!(
                "initialized rustbrain at {}",
                brain.brain_dir().join("db.sqlite").display()
            );
            println!("nodes: {}", brain.database().count_nodes()?);
            println!("hint: run `rustbrain setup --yes` (or bootstrap --yes --write && sync)");
            if let Ok(mut reg) = GlobalRegistry::load() {
                let _ = reg.register(brain.workspace());
            }
            Ok(ExitCode::SUCCESS)
        }
        Commands::Setup {
            workspace,
            yes,
            force,
            no_doctor,
            no_bootstrap,
            no_agents_md,
            agents_template,
        } => {
            let _ = yes; // always non-interactive for setup
            let brain = Brain::create(&workspace)
                .with_context(|| format!("failed to init brain in {}", workspace.display()))?;
            println!(
                "setup: initialized {}",
                brain.brain_dir().join("db.sqlite").display()
            );
            if let Ok(mut reg) = GlobalRegistry::load() {
                let _ = reg.register(brain.workspace());
            }
            if !no_bootstrap {
                let import = workspace.join(".gitignore").is_file();
                let opts = BootstrapOptions {
                    mode: BootstrapMode::NonInteractive,
                    write: true,
                    force,
                    setup_ignore: Some(true),
                    import_gitignore: Some(import),
                    ignore_extras: true,
                    harvest_readme: true,
                    module_map: true,
                    scaffold_docs: true,
                    write_agents_md: Some(!no_agents_md),
                    agents_template,
                };
                let report = bootstrap_workspace(&workspace, opts)?;
                for a in &report.actions {
                    if a.action != "next" {
                        println!("  [{}] {} — {}", a.action, a.path, a.detail);
                    }
                }
                println!("setup: bootstrap complete");
            }
            let mut brain = Brain::open_or_create(&workspace)?;
            println!("setup: syncing {} ...", brain.workspace().display());
            let stats = brain.sync()?;
            println!(
                "setup: sync complete nodes_upserted={} symbols={} pending={} file_errors={}",
                stats.nodes_upserted, stats.symbol_anchors, stats.edges_pending, stats.file_errors
            );
            if let Ok(mut reg) = GlobalRegistry::load() {
                let _ = reg.register(brain.workspace());
            }
            if !no_doctor {
                let report = run_doctor(brain.workspace())?;
                print!("{}", report.to_text());
                if !report.healthy {
                    return Ok(ExitCode::FAILURE);
                }
            }
            println!("setup: done — try `rustbrain context \"topic\"`");
            print_note_tip();
            Ok(ExitCode::SUCCESS)
        }
        Commands::Bootstrap {
            workspace,
            write,
            dry_run,
            yes,
            force,
            no_ignore,
            import_gitignore,
            no_import_gitignore,
            no_agents_md,
            agents_template,
        } => {
            let write = if dry_run { false } else { write || yes };
            let mode = if yes {
                BootstrapMode::NonInteractive
            } else {
                BootstrapMode::Interactive
            };
            let import = if no_import_gitignore {
                Some(false)
            } else if import_gitignore {
                Some(true)
            } else if yes {
                Some(workspace.join(".gitignore").is_file())
            } else {
                None // interactive may ask
            };
            let write_agents = if no_agents_md {
                Some(false)
            } else if yes {
                Some(true)
            } else {
                None // interactive may ask
            };
            let opts = BootstrapOptions {
                mode,
                write,
                force,
                setup_ignore: if no_ignore { Some(false) } else if yes { Some(true) } else { None },
                import_gitignore: import,
                ignore_extras: true,
                harvest_readme: true,
                module_map: true,
                scaffold_docs: true,
                write_agents_md: write_agents,
                agents_template,
            };
            let report = bootstrap_workspace(&workspace, opts)?;
            for a in &report.actions {
                println!("[{}] {} — {}", a.action, a.path, a.detail);
            }
            if report.wrote {
                println!("\nbootstrap wrote files under {}", report.workspace.display());
                println!("next: rustbrain sync && rustbrain doctor");
            } else {
                println!("\ndry-run complete (no files written). pass --write or --yes --write");
            }
            Ok(ExitCode::SUCCESS)
        }
        Commands::Doctor {
            workspace,
            json,
            strict,
        } => {
            let report = run_doctor(&workspace)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print!("{}", report.to_text());
                print_note_tip();
            }
            let fail = !report.healthy || (strict && report.pending_links > 0);
            Ok(if fail {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            })
        }
        Commands::Note { cmd } => match cmd {
            NoteCmd::New {
                r#type,
                title,
                note,
                tags,
                aliases,
                dir,
                force,
                workspace,
                no_sync,
                sync,
            } => {
                let do_sync = sync && !no_sync;
                let node_type = NodeType::parse(&r#type).ok_or_else(|| {
                    anyhow::anyhow!(
                        "unknown type '{type}'. use: goal, adr, alternative, concept, reference, edge_case"
                    )
                })?;
                let tags = tags
                    .map(|s| {
                        s.split(',')
                            .map(|t| t.trim().to_string())
                            .filter(|t| !t.is_empty())
                            .collect()
                    })
                    .unwrap_or_default();
                let aliases = aliases
                    .map(|s| {
                        s.split(',')
                            .map(|t| t.trim().to_string())
                            .filter(|t| !t.is_empty())
                            .collect()
                    })
                    .unwrap_or_default();
                let created = create_note(
                    &workspace,
                    &NoteNewOptions {
                        node_type,
                        title,
                        note,
                        tags,
                        aliases,
                        dir,
                        force,
                    },
                )?;
                println!(
                    "wrote {} (node id after sync: {})",
                    created.rel_path.display(),
                    created.node_id
                );
                if do_sync {
                    let mut brain = Brain::open_or_create(&workspace)?;
                    let stats = brain.sync()?;
                    println!(
                        "synced: upserted={} pending={} file_errors={}",
                        stats.nodes_upserted, stats.edges_pending, stats.file_errors
                    );
                } else {
                    println!("hint: not indexed yet — run `rustbrain sync` (or omit `--no-sync`)");
                }
                Ok(ExitCode::SUCCESS)
            }
        },
        Commands::Links {
            workspace,
            pending,
            json,
        } => {
            let brain = Brain::open(&workspace).with_context(|| {
                format!(
                    "database not found under {}. run `rustbrain sync` first",
                    workspace.display()
                )
            })?;
            if pending {
                let list = brain.database().list_pending_links()?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&list)?);
                } else if list.is_empty() {
                    println!("no pending links");
                } else {
                    println!("{} pending link(s):", list.len());
                    for p in &list {
                        println!(
                            "  {} -[{}]-> {}",
                            p.source_id, p.relation_type, p.raw_target
                        );
                    }
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        Commands::Sync { workspace } => {
            let mut brain = Brain::open_or_create(&workspace)
                .with_context(|| format!("failed to open brain in {}", workspace.display()))?;
            println!("indexing workspace {} ...", brain.workspace().display());
            let stats = brain.sync()?;
            println!(
                "sync complete: md={} canvas={} rs={} nodes_upserted={} skipped={} edges={} pending={} symbols={} mmap={} file_errors={}",
                stats.markdown_files,
                stats.canvas_files,
                stats.rust_files,
                stats.nodes_upserted,
                stats.nodes_skipped_unchanged,
                stats.edges_created,
                stats.edges_pending,
                stats.symbol_anchors,
                stats.mmap_written,
                stats.file_errors
            );
            if let Ok(mut reg) = GlobalRegistry::load() {
                let _ = reg.register(brain.workspace());
            }
            print_note_tip();
            Ok(ExitCode::SUCCESS)
        }
        Commands::Query {
            query,
            all_workspaces,
            limit,
            scores,
            no_symbols,
            with_symbols,
            r#type,
            all_types,
            workspace,
        } => {
            // Note-first by default (0.3.1+). Symbols only with --with-symbols / --all-types.
            let include_symbols = with_symbols || all_types;
            let _ = no_symbols; // legacy no-op when already note-first
            let mut opts = if include_symbols {
                QueryOptions::default()
            } else {
                QueryOptions::human()
            };
            opts.limit = limit;
            if all_types {
                opts.no_symbols = false;
                opts.include_types.clear();
            }
            if let Some(types) = r#type {
                opts.include_types = parse_types_list(&types)?;
                opts.no_symbols = false;
            }

            if all_workspaces {
                println!("searching all registered workspaces for '{query}' ...");
                let reg = GlobalRegistry::load()?;
                let results = reg.search_all_ranked(&query, &opts)?;
                if results.is_empty() {
                    println!("no matching nodes found");
                    return Ok(ExitCode::SUCCESS);
                }
                for (idx, gh) in results.iter().enumerate() {
                    let node = &gh.hit.node;
                    if scores {
                        println!(
                            "{}. [{:.3}] [{}] {} (id: {})  @{}",
                            idx + 1,
                            gh.hit.score,
                            node.node_type,
                            node.title,
                            node.id,
                            gh.workspace
                        );
                    } else {
                        println!(
                            "{}. [{}] {} (id: {})  @{}",
                            idx + 1,
                            node.node_type,
                            node.title,
                            node.id,
                            gh.workspace
                        );
                    }
                }
                return Ok(ExitCode::SUCCESS);
            }

            let brain = Brain::open(&workspace).with_context(|| {
                format!(
                    "no brain found at {} or parents. run `rustbrain setup --yes` or `rustbrain sync`",
                    workspace.display()
                )
            })?;
            println!("searching for '{query}' ...");
            let results = brain.query_ranked(&query, &opts)?;
            if results.is_empty() {
                println!("no nodes found matching '{query}'");
                if !include_symbols {
                    println!("hint: try `rustbrain query \"{query}\" --with-symbols` or broader terms");
                } else {
                    println!("hint: run `rustbrain sync` or check `rustbrain doctor`");
                }
            } else {
                for (idx, hit) in results.iter().enumerate() {
                    let node = &hit.node;
                    if scores {
                        println!(
                            "{}. [{:.3}] [{}] {} (id: {})",
                            idx + 1,
                            hit.score,
                            node.node_type,
                            node.title,
                            node.id
                        );
                    } else {
                        println!(
                            "{}. [{}] {} (id: {})",
                            idx + 1,
                            node.node_type,
                            node.title,
                            node.id
                        );
                    }
                    if let Some(path) = &node.file_path {
                        println!("   path: {path}");
                    }
                    if let Some(sum) = &node.summary {
                        println!("   summary: {sum}");
                    }
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        Commands::Context {
            prompt,
            for_prompt,
            max_tokens,
            hops,
            with_symbols,
            all_types,
            no_hop_symbols,
            no_symbols,
            r#type,
            format,
            workspace,
        } => {
            let topic = for_prompt
                .or(prompt)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "missing prompt: pass a positional topic or `-p \"…\"` / `--for-prompt`"
                    )
                })?;
            let brain = Brain::open(&workspace).with_context(|| {
                format!(
                    "no brain found at {} or parents (looking for .brain/db.sqlite). run `rustbrain setup --yes`",
                    workspace.display()
                )
            })?;
            let include_symbol_seeds = with_symbols || all_types;
            let _ = no_symbols; // accepted for back-compat with older scripts
            let mut opts = rustbrain_core::ContextOptions {
                max_tokens,
                hop_depth: hops,
                // Note-first by default; symbols still hop in via anchors unless --no-hop-symbols.
                no_symbols: !include_symbol_seeds,
                hop_to_symbols: !no_hop_symbols,
                ..rustbrain_core::ContextOptions::default()
            };
            if let Some(types) = r#type {
                opts.include_types = parse_types_list(&types)?;
                opts.no_symbols = false;
            }
            let bundle = brain.context_for_prompt_with(&topic, &opts)?;
            match format.as_str() {
                "markdown" | "md" => print!("{}", bundle.to_markdown()),
                "xml" => print!("{}", bundle.to_xml()),
                other => bail!("unknown format '{other}' (expected xml or markdown)"),
            }
            Ok(ExitCode::SUCCESS)
        }
        Commands::Watch {
            workspace,
            debounce_ms,
        } => {
            let brain = Brain::open_or_create(&workspace)?;
            println!(
                "watching {} (debounce {debounce_ms}ms); Ctrl-C to stop",
                brain.workspace().display()
            );
            brain.watch(debounce_ms)?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Export {
            out,
            decouple_ast,
            workspace,
        } => {
            let brain = Brain::open(&workspace).with_context(|| {
                format!(
                    "database not found under {}. run `rustbrain sync` first",
                    workspace.display()
                )
            })?;
            println!(
                "exporting to {} (decouple_ast={decouple_ast}) ...",
                out.display()
            );
            brain.export(&out, decouple_ast)?;
            println!("export complete");
            Ok(ExitCode::SUCCESS)
        }
        Commands::Import { input, workspace } => {
            let mut brain = Brain::open_or_create(&workspace)?;
            println!("importing {} ...", input.display());
            let n = brain.import(&input)?;
            println!("imported {n} nodes");
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn parse_types_list(s: &str) -> Result<Vec<NodeType>> {
    let mut out = Vec::new();
    for part in s.split(',') {
        let t = part.trim();
        if t.is_empty() {
            continue;
        }
        let ty = NodeType::parse(t).ok_or_else(|| {
            anyhow::anyhow!(
                "unknown node type '{t}'. use: goal, adr, alternative, concept, symbol, reference, edge_case"
            )
        })?;
        out.push(ty);
    }
    if out.is_empty() {
        bail!("--type requires at least one node type");
    }
    Ok(out)
}
