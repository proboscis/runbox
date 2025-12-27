use anyhow::{Context, Result};
use clap::{ArgAction, Parser, Subcommand};
use dialoguer::{theme::ColorfulTheme, Input};
use runbox_core::{
    BindingResolver, GitContext, Playlist, PlaylistItem, RunTemplate, Storage, Validator,
};
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

#[derive(Parser)]
#[command(name = "runbox")]
#[command(about = "Reproducible command execution system")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run from a template
    Run {
        /// Template ID
        #[arg(short, long)]
        template: String,

        /// Variable bindings (key=value)
        #[arg(short, long)]
        binding: Vec<String>,

        /// Skip execution (dry run)
        #[arg(long)]
        dry_run: bool,
    },

    /// Manage templates
    Template {
        #[command(subcommand)]
        command: TemplateCommands,
    },

    /// Manage playlists
    Playlist {
        #[command(subcommand)]
        command: PlaylistCommands,
    },

    /// Show run history
    History {
        /// Limit number of results
        #[arg(short, long, default_value = "10")]
        limit: usize,
    },

    /// Show details of a run
    Show {
        /// Run ID
        run_id: String,
    },

    /// Replay a previous run
    Replay {
        /// Run ID
        run_id: String,

        /// Override worktree directory
        #[arg(long)]
        worktree_dir: Option<PathBuf>,

        /// Keep worktree after execution (default)
        #[arg(long, conflicts_with = "cleanup")]
        keep: bool,

        /// Remove worktree after execution
        #[arg(long, conflicts_with = "keep")]
        cleanup: bool,

        /// Reuse existing worktree if possible (default)
        #[arg(long, conflicts_with = "fresh")]
        reuse: bool,

        /// Always create a fresh worktree
        #[arg(long, conflicts_with = "reuse")]
        fresh: bool,

        /// Verbose output (-v, -vv, -vvv)
        #[arg(short, long, action = ArgAction::Count)]
        verbose: u8,
    },

    /// Validate a JSON file
    Validate {
        /// Path to JSON file
        path: String,
    },
}

#[derive(Subcommand)]
enum TemplateCommands {
    /// List all templates
    List,
    /// Show template details
    Show { template_id: String },
    /// Create a new template from JSON file
    Create { path: String },
    /// Delete a template
    Delete { template_id: String },
}

