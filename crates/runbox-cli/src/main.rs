use anyhow::{bail, Context, Result};
use chrono::Utc;
use clap::{Parser, Subcommand};
use dialoguer::{theme::ColorfulTheme, Input};
use runbox_core::{
    available_runtimes, get_adapter, BindingResolver, ConfigResolver, GitContext, LogRef,
    Playlist, PlaylistItem, Run, RunStatus, RunTemplate, RuntimeHandle, Storage, Timeline,
    Validator, VerboseLogger,
};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::Command;

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

        /// Runtime: background (bg) or tmux (default: background)
        #[arg(long, default_value = "background")]
        runtime: String,
    },

    /// List running and recent runs
    Ps {
        /// Filter by status (running, exited, failed, etc.)
        #[arg(long)]
        status: Option<String>,

        /// Number of runs to show (default: 20)
        #[arg(short, long, default_value = "20")]
        limit: usize,
    },

    /// Stop a running process
    Stop {
        /// Run ID (full or short)
        run_id: String,

        /// Force kill with SIGKILL (default: SIGTERM)
        #[arg(long)]
        force: bool,
    },

    /// View logs for a run
    Logs {
        /// Run ID (full or short)
        run_id: String,

        /// Follow log output (like tail -f)
        #[arg(short, long)]
        follow: bool,

        /// Number of lines to show (default: all)
        #[arg(short = 'n', long)]
        lines: Option<usize>,
    },

    /// Attach to a running process (tmux/zellij only)
    Attach {
        /// Run ID (full or short)
        run_id: String,
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

    /// Replay a previous run in an isolated worktree
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

        /// Reuse existing worktree if commit matches (default)
        #[arg(long, conflicts_with = "fresh")]
        reuse: bool,

        /// Always create fresh worktree
        #[arg(long, conflicts_with = "reuse")]
        fresh: bool,

        /// Verbose output (can be repeated: -v, -vv, -vvv)
        #[arg(short, long, action = clap::ArgAction::Count)]
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
            runtime,
        } => cmd_run(&storage, &template, binding, dry_run, &runtime),
        Commands::Ps { status, limit } => cmd_ps(&storage, status, limit),
        Commands::Stop { run_id, force } => cmd_stop(&storage, &run_id, force),
        Commands::Logs {
            run_id,
            follow,
            lines,
        } => cmd_logs(&storage, &run_id, follow, lines),
        Commands::Attach { run_id } => cmd_attach(&storage, &run_id),
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
            worktree_dir,
            keep,
            cleanup,
            reuse,
            fresh,
            verbose,
        ),
        Commands::Validate { path } => cmd_validate(&path),
    }
}

// === Run Command ===

