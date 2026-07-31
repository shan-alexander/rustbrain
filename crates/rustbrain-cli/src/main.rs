//! # rustbrain CLI
//!
//! Command-line interface for the [rustbrain](https://github.com/shan-alexander/rustbrain)
//! second-brain engine. Thin wrapper around [`rustbrain_core::Brain`].
//!
//! ## Install
//!
//! ```bash
//! cargo install rustbrain --locked
//! ```
//!
//! ## Typical workflow
//!
//! ```bash
//! rustbrain init
//! rustbrain sync
//! rustbrain query "topic" --scores
//! rustbrain context -p "explain X" -F markdown --hops 1
//! rustbrain watch --debounce-ms 300
//! ```
//!
//! Exit code `0` on success, `1` on error (message on stderr).

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use rustbrain_core::{Brain, GlobalRegistry, QueryOptions};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "rustbrain")]
#[command(
    about = "Project-scoped, Rust-first second-brain knowledge engine for engineers and AI agents",
    long_about = None,
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a `.brain` directory in the workspace
    Init {
        /// Workspace directory (defaults to current directory)
        #[arg(default_value = ".")]
        workspace: PathBuf,
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
        /// Workspace root containing `.brain/`
        #[arg(short = 'w', long, default_value = ".")]
        workspace: PathBuf,
    },
    /// Build graph-aware prompt context (FTS seeds + CSR neighbors)
    Context {
        /// Topic or prompt requirement
        #[arg(short = 'p', long = "for-prompt")]
        for_prompt: String,
        /// Approximate max tokens for context output
        #[arg(short = 'm', long, default_value = "2048")]
        max_tokens: usize,
        /// Graph expansion hop depth (0 = seeds only)
        #[arg(long, default_value_t = 1)]
        hops: usize,
        /// Output format: `xml` or `markdown`
        #[arg(short = 'F', long, default_value = "xml")]
        format: String,
        /// Workspace root
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

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
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

            if let Ok(mut reg) = GlobalRegistry::load() {
                let _ = reg.register(brain.workspace());
            }
        }
        Commands::Sync { workspace } => {
            let mut brain = Brain::open_or_create(&workspace)
                .with_context(|| format!("failed to open brain in {}", workspace.display()))?;
            println!("indexing workspace {} ...", brain.workspace().display());
            let stats = brain.sync()?;
            println!(
                "sync complete: md={} canvas={} rs={} nodes_upserted={} skipped={} edges={} pending={} symbols={} mmap={}",
                stats.markdown_files,
                stats.canvas_files,
                stats.rust_files,
                stats.nodes_upserted,
                stats.nodes_skipped_unchanged,
                stats.edges_created,
                stats.edges_pending,
                stats.symbol_anchors,
                stats.mmap_written
            );

            if let Ok(mut reg) = GlobalRegistry::load() {
                let _ = reg.register(brain.workspace());
            }
        }
        Commands::Query {
            query,
            all_workspaces,
            limit,
            scores,
            workspace,
        } => {
            let opts = QueryOptions {
                limit,
                ..QueryOptions::default()
            };

            if all_workspaces {
                println!("searching all registered workspaces for '{query}' ...");
                let reg = GlobalRegistry::load()?;
                let results = reg.search_all_ranked(&query, &opts)?;
                if results.is_empty() {
                    println!("no matching nodes found");
                    return Ok(());
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
                    if let Some(p) = &node.file_path {
                        println!("     path: {p}");
                    }
                }
                return Ok(());
            }

            let brain = Brain::open(&workspace).with_context(|| {
                format!(
                    "database not found under {}. run `rustbrain sync` first",
                    workspace.display()
                )
            })?;
            println!("searching for '{query}' ...");
            let results = brain.query_ranked(&query, &opts)?;
            if results.is_empty() {
                println!("no nodes found matching '{query}'");
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
        }
        Commands::Context {
            for_prompt,
            max_tokens,
            hops,
            format,
            workspace,
        } => {
            let brain = Brain::open(&workspace).with_context(|| {
                format!(
                    "database not found under {}. run `rustbrain sync` first",
                    workspace.display()
                )
            })?;
            let opts = rustbrain_core::ContextOptions {
                max_tokens,
                hop_depth: hops,
                ..rustbrain_core::ContextOptions::default()
            };
            let bundle = brain.context_for_prompt_with(&for_prompt, &opts)?;
            match format.as_str() {
                "markdown" | "md" => print!("{}", bundle.to_markdown()),
                "xml" => print!("{}", bundle.to_xml()),
                other => bail!("unknown format '{other}' (expected xml or markdown)"),
            }
        }
        Commands::Watch {
            workspace,
            debounce_ms,
        } => {
            let brain = Brain::open_or_create(&workspace)?;
            // Ensure initial index
            // (caller can sync first; we still open cleanly)
            println!(
                "watching {} (debounce {debounce_ms}ms); Ctrl-C to stop",
                brain.workspace().display()
            );
            brain.watch(debounce_ms)?;
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
        }
        Commands::Import { input, workspace } => {
            let mut brain = Brain::open_or_create(&workspace)?;
            println!("importing {} ...", input.display());
            let n = brain.import(&input)?;
            println!("imported {n} nodes");
        }
    }

    Ok(())
}