#[derive(Subcommand)]
enum PlaylistCommands {
    /// List all playlists
    List,
    /// Show playlist details
    Show { playlist_id: String },
    /// Create a new playlist from JSON file
    Create { path: String },
    /// Add template to playlist
    Add {
        playlist_id: String,
        template_id: String,
        /// Optional label
        #[arg(short, long)]
        label: Option<String>,
    },
    /// Remove template from playlist
    Remove {
        playlist_id: String,
        template_id: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let storage = Storage::new()?;

    match cli.command {
        Commands::Run {
            template,
            binding,
            dry_run,
        } => cmd_run(&storage, &template, binding, dry_run),
        Commands::Template { command } => match command {
            TemplateCommands::List => cmd_template_list(&storage),
            TemplateCommands::Show { template_id } => cmd_template_show(&storage, &template_id),
            TemplateCommands::Create { path } => cmd_template_create(&storage, &path),
            TemplateCommands::Delete { template_id } => cmd_template_delete(&storage, &template_id),
        },
        Commands::Playlist { command } => match command {
            PlaylistCommands::List => cmd_playlist_list(&storage),
            PlaylistCommands::Show { playlist_id } => cmd_playlist_show(&storage, &playlist_id),
            PlaylistCommands::Create { path } => cmd_playlist_create(&storage, &path),
            PlaylistCommands::Add {
                playlist_id,
                template_id,
                label,
            } => cmd_playlist_add(&storage, &playlist_id, &template_id, label),
            PlaylistCommands::Remove {
                playlist_id,
                template_id,
            } => cmd_playlist_remove(&storage, &playlist_id, &template_id),
        },
        Commands::History { limit } => cmd_history(&storage, limit),
        Commands::Show { run_id } => cmd_show(&storage, &run_id),
        Commands::Replay {
            run_id,
            worktree_dir,
            keep,
            cleanup,
            reuse,
            fresh,
            verbose,
        } => cmd_replay(
            &storage,
            &run_id,
            ReplayCliOptions {
                worktree_dir,
                keep,
                cleanup,
                reuse,
                fresh,
                verbose,
            },
        ),
        Commands::Validate { path } => cmd_validate(&path),
    }
}

// === Run Command ===

fn cmd_run(storage: &Storage, template_id: &str, bindings: Vec<String>, dry_run: bool) -> Result<()> {
    let template = storage.load_template(template_id)?;

    // Create interactive callback
    let interactive_callback: Box<dyn Fn(&str, Option<&serde_json::Value>) -> Result<String>> =
        Box::new(|var, default| {
            let prompt = format!("Enter value for '{}'", var);
            let theme = ColorfulTheme::default();
            let mut input = Input::<String>::with_theme(&theme).with_prompt(&prompt);

            if let Some(def) = default {
                let def_str = match def {
                    serde_json::Value::String(s) => s.clone(),
                    serde_json::Value::Number(n) => n.to_string(),
                    serde_json::Value::Bool(b) => b.to_string(),
                    _ => def.to_string(),
                };
                input = input.default(def_str);
            }

            input.interact_text().context("Failed to read input")
        });

    let resolver = BindingResolver::new()
        .with_bindings(bindings)
        .with_interactive(interactive_callback);

    // Get git context
    let git = GitContext::from_current_dir()?;

    // Generate run_id first so we can use it for the patch ref
    let temp_run_id = format!("run_{}", uuid::Uuid::new_v4());
    let code_state = git.build_code_state(&temp_run_id)?;

    // Build run
    let run = resolver.build_run(&template, code_state)?;

    // Validate
    run.validate()?;

    if dry_run {
        println!("Dry run - would execute:");
        println!("{}", serde_json::to_string_pretty(&run)?);
        return Ok(());
    }

    // Save run
    let path = storage.save_run(&run)?;
    println!("Run saved: {}", path.display());

    // Execute
    println!("\nExecuting: {:?}", run.exec.argv);
    let status = Command::new(&run.exec.argv[0])
        .args(&run.exec.argv[1..])
        .current_dir(&run.exec.cwd)
        .envs(&run.exec.env)
        .status()
        .context("Failed to execute command")?;

    if status.success() {
        println!("\nRun completed successfully: {}", run.run_id);
    } else {
        println!("\nRun failed with status: {:?}", status.code());
    }

    Ok(())
}

// === Template Commands ===

fn cmd_template_list(storage: &Storage) -> Result<()> {
    let templates = storage.list_templates()?;

    if templates.is_empty() {
        println!("No templates found.");
        return Ok(());
    }

    println!("{:<30} {:<40}", "ID", "NAME");
    println!("{}", "-".repeat(70));
    for t in templates {
        println!("{:<30} {:<40}", t.template_id, t.name);
    }

    Ok(())
}

fn cmd_template_show(storage: &Storage, template_id: &str) -> Result<()> {
    let template = storage.load_template(template_id)?;
    println!("{}", serde_json::to_string_pretty(&template)?);
    Ok(())
}

fn cmd_template_create(storage: &Storage, path: &str) -> Result<()> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read file: {}", path))?;

    // Validate first
    let validator = Validator::new()?;
    let value: serde_json::Value = serde_json::from_str(&content)?;
    validator.validate_template(&value)?;

    let template: RunTemplate = serde_json::from_str(&content)?;
    let saved_path = storage.save_template(&template)?;

    println!("Template created: {}", saved_path.display());
    Ok(())
}

fn cmd_template_delete(storage: &Storage, template_id: &str) -> Result<()> {
    storage.delete_template(template_id)?;
    println!("Template deleted: {}", template_id);
    Ok(())
}

