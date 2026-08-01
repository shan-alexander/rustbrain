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
    absorb_all_to_main, absorb_scope, add_scope, attach_subbrain, bootstrap_workspace,
    count_nodes_by_scope, create_note, detect_path, disable_multi, enable_multi,
    format_scopes_text, import_brain, load_manifest, normalize_target_arg, reconcile_scopes,
    remove_scope_def, run_doctor, run_doctor_with, scope_for_cwd, ApplyOptions, ApplyStyle,
    BootstrapMode, BootstrapOptions, Brain, DoctorOptions, GlobalRegistry, GraphDirection,
    GraphOptions, ImportBrainOptions, NoteNewOptions, NodeType, QueryOptions, ScopeMainInclude,
    ScopeSource, MAIN_SCOPE,
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
  rustbrain scopes list
  rustbrain scopes enable --cargo
  rustbrain query \"topic\" --scope rustbrain-cli
  rustbrain context \"why <decision>\" --scope rustbrain-core
  rustbrain graph docs/concepts/foo.md

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
        /// Skip harvesting Cargo.toml deps → docs.rs notes
        #[arg(long, default_value_t = false)]
        no_crate_docs: bool,
        /// After setup, enable multi-brain from Cargo workspace members
        #[arg(long, default_value_t = false)]
        multi_cargo: bool,
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
        /// Skip harvesting Cargo.toml deps → docs.rs notes
        #[arg(long, default_value_t = false)]
        no_crate_docs: bool,
        /// Custom AGENTS.md template file (overrides AGENTS.template.md / built-in)
        #[arg(long, value_name = "PATH")]
        agents_template: Option<PathBuf>,
    },
    /// Health check: pending links, ratios, schema, orphans
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
        /// Detailed orphan analysis (also: `--orphan`)
        #[arg(long = "orphans", visible_alias = "orphan", default_value_t = false)]
        orphans: bool,
    },
    /// Create structured Markdown notes
    Note {
        #[command(subcommand)]
        cmd: NoteCmd,
    },
    /// Pending links, soft auto-links, or apply rewrites (`--apply`)
    #[command(visible_alias = "link")]
    Links {
        /// Workspace root
        #[arg(short = 'w', long, default_value = ".")]
        workspace: PathBuf,
        /// List pending WikiLink / symbol targets (default when not using --auto/--apply)
        #[arg(long, default_value_t = false)]
        pending: bool,
        /// Create soft auto-links (filename stem + shared tags)
        #[arg(long, default_value_t = false)]
        auto: bool,
        /// Plan/apply pending WikiLink normalizations (Phase 0); add `--discover` for AC mentions
        #[arg(long, default_value_t = false)]
        apply: bool,
        /// With `--apply`: write files (without this flag, apply is always dry-run)
        #[arg(long, default_value_t = false)]
        write: bool,
        /// With `--apply`: force dry-run even if `--write` is set
        #[arg(long, default_value_t = false)]
        dry_run: bool,
        /// With `--apply`: Phase 1 Aho–Corasick discover of unmarked entity mentions
        #[arg(long, default_value_t = false)]
        discover: bool,
        /// With `--apply`: allow rewriting generated notes
        #[arg(long, default_value_t = false)]
        force: bool,
        /// With `--apply`: max auto-tier edits (default 200)
        #[arg(long, default_value_t = 200)]
        limit: usize,
        /// With `--apply --write`: skip automatic sync after writes
        #[arg(long, default_value_t = false)]
        no_sync: bool,
        /// With `--apply --discover`: `wrap` (default, inline WikiLink) or `related` (## Related)
        #[arg(long, default_value = "wrap")]
        style: String,
        /// With `--apply --discover`: disable graph-neighbor boost for scoring
        #[arg(long, default_value_t = false)]
        no_graph_priors: bool,
        /// Optional path or node id (auto-link focus, or apply source filter)
        #[arg(value_name = "TARGET")]
        target: Option<PathBuf>,
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
        /// SubBrain scope filter (multi-brain mode): seeds in this scope (+ MainBrain hubs)
        #[arg(long, value_name = "ID")]
        scope: Option<String>,
        /// With `--scope`: only the SubBrain (no MainBrain hubs)
        #[arg(long, default_value_t = false)]
        scope_strict: bool,
        /// With `--scope`: include all MainBrain nodes (default is hubs only)
        #[arg(long, default_value_t = false)]
        scope_with_main: bool,
        /// Do not auto-detect SubBrain from current working directory
        #[arg(long, default_value_t = false)]
        no_scope_auto: bool,
        /// Workspace root containing `.brain/`
        #[arg(short = 'w', long, default_value = ".")]
        workspace: PathBuf,
    },
    /// Inspect graph neighborhood (ASCII tree or JSON) or print graph stats
    Graph {
        /// Node id, path, title, or `symbol:Name` (omit for workspace graph stats)
        #[arg(value_name = "TARGET")]
        target: Option<String>,
        /// Expansion depth (default 1 = direct neighbors)
        #[arg(long, default_value_t = 1)]
        hops: usize,
        /// Edge direction: `both` (default), `out`, or `in`
        #[arg(long, default_value = "both")]
        direction: String,
        /// Hide soft `auto_*` edges
        #[arg(long, default_value_t = false)]
        no_auto: bool,
        /// Hide symbol neighbors
        #[arg(long, default_value_t = false)]
        no_symbols: bool,
        /// Only these neighbor types (comma-separated)
        #[arg(long, value_name = "TYPES")]
        r#type: Option<String>,
        /// Max edges to show (default 200)
        #[arg(long, default_value_t = 200)]
        limit: usize,
        /// JSON output (agents/tools)
        #[arg(long, default_value_t = false)]
        json: bool,
        /// Force stats summary even when TARGET is set
        #[arg(long, default_value_t = false)]
        stats: bool,
        /// SubBrain filter for neighbors (hubs-only Main mix by default)
        #[arg(long, value_name = "ID")]
        scope: Option<String>,
        #[arg(long, default_value_t = false)]
        scope_strict: bool,
        #[arg(long, default_value_t = false)]
        scope_with_main: bool,
        #[arg(long, default_value_t = false)]
        no_scope_auto: bool,
        /// Workspace root (walks parents for `.brain`)
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
        /// SubBrain scope filter for seeds (neighbors may hop out)
        #[arg(long, value_name = "ID")]
        scope: Option<String>,
        /// With `--scope`: only the SubBrain (no MainBrain hubs)
        #[arg(long, default_value_t = false)]
        scope_strict: bool,
        /// With `--scope`: include all MainBrain nodes (default is hubs only)
        #[arg(long, default_value_t = false)]
        scope_with_main: bool,
        /// Do not auto-detect SubBrain from CWD
        #[arg(long, default_value_t = false)]
        no_scope_auto: bool,
        /// Workspace root (walks parents for `.brain` like git)
        #[arg(short = 'w', long, default_value = ".")]
        workspace: PathBuf,
    },
    /// MainBrain / SubBrain scopes (multi-crate / multi-root; opt-in)
    Scopes {
        #[command(subcommand)]
        cmd: ScopesCmd,
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
        /// Export only one SubBrain (+ hubs) for sharing without full merge
        #[arg(long, value_name = "ID")]
        scope: Option<String>,
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
        /// Node type: goal, adr, alternative, concept, analysis, plan, changelog, reference, edge_case
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
        /// SubBrain id: write under that scope's tree (multi-brain)
        #[arg(long, value_name = "ID")]
        scope: Option<String>,
    },
}

