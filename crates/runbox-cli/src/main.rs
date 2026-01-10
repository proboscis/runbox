use anyhow::{bail, Context, Result};
use chrono::Utc;
use clap::{Parser, Subcommand, ValueEnum};
use dialoguer::{theme::ColorfulTheme, Input};
use runbox_core::{
    default_pid_path, default_socket_path, short_id, BindingResolver, ConfigResolver, DaemonClient,
    GitContext, LogRef, Playlist, PlaylistItem, RunStatus, RunTemplate, RuntimeRegistry, Storage,
    Timeline, Validator, VerboseLogger,
};
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;

/// Tutorial content embedded at compile time
const TUTORIAL: &str = include_str!("../../../docs/tutorial.md");

#[derive(Parser)]
#[command(name = "runbox")]
#[command(about = "Reproducible command execution system")]
#[command(
    long_about = "Runbox captures command executions with full git context, stores them for \
later reference, and allows you to replay them in isolated git worktrees \
with the exact same code state."
)]
#[command(after_help = "\
QUICK START:
  # Execute a command directly (captures git context)
  runbox run -- echo 'Hello, World!'
  runbox run -- python train.py --epochs 10

  # Check running processes
  runbox ps

  # View logs
  runbox logs <run_id>

  # Replay a previous run with exact code state
  runbox replay <run_id>

TEMPLATE-BASED EXECUTION:
  # Run from a pre-defined template
  runbox run --template tpl_train_model --binding epochs=100

  # List available templates
  runbox template list

LEARN MORE:
  runbox tutorial       Show the full interactive tutorial
  runbox <command> -h   Show help for a specific command
  
DOCUMENTATION:
  https://github.com/your-org/runbox")]
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
    #[command(after_help = "\
EXAMPLES:
  # Direct execution (everything after -- is the command)
  runbox run -- echo 'Hello, World!'
  runbox run -- python train.py --epochs 10
  runbox run -- make test

  # With options for direct execution
  runbox run --runtime tmux -- python debug.py
  runbox run --timeout 3600 -- ./long_job.sh
  runbox run --env CUDA_VISIBLE_DEVICES=0 -- python train.py
  runbox run --cwd /path/to/project -- npm test
  runbox run --no-git -- echo 'skip git capture'
  runbox run --dry-run -- python train.py

  # Template-based execution
  runbox run --template tpl_train_model
  runbox run --template tpl_train_model --binding epochs=100
  runbox run --template tpl_hello --binding name=World --runtime tmux

RELATED COMMANDS:
  runbox log       Alias for direct execution (runbox log -- <cmd>)
  runbox ps        List runs to check status
  runbox logs      View stdout/stderr from a run
  runbox template  Manage templates")]
    Run {
        /// Template ID (for template-based execution)
        #[arg(short, long)]
        template: Option<String>,

        /// Variable bindings (key=value) - only for template mode
        #[arg(short, long)]
        binding: Vec<String>,

        /// Runtime environment (bg, background, tmux)
        #[arg(long, default_value = "bg")]
        runtime: RuntimeType,

        /// Skip execution (dry run)
        #[arg(long)]
        dry_run: bool,

        /// Command timeout in seconds (0 = no timeout) - only for direct mode
        #[arg(long, default_value = "0")]
        timeout: u64,

        /// Additional environment variables (KEY=VALUE) - only for direct mode
        #[arg(long = "env", value_name = "KEY=VALUE")]
        env_vars: Vec<String>,

        /// Working directory - only for direct mode (default: current directory)
        #[arg(long)]
        cwd: Option<String>,

        /// Skip git context capture - only for direct mode
        #[arg(long)]
        no_git: bool,

        /// Command to execute directly (everything after --)
        #[arg(last = true, value_name = "COMMAND")]
        command: Vec<String>,
    },

    /// Log and execute a command directly (alias for 'run --')
    #[command(after_help = "\
EXAMPLES:
  # Execute and log a command
  runbox log -- echo 'Hello, World!'
  runbox log -- python train.py --epochs 10
  runbox log -- make test
  runbox log -- npm run build

  # With options
  runbox log --runtime tmux -- python debug.py
  runbox log --timeout 3600 -- ./long_job.sh
  runbox log --env KEY=value -- ./script.sh
  runbox log --cwd /path/to/project -- npm test
  runbox log --no-git -- echo 'skip git capture'
  runbox log --dry-run -- python train.py

