use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use dialoguer::{theme::ColorfulTheme, Input};
use runbox_core::{
    BindingResolver, ConfigResolver, GitContext, LogRef, Playlist, PlaylistItem, Run, RunStatus,
    RunTemplate, Runtime, Storage, Validator, VerboseLogger,
};
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Runtime argument for CLI
#[derive(Debug, Clone, Copy, ValueEnum)]
enum RuntimeArg {
    Bg,
    Tmux,
    Zellij,
    Foreground,
}

impl From<RuntimeArg> for Runtime {
    fn from(arg: RuntimeArg) -> Self {
        match arg {
            RuntimeArg::Bg => Runtime::Background,
            RuntimeArg::Tmux => Runtime::Tmux,
            RuntimeArg::Zellij => Runtime::Zellij,
            RuntimeArg::Foreground => Runtime::Foreground,
        }
    }
}

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

        /// Runtime environment (bg, tmux, zellij, foreground)
        #[arg(long, value_enum, default_value = "foreground")]
        runtime: RuntimeArg,
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

    /// List running and recent runs
    Ps {
        /// Filter by status (running, pending, exited, failed, killed)
        #[arg(long)]
        status: Option<String>,

        /// Limit number of results
        #[arg(short, long, default_value = "10")]
        limit: usize,
    },

    /// Show logs for a run
    Logs {
        /// Run ID
        run_id: String,

        /// Follow log output (tail -f style)
        #[arg(short, long)]
        follow: bool,

        /// Number of lines to show from the end
        #[arg(short = 'n', long, default_value = "100")]
        lines: usize,
    },

    /// Stop a running run
    Stop {
        /// Run ID
        run_id: String,
    },

    /// Attach to a running run's session (tmux/zellij)
    Attach {
        /// Run ID
        run_id: String,
    },

    /// Internal: Called when a run exits (do not use directly)
    #[command(name = "_on-exit", hide = true)]
    OnExit {
        /// Run ID
        run_id: String,

        /// Exit code
        exit_code: i32,
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
        } => cmd_run(&storage, &template, binding, dry_run, runtime.into()),
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
        Commands::Ps { status, limit } => cmd_ps(&storage, status, limit),
        Commands::Logs {
            run_id,
            follow,
            lines,
        } => cmd_logs(&storage, &run_id, follow, lines),
        Commands::Stop { run_id } => cmd_stop(&storage, &run_id),
        Commands::Attach { run_id } => cmd_attach(&storage, &run_id),
        Commands::OnExit { run_id, exit_code } => cmd_on_exit(&storage, &run_id, exit_code),
    }
}

// === Run Command ===

fn cmd_run(
    storage: &Storage,
    template_id: &str,
    bindings: Vec<String>,
    dry_run: bool,
    runtime: Runtime,
) -> Result<()> {
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
    let mut run = resolver.build_run(&template, code_state)?;

    // Set runtime and log path
    run.runtime = runtime.clone();
    let log_path = storage.log_path(&run.run_id);
    run.log_ref = Some(LogRef {
        path: log_path.clone(),
    });

    // Validate
    run.validate()?;

    if dry_run {
        println!("Dry run - would execute:");
        println!("{}", serde_json::to_string_pretty(&run)?);
        return Ok(());
    }

    // Save initial run state
    storage.save_run(&run)?;
    println!("Run created: {}", run.run_id);

    match runtime {
        Runtime::Foreground => execute_foreground(storage, &mut run)?,
        Runtime::Background => execute_background(storage, &mut run, &log_path)?,
        Runtime::Tmux => execute_tmux(storage, &mut run, &log_path)?,
        Runtime::Zellij => execute_zellij(storage, &mut run, &log_path)?,
    }

    Ok(())
}

/// Execute command in foreground (blocking)
fn execute_foreground(storage: &Storage, run: &mut Run) -> Result<()> {
    // Mark as started
    run.mark_started();
    storage.save_run_atomic(run)?;

    println!("\nExecuting: {:?}", run.exec.argv);

    // Create log file and execute with tee-like behavior
    let log_path = run.log_ref.as_ref().map(|l| &l.path);

    let mut child = Command::new(&run.exec.argv[0])
        .args(&run.exec.argv[1..])
        .current_dir(&run.exec.cwd)
        .envs(&run.exec.env)
        .stdout(if log_path.is_some() {
            Stdio::piped()
        } else {
            Stdio::inherit()
        })
        .stderr(if log_path.is_some() {
            Stdio::piped()
        } else {
            Stdio::inherit()
        })
        .spawn()
        .context("Failed to execute command")?;

    // If we have a log path, tee output to both console and file
    if let Some(path) = log_path {
        let mut log_file = File::create(path)?;

        // Simple approach: wait for completion and capture output
        let output = child.wait_with_output()?;

        use std::io::Write;
        // Write to log file
        log_file.write_all(&output.stdout)?;
        log_file.write_all(&output.stderr)?;

        // Also print to console
        std::io::stdout().write_all(&output.stdout)?;
        std::io::stderr().write_all(&output.stderr)?;

        let exit_code = output.status.code().unwrap_or(-1);
        run.mark_exited(exit_code);
        storage.save_run_atomic(run)?;

        if output.status.success() {
            println!("\nRun completed successfully: {}", run.run_id);
        } else {
            println!("\nRun failed with exit code: {}", exit_code);
        }
    } else {
        let status = child.wait()?;
        let exit_code = status.code().unwrap_or(-1);
        run.mark_exited(exit_code);
        storage.save_run_atomic(run)?;

        if status.success() {
            println!("\nRun completed successfully: {}", run.run_id);
        } else {
            println!("\nRun failed with exit code: {}", exit_code);
        }
    }

    Ok(())
}