/// `rustbrain scopes` subcommands.
#[derive(Subcommand)]
enum ScopesCmd {
    /// List mode + SubBrains + node counts (default)
    List {
        #[arg(short = 'w', long, default_value = ".")]
        workspace: PathBuf,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Enable multi-brain mode (optional Cargo workspace discovery)
    Enable {
        #[arg(short = 'w', long, default_value = ".")]
        workspace: PathBuf,
        /// Discover Cargo `[workspace].members` as SubBrains
        #[arg(long, default_value_t = false)]
        cargo: bool,
        /// Enable multi with no SubBrains yet (manual `scopes add`)
        #[arg(long, default_value_t = false)]
        empty: bool,
        /// Re-sync after enabling so node.scope is assigned
        #[arg(long, default_value_t = true)]
        sync: bool,
        #[arg(long, default_value_t = false)]
        no_sync: bool,
    },
    /// Disable multi-brain (single MainBrain bag)
    Disable {
        #[arg(short = 'w', long, default_value = ".")]
        workspace: PathBuf,
        /// Reassign every node to main and clear SubBrain defs
        #[arg(long, default_value_t = false)]
        absorb_all: bool,
        /// Drop scope defs from manifest (without absorb_all, SQLite scopes stay until re-sync)
        #[arg(long, default_value_t = false)]
        clear: bool,
    },
    /// Add or update a SubBrain root
    Add {
        /// Scope id (path-stable, e.g. rustbrain-core)
        id: String,
        /// Workspace-relative root(s) owned by this SubBrain
        #[arg(long = "root", required = true)]
        roots: Vec<String>,
        /// Optional aliases (e.g. Cargo package name)
        #[arg(long = "alias")]
        aliases: Vec<String>,
        #[arg(short = 'w', long, default_value = ".")]
        workspace: PathBuf,
        #[arg(long, default_value_t = true)]
        sync: bool,
        #[arg(long, default_value_t = false)]
        no_sync: bool,
    },
    /// Remove a SubBrain definition from the manifest
    Remove {
        id: String,
        /// Reassign that scope's nodes to MainBrain first
        #[arg(long, default_value_t = false)]
        absorb: bool,
        #[arg(short = 'w', long, default_value = ".")]
        workspace: PathBuf,
    },
    /// Reassign a SubBrain's nodes into MainBrain and drop the SubBrain def
    Absorb {
        /// SubBrain id, or `all` for every SubBrain + single mode
        id: String,
        #[arg(short = 'w', long, default_value = ".")]
        workspace: PathBuf,
    },
    /// Import another workspace as a SubBrain (separate) or merge into MainBrain
    Import {
        /// Source workspace path (contains notes / optional `.brain`)
        #[arg(long = "from")]
        from: PathBuf,
        /// Keep separate as SubBrain id (share without merge)
        #[arg(long = "as")]
        as_scope: Option<String>,
        /// Merge into MainBrain (`main`) — default if `--as` omitted
        #[arg(long = "into")]
        into: Option<String>,
        /// Destination root when copying (not used with `--mount`)
        #[arg(long)]
        root: Option<String>,
        /// Attach source path under this workspace without copying (umbrella)
        #[arg(long, default_value_t = false)]
        mount: bool,
        /// Overwrite existing files when copying
        #[arg(long, default_value_t = false)]
        force: bool,
        #[arg(short = 'w', long, default_value = ".")]
        workspace: PathBuf,
        #[arg(long, default_value_t = true)]
        sync: bool,
        #[arg(long, default_value_t = false)]
        no_sync: bool,
    },
    /// Attach an existing subdirectory as SubBrain (no copy) — umbrella workspaces
    Attach {
        /// SubBrain id
        id: String,
        /// Workspace-relative root (e.g. project-a)
        #[arg(long)]
        root: String,
        #[arg(long = "alias")]
        aliases: Vec<String>,
        #[arg(short = 'w', long, default_value = ".")]
        workspace: PathBuf,
        #[arg(long, default_value_t = true)]
        sync: bool,
        #[arg(long, default_value_t = false)]
        no_sync: bool,
    },
    /// Recompute every node's scope from manifest (+ frontmatter); clear drift
    Reconcile {
        #[arg(short = 'w', long, default_value = ".")]
        workspace: PathBuf,
    },
    /// Suggest SubBrain id / mount options for a path (run before import)
    Detect {
        /// Directory to inspect (foreign mono-repo or subfolder)
        path: PathBuf,
        #[arg(short = 'w', long, default_value = ".")]
        workspace: PathBuf,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
}

/// Apply explicit `--scope` or CWD auto-detect into query/context options.
fn apply_scope_cli(
    opts_scope: &mut Option<String>,
    opts_main_scope: &mut String,
    opts_scope_main: &mut ScopeMainInclude,
    brain_ws: &std::path::Path,
    cli_ws: &std::path::Path,
    explicit: Option<String>,
    no_auto: bool,
    strict: bool,
    with_main: bool,
) {
    let resolved = if let Some(s) = explicit {
        let m = load_manifest(brain_ws).unwrap_or_else(|_| {
            rustbrain_core::WorkspaceManifest::single(brain_ws)
        });
        *opts_main_scope = m.main_id.clone();
        Some(
            m.find_scope(&s)
                .map(|sc| sc.id.clone())
                .unwrap_or(s),
        )
    } else if !no_auto {
        let cwd = std::env::current_dir().unwrap_or_else(|_| cli_ws.to_path_buf());
        if let Some(s) = scope_for_cwd(brain_ws, &cwd) {
            let m = load_manifest(brain_ws).unwrap_or_else(|_| {
                rustbrain_core::WorkspaceManifest::single(brain_ws)
            });
            *opts_main_scope = m.main_id;
            Some(s)
        } else {
            None
        }
    } else {
        None
    };
    if let Some(s) = resolved {
        *opts_scope = Some(s);
        *opts_scope_main = if strict {
            ScopeMainInclude::Strict
        } else if with_main {
            ScopeMainInclude::AllMain
        } else {
            ScopeMainInclude::HubsOnly
        };
    }
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
            no_crate_docs,
            multi_cargo,
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
                    crate_docs: !no_crate_docs,
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
            if multi_cargo {
                let m = enable_multi(&workspace, true)?;
                println!(
                    "setup: multi-brain enabled ({} SubBrain(s) from Cargo)",
                    m.scopes.len()
                );
            }
            let mut brain = Brain::open_or_create(&workspace)?;
            println!("setup: syncing {} ...", brain.workspace().display());
            let stats = brain.sync()?;
            println!(
                "setup: sync complete nodes_upserted={} symbols={} pending={} file_errors={}",
                stats.nodes_upserted, stats.symbol_anchors, stats.edges_pending, stats.file_errors
            );
            if !stats.by_scope.is_empty() {
                let parts: Vec<String> = stats
                    .by_scope
                    .iter()
                    .map(|(s, n)| format!("{s}={n}"))
                    .collect();
                println!("setup: by_scope {}", parts.join(" "));
            }
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
            no_crate_docs,
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
                crate_docs: !no_crate_docs,
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
            orphans,
        } => {
            let report = run_doctor_with(
                &workspace,
                &DoctorOptions {
                    detail_orphans: orphans,
                },
            )?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print!("{}", report.to_text());
                if !orphans {
                    print_note_tip();
                }
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
                scope,
            } => {
                let do_sync = sync && !no_sync;
                let node_type = NodeType::parse(&r#type).ok_or_else(|| {
                    anyhow::anyhow!(
                        "unknown type '{type}'. use: goal, adr, alternative, concept, analysis, plan, changelog, reference, edge_case (plan aliases: roadmap, backlog, todo, tasklist)"
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
                        scope,
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
            auto,
            apply,
            write,
            dry_run,
            discover,
            force,
            limit,
            no_sync,
            style,
            no_graph_priors,
            target,
            json,
        } => {
            if auto && apply {
                bail!("use either `--auto` (soft DB edges) or `--apply` (Markdown rewrites), not both");
            }

            if apply {
                let mut brain = Brain::open(&workspace).with_context(|| {
                    format!(
                        "no brain found at {} or parents. run `rustbrain setup --yes` or `rustbrain sync`",
                        workspace.display()
                    )
                })?;
                if write && dry_run {
                    bail!("`--write` and `--dry-run` conflict; omit one");
                }
                let style = ApplyStyle::parse(&style).ok_or_else(|| {
                    anyhow::anyhow!("invalid --style '{style}'. use: wrap or related")
                })?;
                let opts = ApplyOptions {
                    write,
                    dry_run: !write || dry_run,
                    discover,
                    force_generated: force,
                    limit,
                    target: target.map(|p| p.to_string_lossy().to_string()),
                    sync_after: !no_sync,
                    report_suggest: true,
                    style,
                    graph_priors: !no_graph_priors,
                    cache_dir: None, // Brain::apply_links fills .brain/
                };
                let report = brain.apply_links(&opts)?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&report)?);
                } else {
                    print!("{}", report.to_text());
                }
                if !report.dry_run && report.written > 0 && report.recommend_sync {
                    let stats = brain.sync()?;
                    if !json {
                        println!(
                            "synced after apply: upserted={} pending={} file_errors={}",
                            stats.nodes_upserted, stats.edges_pending, stats.file_errors
                        );
                    }
                }
                return Ok(ExitCode::SUCCESS);
            }

            if auto {
                let mut brain = Brain::open(&workspace).with_context(|| {
                    format!(
                        "no brain found at {} or parents. run `rustbrain setup --yes`",
                        workspace.display()
                    )
                })?;
                let target = target.map(|p| normalize_target_arg(&p.to_string_lossy()));
                let report = brain.auto_link(target.as_deref())?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&report)?);
                } else {
                    println!(
                        "auto-link complete: scope={} pairs≈{} edges_upserted={}",
                        report.scope, report.pairs_considered, report.edges_upserted
                    );
                    if report.applied.is_empty() {
                        println!("no soft links created (need matching filename stems and/or shared tags)");
                    } else {
                        println!("sample applied (up to 50):");
                        for s in &report.applied {
                            let tp = s.target_path.as_deref().unwrap_or("-");
                            println!(
                                "  [{} w={:.2}] {} → {} ({})",
                                s.relation_type, s.weight, s.reason, s.target_id, tp
                            );
                        }
                    }
                    println!("tip: soft links are `auto_*` edges (low weight). Explicit WikiLinks stay preferred.");
                    println!("     re-check orphans: `rustbrain doctor --orphans`");
                    println!("     normalize pending WikiLinks: `rustbrain links --apply --dry-run`");
                }
                return Ok(ExitCode::SUCCESS);
            }

            let brain = Brain::open(&workspace).with_context(|| {
                format!(
                    "database not found under {}. run `rustbrain sync` first",
                    workspace.display()
                )
            })?;
            // Default behaviour: list pending unresolved links.
            let _ = pending; // accepted; pending list is the default non-auto mode
            let list = brain.database().list_pending_links()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&list)?);
            } else if list.is_empty() {
                println!("no pending links");
                println!("tip: soft-link orphans with `rustbrain links --auto` (or `rustbrain link --auto`)");
                println!("     discover unmarked mentions: `rustbrain links --apply --discover --dry-run`");
            } else {
                println!("{} pending link(s):", list.len());
                for p in &list {
                    println!(
                        "  {} -[{}]-> {}",
                        p.source_id, p.relation_type, p.raw_target
                    );
                }
                println!("tip: `rustbrain links --apply --dry-run` plans unique pending rewrites");
                println!("     `rustbrain links --apply --write` applies them (then syncs)");
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
            if !stats.by_scope.is_empty() {
                let parts: Vec<String> = stats
                    .by_scope
                    .iter()
                    .map(|(s, n)| format!("{s}={n}"))
                    .collect();
                println!("by_scope: {}", parts.join(" "));
            }
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
            scope,
            scope_strict,
            scope_with_main,
            no_scope_auto,
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
            apply_scope_cli(
                &mut opts.scope,
                &mut opts.main_scope,
                &mut opts.scope_main,
                brain.workspace(),
                &workspace,
                scope,
                no_scope_auto,
                scope_strict,
                scope_with_main,
            );
            if let Some(ref sc) = opts.scope {
                eprintln!("scope: {sc}");
            }
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
        Commands::Graph {
            target,
            hops,
            direction,
            no_auto,
            no_symbols,
            r#type,
            limit,
            json,
            stats,
            scope,
            scope_strict,
            scope_with_main,
            no_scope_auto,
            workspace,
        } => {
            let brain = Brain::open(&workspace).with_context(|| {
                format!(
                    "no brain found at {} or parents. run `rustbrain setup --yes` or `rustbrain sync`",
                    workspace.display()
                )
            })?;

            // No target → workspace stats only.
            if target.is_none() {
                let report = brain.graph_stats()?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&report)?);
                } else {
                    print!("{}", report.to_ascii());
                    println!(
                        "tip: `rustbrain graph <path|id|title|symbol:Name> [--hops N] [--json] [--scope ID]`"
                    );
                }
                return Ok(ExitCode::SUCCESS);
            }

            // Optional stats header before neighborhood (text mode only).
            if stats && !json {
                let report = brain.graph_stats()?;
                print!("{}", report.to_ascii());
                println!();
            }

            let target = target.expect("checked above");

            let dir = GraphDirection::parse(&direction).ok_or_else(|| {
                anyhow::anyhow!(
                    "invalid --direction '{direction}'. use: both, out, or in"
                )
            })?;
            let mut opts = GraphOptions {
                hops: hops.max(1),
                include_auto: !no_auto,
                include_symbols: !no_symbols,
                direction: dir,
                max_edges: limit.max(1),
                type_filter: None,
                ..GraphOptions::default()
            };
            if let Some(types) = r#type {
                opts.type_filter = Some(parse_types_list(&types)?);
            }
            apply_scope_cli(
                &mut opts.scope,
                &mut opts.main_scope,
                &mut opts.scope_main,
                brain.workspace(),
                &workspace,
                scope,
                no_scope_auto,
                scope_strict,
                scope_with_main,
            );
            if let Some(ref sc) = opts.scope {
                eprintln!("scope: {sc}");
            }
            let nb = brain.graph_neighborhood(&target, &opts)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&nb)?);
            } else {
                print!("{}", nb.to_ascii());
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
            scope,
            scope_strict,
            scope_with_main,
            no_scope_auto,
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
            apply_scope_cli(
                &mut opts.scope,
                &mut opts.main_scope,
                &mut opts.scope_main,
                brain.workspace(),
                &workspace,
                scope,
                no_scope_auto,
                scope_strict,
                scope_with_main,
            );
            if let Some(ref sc) = opts.scope {
                eprintln!("scope: {sc}");
            }
            let bundle = brain.context_for_prompt_with(&topic, &opts)?;
            match format.as_str() {
                "markdown" | "md" => print!("{}", bundle.to_markdown()),
                "xml" => print!("{}", bundle.to_xml()),
                other => bail!("unknown format '{other}' (expected xml or markdown)"),
            }
            Ok(ExitCode::SUCCESS)
        }
        Commands::Scopes { cmd } => match cmd {
            ScopesCmd::List { workspace, json } => {
                let brain = Brain::open_or_create(&workspace)?;
                let m = load_manifest(brain.workspace())?;
                let counts = count_nodes_by_scope(brain.database())?;
                if json {
                    let payload = serde_json::json!({
                        "manifest": m,
                        "counts": counts,
                    });
                    println!("{}", serde_json::to_string_pretty(&payload)?);
                } else {
                    print!("{}", format_scopes_text(&m, &counts));
                }
                Ok(ExitCode::SUCCESS)
            }
            ScopesCmd::Enable {
                workspace,
                cargo,
                empty,
                sync,
                no_sync,
            } => {
                if !cargo && !empty {
                    bail!("pass --cargo (discover workspace members) and/or --empty (multi with no SubBrains yet)");
                }
                let m = if cargo {
                    enable_multi(&workspace, true)?
                } else {
                    enable_multi(&workspace, false)?
                };
                println!(
                    "multi-brain enabled ({} SubBrain(s))",
                    m.scopes.len()
                );
                for sc in &m.scopes {
                    println!("  {} → {}", sc.id, sc.roots.join(", "));
                }
                if sync && !no_sync {
                    let mut brain = Brain::open_or_create(&workspace)?;
                    let stats = brain.sync()?;
                    println!(
                        "synced: upserted={} skipped={} file_errors={}",
                        stats.nodes_upserted, stats.nodes_skipped_unchanged, stats.file_errors
                    );
                } else {
                    println!("tip: rustbrain sync  # assign node.scope from roots");
                }
                Ok(ExitCode::SUCCESS)
            }
            ScopesCmd::Disable {
                workspace,
                absorb_all,
                clear,
            } => {
                let brain = Brain::open_or_create(&workspace)?;
                if absorb_all {
                    let report = absorb_all_to_main(brain.workspace(), brain.database())?;
                    println!(
                        "disabled multi-brain; absorbed {} SubBrain(s); reassigned {} node(s) → main",
                        report.scopes_removed.len(),
                        report.nodes_reassigned
                    );
                } else {
                    let m = disable_multi(brain.workspace(), clear)?;
                    println!(
                        "mode: {} (scopes kept in manifest: {})",
                        m.mode.as_str(),
                        m.scopes.len()
                    );
                    println!("tip: use --absorb-all to reassign all nodes to main and clear SubBrains");
                }
                Ok(ExitCode::SUCCESS)
            }
            ScopesCmd::Add {
                id,
                roots,
                aliases,
                workspace,
                sync,
                no_sync,
            } => {
                let m = add_scope(
                    &workspace,
                    &id,
                    &roots,
                    &aliases,
                    ScopeSource::Manual,
                )?;
                println!("SubBrain {:?} roots={:?}", id, roots);
                println!("mode: {}", m.mode.as_str());
                if sync && !no_sync {
                    let mut brain = Brain::open_or_create(&workspace)?;
                    let stats = brain.sync()?;
                    println!(
                        "synced: upserted={} skipped={}",
                        stats.nodes_upserted, stats.nodes_skipped_unchanged
                    );
                }
                Ok(ExitCode::SUCCESS)
            }
            ScopesCmd::Remove {
                id,
                absorb,
                workspace,
            } => {
                let brain = Brain::open_or_create(&workspace)?;
                if absorb {
                    let report = absorb_scope(brain.workspace(), brain.database(), &id)?;
                    println!(
                        "absorbed {:?} → main ({} nodes); removed SubBrain def",
                        report.absorbed_id, report.nodes_reassigned
                    );
                } else {
                    remove_scope_def(brain.workspace(), &id)?;
                    println!(
                        "removed SubBrain def {id:?} (nodes keep old scope until absorb or re-sync)"
                    );
                    println!("tip: rustbrain scopes absorb {id}");
                }
                Ok(ExitCode::SUCCESS)
            }
            ScopesCmd::Absorb { id, workspace } => {
                let brain = Brain::open_or_create(&workspace)?;
                if id.eq_ignore_ascii_case("all") {
                    let report = absorb_all_to_main(brain.workspace(), brain.database())?;
                    println!(
                        "absorbed all SubBrains → main ({} scopes, {} nodes); mode=single",
                        report.scopes_removed.len(),
                        report.nodes_reassigned
                    );
                } else {
                    let report = absorb_scope(brain.workspace(), brain.database(), &id)?;
                    println!(
                        "absorbed {:?} → main ({} nodes)",
                        report.absorbed_id, report.nodes_reassigned
                    );
                }
                Ok(ExitCode::SUCCESS)
            }
            ScopesCmd::Import {
                from,
                as_scope,
                into,
                root,
                mount,
                force,
                workspace,
                sync,
                no_sync,
            } => {
                let into_scope = if let Some(a) = as_scope {
                    a
                } else if let Some(i) = into {
                    i
                } else {
                    MAIN_SCOPE.to_string()
                };
                let opts = ImportBrainOptions {
                    into_scope: into_scope.clone(),
                    dest_root: root,
                    copy_markdown: !mount,
                    force,
                    mount,
                    ..Default::default()
                };
                let report = import_brain(&workspace, &from, &opts)?;
                if report.mounted {
                    println!(
                        "mounted {} as SubBrain {:?} (root={}, no copy)",
                        report.source, report.into_scope, report.dest_root
                    );
                } else {
                    println!(
                        "imported from {} → scope {:?} root={} copied={} skipped={} bytes={}",
                        report.source,
                        report.into_scope,
                        report.dest_root,
                        report.files_copied,
                        report.files_skipped,
                        report.bytes_copied
                    );
                }
                if report.scope_registered {
                    println!("SubBrain registered (kept separate — not merged into MainBrain)");
                } else if report.into_scope == MAIN_SCOPE {
                    println!("merged into MainBrain (use --as <id> to keep a separate SubBrain)");
                }
                if sync && !no_sync {
                    let mut brain = Brain::open_or_create(&workspace)?;
                    let stats = brain.sync()?;
                    let rec = reconcile_scopes(brain.workspace(), brain.database())?;
                    println!(
                        "synced: upserted={} · reconcile: updated={} unchanged={}",
                        stats.nodes_upserted, rec.updated, rec.unchanged
                    );
                } else {
                    println!("tip: rustbrain sync && rustbrain scopes reconcile");
                }
                Ok(ExitCode::SUCCESS)
            }
            ScopesCmd::Attach {
                id,
                root,
                aliases,
                workspace,
                sync,
                no_sync,
            } => {
                let m = attach_subbrain(&workspace, &id, &root, &aliases)?;
                println!(
                    "attached SubBrain {:?} → root {root} (mode={})",
                    id,
                    m.mode.as_str()
                );
                if sync && !no_sync {
                    let mut brain = Brain::open_or_create(&workspace)?;
                    let stats = brain.sync()?;
                    let rec = reconcile_scopes(brain.workspace(), brain.database())?;
                    println!(
                        "synced upserted={} · reconcile updated={}",
                        stats.nodes_upserted, rec.updated
                    );
                }
                Ok(ExitCode::SUCCESS)
            }
            ScopesCmd::Reconcile { workspace } => {
                let brain = Brain::open_or_create(&workspace)?;
                let rep = reconcile_scopes(brain.workspace(), brain.database())?;
                println!(
                    "reconcile: mode={} updated={} unchanged={} orphan_scopes={:?}",
                    rep.mode, rep.updated, rep.unchanged, rep.orphan_scopes_cleared
                );
                Ok(ExitCode::SUCCESS)
            }
            ScopesCmd::Detect {
                path,
                workspace,
                json,
            } => {
                let rep = detect_path(&workspace, &path)?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&rep)?);
                } else {
                    println!("path: {}", rep.path);
                    println!("suggested_id: {}", rep.suggested_id);
                    println!("under_workspace: {}", rep.under_workspace);
                    if let Some(r) = &rep.relative_root {
                        println!("relative_root: {r}");
                    }
                    println!("mountable: {}", rep.mountable);
                    println!("has_nested_brain: {}", rep.has_nested_brain);
                    if let Some(e) = &rep.existing_scope {
                        println!("existing_scope: {e}");
                    }
                    for t in &rep.tips {
                        println!("tip: {t}");
                    }
                }
                Ok(ExitCode::SUCCESS)
            }
        },
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
            scope,
            workspace,
        } => {
            let brain = Brain::open(&workspace).with_context(|| {
                format!(
                    "database not found under {}. run `rustbrain sync` first",
                    workspace.display()
                )
            })?;
            println!(
                "exporting to {} (decouple_ast={decouple_ast}{}) ...",
                out.display(),
                scope
                    .as_ref()
                    .map(|s| format!(" scope={s}"))
                    .unwrap_or_default()
            );
            brain.export_scope(&out, decouple_ast, scope.as_deref())?;
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
                "unknown node type '{t}'. use: goal, adr, alternative, concept, analysis, symbol, reference, edge_case"
            )
        })?;
        out.push(ty);
    }
    if out.is_empty() {
        bail!("--type requires at least one node type");
    }
    Ok(out)
}