RELATED COMMANDS:
  runbox run       Full run command with template support
  runbox ps        List runs to check status
  runbox logs      View stdout/stderr from a run")]
    Log {
        /// Runtime environment (bg, background, tmux)
        #[arg(long, default_value = "bg")]
        runtime: RuntimeType,

        /// Skip execution (dry run)
        #[arg(long)]
        dry_run: bool,

        /// Command timeout in seconds (0 = no timeout)
        #[arg(long, default_value = "0")]
        timeout: u64,

        /// Additional environment variables (KEY=VALUE)
        #[arg(long = "env", value_name = "KEY=VALUE")]
        env_vars: Vec<String>,

        /// Working directory (default: current directory)
        #[arg(long)]
        cwd: Option<String>,

        /// Skip git context capture
        #[arg(long)]
        no_git: bool,

        /// Command to execute (everything after --)
        #[arg(last = true, required = true, value_name = "COMMAND")]
        command: Vec<String>,
    },

    /// List running and recent runs
    #[command(after_help = "\
EXAMPLES:
  # List recent runs (default: last 20)
  runbox ps

  # Filter by status
  runbox ps --status running
  runbox ps --status exited
  runbox ps --status failed

  # Limit number of results
  runbox ps --limit 5
  runbox ps -l 10

  # Show all runs
  runbox ps --all

OUTPUT:
  SHORT ID     STATUS     RUNTIME    COMMAND
  ----------------------------------------------------------------
  550e8400     running    tmux       python train.py --epochs 10
  6ba7b810     exited     background echo Hello, World!

RELATED COMMANDS:
  runbox logs <id>   View logs for a specific run
  runbox show <id>   Show detailed run information
  runbox stop <id>   Stop a running process")]
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
    #[command(after_help = "\
EXAMPLES:
  # Graceful stop (sends SIGTERM)
  runbox stop 550e8400

  # Force kill (sends SIGKILL)
  runbox stop 550e8400 --force
  runbox stop 550e8400 -f

  # Using full run ID
  runbox stop run_550e8400-e29b-41d4-a716-446655440000

NOTES:
  - Short IDs (first 8 characters) can be used instead of full run IDs
  - Graceful stop (SIGTERM) allows the process to clean up
  - Force stop (SIGKILL) immediately terminates the process

RELATED COMMANDS:
  runbox ps          List runs to find run IDs
  runbox logs <id>   Check output before stopping")]
    Stop {
        /// Run ID (or short ID prefix, e.g., '550e8400')
        run_id: String,

        /// Force kill (SIGKILL instead of SIGTERM)
        #[arg(long, short)]
        force: bool,
    },

    /// Show stdout/stderr logs for a run
    #[command(after_help = "\
EXAMPLES:
  # View all logs for a run
  runbox logs 550e8400

  # Follow logs in real-time (like tail -f)
  runbox logs 550e8400 --follow
  runbox logs 550e8400 -f

  # Show last N lines only
  runbox logs 550e8400 --lines 50
  runbox logs 550e8400 -l 100

OUTPUT:
  [stdout/stderr content from the run]
  Training started...
  Epoch 1/10: loss=0.5
  Epoch 2/10: loss=0.3
  ...

NOTES:
  - Logs are captured from stdout and stderr combined
  - Use --follow for running processes to see live output
  - Press Ctrl+C to stop following

RELATED COMMANDS:
  runbox ps        List runs to find run IDs
  runbox show      Show run metadata including log file path
  runbox attach    Attach to tmux session (for tmux runtime)")]
    Logs {
        /// Run ID (or short ID prefix, e.g., '550e8400')
        run_id: String,

        /// Follow log output in real-time (like tail -f)
        #[arg(short, long)]
        follow: bool,

        /// Show last N lines only (default: all)
        #[arg(short, long)]
        lines: Option<usize>,
    },

    /// Attach to a running tmux session for interactive access
    #[command(after_help = "\
EXAMPLES:
  # Attach to a tmux-based run
  runbox attach 550e8400

  # Using full run ID
  runbox attach run_550e8400-e29b-41d4-a716-446655440000

