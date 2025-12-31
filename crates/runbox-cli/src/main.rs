use anyhow::{bail, Context, Result};
use chrono::Utc;
use clap::{Parser, Subcommand, ValueEnum};
use dialoguer::{theme::ColorfulTheme, Input};
use runbox_core::{
    default_pid_path, default_socket_path, short_id, BindingResolver, CodeState, ConfigResolver,
    DaemonClient, Exec, GitContext, LogRef, Playlist, PlaylistItem, Run, RunSource, RunStatus,
    RunTemplate, RuntimeRegistry, Storage, Timeline, Validator, VerboseLogger,
};
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;

#[derive(Parser)]
#[command(name = "runbox")]
#[command(about = "Reproducible command execution system")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

/// Runtime type for execution
#[derive(Debug, Clone, Copy, Default, ValueEnum)]
enum RuntimeType {
    /// Background process (default)
    #[default]
    Bg,
    /// Background process (alias)
    Background,
    /// Tmux window
    Tmux,
}

impl std::fmt::Display for RuntimeType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RuntimeType::Bg | RuntimeType::Background => write!(f, "background"),
            RuntimeType::Tmux => write!(f, "tmux"),
        }
    }
}

#[derive(Subcommand)]
enum Commands {
    /// Run from a template or execute a command directly
    ///
    /// To run from a template: runbox run --template <id> [--binding key=value]
    /// To run directly: runbox run -- <command...>
    Run {
        /// Template ID (for template-based runs)
        #[arg(short, long)]
        template: Option<String>,

        /// Variable bindings (key=value) for template runs
        #[arg(short, long)]
        binding: Vec<String>,

        /// Runtime environment (bg, background, tmux)
        #[arg(long, default_value = "bg")]
        runtime: RuntimeType,

        /// Command timeout in seconds (0 = no timeout)
        #[arg(long, default_value = "0")]
        timeout: u64,

        /// Additional environment variables (KEY=VALUE)
        #[arg(long = "env", short = 'e')]
        env_vars: Vec<String>,

        /// Working directory (default: current)
        #[arg(long)]
        cwd: Option<PathBuf>,

        /// Skip git context capture (for direct runs)
        #[arg(long)]
        no_git: bool,

        /// Skip execution (dry run)
        #[arg(long)]
        dry_run: bool,

        /// Command to execute directly (everything after --)
        #[arg(last = true)]
        command: Vec<String>,
    },

    /// Log a command execution (alias for `runbox run --`)
    Log {
        /// Runtime environment (bg, background, tmux)
        #[arg(long, default_value = "bg")]
        runtime: RuntimeType,

        /// Command timeout in seconds (0 = no timeout)
        #[arg(long, default_value = "0")]
        timeout: u64,

        /// Additional environment variables (KEY=VALUE)
        #[arg(long = "env", short = 'e')]
        env_vars: Vec<String>,

        /// Working directory (default: current)
        #[arg(long)]
        cwd: Option<PathBuf>,

        /// Skip git context capture
        #[arg(long)]
        no_git: bool,

        /// Skip execution (dry run)
        #[arg(long)]
        dry_run: bool,

        /// Command to execute (everything after --)
        #[arg(last = true, required = true)]
        command: Vec<String>,
    },

    /// List running and recent runs
    Ps {
        /// Filter by status
        #[arg(long)]
        status: Option<String>,

        /// Show all runs (not just recent)
        #[arg(short, long)]
        all: bool,

        /// Limit number of results
        #[arg(short, long, default_value = "20")]
        limit: usize,
    },

    /// Stop a running process
    Stop {
        /// Run ID (or short ID)
        run_id: String,

        /// Force kill (SIGKILL instead of SIGTERM)
        #[arg(long, short)]
        force: bool,
    },

    /// Show logs for a run
    Logs {
        /// Run ID (or short ID)
        run_id: String,

        /// Follow log output (like tail -f)
        #[arg(short, long)]
        follow: bool,

        /// Number of lines to show (default: all)
        #[arg(short, long)]
        lines: Option<usize>,
    },