// === Playlist Commands ===

fn cmd_playlist_list(storage: &Storage) -> Result<()> {
    let playlists = storage.list_playlists()?;

    if playlists.is_empty() {
        println!("No playlists found.");
        return Ok(());
    }

    println!("{:<30} {:<30} {:<10}", "ID", "NAME", "ITEMS");
    println!("{}", "-".repeat(70));
    for p in playlists {
        println!("{:<30} {:<30} {:<10}", p.playlist_id, p.name, p.items.len());
    }

    Ok(())
}

fn cmd_playlist_show(storage: &Storage, playlist_id: &str) -> Result<()> {
    let playlist = storage.load_playlist(playlist_id)?;
    println!("{}", serde_json::to_string_pretty(&playlist)?);
    Ok(())
}

fn cmd_playlist_create(storage: &Storage, path: &str) -> Result<()> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read file: {}", path))?;

    // Validate first
    let validator = Validator::new()?;
    let value: serde_json::Value = serde_json::from_str(&content)?;
    validator.validate_playlist(&value)?;

    let playlist: Playlist = serde_json::from_str(&content)?;
    let saved_path = storage.save_playlist(&playlist)?;

    println!("Playlist created: {}", saved_path.display());
    Ok(())
}

fn cmd_playlist_add(
    storage: &Storage,
    playlist_id: &str,
    template_id: &str,
    label: Option<String>,
) -> Result<()> {
    let mut playlist = storage.load_playlist(playlist_id)?;
    playlist.items.push(PlaylistItem {
        template_id: template_id.to_string(),
        label,
    });
    storage.save_playlist(&playlist)?;
    println!("Added {} to {}", template_id, playlist_id);
    Ok(())
}

fn cmd_playlist_remove(storage: &Storage, playlist_id: &str, template_id: &str) -> Result<()> {
    let mut playlist = storage.load_playlist(playlist_id)?;
    let initial_len = playlist.items.len();
    playlist.items.retain(|item| item.template_id != template_id);

    if playlist.items.len() == initial_len {
        anyhow::bail!("Template {} not found in playlist", template_id);
    }

    storage.save_playlist(&playlist)?;
    println!("Removed {} from {}", template_id, playlist_id);
    Ok(())
}

// === History Commands ===

fn cmd_history(storage: &Storage, limit: usize) -> Result<()> {
    let runs = storage.list_runs(limit)?;

    if runs.is_empty() {
        println!("No runs found.");
        return Ok(());
    }

    println!("{:<50} {:<30}", "RUN ID", "COMMAND");
    println!("{}", "-".repeat(80));
    for run in runs {
        let cmd = run.exec.argv.join(" ");
        let cmd_truncated = if cmd.len() > 30 {
            format!("{}...", &cmd[..27])
        } else {
            cmd
        };
        println!("{:<50} {:<30}", run.run_id, cmd_truncated);
    }

    Ok(())
}

fn cmd_show(storage: &Storage, run_id: &str) -> Result<()> {
    let run = storage.load_run(run_id)?;
    println!("{}", serde_json::to_string_pretty(&run)?);
    Ok(())
}

// === Replay Command ===

#[derive(Debug)]
struct ReplayCliOptions {
    worktree_dir: Option<PathBuf>,
    keep: bool,
    cleanup: bool,
    reuse: bool,
    fresh: bool,
    verbose: u8,
}

#[derive(Debug, Deserialize, Default)]
struct GlobalConfig {
    replay: Option<ReplayConfig>,
    logging: Option<LoggingConfig>,
}

#[derive(Debug, Deserialize, Default)]
struct ReplayConfig {
    worktree_dir: Option<String>,
    cleanup: Option<bool>,
    reuse: Option<bool>,
}

#[derive(Debug, Deserialize, Default)]
struct LoggingConfig {
    verbosity: Option<u8>,
}

#[derive(Clone, Copy, Debug)]
enum ConfigSource {
    Cli,
    Git,
    Global,
    Default,
}