NOTES:
  - Only works for runs started with --runtime tmux
  - Use Ctrl+B, D to detach from the tmux session
  - The process continues running after detaching

RELATED COMMANDS:
  runbox ps               List runs to find run IDs
  runbox logs <id>        View logs (for background runs)
  runbox run --runtime tmux  Start a new run in tmux")]
    Attach {
        /// Run ID (or short ID prefix, e.g., '550e8400')
        run_id: String,
    },

    /// Manage run templates (create, list, show, delete)
    #[command(after_help = "\
EXAMPLES:
  # List all templates
  runbox template list

  # Show template details
  runbox template show tpl_hello
  runbox template show hello    # short ID works too

  # Create a new template from JSON file
  runbox template create my_template.json

  # Delete a template
  runbox template delete tpl_hello

TEMPLATE JSON FORMAT:
  {
    \"template_id\": \"tpl_hello\",
    \"name\": \"Hello World\",
    \"exec\": {
      \"argv\": [\"echo\", \"Hello, {name}!\"],
      \"cwd\": \".\",
      \"env\": {},
      \"timeout_sec\": 60
    },
    \"bindings\": {
      \"defaults\": { \"name\": \"World\" },
      \"interactive\": []
    }
  }

RELATED COMMANDS:
  runbox run --template <id>   Execute a template
  runbox validate              Validate a template JSON file")]
    Template {
        #[command(subcommand)]
        command: TemplateCommands,
    },

    /// Manage playlists (collections of templates)
    #[command(after_help = "\
EXAMPLES:
  # List all playlists
  runbox playlist list

  # Show playlist contents
  runbox playlist show pl_daily

  # Create a playlist from JSON file
  runbox playlist create my_playlist.json

  # Add a template to a playlist
  runbox playlist add pl_daily tpl_backup --label 'Backup Data'

  # Remove a template from a playlist (by template ID or index)
  runbox playlist remove pl_daily tpl_backup
  runbox playlist remove pl_daily 0    # remove first item