    /// Attach to a running process (tmux only)
    Attach {
        /// Run ID (or short ID)
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
        /// Run ID (or short ID)
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

    /// Manage the background daemon (for debugging)
    Daemon {
        #[command(subcommand)]
        command: DaemonCommands,
    },
}

#[derive(Subcommand)]
enum DaemonCommands {
    /// Start the daemon in foreground mode
    Start,
    /// Stop the running daemon
    Stop,
    /// Check daemon status
    Status,
    /// Ping the daemon
    Ping,
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
    let storage = if let Ok(home) = std::env::var("RUNBOX_HOME") {
        Storage::with_base_dir(PathBuf::from(home))?
    } else {
        Storage::new()?
    };

    match cli.command {
        Commands::Run {
            template,
            binding,
            runtime,
            timeout,
            env_vars,
            cwd,
            no_git,
            dry_run,
            command,
        } => {
            if let Some(template_id) = template {
                // Template-based run
                cmd_run_template(&storage, &template_id, binding, runtime, dry_run)
            } else if !command.is_empty() {
                // Direct command execution
                cmd_run_direct(&storage, command, runtime, timeout, env_vars, cwd, no_git, dry_run)
            } else {
                bail!("Either --template or a command after -- is required.\n\nUsage:\n  runbox run --template <id>  # Run from template\n  runbox run -- <command>     # Run command directly")
            }
        }
        Commands::Log {
            runtime,
            timeout,
            env_vars,
            cwd,
            no_git,
            dry_run,
            command,
        } => cmd_run_direct(&storage, command, runtime, timeout, env_vars, cwd, no_git, dry_run),
        Commands::Ps { status, all, limit } => cmd_ps(&storage, status, all, limit),
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
        Commands::Daemon { command } => match command {
            DaemonCommands::Start => cmd_daemon_start(),
            DaemonCommands::Stop => cmd_daemon_stop(),
            DaemonCommands::Status => cmd_daemon_status(),
            DaemonCommands::Ping => cmd_daemon_ping(),
        },
    }
}

// === Daemon Commands ===

fn cmd_daemon_start() -> Result<()> {
    use std::process::Command as StdCommand;

    // Find the daemon binary
    let daemon_path = which_daemon()?;

    println!("Starting daemon in foreground mode...");
    println!("Daemon path: {}", daemon_path.display());
    println!("Socket: {}", default_socket_path().display());
    println!("Press Ctrl+C to stop");

    // Start daemon in foreground
    let status = StdCommand::new(&daemon_path)
        .arg("--foreground")
        .status()
        .with_context(|| format!("Failed to start daemon from {}", daemon_path.display()))?;

    if status.success() {
        println!("Daemon exited normally");
    } else {
        bail!("Daemon exited with status: {:?}", status.code());
    }

    Ok(())
}

fn cmd_daemon_stop() -> Result<()> {
    let client = DaemonClient::new();

    if !client.is_running() {
        println!("Daemon is not running");
        return Ok(());
    }

    println!("Stopping daemon...");
    client.shutdown()?;
    println!("Daemon stopped");

    Ok(())
}

fn cmd_daemon_status() -> Result<()> {
    let socket_path = default_socket_path();
    let pid_path = default_pid_path();

    println!("Socket path: {}", socket_path.display());
    println!("PID file:    {}", pid_path.display());

    // Check if socket exists
    if !socket_path.exists() {
        println!("Status:      not running (no socket)");
        return Ok(());
    }

    // Try to connect
    let client = DaemonClient::new();
    if client.is_running() {
        // Read PID if available
        if let Ok(pid_str) = std::fs::read_to_string(&pid_path) {
            println!("PID:         {}", pid_str.trim());
        }
        println!("Status:      running");
    } else {
        println!("Status:      not running (socket exists but not responding)");
    }

    Ok(())
}

fn cmd_daemon_ping() -> Result<()> {
    let client = DaemonClient::new();

    println!("Pinging daemon...");

    match client.ping() {
        Ok(true) => {
            println!("Daemon is alive (pong received)");
            Ok(())
        }
        Ok(false) => {
            bail!("Daemon responded but ping failed");
        }
        Err(e) => {
            bail!("Failed to ping daemon: {}", e);
        }
    }
}

/// Find the daemon binary path
fn which_daemon() -> Result<PathBuf> {
    // First, check if RUNBOX_DAEMON_PATH env var is set
    if let Ok(path) = std::env::var("RUNBOX_DAEMON_PATH") {
        let path = PathBuf::from(path);
        if path.exists() {
            return Ok(path);
        }
    }

    // Check if runbox-daemon is in the same directory as the current executable
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let daemon_path = dir.join("runbox-daemon");
            if daemon_path.exists() {
                return Ok(daemon_path);
            }
        }
    }

    // Check PATH
    if let Ok(path_env) = std::env::var("PATH") {
        for dir in path_env.split(':') {
            let daemon_path = PathBuf::from(dir).join("runbox-daemon");
            if daemon_path.exists() {
                return Ok(daemon_path);
            }
        }
    }

    // Fallback to hoping it's in PATH
    Ok(PathBuf::from("runbox-daemon"))
}