impl ConfigSource {
    fn label(self) -> &'static str {
        match self {
            Self::Cli => "CLI flag",
            Self::Git => "git config",
            Self::Global => "global config",
            Self::Default => "default",
        }
    }
}

struct Resolution<T> {
    name: &'static str,
    value: T,
    source: ConfigSource,
    checks: Vec<String>,
    value_display: String,
}

impl<T> Resolution<T> {
    fn log(&self, verbosity: u8) {
        if verbosity >= 2 {
            for line in &self.checks {
                println!("{}", line);
            }
            println!(
                "[config] -> using {}: {} (source: {})",
                self.name,
                self.value_display,
                self.source.label()
            );
        } else if verbosity >= 1 {
            println!(
                "[config] {}: {} (from: {})",
                self.name,
                self.value_display,
                self.source.label()
            );
        }
    }
}

#[derive(Debug)]
struct WorktreeInfo {
    path: PathBuf,
    head: Option<String>,
}

fn cmd_replay(storage: &Storage, run_id: &str, opts: ReplayCliOptions) -> Result<()> {
    let run = storage.load_run(run_id)?;
    let git = GitContext::from_current_dir()?;
    let repo_root = git.repo_root().to_path_buf();

    let global_config = load_global_config()?;
    let git_worktree_dir = git_config_get(&repo_root, "runbox.worktreeDir")?;
    let git_cleanup = git_config_get(&repo_root, "runbox.worktreeCleanup")?;
    let git_reuse = git_config_get(&repo_root, "runbox.worktreeReuse")?;
    let git_verbosity = git_config_get(&repo_root, "runbox.verbosity")?;

    let cli_cleanup = if opts.cleanup {
        Some(true)
    } else if opts.keep {
        Some(false)
    } else {
        None
    };
    let cli_reuse = if opts.reuse {
        Some(true)
    } else if opts.fresh {
        Some(false)
    } else {
        None
    };
    let cli_verbosity = if opts.verbose > 0 {
        Some(opts.verbose)
    } else {
        None
    };

    let global_replay = global_config.as_ref().and_then(|config| config.replay.as_ref());
    let global_logging = global_config.as_ref().and_then(|config| config.logging.as_ref());

    let verbosity_res = resolve_u8_setting(
        "verbosity",
        "-v/--verbose",
        "git config runbox.verbosity",
        "global config logging.verbosity",
        cli_verbosity,
        git_verbosity,
        global_logging.and_then(|logging| logging.verbosity),
        0,
    )?;
    let verbosity = verbosity_res.value;

    let worktree_dir_res = resolve_worktree_dir(
        &repo_root,
        opts.worktree_dir,
        git_worktree_dir,
        global_replay.and_then(|replay| replay.worktree_dir.clone()),
    )?;
    let cleanup_res = resolve_bool_setting(
        "cleanup",
        "--cleanup/--keep",
        "git config runbox.worktreeCleanup",
        "global config replay.cleanup",
        cli_cleanup,
        git_cleanup,
        global_replay.and_then(|replay| replay.cleanup),
        false,
    )?;
    let reuse_res = resolve_bool_setting(
        "reuse",
        "--reuse/--fresh",
        "git config runbox.worktreeReuse",
        "global config replay.reuse",
        cli_reuse,
        git_reuse,
        global_replay.and_then(|replay| replay.reuse),
        true,
    )?;

    verbosity_res.log(verbosity);
    worktree_dir_res.log(verbosity);
    cleanup_res.log(verbosity);
    reuse_res.log(verbosity);

    println!("Replaying: {}", run_id);
    println!("Command: {:?}", run.exec.argv);
    println!("Commit: {}", run.code_state.base_commit);

    let worktree_base = worktree_dir_res.value;
    let worktree_path = worktree_base.join(&run.run_id);

    log_at(
        verbosity,
        2,
        format!(
            "[worktree] checking existing worktree at {} for commit {}",
            worktree_path.display(),
            run.code_state.base_commit
        ),
    );

    let worktrees = list_worktrees(&repo_root, verbosity)?;
    let existing = worktrees.iter().find(|worktree| worktree.path == worktree_path);
    let mut reused = false;

    if let Some(existing) = existing {
        if reuse_res.value {
            if existing.head.as_deref() == Some(run.code_state.base_commit.as_str()) {
                log_at(
                    verbosity,
                    2,
                    format!(
                        "[worktree] reusing existing worktree at {}",
                        existing.path.display()
                    ),
                );
                reused = true;
            } else {
                anyhow::bail!(
                    "Worktree at {} is at commit {:?}, expected {}; use --fresh to recreate",
                    existing.path.display(),
                    existing.head.as_deref().unwrap_or("unknown"),
                    run.code_state.base_commit
                );
            }
        } else {
            log_at(
                verbosity,
                2,
                format!(
                    "[worktree] removing existing worktree at {}",
                    existing.path.display()
                ),
            );
            remove_worktree(&repo_root, &existing.path, true, verbosity)?;
        }
    } else if worktree_path.exists() {
        anyhow::bail!(
            "Worktree path {} exists but is not a registered worktree",
            worktree_path.display()
        );
    }

    if !reused {
        fs::create_dir_all(&worktree_base).with_context(|| {
            format!(
                "Failed to create worktree base directory: {}",
                worktree_base.display()
            )
        })?;
        log_at(
            verbosity,
            2,
            format!(
                "[worktree] no match, creating new at {}",
                worktree_path.display()
            ),
        );
        add_worktree(
            &repo_root,
            &worktree_path,
            &run.code_state.base_commit,
            verbosity,
        )?;
    }

    if let Some(patch) = &run.code_state.patch {
        log_at(
            verbosity,
            2,
            format!("[git] fetching patch: {}", patch.ref_),
        );
        let patch_content = fetch_patch_content(&repo_root, &patch.ref_, verbosity)?;
        log_at(
            verbosity,
            2,
            format!("[git] applying patch: {}", patch.ref_),
        );

        if reused && patch_already_applied(&worktree_path, &patch_content, verbosity)? {
            log_at(verbosity, 2, "[git] patch already applied, skipping");
        } else {
            apply_patch_to_worktree(&worktree_path, &patch_content, verbosity)?;
        }
    }

    let exec_cwd = worktree_path.join(&run.exec.cwd);
    log_at(verbosity, 2, format!("[exec] cwd: {}", exec_cwd.display()));
    log_at(verbosity, 2, format!("[exec] argv: {:?}", run.exec.argv));

    println!("\nExecuting...");
    let status = Command::new(&run.exec.argv[0])
        .args(&run.exec.argv[1..])
        .current_dir(&exec_cwd)
        .envs(&run.exec.env)
        .status()
        .context("Failed to execute command")?;

    log_at(
        verbosity,
        2,
        format!(
            "[exec] exit_code: {}",
            status.code().unwrap_or(-1)
        ),
    );

    if cleanup_res.value {
        if reused {
            log_at(
                verbosity,
                1,
                "[worktree] cleanup requested, skipping reused worktree",
            );
        } else {
            log_at(
                verbosity,
                2,
                format!("[worktree] removing {}", worktree_path.display()),
            );
            remove_worktree(&repo_root, &worktree_path, true, verbosity)?;
        }
    }

    if status.success() {
        println!("\nReplay completed successfully");
    } else {
        println!("\nReplay failed with status: {:?}", status.code());
    }

    Ok(())
}