fn cmd_run(
    storage: &Storage,
    template_id: &str,
    bindings: Vec<String>,
    dry_run: bool,
    runtime_name: &str,
) -> Result<()> {
    // Validate runtime
    let adapter = get_adapter(runtime_name)
        .ok_or_else(|| anyhow::anyhow!("Unknown runtime: {}. Available: {:?}", runtime_name, available_runtimes()))?;

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

    // Build run (this creates a Run with Pending status)
    let mut run = resolver.build_run(&template, code_state)?;

    // Validate
    run.validate()?;

    if dry_run {
        println!("Dry run - would execute:");
        println!("{}", serde_json::to_string_pretty(&run)?);
        return Ok(());
    }

    // Set runtime and log path
    let log_path = storage.log_path(&run.run_id);
    run.runtime = adapter.name().to_string();
    run.log_ref = Some(LogRef {
        path: log_path.clone(),
    });
    run.timeline = Timeline {
        created_at: Some(Utc::now()),
        started_at: None,
        ended_at: None,
    };

    // Save run before spawning
    storage.save_run(&run)?;

    // Spawn the process
    println!("Starting run: {}", run.run_id);
    println!("Runtime: {}", adapter.name());
    println!("Command: {:?}", run.exec.argv);

    let handle = adapter.spawn(&run.exec, &run.run_id, &log_path)?;

    // Update run with handle and status
    run.handle = Some(handle);
    run.status = RunStatus::Running;
    run.timeline.started_at = Some(Utc::now());

    // Save updated run
    let path = storage.save_run(&run)?;

    println!("\nRun started: {}", run.run_id);
    println!("Log file: {}", log_path.display());
    println!("Run saved: {}", path.display());
    println!("\nUse 'runbox logs {}' to view output", run.short_id());
    println!("Use 'runbox stop {}' to stop the run", run.short_id());

    if adapter.name() == "tmux" {
        println!("Use 'runbox attach {}' to attach to tmux", run.short_id());
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
    let run = find_run(storage, run_id)?;

    println!("Run ID:      {}", run.run_id);
    println!("Short ID:    {}", run.short_id());
    println!("Status:      {}", run.status);
    println!("Runtime:     {}", if run.runtime.is_empty() { "none" } else { &run.runtime });
    println!("Command:     {}", run.exec.argv.join(" "));
    println!("Working Dir: {}", run.exec.cwd);

    if !run.exec.env.is_empty() {
        println!("Environment:");
        for (k, v) in &run.exec.env {
            println!("  {}={}", k, v);
        }
    }

    println!("\nCode State:");
    println!("  Repo:   {}", run.code_state.repo_url);
    println!("  Commit: {}", run.code_state.base_commit);
    if let Some(patch) = &run.code_state.patch {
        println!("  Patch:  {}", patch.ref_);
    }

    println!("\nTimeline:");
    if let Some(created) = &run.timeline.created_at {
        println!("  Created: {}", created);
    }
    if let Some(started) = &run.timeline.started_at {
        println!("  Started: {}", started);
    }
    if let Some(ended) = &run.timeline.ended_at {
        println!("  Ended:   {}", ended);
    }

    if let Some(exit_code) = run.exit_code {
        println!("\nExit Code: {}", exit_code);
    }

    if let Some(reason) = &run.reconcile_reason {
        println!("\nReconcile Reason: {}", reason);
    }

    if let Some(log_ref) = &run.log_ref {
        println!("\nLog File: {}", log_ref.path.display());
    }

    if let Some(handle) = &run.handle {
        println!("\nRuntime Handle:");
        match handle {
            RuntimeHandle::Background { pid, pgid } => {
                println!("  Type: Background");
                println!("  PID:  {}", pid);
                println!("  PGID: {}", pgid);
            }
            RuntimeHandle::Tmux { session, window } => {
                println!("  Type:    Tmux");
                println!("  Session: {}", session);
                println!("  Window:  {}", window);
            }
            RuntimeHandle::Zellij { session, tab } => {
                println!("  Type:    Zellij");
                println!("  Session: {}", session);
                println!("  Tab:     {}", tab);
            }
        }
    }

    Ok(())
}

/// Find a run by full ID or short ID
fn find_run(storage: &Storage, run_id: &str) -> Result<Run> {
    // Try loading by full ID first
    if let Ok(run) = storage.load_run(run_id) {
        return Ok(run);
    }

    // Try prefixing with "run_" if not already
    if !run_id.starts_with("run_") {
        let full_id = format!("run_{}", run_id);
        if let Ok(run) = storage.load_run(&full_id) {
            return Ok(run);
        }
    }

    // Search by short ID prefix
    let runs = storage.list_runs(1000)?;
    let matches: Vec<_> = runs
        .into_iter()
        .filter(|r| r.short_id().starts_with(run_id) || r.run_id.contains(run_id))
        .collect();

    match matches.len() {
        0 => bail!("Run not found: {}", run_id),
        1 => Ok(matches.into_iter().next().unwrap()),
        _ => {
            println!("Multiple matches found:");
            for r in &matches {
                println!("  {} ({})", r.short_id(), r.run_id);
            }
            bail!("Ambiguous run ID: {}", run_id)
        }
    }
}

/// Reconcile run statuses by checking if processes are still alive
/// Uses CAS-style updates: only updates if status is Running
fn reconcile_runs(storage: &Storage) -> Result<()> {
    let runs = storage.list_runs(1000)?;

    for run in runs {
        if run.status != RunStatus::Running {
            continue;
        }

        let reason = match &run.handle {
            None => Some("no runtime handle".to_string()),
            Some(handle) => {
                let Some(adapter) = get_adapter(&run.runtime) else {
                    continue;
                };

                if !adapter.is_alive(handle) {
                    // Process is dead but status is Running
                    let reason = match handle {
                        RuntimeHandle::Background { pid, .. } => {
                            format!("process {} not found", pid)
                        }
                        RuntimeHandle::Tmux { session, window } => {
                            format!("tmux window '{}:{}' not found", session, window)
                        }
                        RuntimeHandle::Zellij { session, tab } => {
                            format!("zellij tab '{}:{}' not found", session, tab)
                        }
                    };
                    Some(reason)
                } else {
                    None
                }
            }
        };

        if let Some(reason) = reason {
            mark_unknown(storage, &run.run_id, &reason)?;
        }
    }

    Ok(())
}

/// Mark a run as Unknown with CAS-style update
/// Only updates if the run is currently Running
/// Does not overwrite ended_at if already set
fn mark_unknown(storage: &Storage, run_id: &str, reason: &str) -> Result<()> {
    let mut run = storage.load_run(run_id)?;

    // CAS: Only update if Running
    if run.status != RunStatus::Running {
        return Ok(());
    }

    run.status = RunStatus::Unknown;
    run.reconcile_reason = Some(reason.to_string());

    // Don't overwrite ended_at (preserve first end time)
    if run.timeline.ended_at.is_none() {
        run.timeline.ended_at = Some(Utc::now());
    }

    storage.save_run(&run)?;
    Ok(())
}

// === Process Status Command ===

fn cmd_ps(storage: &Storage, status_filter: Option<String>, limit: usize) -> Result<()> {
    // Run reconcile to update stale statuses
    reconcile_runs(storage)?;

    let runs = storage.list_runs(limit)?;

    let filtered: Vec<_> = if let Some(ref filter) = status_filter {
        let filter_status = match filter.to_lowercase().as_str() {
            "pending" => RunStatus::Pending,
            "running" => RunStatus::Running,
            "exited" => RunStatus::Exited,
            "failed" => RunStatus::Failed,
            "killed" => RunStatus::Killed,
            "unknown" => RunStatus::Unknown,
            _ => bail!("Invalid status: {}. Valid: pending, running, exited, failed, killed, unknown", filter),
        };
        runs.into_iter().filter(|r| r.status == filter_status).collect()
    } else {
        runs
    };

    if filtered.is_empty() {
        if status_filter.is_some() {
            println!("No runs with status '{}'", status_filter.unwrap());
        } else {
            println!("No runs found.");
        }
        return Ok(());
    }

    println!(
        "{:<10} {:<10} {:<12} {:<40}",
        "SHORT_ID", "STATUS", "RUNTIME", "COMMAND"
    );
    println!("{}", "-".repeat(75));

    for run in filtered {
        let cmd = run.exec.argv.join(" ");
        let cmd_truncated = if cmd.len() > 38 {
            format!("{}...", &cmd[..35])
        } else {
            cmd
        };
        let runtime = if run.runtime.is_empty() {
            "-"
        } else {
            &run.runtime
        };

        println!(
            "{:<10} {:<10} {:<12} {:<40}",
            run.short_id(),
            run.status,
            runtime,
            cmd_truncated
        );
    }

    Ok(())
}

// === Stop Command ===

fn cmd_stop(storage: &Storage, run_id: &str, force: bool) -> Result<()> {
    let mut run = find_run(storage, run_id)?;

    if run.status != RunStatus::Running {
        bail!(
            "Run {} is not running (status: {})",
            run.short_id(),
            run.status
        );
    }

    let Some(ref handle) = run.handle else {
        bail!("Run {} has no runtime handle", run.short_id());
    };

    let adapter = get_adapter(&run.runtime)
        .ok_or_else(|| anyhow::anyhow!("Unknown runtime: {}", run.runtime))?;

    if force {
        println!("Force stopping run {} (SIGKILL)...", run.short_id());
    } else {
        println!("Stopping run {} (SIGTERM)...", run.short_id());
    }
    adapter.stop(handle, force)?;

    // CAS-style update: reload and check status
    run = storage.load_run(&run.run_id)?;
    if run.status != RunStatus::Running {
        // Already updated by another process
        println!("Run {} already stopped", run.short_id());
        return Ok(());
    }

    // Update run status
    run.status = RunStatus::Killed;
    // Don't overwrite ended_at if already set
    if run.timeline.ended_at.is_none() {
        run.timeline.ended_at = Some(Utc::now());
    }
    storage.save_run(&run)?;

    println!("Run {} stopped", run.short_id());
    Ok(())
}

// === Logs Command ===

fn cmd_logs(storage: &Storage, run_id: &str, follow: bool, lines: Option<usize>) -> Result<()> {
    let run = find_run(storage, run_id)?;

    let Some(ref log_ref) = run.log_ref else {
        bail!("Run {} has no log file", run.short_id());
    };

    if !log_ref.path.exists() {
        bail!("Log file not found: {}", log_ref.path.display());
    }

    if follow {
        // Follow mode - similar to tail -f
        let mut file = std::fs::File::open(&log_ref.path)?;
        let mut reader = BufReader::new(&mut file);

        // Print existing content
        let mut line = String::new();
        while reader.read_line(&mut line)? > 0 {
            print!("{}", line);
            line.clear();
        }

        // Follow new content
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => {
                    // No new content - check if process is still running
                    if run.status != RunStatus::Running {
                        break;
                    }
                    // Also check if process is actually alive
                    if let (Some(ref handle), Some(adapter)) =
                        (&run.handle, get_adapter(&run.runtime))
                    {
                        if !adapter.is_alive(handle) {
                            println!("\n[Process exited]");
                            break;
                        }
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                Ok(_) => {
                    print!("{}", line);
                }
                Err(e) => {
                    eprintln!("Error reading log: {}", e);
                    break;
                }
            }
        }
    } else {
        // Regular mode - print all or last N lines
        let content = std::fs::read_to_string(&log_ref.path)?;

        if let Some(n) = lines {
            let all_lines: Vec<_> = content.lines().collect();
            let start = if all_lines.len() > n {
                all_lines.len() - n
            } else {
                0
            };
            for line in &all_lines[start..] {
                println!("{}", line);
            }
        } else {
            print!("{}", content);
        }
    }

    Ok(())
}

// === Attach Command ===

fn cmd_attach(storage: &Storage, run_id: &str) -> Result<()> {
    let run = find_run(storage, run_id)?;

    if run.runtime == "background" {
        bail!("Cannot attach to background runtime. Use 'runbox logs -f {}' instead.", run.short_id());
    }

    let Some(ref handle) = run.handle else {
        bail!("Run {} has no runtime handle", run.short_id());
    };

    let adapter = get_adapter(&run.runtime)
        .ok_or_else(|| anyhow::anyhow!("Unknown runtime: {}", run.runtime))?;

    println!("Attaching to run {}...", run.short_id());
    adapter.attach(handle)?;

    // Note: attach() calls exec() and replaces this process, so we never reach here
    Ok(())
}

// === Replay Command ===

fn cmd_replay(
    storage: &Storage,
    run_id: &str,
    worktree_dir: Option<PathBuf>,
    keep: bool,
    cleanup: bool,
    reuse: bool,
    fresh: bool,
    verbose: u8,
) -> Result<()> {
    let run = storage.load_run(run_id)?;

    // Initialize git context from current directory
    let git = GitContext::from_current_dir()?;

    // Create config resolver
    let config_resolver = ConfigResolver::new(Some(git.repo_root().to_path_buf()))?;

    // Resolve verbosity
    let resolved_verbosity = config_resolver.resolve_verbosity(verbose);
    let logger = VerboseLogger::new(resolved_verbosity.value);

    logger.log_v(
        "config",
        &format!(
            "verbosity: {} (from: {})",
            resolved_verbosity.value, resolved_verbosity.source
        ),
    );

    // Resolve worktree directory
    let resolved_worktree_dir = config_resolver.resolve_worktree_dir(worktree_dir.as_ref());
    logger.log_v(
        "config",
        &format!(
            "worktree_dir: {} (from: {})",
            resolved_worktree_dir.value.display(),
            resolved_worktree_dir.source
        ),
    );

    // Resolve cleanup setting
    let cli_cleanup = if cleanup {
        Some(true)
    } else if keep {
        Some(false)
    } else {
        None
    };
    let resolved_cleanup = config_resolver.resolve_cleanup(cli_cleanup);
    logger.log_v(
        "config",
        &format!(
            "cleanup: {} (from: {})",
            resolved_cleanup.value, resolved_cleanup.source
        ),
    );

    // Resolve reuse setting
    let cli_reuse = if fresh {
        Some(false)
    } else if reuse {
        Some(true)
    } else {
        None
    };
    let resolved_reuse = config_resolver.resolve_reuse(cli_reuse);
    logger.log_v(
        "config",
        &format!(
            "reuse: {} (from: {})",
            resolved_reuse.value, resolved_reuse.source
        ),
    );

    // Print run info
    println!("Replaying: {}", run_id);
    println!("Command: {:?}", run.exec.argv);
    println!("Commit: {}", run.code_state.base_commit);
    if run.code_state.patch.is_some() {
        println!("Patch: yes");
    }

    // Restore code state in worktree
    let worktree_result = git.restore_code_state_in_worktree(
        &run.code_state,
        run_id,
        &resolved_worktree_dir.value,
        resolved_reuse.value,
        &logger,
    )?;

    if worktree_result.reused {
        println!(
            "Reusing existing worktree: {}",
            worktree_result.worktree_path.display()
        );
    } else {
        println!(
            "Created worktree: {}",
            worktree_result.worktree_path.display()
        );
    }

    // Resolve the execution directory relative to worktree
    let exec_cwd = if Path::new(&run.exec.cwd).is_absolute() {
        // If cwd is absolute, make it relative to worktree
        PathBuf::from(&run.exec.cwd)
    } else {
        // Relative path - combine with worktree
        worktree_result.worktree_path.join(&run.exec.cwd)
    };

    logger.log_vv("exec", &format!("cwd: {}", exec_cwd.display()));
    logger.log_vv("exec", &format!("argv: {:?}", run.exec.argv));
    if !run.exec.env.is_empty() {
        logger.log_vvv("exec", &format!("env: {:?}", run.exec.env));
    }

    // Execute
    println!("\nExecuting...");
    let status = Command::new(&run.exec.argv[0])
        .args(&run.exec.argv[1..])
        .current_dir(&exec_cwd)
        .envs(&run.exec.env)
        .status()
        .context("Failed to execute command")?;

    let exit_code = status.code().unwrap_or(-1);
    logger.log_vv("exec", &format!("exit_code: {}", exit_code));

    if status.success() {
        println!("\nReplay completed successfully");
    } else {
        println!("\nReplay failed with status: {:?}", status.code());
    }

    // Cleanup if requested
    if resolved_cleanup.value {
        println!("Cleaning up worktree...");
        git.remove_worktree(&worktree_result.worktree_path, &logger)?;
        println!("Worktree removed");
    } else {
        println!(
            "Worktree kept at: {}",
            worktree_result.worktree_path.display()
        );
    }

    Ok(())
}

// === Validate Command ===

fn cmd_validate(path: &str) -> Result<()> {
    let validator = Validator::new()?;
    let validation_type = validator.validate_file(Path::new(path))?;
    println!("Valid {} file: {}", validation_type, path);
    Ok(())
}