// === Run Commands ===

/// Run a command from a template
fn cmd_run_template(
    storage: &Storage,
    template_id: &str,
    bindings: Vec<String>,
    runtime: RuntimeType,
    dry_run: bool,
) -> Result<()> {
    let resolved_template_id = storage.resolve_template_id(template_id)?;
    let template = storage.load_template(&resolved_template_id)?;

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
    let mut run = resolver.build_run(&template, code_state)?;

    // Validate
    run.validate()?;

    if dry_run {
        println!("Dry run - would execute:");
        println!("{}", serde_json::to_string_pretty(&run)?);
        return Ok(());
    }

    // Get runtime adapter
    let registry = RuntimeRegistry::new();
    let runtime_name = runtime.to_string();
    let adapter = registry
        .get(&runtime_name)
        .context(format!("Unknown runtime: {}", runtime_name))?;

    // Set up log path
    let log_path = storage.log_path(&run.run_id);

    // Update run with runtime info
    run.runtime = runtime_name.clone();
    run.log_ref = Some(LogRef {
        path: log_path.clone(),
    });
    run.timeline = Timeline {
        created_at: Some(Utc::now()),
        started_at: None,
        ended_at: None,
    };
    run.status = RunStatus::Pending;
    run.source = RunSource::Template;

    // Save run (before spawning)
    storage.save_run(&run)?;

    // Spawn process
    println!("Starting run: {}", run.run_id);
    println!("Runtime: {}", runtime_name);
    println!("Command: {:?}", run.exec.argv);

    let handle = adapter.spawn(&run.exec, &run.run_id, &log_path)?;

    // CAS-style update with lock: only update if still Pending
    // This prevents overwriting terminal state if process exited very fast
    let saved = storage.save_run_if_status_with(
        &run.run_id,
        &[RunStatus::Pending],
        |current| {
            current.handle = Some(handle.clone());
            current.status = RunStatus::Running;
            current.timeline.started_at = Some(Utc::now());
        }
    )?;

    if !saved {
        // Process already exited - daemon captured the status
        // Just update handle if not set (using another CAS)
        let _ = storage.save_run_if_status_with(
            &run.run_id,
            &[RunStatus::Exited, RunStatus::Failed, RunStatus::Unknown],
            |current| {
                if current.handle.is_none() {
                    current.handle = Some(handle.clone());
                }
            }
        );
        log::debug!(
            "Run {} already exited - daemon captured status",
            run.run_id
        );
    }

    println!("Run started: {}", run.run_id);
    println!("Short ID: {}", run.short_id());
    println!("Logs: {}", log_path.display());

    if matches!(runtime, RuntimeType::Tmux) {
        println!("Attach with: runbox attach {}", run.short_id());
    }

    Ok(())
}