PLAYLIST JSON FORMAT:
  {
    \"playlist_id\": \"pl_daily\",
    \"name\": \"Daily Tasks\",
    \"items\": [
      { \"template_id\": \"tpl_sync\", \"label\": \"Sync Data\" },
      { \"template_id\": \"tpl_train\", \"label\": \"Train Model\" }
    ]
  }

RELATED COMMANDS:
  runbox template list   List available templates to add")]
    Playlist {
        #[command(subcommand)]
        command: PlaylistCommands,
    },

    /// Manage run results (captured execution outputs)
    #[command(after_help = "\
EXAMPLES:
  # List recent results
  runbox result list

  # Show result details
  runbox result show <result_id>

  # Get result for a specific run
  runbox result for-run <run_id>

  # View stdout/stderr
  runbox result stdout <result_id>
  runbox result stderr <result_id>

RELATED COMMANDS:
  runbox show <run_id>   Show run details
  runbox logs <run_id>   View run logs")]
    Result {
        #[command(subcommand)]
        command: ResultCommands,
    },

    /// Show run history (past executions)
    #[command(after_help = "\
EXAMPLES:
  # Show recent run history (default: last 10)
  runbox history

  # Limit number of results
  runbox history --limit 20
  runbox history -l 5
  runbox history -n 50

OUTPUT:
  ID         COMMAND
  ----------------------------------------------------------------
  550e8400   python train.py --epochs 10
  6ba7b810   echo Hello, World!
  7c9e6679   make test

RELATED COMMANDS:
  runbox show <id>    Show detailed run information
  runbox replay <id>  Replay a previous run
  runbox ps           Show running and recent runs with status")]
    History {
        /// Maximum number of runs to show
        #[arg(short, long, short_alias = 'n', default_value = "10")]
        limit: usize,
    },

    /// Show detailed information about a run
    #[command(after_help = "\
EXAMPLES:
  # Show run details
  runbox show 550e8400

  # Using full run ID
  runbox show run_550e8400-e29b-41d4-a716-446655440000

OUTPUT:
  Run ID:     run_550e8400-e29b-41d4-a716-446655440000
  Short ID:   550e8400
  Status:     exited
  Runtime:    background

  Command:    [\"python\", \"train.py\", \"--epochs\", \"10\"]
  Cwd:        .

  Repo:       git@github.com:org/repo.git
  Commit:     abc123def456...
  Patch:      yes

  Created:    2025-01-10T10:30:00Z
  Started:    2025-01-10T10:30:01Z
  Ended:      2025-01-10T11:45:30Z
  Exit Code:  0
  Log:        /home/user/.local/share/runbox/logs/run_550e8400...

RELATED COMMANDS:
  runbox ps           List runs to find run IDs
  runbox logs <id>    View stdout/stderr logs
  runbox replay <id>  Replay the run with same code state")]
    Show {
        /// Run ID (or short ID prefix, e.g., '550e8400')
        run_id: String,
    },

    /// Replay a previous run in an isolated git worktree with exact code state
    #[command(after_help = "\
EXAMPLES:
  # Basic replay
  runbox replay 550e8400

  # Specify worktree directory
  runbox replay 550e8400 --worktree-dir /tmp/replay

  # Clean up worktree after execution
  runbox replay 550e8400 --cleanup

  # Keep worktree after execution (default)
  runbox replay 550e8400 --keep

  # Always create fresh worktree (don't reuse existing)
  runbox replay 550e8400 --fresh

  # Reuse existing worktree if commit matches (default)
  runbox replay 550e8400 --reuse

  # Verbose output levels
  runbox replay 550e8400 -v      # Level 1: basic info
  runbox replay 550e8400 -vv     # Level 2: detailed info
  runbox replay 550e8400 -vvv    # Level 3: debug info

HOW IT WORKS:
  1. Creates a git worktree at the original commit
  2. Applies any uncommitted changes (patch) if present
  3. Executes the same command in that environment

CONFIGURATION:
  Set defaults in .git/config or ~/.gitconfig:
    git config runbox.worktree-dir /path/to/worktrees
    git config runbox.cleanup false
    git config runbox.reuse true

RELATED COMMANDS:
  runbox show <id>     View run details including code state
  runbox history       List past runs to replay")]
    Replay {
        /// Run ID (or short ID prefix, e.g., '550e8400')
        run_id: String,

        /// Directory to create worktree in
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

        /// Always create fresh worktree (ignore existing)
        #[arg(long, conflicts_with = "reuse")]
        fresh: bool,

        /// Increase verbosity (-v, -vv, -vvv)
        #[arg(short, long, action = clap::ArgAction::Count)]
        verbose: u8,
    },

    /// Validate a template, run, or playlist JSON file
    #[command(after_help = "\
EXAMPLES:
  # Validate a template file
  runbox validate my_template.json

  # Validate a playlist file
  runbox validate my_playlist.json

  # Validate a run file
  runbox validate run_record.json

AUTO-DETECTION:
  The file type is auto-detected based on the ID field prefix:
    - 'tpl_' prefix  -> Template
    - 'run_' prefix  -> Run
    - 'pl_' prefix   -> Playlist

OUTPUT:
  Valid template file: my_template.json
  
  # On error:
  Error: Invalid template: missing required field 'exec'

RELATED COMMANDS:
  runbox template create   Create a new template from validated JSON")]
    Validate {
        /// Path to JSON file (template, run, or playlist)
        path: String,
    },

    /// Manage the background daemon process
    #[command(after_help = "\
EXAMPLES:
  # Check daemon status
  runbox daemon status

  # Start daemon in foreground (for debugging)
  runbox daemon start

  # Stop the running daemon
  runbox daemon stop

  # Ping the daemon to verify it's responding
  runbox daemon ping

ABOUT THE DAEMON:
  The daemon tracks background processes and captures their exit status.
  It starts automatically when running commands with 'background' runtime.

TROUBLESHOOTING:
  If runs show 'unknown' status, the daemon may not be running:
    runbox daemon status
    runbox daemon ping

RELATED COMMANDS:
  runbox ps     List runs (daemon tracks their status)
  runbox stop   Stop a running process")]
    Daemon {
        #[command(subcommand)]
        command: DaemonCommands,
    },

    /// Display the full tutorial in the terminal
    #[command(after_help = "\
EXAMPLES:
  # Show the complete tutorial
  runbox tutorial

  # Pipe to a pager for easier reading
  runbox tutorial | less

CONTENTS:
  - Installation
  - Quick Start
  - Direct Execution
  - Core Concepts (Run, Template, Playlist)
  - Templates
  - Running Commands
  - Monitoring and Logs
  - Playlists
  - Replay
  - Configuration
  - Troubleshooting
  - Examples")]
    Tutorial,
}

#[derive(Subcommand)]
enum DaemonCommands {
    /// Start the daemon in foreground mode (for debugging)
    #[command(after_help = "\
EXAMPLES:
  runbox daemon start

NOTE: Press Ctrl+C to stop the daemon when running in foreground.
For normal operation, the daemon starts automatically.")]
    Start,

    /// Stop the running daemon gracefully
    #[command(after_help = "\
EXAMPLES:
  runbox daemon stop")]
    Stop,

    /// Check if the daemon is running and show connection info
    #[command(after_help = "\
EXAMPLES:
  runbox daemon status

OUTPUT:
  Socket path: /tmp/runbox-daemon.sock
  PID file:    /tmp/runbox-daemon.pid
  PID:         12345
  Status:      running")]
    Status,

    /// Ping the daemon to verify it's responding
    #[command(after_help = "\
EXAMPLES:
  runbox daemon ping

OUTPUT:
  Pinging daemon...
  Daemon is alive (pong received)")]
    Ping,
}

#[derive(Subcommand)]
enum TemplateCommands {
    /// List all registered templates
    #[command(after_help = "\
EXAMPLES:
  runbox template list

OUTPUT:
  ID         NAME
  ----------------------------------------------------------------
  tpl_hello  Hello World
  tpl_train  Train ML Model")]
    List,

    /// Show template details as JSON
    #[command(after_help = "\
EXAMPLES:
  runbox template show tpl_hello
  runbox template show hello    # short ID prefix works")]
    Show {
        /// Template ID (or short ID prefix)
        template_id: String,
    },

    /// Register a new template from a JSON file
    #[command(after_help = "\
EXAMPLES:
  runbox template create my_template.json

NOTE: Use 'runbox validate' to check the JSON before creating.")]
    Create {
        /// Path to template JSON file
        path: String,
    },

    /// Delete a registered template
    #[command(after_help = "\
EXAMPLES:
  runbox template delete tpl_hello
  runbox template delete hello    # short ID prefix works")]
    Delete {
        /// Template ID (or short ID prefix)
        template_id: String,
    },
}

#[derive(Subcommand)]
enum PlaylistCommands {
    /// List all registered playlists
    #[command(after_help = "\
EXAMPLES:
  runbox playlist list

OUTPUT:
  ID         NAME                           ITEMS
  ----------------------------------------------------------------
  pl_daily   Daily Tasks                    3")]
    List,

    /// Show playlist details as JSON
    #[command(after_help = "\
EXAMPLES:
  runbox playlist show pl_daily
  runbox playlist show daily    # short ID prefix works")]
    Show {
        /// Playlist ID (or short ID prefix)
        playlist_id: String,
    },

    /// Register a new playlist from a JSON file
    #[command(after_help = "\
EXAMPLES:
  runbox playlist create my_playlist.json

NOTE: Use 'runbox validate' to check the JSON before creating.")]
    Create {
        /// Path to playlist JSON file
        path: String,
    },

    /// Add a template to a playlist
    #[command(after_help = "\
EXAMPLES:
  # Add with auto-generated label
  runbox playlist add pl_daily tpl_backup

  # Add with custom label
  runbox playlist add pl_daily tpl_backup --label 'Backup Data'")]
    Add {
        /// Playlist ID (or short ID prefix)
        playlist_id: String,
        /// Template ID to add (or short ID prefix)
        template_id: String,
        /// Display label for this item in the playlist
        #[arg(short, long)]
        label: Option<String>,
    },

    /// Remove a template from a playlist by ID or index
    #[command(after_help = "\
EXAMPLES:
  # Remove by template ID
  runbox playlist remove pl_daily tpl_backup

  # Remove by index (0-based)
  runbox playlist remove pl_daily 0    # remove first item
  runbox playlist remove pl_daily 2    # remove third item")]
    Remove {
        /// Playlist ID (or short ID prefix)
        playlist_id: String,
        /// Template ID or index (0-based) to remove
        template_or_index: String,
    },
}

#[derive(Subcommand)]
enum ResultCommands {
    List {
        #[arg(short, long, default_value = "20")]
        limit: usize,
    },
    Show {
        result_id: String,
    },
    ForRun {
        run_id: String,
    },
    Stdout {
        result_id: String,
    },
    Stderr {
        result_id: String,
    },
    Delete {
        result_id: String,
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
            dry_run,
            timeout,
            env_vars,
            cwd,
            no_git,
            command,
        } => {
            if let Some(tpl_id) = template {
                cmd_run_template(&storage, &tpl_id, binding, runtime, dry_run)
            } else if !command.is_empty() {
                cmd_run_direct(
                    &storage, command, runtime, dry_run, timeout, env_vars, cwd, no_git,
                )
            } else {
                anyhow::bail!("Either --template or a command (after --) is required.\n\nUsage:\n  runbox run --template <id> [--binding key=value]\n  runbox run [OPTIONS] -- <command...>")
            }
        }
        Commands::Log {
            runtime,
            dry_run,
            timeout,
            env_vars,
            cwd,
            no_git,
            command,
        } => cmd_run_direct(
            &storage, command, runtime, dry_run, timeout, env_vars, cwd, no_git,
        ),
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
                template_or_index,
            } => cmd_playlist_remove(&storage, &playlist_id, &template_or_index),
        },
        Commands::Result { command } => match command {
            ResultCommands::List { limit } => cmd_result_list(&storage, limit),
            ResultCommands::Show { result_id } => cmd_result_show(&storage, &result_id),
            ResultCommands::ForRun { run_id } => cmd_result_for_run(&storage, &run_id),
            ResultCommands::Stdout { result_id } => cmd_result_stdout(&storage, &result_id),
            ResultCommands::Stderr { result_id } => cmd_result_stderr(&storage, &result_id),
            ResultCommands::Delete { result_id } => cmd_result_delete(&storage, &result_id),
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
        Commands::Tutorial => cmd_tutorial(),
    }
}