fn load_global_config() -> Result<Option<GlobalConfig>> {
    let Some(config_dir) = dirs::config_dir() else {
        return Ok(None);
    };
    let config_path = config_dir.join("runbox").join("config.toml");
    if !config_path.exists() {
        return Ok(None);
    }

    let contents = fs::read_to_string(&config_path)
        .with_context(|| format!("Failed to read config file: {}", config_path.display()))?;
    let config: GlobalConfig = toml::from_str(&contents)
        .with_context(|| format!("Failed to parse config file: {}", config_path.display()))?;
    Ok(Some(config))
}

fn resolve_worktree_dir(
    repo_root: &Path,
    cli_value: Option<PathBuf>,
    git_value: Option<String>,
    global_value: Option<String>,
) -> Result<Resolution<PathBuf>> {
    let cli_value = cli_value.map(|path| normalize_path(repo_root, expand_tilde_path(&path)));
    let git_value = match git_value {
        Some(value) => Some(normalize_config_path(
            repo_root,
            &value,
            "runbox.worktreeDir",
        )?),
        None => None,
    };
    let global_value = match global_value {
        Some(value) => Some(normalize_config_path(
            repo_root,
            &value,
            "replay.worktree_dir",
        )?),
        None => None,
    };

    let checks = vec![
        format_config_check(
            "--worktree-dir",
            cli_value
                .as_ref()
                .map(|value| value.display().to_string()),
        ),
        format_config_check(
            "git config runbox.worktreeDir",
            git_value
                .as_ref()
                .map(|value| value.display().to_string()),
        ),
        format_config_check(
            "global config replay.worktree_dir",
            global_value
                .as_ref()
                .map(|value| value.display().to_string()),
        ),
    ];

    let default_value = repo_root.join(".git-worktrees/replay");
    let (value, source) = if let Some(value) = cli_value {
        (value, ConfigSource::Cli)
    } else if let Some(value) = git_value {
        (value, ConfigSource::Git)
    } else if let Some(value) = global_value {
        (value, ConfigSource::Global)
    } else {
        (default_value, ConfigSource::Default)
    };

    Ok(Resolution {
        name: "worktree_dir",
        value_display: value.display().to_string(),
        value,
        source,
        checks,
    })
}