/// Run a command directly without a template
fn cmd_run_direct(
    storage: &Storage,
    command: Vec<String>,
    runtime: RuntimeType,
    timeout: u64,
    env_vars: Vec<String>,
    cwd: Option<PathBuf>,
    no_git: bool,
    dry_run: bool,
) -> Result<()> {
    if command.is_empty() {
        bail!("No command specified. Usage: runbox run -- <command>");
    }

    // Generate run_id
    let run_id = format!("run_{}", uuid::Uuid::new_v4());

    // Parse environment variables
    let mut env = std::collections::HashMap::new();
    for env_var in env_vars {
        if let Some((key, value)) = env_var.split_once('=') {
            env.insert(key.to_string(), value.to_string());
        } else {
            bail!("Invalid environment variable format: '{}'. Use KEY=VALUE", env_var);
        }
    }

    // Determine working directory
    let cwd_str = if let Some(ref dir) = cwd {
        dir.to_string_lossy().to_string()
    } else {
        std::env::current_dir()?.to_string_lossy().to_string()
    };

    // Build exec
    let exec = Exec {
        argv: command.clone(),
        cwd: cwd_str,
        env,
        timeout_sec: timeout,
    };

    // Build code state (optionally skip git)
    let code_state = if no_git {
        // Create a placeholder code state when git is skipped
        CodeState {
            repo_url: "none".to_string(),
            base_commit: "0".repeat(40),  // Placeholder 40-char SHA
            patch: None,
        }
    } else {
        let git = GitContext::from_current_dir()?;
        git.build_code_state(&run_id)?
    };

    // Create run
    let mut run = Run::new_direct(exec, code_state);
    run.run_id = run_id;

    // Validate
    run.validate()?;

    if dry_run {
        println!("Dry run - would execute:");
        println!("{}", serde_json::to_string_pretty(&run)?);
        return Ok(());
    }

    // Get runtime adapter
    let registry = RuntimeRegistry::new();
    let runtime_name = runtime.to_string();
    let adapter = registry
        .get(&runtime_name)
        .context(format!("Unknown runtime: {}", runtime_name))?;

    // Set up log path
    let log_path = storage.log_path(&run.run_id);

    // Update run with runtime info
    run.runtime = runtime_name.clone();
    run.log_ref = Some(LogRef {
        path: log_path.clone(),
    });
    run.timeline = Timeline {
        created_at: Some(Utc::now()),
        started_at: None,
        ended_at: None,
    };
    run.status = RunStatus::Pending;

    // Save run (before spawning)
    storage.save_run(&run)?;

    // Spawn process
    println!("Starting run: {}", run.run_id);
    println!("Source: direct");
    println!("Runtime: {}", runtime_name);
    println!("Command: {:?}", run.exec.argv);

    let handle = adapter.spawn(&run.exec, &run.run_id, &log_path)?;

    // CAS-style update with lock: only update if still Pending
    let saved = storage.save_run_if_status_with(
        &run.run_id,
        &[RunStatus::Pending],
        |current| {
            current.handle = Some(handle.clone());
            current.status = RunStatus::Running;
            current.timeline.started_at = Some(Utc::now());
        }
    )?;

    if !saved {
        // Process already exited - daemon captured the status
        let _ = storage.save_run_if_status_with(
            &run.run_id,
            &[RunStatus::Exited, RunStatus::Failed, RunStatus::Unknown],
            |current| {
                if current.handle.is_none() {
                    current.handle = Some(handle.clone());
                }
            }
        );
        log::debug!(
            "Run {} already exited - daemon captured status",
            run.run_id
        );
    }

    println!("Run started: {}", run.run_id);
    println!("Short ID: {}", run.short_id());
    println!("Logs: {}", log_path.display());

    if matches!(runtime, RuntimeType::Tmux) {
        println!("Attach with: runbox attach {}", run.short_id());
    }

    Ok(())
}

// === Ps Command ===

fn cmd_ps(storage: &Storage, status_filter: Option<String>, _all: bool, limit: usize) -> Result<()> {
    // First, reconcile running processes
    reconcile_runs(storage)?;

    let runs = storage.list_runs(limit)?;

    if runs.is_empty() {
        println!("No runs found.");
        return Ok(());
    }

    // Filter by status if specified
    let runs: Vec<_> = if let Some(ref status_str) = status_filter {
        runs.into_iter()
            .filter(|r| r.status.to_string() == *status_str)
            .collect()
    } else {
        runs
    };

    println!(
        "{:<12} {:<10} {:<10} {:<30}",
        "SHORT ID", "STATUS", "RUNTIME", "COMMAND"
    );
    println!("{}", "-".repeat(70));

    for run in runs {
        let cmd = run.exec.argv.join(" ");
        let cmd_truncated = if cmd.len() > 30 {
            format!("{}...", &cmd[..27])
        } else {
            cmd
        };
        let runtime_display = if run.runtime.is_empty() {
            "-"
        } else {
            &run.runtime
        };

        println!(
            "{:<12} {:<10} {:<10} {:<30}",
            run.short_id(),
            run.status,
            runtime_display,
            cmd_truncated
        );
    }

    Ok(())
}