/// Execute command in background
fn execute_background(storage: &Storage, run: &mut Run, log_path: &PathBuf) -> Result<()> {
    // Create log file
    let log_file = File::create(log_path)?;

    // Build the shell command that will run the actual command and then call _on-exit
    let cmd_str = run.exec.argv.join(" ");

    // Get the path to the runbox executable
    let runbox_exe = std::env::current_exe()?;

    // Create a shell script that runs the command and then calls _on-exit
    let script = format!(
        "cd {} && {} 2>&1; {} _on-exit {} $?",
        shell_escape(&run.exec.cwd),
        cmd_str,
        runbox_exe.display(),
        run.run_id
    );

    let child = Command::new("sh")
        .args(["-c", &script])
        .stdout(log_file.try_clone()?)
        .stderr(log_file)
        .envs(&run.exec.env)
        .spawn()
        .context("Failed to spawn background process")?;

    run.pid = Some(child.id());
    run.mark_started();
    storage.save_run_atomic(run)?;

    println!("Started in background (PID: {})", child.id());
    println!("Log: {}", log_path.display());
    println!("Use 'runbox logs -f {}' to follow output", run.run_id);

    Ok(())
}

/// Execute command in tmux session
fn execute_tmux(storage: &Storage, run: &mut Run, log_path: &PathBuf) -> Result<()> {
    // Check if tmux is available
    let tmux_check = Command::new("tmux").arg("-V").output();
    if tmux_check.is_err() {
        bail!("tmux is not installed or not in PATH");
    }

    // Ensure session exists
    let session_name = "runbox";
    let has_session = Command::new("tmux")
        .args(["has-session", "-t", session_name])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if !has_session {
        Command::new("tmux")
            .args(["new-session", "-d", "-s", session_name])
            .status()
            .context("Failed to create tmux session")?;
    }

    // Build the command with logging and exit callback
    let cmd_str = run.exec.argv.join(" ");
    let runbox_exe = std::env::current_exe()?;

    let script = format!(
        "cd {} && {} 2>&1 | tee {}; {} _on-exit {} ${{PIPESTATUS[0]}}",
        shell_escape(&run.exec.cwd),
        cmd_str,
        log_path.display(),
        runbox_exe.display(),
        run.run_id
    );

    // Create a new window in the session
    let window_name = run.run_id.clone();
    Command::new("tmux")
        .args([
            "new-window",
            "-t",
            session_name,
            "-n",
            &window_name,
            "sh",
            "-c",
            &script,
        ])
        .envs(&run.exec.env)
        .status()
        .context("Failed to create tmux window")?;

    // Record session reference
    run.session_ref = Some(format!("tmux:session={};window={}", session_name, window_name));
    run.mark_started();
    storage.save_run_atomic(run)?;

    println!("Started in tmux session '{}'", session_name);
    println!("Window: {}", window_name);
    println!("Log: {}", log_path.display());
    println!("Use 'runbox attach {}' to attach", run.run_id);
    println!("Use 'runbox logs -f {}' to follow output", run.run_id);

    Ok(())
}