fn resolve_bool_setting(
    name: &'static str,
    cli_label: &'static str,
    git_label: &'static str,
    global_label: &'static str,
    cli_value: Option<bool>,
    git_value: Option<String>,
    global_value: Option<bool>,
    default_value: bool,
) -> Result<Resolution<bool>> {
    let git_value = match git_value {
        Some(value) => Some(parse_bool(&value, git_label)?),
        None => None,
    };
    let checks = vec![
        format_config_check(
            cli_label,
            cli_value.map(|value| value.to_string()),
        ),
        format_config_check(
            git_label,
            git_value.map(|value| value.to_string()),
        ),
        format_config_check(
            global_label,
            global_value.map(|value| value.to_string()),
        ),
    ];

    let (value, source) = if let Some(value) = cli_value {
        (value, ConfigSource::Cli)
    } else if let Some(value) = git_value {
        (value, ConfigSource::Git)
    } else if let Some(value) = global_value {
        (value, ConfigSource::Global)
    } else {
        (default_value, ConfigSource::Default)
    };

    Ok(Resolution {
        name,
        value_display: value.to_string(),
        value,
        source,
        checks,
    })
}

fn resolve_u8_setting(
    name: &'static str,
    cli_label: &'static str,
    git_label: &'static str,
    global_label: &'static str,
    cli_value: Option<u8>,
    git_value: Option<String>,
    global_value: Option<u8>,
    default_value: u8,
) -> Result<Resolution<u8>> {
    let git_value = match git_value {
        Some(value) => Some(parse_u8(&value, git_label)?),
        None => None,
    };
    let checks = vec![
        format_config_check(
            cli_label,
            cli_value.map(|value| value.to_string()),
        ),
        format_config_check(
            git_label,
            git_value.map(|value| value.to_string()),
        ),
        format_config_check(
            global_label,
            global_value.map(|value| value.to_string()),
        ),
    ];

    let (value, source) = if let Some(value) = cli_value {
        (value, ConfigSource::Cli)
    } else if let Some(value) = git_value {
        (value, ConfigSource::Git)
    } else if let Some(value) = global_value {
        (value, ConfigSource::Global)
    } else {
        (default_value, ConfigSource::Default)
    };

    Ok(Resolution {
        name,
        value_display: value.to_string(),
        value,
        source,
        checks,
    })
}