// === Stop Command ===

fn cmd_stop(storage: &Storage, run_id: &str, force: bool) -> Result<()> {
    let full_run_id = resolve_run_id(storage, run_id)?;
    let run = storage.load_run(&full_run_id)?;

    // CAS: Only allow stopping if status is Running
    if run.status != RunStatus::Running {
        bail!("Run {} is not running (status: {})", run_id, run.status);
    }

    let registry = RuntimeRegistry::new();
    let adapter = registry
        .get(&run.runtime)
        .context(format!("Unknown runtime: {}", run.runtime))?;

    if let Some(ref handle) = run.handle {
        adapter.stop(handle, force)?;

        // CAS-style update with lock: only update if still in stoppable state
        // This prevents overwriting daemon's exit capture
        let _ = storage.save_run_if_status_with(
            &full_run_id,
            &[RunStatus::Running, RunStatus::Pending],
            |current| {
                current.status = RunStatus::Killed;
                if current.timeline.ended_at.is_none() {
                    current.timeline.ended_at = Some(Utc::now());
                }
            }
        );
        // Note: if CAS failed, daemon already set terminal state, which is fine

        if force {
            println!("Force stopped run: {}", full_run_id);
        } else {
            println!("Stopped run: {}", full_run_id);
        }
    } else {
        bail!("Run {} has no handle", run_id);
    }

    Ok(())
}

// === Logs Command ===

fn cmd_logs(storage: &Storage, run_id: &str, follow: bool, lines: Option<usize>) -> Result<()> {
    let full_run_id = resolve_run_id(storage, run_id)?;
    let run = storage.load_run(&full_run_id)?;

    let log_path = if let Some(ref log_ref) = run.log_ref {
        &log_ref.path
    } else {
        // Fallback to default log path
        &storage.log_path(&full_run_id)
    };

    if !log_path.exists() {
        bail!("Log file not found: {}", log_path.display());
    }

    if follow {
        // Tail -f mode
        let mut file = File::open(log_path)?;

        // Show existing content first
        let reader = BufReader::new(&file);
        for line in reader.lines() {
            println!("{}", line?);
        }

        // Then follow new content
        loop {
            let pos = file.stream_position()?;
            file.seek(SeekFrom::End(0))?;
            let end = file.stream_position()?;
            file.seek(SeekFrom::Start(pos))?;

            if end > pos {
                let reader = BufReader::new(&file);
                for line_result in reader.lines() {
                    println!("{}", line_result?);
                }
            }

            // Check if process is still running
            if run.status != RunStatus::Running {
                // Do one final read then exit
                thread::sleep(Duration::from_millis(100));
                let reader = BufReader::new(&file);
                for line_result in reader.lines() {
                    println!("{}", line_result?);
                }
                break;
            }

            thread::sleep(Duration::from_millis(100));
        }
    } else {
        // Show all or last N lines
        let content = std::fs::read_to_string(log_path)?;
        let all_lines: Vec<&str> = content.lines().collect();

        let lines_to_show = if let Some(n) = lines {
            let start = all_lines.len().saturating_sub(n);
            &all_lines[start..]
        } else {
            &all_lines[..]
        };

        for line in lines_to_show {
            println!("{}", line);
        }
    }

    Ok(())
}

// === Attach Command ===

fn cmd_attach(storage: &Storage, run_id: &str) -> Result<()> {
    let full_run_id = resolve_run_id(storage, run_id)?;
    let run = storage.load_run(&full_run_id)?;

    if run.runtime != "tmux" {
        bail!(
            "Attach is only supported for tmux runtime (current: {})",
            if run.runtime.is_empty() {
                "none"
            } else {
                &run.runtime
            }
        );
    }

    let registry = RuntimeRegistry::new();
    let adapter = registry
        .get(&run.runtime)
        .context(format!("Unknown runtime: {}", run.runtime))?;

    if let Some(ref handle) = run.handle {
        adapter.attach(handle)?;
    } else {
        bail!("Run {} has no handle", run_id);
    }

    Ok(())
}