/// Execute command in zellij session
fn execute_zellij(storage: &Storage, run: &mut Run, log_path: &PathBuf) -> Result<()> {
    // Check if zellij is available
    let zellij_check = Command::new("zellij").arg("--version").output();
    if zellij_check.is_err() {
        bail!("zellij is not installed or not in PATH");
    }

    // Build the command with logging and exit callback
    let cmd_str = run.exec.argv.join(" ");
    let runbox_exe = std::env::current_exe()?;

    let script = format!(
        "cd {} && {} 2>&1 | tee {}; {} _on-exit {} ${{PIPESTATUS[0]}}",
        shell_escape(&run.exec.cwd),
        cmd_str,
        log_path.display(),
        runbox_exe.display(),
        run.run_id
    );

    // Try to attach to existing session or create new one
    let session_name = "runbox";

    // Check if session exists
    let list_output = Command::new("zellij")
        .args(["list-sessions"])
        .output()
        .context("Failed to list zellij sessions")?;

    let sessions = String::from_utf8_lossy(&list_output.stdout);
    let has_session = sessions.lines().any(|line| line.trim().starts_with(session_name));

    if has_session {
        // Run in existing session
        Command::new("zellij")
            .args(["-s", session_name, "action", "new-tab", "-n", &run.run_id, "--", "sh", "-c", &script])
            .envs(&run.exec.env)
            .status()
            .context("Failed to create zellij tab")?;
    } else {
        // Create new detached session with the command
        Command::new("zellij")
            .args(["-s", session_name, "--", "sh", "-c", &script])
            .envs(&run.exec.env)
            .spawn()
            .context("Failed to create zellij session")?;
    }

    // Record session reference
    run.session_ref = Some(format!("zellij:session={};tab={}", session_name, run.run_id));
    run.mark_started();
    storage.save_run_atomic(run)?;

    println!("Started in zellij session '{}'", session_name);
    println!("Tab: {}", run.run_id);
    println!("Log: {}", log_path.display());
    println!("Use 'runbox attach {}' to attach", run.run_id);
    println!("Use 'runbox logs -f {}' to follow output", run.run_id);

    Ok(())
}

/// Escape a string for shell use
fn shell_escape(s: &str) -> String {
    if s.contains(char::is_whitespace) || s.contains('\'') || s.contains('"') {
        format!("'{}'", s.replace('\'', "'\\''"))
    } else {
        s.to_string()
    }
}

// === Ps Command ===

fn cmd_ps(storage: &Storage, status_filter: Option<String>, limit: usize) -> Result<()> {
    let status = if let Some(s) = status_filter {
        Some(match s.to_lowercase().as_str() {
            "pending" => RunStatus::Pending,
            "running" => RunStatus::Running,
            "exited" => RunStatus::Exited,
            "failed" => RunStatus::Failed,
            "killed" => RunStatus::Killed,
            _ => bail!("Invalid status: {}. Valid values: pending, running, exited, failed, killed", s),
        })
    } else {
        None
    };

    let runs = storage.list_runs_by_status(status.as_ref(), limit)?;

    if runs.is_empty() {
        println!("No runs found.");
        return Ok(());
    }

    println!(
        "{:<45} {:<10} {:<10} {:<10}",
        "RUN ID", "STATUS", "RUNTIME", "EXIT"
    );
    println!("{}", "-".repeat(80));

    for run in runs {
        let exit_str = run
            .exit_code
            .map(|c| c.to_string())
            .unwrap_or_else(|| "-".to_string());
        println!(
            "{:<45} {:<10} {:<10} {:<10}",
            run.run_id,
            run.status.to_string(),
            run.runtime.to_string(),
            exit_str
        );
    }

    Ok(())
}

// === Logs Command ===