fn git_config_get(repo_root: &Path, key: &str) -> Result<Option<String>> {
    let output = Command::new("git")
        .current_dir(repo_root)
        .args(["config", "--local", "--get", key])
        .output()
        .context("Failed to run git config")?;

    if output.status.success() {
        let value = String::from_utf8(output.stdout)?.trim().to_string();
        if value.is_empty() {
            anyhow::bail!("Git config {} is empty", key);
        }
        return Ok(Some(value));
    }

    if output.status.code() == Some(1) {
        return Ok(None);
    }

    anyhow::bail!(
        "Failed to read git config {}: {}",
        key,
        String::from_utf8_lossy(&output.stderr)
    );
}

fn normalize_config_path(repo_root: &Path, value: &str, label: &str) -> Result<PathBuf> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        anyhow::bail!("Config {} is empty", label);
    }
    let path = PathBuf::from(trimmed);
    Ok(normalize_path(repo_root, expand_tilde_path(&path)))
}

fn expand_tilde_path(path: &Path) -> PathBuf {
    let path_str = path.to_string_lossy();
    if path_str == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    } else if let Some(stripped) = path_str.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped);
        }
    }
    path.to_path_buf()
}

fn normalize_path(repo_root: &Path, path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        repo_root.join(path)
    }
}

fn format_config_check(label: &str, value: Option<String>) -> String {
    let value = value.unwrap_or_else(|| "not set".to_string());
    format!("[config] checking {}: {}", label, value)
}

fn parse_bool(value: &str, label: &str) -> Result<bool> {
    let normalized = value.trim().to_lowercase();
    match normalized.as_str() {
        "true" | "1" | "yes" | "on" => Ok(true),
        "false" | "0" | "no" | "off" => Ok(false),
        _ => anyhow::bail!("Invalid boolean for {}: {}", label, value),
    }
}

fn parse_u8(value: &str, label: &str) -> Result<u8> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        anyhow::bail!("Config {} is empty", label);
    }
    trimmed
        .parse::<u8>()
        .with_context(|| format!("Invalid integer for {}: {}", label, value))
}

fn log_at(verbosity: u8, level: u8, message: impl AsRef<str>) {
    if verbosity >= level {
        println!("{}", message.as_ref());
    }
}

fn list_worktrees(repo_root: &Path, verbosity: u8) -> Result<Vec<WorktreeInfo>> {
    let output = run_git(
        repo_root,
        vec!["worktree".to_string(), "list".to_string(), "--porcelain".to_string()],
        verbosity,
    )?;
    let stdout = String::from_utf8(output.stdout)?;
    Ok(parse_worktrees(&stdout))
}

fn parse_worktrees(output: &str) -> Vec<WorktreeInfo> {
    let mut worktrees = Vec::new();
    let mut current: Option<WorktreeInfo> = None;

    for line in output.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            if let Some(worktree) = current.take() {
                worktrees.push(worktree);
            }
            current = Some(WorktreeInfo {
                path: PathBuf::from(path.trim()),
                head: None,
            });
        } else if let Some(head) = line.strip_prefix("HEAD ") {
            if let Some(worktree) = current.as_mut() {
                worktree.head = Some(head.trim().to_string());
            }
        }
    }

    if let Some(worktree) = current {
        worktrees.push(worktree);
    }

    worktrees
}

fn add_worktree(repo_root: &Path, path: &Path, commit: &str, verbosity: u8) -> Result<()> {
    run_git(
        repo_root,
        vec![
            "worktree".to_string(),
            "add".to_string(),
            path.display().to_string(),
            commit.to_string(),
        ],
        verbosity,
    )?;
    Ok(())
}