/// Resolve a short ID or full ID to a full run ID
fn resolve_run_id(storage: &Storage, id: &str) -> Result<String> {
    // If it already starts with "run_", assume it's a full ID
    if id.starts_with("run_") {
        // Verify it exists
        storage.load_run(id)?;
        return Ok(id.to_string());
    }

    // Otherwise, search for a matching short ID
    let runs = storage.list_runs(usize::MAX)?;
    let matches: Vec<_> = runs
        .iter()
        .filter(|r| r.short_id().starts_with(id))
        .collect();

    match matches.len() {
        0 => bail!("No run found matching: {}", id),
        1 => Ok(matches[0].run_id.clone()),
        _ => {
            eprintln!("Multiple runs match '{}':", id);
            for run in &matches {
                eprintln!("  {} ({})", run.short_id(), run.run_id);
            }
            bail!("Ambiguous run ID: {}", id);
        }
    }
}

/// Reconcile run statuses by checking if processes are still alive
/// Uses CAS-style updates with locking: only update if Running
fn reconcile_runs(storage: &Storage) -> Result<()> {
    let runs = storage.list_runs(usize::MAX)?;
    let registry = RuntimeRegistry::new();

    for run in runs {
        // Only reconcile Running runs
        if run.status != RunStatus::Running {
            continue;
        }

        let reason = match &run.handle {
            None => {
                // Running status but no handle
                Some("no runtime handle".to_string())
            }
            Some(handle) => {
                let Some(adapter) = registry.get(&run.runtime) else {
                    continue;
                };

                if !adapter.is_alive(handle) {
                    // Process is dead but status is still Running
                    let reason = match handle {
                        runbox_core::RuntimeHandle::Background { pid, .. } => {
                            format!("process {} not found", pid)
                        }
                        runbox_core::RuntimeHandle::Tmux { session, window } => {
                            format!("tmux window '{}:{}' not found", session, window)
                        }
                        runbox_core::RuntimeHandle::Zellij { session, tab } => {
                            format!("zellij tab '{}:{}' not found", session, tab)
                        }
                    };
                    Some(reason)
                } else {
                    None // Still alive, no update needed
                }
            }
        };

        if let Some(reason) = reason {
            // Use CAS-style save with lock
            let _ = storage.save_run_if_status_with(
                &run.run_id,
                &[RunStatus::Running],
                |current| {
                    current.status = RunStatus::Unknown;
                    current.reconcile_reason = Some(reason.clone());
                    let now = Utc::now();
                    if current.timeline.started_at.is_none() {
                        current.timeline.started_at = Some(now);
                    }
                    if current.timeline.ended_at.is_none() {
                        current.timeline.ended_at = Some(now);
                    }
                }
            );
        }
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

    println!("{:<10} {:<40}", "ID", "NAME");
    println!("{}", "-".repeat(50));
    for t in templates {
        println!("{:<10} {:<40}", short_id(&t.template_id), t.name);
    }

    Ok(())
}

fn cmd_template_show(storage: &Storage, template_id: &str) -> Result<()> {
    let resolved_id = storage.resolve_template_id(template_id)?;
    let template = storage.load_template(&resolved_id)?;
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
    let resolved_id = storage.resolve_template_id(template_id)?;
    storage.delete_template(&resolved_id)?;
    println!("Template deleted: {}", short_id(&resolved_id));
    Ok(())
}

// === Playlist Commands ===

fn cmd_playlist_list(storage: &Storage) -> Result<()> {
    let playlists = storage.list_playlists()?;

    if playlists.is_empty() {
        println!("No playlists found.");
        return Ok(());
    }

    println!("{:<10} {:<30} {:<10}", "ID", "NAME", "ITEMS");
    println!("{}", "-".repeat(50));
    for p in playlists {
        println!("{:<10} {:<30} {:<10}", short_id(&p.playlist_id), p.name, p.items.len());
    }

    Ok(())
}

fn cmd_playlist_show(storage: &Storage, playlist_id: &str) -> Result<()> {
    let resolved_id = storage.resolve_playlist_id(playlist_id)?;
    let playlist = storage.load_playlist(&resolved_id)?;
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
    let resolved_playlist_id = storage.resolve_playlist_id(playlist_id)?;
    let resolved_template_id = storage.resolve_template_id(template_id)?;
    let mut playlist = storage.load_playlist(&resolved_playlist_id)?;
    playlist.items.push(PlaylistItem {
        template_id: resolved_template_id.clone(),
        label,
    });
    storage.save_playlist(&playlist)?;
    println!("Added {} to {}", short_id(&resolved_template_id), short_id(&resolved_playlist_id));
    Ok(())
}

fn cmd_playlist_remove(storage: &Storage, playlist_id: &str, template_id: &str) -> Result<()> {
    let resolved_playlist_id = storage.resolve_playlist_id(playlist_id)?;
    let resolved_template_id = storage.resolve_template_id(template_id)?;
    let mut playlist = storage.load_playlist(&resolved_playlist_id)?;
    let initial_len = playlist.items.len();
    playlist.items.retain(|item| item.template_id != resolved_template_id);

    if playlist.items.len() == initial_len {
        anyhow::bail!("Template {} not found in playlist", short_id(&resolved_template_id));
    }

    storage.save_playlist(&playlist)?;
    println!("Removed {} from {}", short_id(&resolved_template_id), short_id(&resolved_playlist_id));
    Ok(())
}

// === History Commands ===

fn cmd_history(storage: &Storage, limit: usize) -> Result<()> {
    let runs = storage.list_runs(limit)?;

    if runs.is_empty() {
        println!("No runs found.");
        return Ok(());
    }

    println!("{:<10} {:<50}", "ID", "COMMAND");
    println!("{}", "-".repeat(60));
    for run in runs {
        let cmd = run.exec.argv.join(" ");
        let cmd_truncated = if cmd.len() > 50 {
            format!("{}...", &cmd[..47])
        } else {
            cmd
        };
        println!("{:<10} {:<50}", short_id(&run.run_id), cmd_truncated);
    }

    Ok(())
}

fn cmd_show(storage: &Storage, run_id: &str) -> Result<()> {
    let resolved_id = storage.resolve_run_id(run_id)?;
    let run = storage.load_run(&resolved_id)?;

    // Display formatted output
    println!("Run ID:     {}", run.run_id);
    println!("Short ID:   {}", run.short_id());
    println!("Status:     {}", run.status);
    println!("Source:     {}", run.source);
    println!("Runtime:    {}", if run.runtime.is_empty() { "-" } else { &run.runtime });
    println!();
    println!("Command:    {:?}", run.exec.argv);
    println!("Cwd:        {}", run.exec.cwd);
    if !run.exec.env.is_empty() {
        println!("Env:        {:?}", run.exec.env);
    }
    println!();
    println!("Repo:       {}", run.code_state.repo_url);
    println!("Commit:     {}", run.code_state.base_commit);
    if run.code_state.patch.is_some() {
        println!("Patch:      yes");
    }
    println!();
    if let Some(ref timeline) = run.timeline.created_at.as_ref() {
        println!("Created:    {}", timeline);
    }
    if let Some(ref timeline) = run.timeline.started_at.as_ref() {
        println!("Started:    {}", timeline);
    }
    if let Some(ref timeline) = run.timeline.ended_at.as_ref() {
        println!("Ended:      {}", timeline);
    }
    if let Some(exit_code) = run.exit_code {
        println!("Exit Code:  {}", exit_code);
    }
    if let Some(ref reason) = run.reconcile_reason {
        println!("Reconcile:  {}", reason);
    }
    if let Some(ref log_ref) = run.log_ref {
        println!("Log:        {}", log_ref.path.display());
    }

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
    // Resolve short ID to full ID
    let resolved_id = storage.resolve_run_id(run_id)?;
    let run = storage.load_run(&resolved_id)?;

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
    println!("Replaying: {}", resolved_id);
    println!("Command: {:?}", run.exec.argv);
    println!("Commit: {}", run.code_state.base_commit);
    if run.code_state.patch.is_some() {
        println!("Patch: yes");
    }

    // Restore code state in worktree
    let worktree_result = git.restore_code_state_in_worktree(
        &run.code_state,
        &resolved_id,
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