fn cmd_logs(storage: &Storage, run_id: &str, follow: bool, lines: usize) -> Result<()> {
    let run = storage.load_run(run_id)?;

    let log_path = run
        .log_ref
        .as_ref()
        .map(|l| &l.path)
        .ok_or_else(|| anyhow::anyhow!("No log file for run {}", run_id))?;

    if !log_path.exists() {
        bail!("Log file not found: {}", log_path.display());
    }

    if follow {
        // Tail -f style following
        let mut file = File::open(log_path)?;
        let mut pos = file.seek(SeekFrom::End(0))?;

        // First, show the last N lines
        let content = std::fs::read_to_string(log_path)?;
        let all_lines: Vec<&str> = content.lines().collect();
        let start = if all_lines.len() > lines {
            all_lines.len() - lines
        } else {
            0
        };
        for line in &all_lines[start..] {
            println!("{}", line);
        }

        // Check if the run is still running
        if run.is_completed() {
            println!("\n--- Run has completed (exit code: {:?}) ---", run.exit_code);
            return Ok(());
        }

        // Now follow new content
        println!("\n--- Following log output (Ctrl+C to stop) ---");
        loop {
            file.seek(SeekFrom::Start(pos))?;
            let mut reader = BufReader::new(&file);
            let mut line = String::new();

            loop {
                match reader.read_line(&mut line) {
                    Ok(0) => break, // No more data
                    Ok(_) => {
                        print!("{}", line);
                        line.clear();
                    }
                    Err(e) => {
                        eprintln!("Error reading log: {}", e);
                        break;
                    }
                }
            }

            pos = file.seek(SeekFrom::Current(0))?;

            // Check if run is still active
            let current_run = storage.load_run(run_id)?;
            if current_run.is_completed() {
                println!("\n--- Run completed (exit code: {:?}) ---", current_run.exit_code);
                break;
            }

            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    } else {
        // Just show the last N lines
        let content = std::fs::read_to_string(log_path)?;
        let all_lines: Vec<&str> = content.lines().collect();
        let start = if all_lines.len() > lines {
            all_lines.len() - lines
        } else {
            0
        };
        for line in &all_lines[start..] {
            println!("{}", line);
        }
    }

    Ok(())
}

// === Stop Command ===

fn cmd_stop(storage: &Storage, run_id: &str) -> Result<()> {
    let run = storage.load_run(run_id)?;

    if !run.is_running() {
        bail!("Run {} is not running (status: {})", run_id, run.status);
    }

    match &run.session_ref {
        Some(session_ref) => {
            let (runtime, params) = parse_session_ref(session_ref)?;
            match runtime {
                "tmux" => {
                    let session = params
                        .get("session")
                        .ok_or_else(|| anyhow::anyhow!("Missing session in session_ref"))?;
                    let window = params
                        .get("window")
                        .ok_or_else(|| anyhow::anyhow!("Missing window in session_ref"))?;

                    Command::new("tmux")
                        .args(["kill-window", "-t", &format!("{}:{}", session, window)])
                        .status()
                        .context("Failed to kill tmux window")?;
                }
                "zellij" => {
                    // Zellij doesn't have a simple way to kill a specific tab
                    // We'll try to send a kill signal to the process
                    if let Some(pid) = run.pid {
                        Command::new("kill")
                            .arg(pid.to_string())
                            .status()
                            .context("Failed to kill process")?;
                    } else {
                        bail!("Cannot stop zellij run without PID");
                    }
                }
                _ => bail!("Unknown runtime in session_ref: {}", runtime),
            }
        }
        None => {
            // Background process - kill by PID
            if let Some(pid) = run.pid {
                Command::new("kill")
                    .arg(pid.to_string())
                    .status()
                    .context("Failed to kill process")?;
            } else {
                bail!("No PID recorded for run {}", run_id);
            }
        }
    }

    // Update status
    storage.update_run(run_id, |r| r.mark_killed())?;
    println!("Stopped run: {}", run_id);

    Ok(())
}

// === Attach Command ===

fn cmd_attach(storage: &Storage, run_id: &str) -> Result<()> {
    let run = storage.load_run(run_id)?;

    let session_ref = run
        .session_ref
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Run {} has no session to attach to", run_id))?;

    let (runtime, params) = parse_session_ref(session_ref)?;

    match runtime {
        "tmux" => {
            let session = params
                .get("session")
                .ok_or_else(|| anyhow::anyhow!("Missing session in session_ref"))?;
            let window = params.get("window");

            // Select window if specified
            if let Some(w) = window {
                Command::new("tmux")
                    .args(["select-window", "-t", &format!("{}:{}", session, w)])
                    .status()
                    .context("Failed to select tmux window")?;
            }

            // Attach or switch client
            if std::env::var("TMUX").is_ok() {
                // Already in tmux - switch client
                let status = Command::new("tmux")
                    .args(["switch-client", "-t", session])
                    .status()
                    .context("Failed to switch tmux client")?;
                if !status.success() {
                    bail!("Failed to switch to tmux session");
                }
            } else {
                // Not in tmux - attach
                let status = Command::new("tmux")
                    .args(["attach", "-t", session])
                    .status()
                    .context("Failed to attach to tmux session")?;
                if !status.success() {
                    bail!("Failed to attach to tmux session");
                }
            }
        }
        "zellij" => {
            let session = params
                .get("session")
                .ok_or_else(|| anyhow::anyhow!("Missing session in session_ref"))?;

            let status = Command::new("zellij")
                .args(["attach", session])
                .status()
                .context("Failed to attach to zellij session")?;
            if !status.success() {
                bail!("Failed to attach to zellij session");
            }
        }
        _ => bail!("Unknown runtime: {}", runtime),
    }

    Ok(())
}

// === OnExit Command (Internal) ===

fn cmd_on_exit(storage: &Storage, run_id: &str, exit_code: i32) -> Result<()> {
    storage.update_run(run_id, |run| {
        run.mark_exited(exit_code);
    })?;

    Ok(())
}

/// Parse session_ref format: "runtime:key1=value1;key2=value2"
fn parse_session_ref(session_ref: &str) -> Result<(&str, std::collections::HashMap<&str, &str>)> {
    let parts: Vec<&str> = session_ref.splitn(2, ':').collect();
    if parts.len() != 2 {
        bail!("Invalid session_ref format: {}", session_ref);
    }

    let runtime = parts[0];
    let mut params = std::collections::HashMap::new();

    for pair in parts[1].split(';') {
        let kv: Vec<&str> = pair.splitn(2, '=').collect();
        if kv.len() == 2 {
            params.insert(kv[0], kv[1]);
        }
    }

    Ok((runtime, params))
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