fn cmd_tutorial() -> Result<()> {
    println!("{}", TUTORIAL);
    Ok(())
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

    // Save run (before spawning)
    storage.save_run(&run)?;

    // Spawn process
    println!("Starting run: {}", run.run_id);
    println!("Runtime: {}", runtime_name);
    println!("Command: {:?}", run.exec.argv);

    let handle = adapter.spawn(&run.exec, &run.run_id, &log_path)?;

    // CAS-style update with lock: only update if still Pending
    // This prevents overwriting terminal state if process exited very fast
    let saved = storage.save_run_if_status_with(&run.run_id, &[RunStatus::Pending], |current| {
        current.handle = Some(handle.clone());
        current.status = RunStatus::Running;
        current.timeline.started_at = Some(Utc::now());
    })?;

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
            },
        );
        log::debug!("Run {} already exited - daemon captured status", run.run_id);
    }

    println!("Run started: {}", run.run_id);
    println!("Short ID: {}", run.short_id());
    println!("Logs: {}", log_path.display());

    if matches!(runtime, RuntimeType::Tmux) {
        println!("Attach with: runbox attach {}", run.short_id());
    }

    Ok(())
}

fn cmd_run_direct(
    storage: &Storage,
    command: Vec<String>,
    runtime: RuntimeType,
    dry_run: bool,
    timeout: u64,
    env_vars: Vec<String>,
    cwd: Option<String>,
    no_git: bool,
) -> Result<()> {
    use runbox_core::{CodeState, Exec, Run};

    if command.is_empty() {
        bail!("No command specified. Usage: runbox run -- <command...>");
    }

    let run_id = format!("run_{}", uuid::Uuid::new_v4());

    let code_state = if no_git {
        CodeState {
            repo_url: String::new(),
            base_commit: "0".repeat(40),
            patch: None,
        }
    } else {
        let git = GitContext::from_current_dir()?;
        git.build_code_state(&run_id)?
    };

    let env: std::collections::HashMap<String, String> = env_vars
        .iter()
        .filter_map(|s| {
            let parts: Vec<&str> = s.splitn(2, '=').collect();
            if parts.len() == 2 {
                Some((parts[0].to_string(), parts[1].to_string()))
            } else {
                None
            }
        })
        .collect();

    let working_dir = cwd.unwrap_or_else(|| ".".to_string());

    let exec = Exec {
        argv: command,
        cwd: working_dir,
        env,
        timeout_sec: timeout,
    };

    let mut run = Run::new(exec, code_state);
    run.run_id = run_id;

    run.validate()?;

    if dry_run {
        println!("Dry run - would execute:");
        println!("{}", serde_json::to_string_pretty(&run)?);
        return Ok(());
    }

    let registry = RuntimeRegistry::new();
    let runtime_name = runtime.to_string();
    let adapter = registry
        .get(&runtime_name)
        .context(format!("Unknown runtime: {}", runtime_name))?;

    let log_path = storage.log_path(&run.run_id);

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

    storage.save_run(&run)?;

    println!("Starting run: {}", run.run_id);
    println!("Runtime: {}", runtime_name);
    println!("Command: {:?}", run.exec.argv);

    let handle = adapter.spawn(&run.exec, &run.run_id, &log_path)?;

    let saved = storage.save_run_if_status_with(&run.run_id, &[RunStatus::Pending], |current| {
        current.handle = Some(handle.clone());
        current.status = RunStatus::Running;
        current.timeline.started_at = Some(Utc::now());
    })?;

    if !saved {
        let _ = storage.save_run_if_status_with(
            &run.run_id,
            &[RunStatus::Exited, RunStatus::Failed, RunStatus::Unknown],
            |current| {
                if current.handle.is_none() {
                    current.handle = Some(handle.clone());
                }
            },
        );
        log::debug!("Run {} already exited - daemon captured status", run.run_id);
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

fn cmd_ps(
    storage: &Storage,
    status_filter: Option<String>,
    _all: bool,
    limit: usize,
) -> Result<()> {
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
            },
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
            let _ =
                storage.save_run_if_status_with(&run.run_id, &[RunStatus::Running], |current| {
                    current.status = RunStatus::Unknown;
                    current.reconcile_reason = Some(reason.clone());
                    let now = Utc::now();
                    if current.timeline.started_at.is_none() {
                        current.timeline.started_at = Some(now);
                    }
                    if current.timeline.ended_at.is_none() {
                        current.timeline.ended_at = Some(now);
                    }
                });
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
    let content =
        std::fs::read_to_string(path).with_context(|| format!("Failed to read file: {}", path))?;

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
        println!(
            "{:<10} {:<30} {:<10}",
            short_id(&p.playlist_id),
            p.name,
            p.items.len()
        );
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
    let content =
        std::fs::read_to_string(path).with_context(|| format!("Failed to read file: {}", path))?;

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
    println!(
        "Added {} to {}",
        short_id(&resolved_template_id),
        short_id(&resolved_playlist_id)
    );
    Ok(())
}

fn cmd_playlist_remove(storage: &Storage, playlist_id: &str, selector: &str) -> Result<()> {
    let resolved_playlist_id = storage.resolve_playlist_id(playlist_id)?;
    let mut playlist = storage.load_playlist(&resolved_playlist_id)?;

    if selector.chars().all(|c| c.is_ascii_digit()) {
        let index: usize = selector
            .parse()
            .with_context(|| format!("Invalid index: {}", selector))?;
        if index >= playlist.items.len() {
            bail!(
                "Index {} out of bounds (playlist has {} items)",
                index,
                playlist.items.len()
            );
        }
        let removed = playlist.items.remove(index);
        storage.save_playlist(&playlist)?;
        println!(
            "Removed {} from {}",
            short_id(&removed.template_id),
            short_id(&resolved_playlist_id)
        );
        return Ok(());
    }

    let resolved_template_id = storage.resolve_template_id(selector)?;
    let initial_len = playlist.items.len();
    playlist
        .items
        .retain(|item| item.template_id != resolved_template_id);

    if playlist.items.len() == initial_len {
        bail!(
            "Template {} not found in playlist",
            short_id(&resolved_template_id)
        );
    }

    storage.save_playlist(&playlist)?;
    println!(
        "Removed {} from {}",
        short_id(&resolved_template_id),
        short_id(&resolved_playlist_id)
    );
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
    println!(
        "Runtime:    {}",
        if run.runtime.is_empty() {
            "-"
        } else {
            &run.runtime
        }
    );
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

// === Result Commands ===

fn cmd_result_list(storage: &Storage, limit: usize) -> Result<()> {
    let results = storage.list_results(limit)?;

    if results.is_empty() {
        println!("No results found.");
        return Ok(());
    }

    println!(
        "{:<12} {:<12} {:<10} {:<12} {:<8}",
        "RESULT ID", "RUN ID", "EXIT", "DURATION", "ARTIFACTS"
    );
    println!("{}", "-".repeat(60));

    for result in results {
        let duration = format!("{}ms", result.execution.duration_ms);
        println!(
            "{:<12} {:<12} {:<10} {:<12} {:<8}",
            result.short_id(),
            short_id(&result.run_id),
            result.execution.exit_code,
            duration,
            result.artifacts.len()
        );
    }

    Ok(())
}

fn cmd_result_show(storage: &Storage, result_id: &str) -> Result<()> {
    let resolved_id = storage.resolve_result_id(result_id)?;
    let result = storage.load_result(&resolved_id)?;

    println!("Result ID:    {}", result.result_id);
    println!("Short ID:     {}", result.short_id());
    println!("Run ID:       {}", result.run_id);
    println!();
    println!("Started:      {}", result.execution.started_at);
    println!("Finished:     {}", result.execution.finished_at);
    println!("Duration:     {}ms", result.execution.duration_ms);
    println!("Exit Code:    {}", result.execution.exit_code);

    if let Some(ref output) = result.output {
        println!();
        if let Some(ref stdout_ref) = output.stdout_ref {
            println!("Stdout:       {}", stdout_ref);
        }
        if let Some(ref stderr_ref) = output.stderr_ref {
            println!("Stderr:       {}", stderr_ref);
        }
    }

    if !result.artifacts.is_empty() {
        println!();
        println!("Artifacts:");
        for artifact in &result.artifacts {
            println!(
                "  - {}: {} ({})",
                artifact.name, artifact.path, artifact.ref_
            );
        }
    }

    Ok(())
}

fn cmd_result_for_run(storage: &Storage, run_id: &str) -> Result<()> {
    let resolved_run_id = storage.resolve_run_id(run_id)?;
    let results = storage.list_results_for_run(&resolved_run_id)?;

    if results.is_empty() {
        println!("No results found for run: {}", short_id(&resolved_run_id));
        return Ok(());
    }

    println!("Results for run: {}", short_id(&resolved_run_id));
    println!();
    println!(
        "{:<12} {:<10} {:<12} {:<20}",
        "RESULT ID", "EXIT", "DURATION", "FINISHED"
    );
    println!("{}", "-".repeat(60));

    for result in results {
        let duration = format!("{}ms", result.execution.duration_ms);
        println!(
            "{:<12} {:<10} {:<12} {:<20}",
            result.short_id(),
            result.execution.exit_code,
            duration,
            result.execution.finished_at.format("%Y-%m-%d %H:%M:%S")
        );
    }

    Ok(())
}

fn cmd_result_stdout(storage: &Storage, result_id: &str) -> Result<()> {
    let resolved_id = storage.resolve_result_id(result_id)?;
    let result = storage.load_result(&resolved_id)?;

    let stdout_ref = result
        .output
        .as_ref()
        .and_then(|o| o.stdout_ref.as_ref())
        .context("No stdout available for this result")?;

    let content = storage.load_blob(stdout_ref)?;
    print!("{}", String::from_utf8_lossy(&content));

    Ok(())
}

fn cmd_result_stderr(storage: &Storage, result_id: &str) -> Result<()> {
    let resolved_id = storage.resolve_result_id(result_id)?;
    let result = storage.load_result(&resolved_id)?;

    let stderr_ref = result
        .output
        .as_ref()
        .and_then(|o| o.stderr_ref.as_ref())
        .context("No stderr available for this result")?;

    let content = storage.load_blob(stderr_ref)?;
    print!("{}", String::from_utf8_lossy(&content));

    Ok(())
}

fn cmd_result_delete(storage: &Storage, result_id: &str) -> Result<()> {
    let resolved_id = storage.resolve_result_id(result_id)?;
    storage.delete_result(&resolved_id)?;
    println!("Result deleted: {}", short_id(&resolved_id));
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