fn remove_worktree(repo_root: &Path, path: &Path, force: bool, verbosity: u8) -> Result<()> {
    let mut args = vec!["worktree".to_string(), "remove".to_string()];
    if force {
        args.push("--force".to_string());
    }
    args.push(path.display().to_string());
    run_git(repo_root, args, verbosity)?;
    Ok(())
}

fn fetch_patch_content(repo_root: &Path, patch_ref: &str, verbosity: u8) -> Result<String> {
    run_git(
        repo_root,
        vec![
            "fetch".to_string(),
            "origin".to_string(),
            patch_ref.to_string(),
        ],
        verbosity,
    )?;
    let output = run_git(
        repo_root,
        vec!["cat-file".to_string(), "-p".to_string(), "FETCH_HEAD".to_string()],
        verbosity,
    )?;
    Ok(String::from_utf8(output.stdout)?)
}

fn patch_already_applied(
    worktree_path: &Path,
    patch_content: &str,
    verbosity: u8,
) -> Result<bool> {
    run_git_check(
        worktree_path,
        vec![
            "apply".to_string(),
            "--reverse".to_string(),
            "--check".to_string(),
        ],
        patch_content,
        verbosity,
    )
}

fn apply_patch_to_worktree(
    worktree_path: &Path,
    patch_content: &str,
    verbosity: u8,
) -> Result<()> {
    run_git_with_input(
        worktree_path,
        vec!["apply".to_string()],
        patch_content,
        verbosity,
    )
}

fn run_git(current_dir: &Path, args: Vec<String>, verbosity: u8) -> Result<std::process::Output> {
    if verbosity >= 3 {
        println!("[git] git {}", args.join(" "));
    }
    let start = Instant::now();
    let output = Command::new("git")
        .current_dir(current_dir)
        .args(&args)
        .output()
        .context("Failed to run git")?;
    if verbosity >= 3 {
        println!(
            "[git] exit_code: {} ({}ms)",
            output.status.code().unwrap_or(-1),
            start.elapsed().as_millis()
        );
    }
    if !output.status.success() {
        anyhow::bail!(
            "Git command failed: git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(output)
}

fn run_git_with_input(
    current_dir: &Path,
    args: Vec<String>,
    input: &str,
    verbosity: u8,
) -> Result<()> {
    if verbosity >= 3 {
        println!("[git] git {} <stdin>", args.join(" "));
    }
    let start = Instant::now();
    let mut child = Command::new("git")
        .current_dir(current_dir)
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Failed to run git")?;

    if let Some(stdin) = child.stdin.as_mut() {
        use std::io::Write;
        stdin.write_all(input.as_bytes())?;
    }

    let output = child.wait_with_output()?;
    if verbosity >= 3 {
        println!(
            "[git] exit_code: {} ({}ms)",
            output.status.code().unwrap_or(-1),
            start.elapsed().as_millis()
        );
    }
    if !output.status.success() {
        anyhow::bail!(
            "Git command failed: git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

fn run_git_check(
    current_dir: &Path,
    args: Vec<String>,
    input: &str,
    verbosity: u8,
) -> Result<bool> {
    if verbosity >= 3 {
        println!("[git] git {} <stdin>", args.join(" "));
    }
    let start = Instant::now();
    let mut child = Command::new("git")
        .current_dir(current_dir)
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Failed to run git")?;

    if let Some(stdin) = child.stdin.as_mut() {
        use std::io::Write;
        stdin.write_all(input.as_bytes())?;
    }

    let output = child.wait_with_output()?;
    if verbosity >= 3 {
        println!(
            "[git] exit_code: {} ({}ms)",
            output.status.code().unwrap_or(-1),
            start.elapsed().as_millis()
        );
    }
    Ok(output.status.success())
}

// === Validate Command ===

fn cmd_validate(path: &str) -> Result<()> {
    let validator = Validator::new()?;
    let validation_type = validator.validate_file(Path::new(path))?;
    println!("Valid {} file: {}", validation_type, path);
    Ok(())
}
