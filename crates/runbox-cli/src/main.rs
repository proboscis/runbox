use anyhow::{bail, Context, Result};
use chrono::Utc;
use clap::{Parser, Subcommand, ValueEnum};
use dialoguer::{theme::ColorfulTheme, Input};
use runbox_core::{
    default_pid_path, default_socket_path, find_skill_by_name, find_skills, short_id,
    BindingResolver, CodeState, ConfigResolver, DaemonClient, EntityType, Exec, GitContext, Index,
    IndexedEntity, LayeredStorage, LogRef, Platform, Playlist, PlaylistItem, Record, RecordCommand,
    RecordGitState, ReplaySpec, Run, RunStatus, RunTemplate, Runnable, RuntimeRegistry, Scope,
    Skill, Storage, Timeline, Validator, VerboseLogger,
};
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;
use terminal_size::{terminal_size, Width};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;
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
  # Replay a previous run or record with exact code state
  runbox replay <id>
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
    /// Zellij tab
    Zellij,
}
impl std::fmt::Display for RuntimeType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RuntimeType::Bg | RuntimeType::Background => write!(f, "background"),
            RuntimeType::Tmux => write!(f, "tmux"),
            RuntimeType::Zellij => write!(f, "zellij"),
        }
    }
}
#[derive(Subcommand)]
enum Commands {
    /// Run from a template, replay a run, or execute a command directly
    #[command(after_help = "\
EXAMPLES:
  # Unified short ID resolution (auto-detects type)
  runbox run 7f3a              # template (auto-detected)
  runbox run 550e              # replay (auto-detected)
  runbox run a1b2              # playlist item (auto-detected)

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

  # Explicit template-based execution
  runbox run --template tpl_train_model
  runbox run --template tpl_train_model --binding epochs=100
  runbox run --template tpl_hello --binding name=World --runtime tmux

  # Explicit replay execution
  runbox run --replay run_550e8400-e29b-41d4-a716-446655440000
  runbox run --replay rec_550e8400-e29b-41d4-a716-446655440000

RELATED COMMANDS:
  runbox log       Alias for direct execution (runbox log -- <cmd>)
  runbox ps        List runs to check status
  runbox logs      View stdout/stderr from a run
  runbox template  Manage templates")]
    Run {
        /// Short ID to resolve (template, replay, or playlist item)
        #[arg(value_name = "SHORT_ID")]
        target: Option<String>,
        /// Explicit template ID (for template-based execution)
        #[arg(short, long)]
        template: Option<String>,
        /// Explicit replay ID (run_... or rec_...)
        #[arg(long)]
        replay: Option<String>,
        /// Variable bindings (key=value) - only for template mode
        #[arg(short, long)]
        binding: Vec<String>,
        /// Runtime environment (bg, background, tmux, zellij)
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
        /// Runtime environment (bg, background, tmux, zellij)
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
    /// List all runnables (templates, replays, playlist items) in unified table
    #[command(after_help = "\
EXAMPLES:
  # List all runnables
  runbox list

  # Filter by type
  runbox list --type template
  runbox list --type replay
  runbox list --type playlist

  # Filter by playlist (implies --type playlist)
  runbox list --playlist daily

  # Filter by repository
  runbox list --repo runbox
  runbox list --repo proboscis/runbox
  runbox list --repo .          # current repo (auto-detect)

  # Filter by tag
  runbox list --tag 311
  runbox list --tag 311 --tag style

  # Show all repos (disable auto-filter)
  runbox list --all-repos

  # Limit results
  runbox list --limit 10
  runbox list --type replay --limit 5

  # Output formats
  runbox list                   # table (default)
  runbox list --json            # JSON array
  runbox list --short           # IDs only

  # Combined filters
  runbox list --type template --repo runbox
  runbox list --type template --tag 311 --tag style
  runbox list --repo . --type replay --limit 5
  runbox list --where-clause \"(',' || tags || ',') LIKE '%,311,%'\"

OUTPUT:
  SHORT     TYPE        SOURCE          NAME                    TAGS
  ────────────────────────────────────────────────────────────────────
  7f3a2b1c  template    -               Echo Message            -
  c4d5e6f7  template    -               Train Model             -
  a1b2c3d4  playlist    daily[0]        Echo Hello              -
  550e8400  replay      550e8400-e      python train.py         -

  4 runnables (2 templates, 1 playlist item, 1 replay)

RELATED COMMANDS:
  runbox run <short>   Run a runnable by short ID
  runbox template list List templates only
  runbox history       List past runs only
  runbox playlist show Show playlist items")]
    List {
        /// Filter by type: template, replay, playlist (default: show all)
        #[arg(short = 't', long, value_name = "TYPE")]
        r#type: Option<String>,
        /// Filter playlist items by playlist ID/prefix
        #[arg(short, long, value_name = "ID")]
        playlist: Option<String>,
        /// Filter by repository (name, org/name, or "." for current)
        #[arg(short, long, value_name = "REPO")]
        repo: Option<String>,
        /// Show runnables from ALL repos (disable auto-filter)
        #[arg(long)]
        all_repos: bool,
        /// Filter by tag (can be repeated; matches all requested tags)
        #[arg(long, value_name = "TAG")]
        tag: Vec<String>,
        /// Max items to show (default: 50)
        #[arg(short, long, default_value = "50")]
        limit: usize,
        /// Output as JSON
        #[arg(long)]
        json: bool,
        /// Output short IDs only (one per line)
        #[arg(long)]
        short: bool,
        /// Show additional details
        #[arg(short, long)]
        verbose: bool,
        /// SQL WHERE clause for filtering (e.g., "exit_code = 0")
        #[arg(long, value_name = "CONDITION")]
        where_clause: Option<String>,
        /// Show only local (.runbox/) items
        #[arg(long, conflicts_with = "global")]
        local: bool,
        /// Show only global items
        #[arg(long, conflicts_with = "local")]
        global: bool,
    },
    /// Execute a raw SQL query against the index
    Query {
        /// SQL query to execute
        #[arg(required = true)]
        sql: String,
        /// Output as JSON array
        #[arg(long)]
        json: bool,
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
  runbox attach    Attach to tmux/zellij session (interactive runtime)")]
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
    /// Attach to a running tmux/zellij session for interactive access
    #[command(after_help = "\
EXAMPLES:
  # Attach to a tmux/zellij-based run
  runbox attach 550e8400
  # Using full run ID
  runbox attach run_550e8400-e29b-41d4-a716-446655440000
NOTES:
  - Only works for runs started with --runtime tmux or --runtime zellij
  - Detach from tmux with Ctrl+B, D or from zellij with Ctrl+O, D
  - The process continues running after detaching
RELATED COMMANDS:
  runbox ps               List runs to find run IDs
  runbox logs <id>        View logs (for background runs)
  runbox run --runtime tmux    Start a new run in tmux
  runbox run --runtime zellij Start a new run in zellij")]
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
  # Ignore captured patch / diff and replay committed state only
  runbox replay 550e8400 --ignore-patch
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
        /// Ignore any recorded patch and replay committed state only
        #[arg(long)]
        ignore_patch: bool,
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
    /// Create runbox entities (records, templates)
    #[command(after_help = "\
EXAMPLES:
  # Create a record from stdin
  cat record.json | runbox create record

  # Create a record from a file
  runbox create record --from-file record.json

  # Create a record with minimal fields
  echo '{ \"command\": { \"argv\": [\"echo\", \"hello\"], \"cwd\": \".\" } }' | runbox create record

RECORD JSON FORMAT:
  {
    \"id\": \"rec_custom-id\",           // optional, auto-generated if missing
    \"git_state\": {
      \"repo_url\": \"git@github.com:org/repo.git\",
      \"commit\": \"abc123...\"           // 40-char hex
    },
    \"command\": {
      \"argv\": [\"python\", \"train.py\"],
      \"cwd\": \".\",
      \"env\": {}                          // optional
    },
    \"exit_code\": 0,                      // optional
    \"started_at\": \"2025-01-19T10:00:00Z\", // optional
    \"ended_at\": \"2025-01-19T10:05:00Z\",   // optional
    \"tags\": [\"ml\", \"training\"],      // optional
    \"source\": \"doeff\"                  // optional, default: external
  }

EXTERNAL TOOL INTEGRATION:
  This command allows external tools like doeff to register execution
  records in runbox for unified history and querying.

RELATED COMMANDS:
  runbox list --type record   List all records
  runbox query                Query records with SQL")]
    Create {
        #[command(subcommand)]
        command: CreateCommands,
    },
    #[command(after_help = "\
EXAMPLES:
  # List available skills
  runbox skill list

  # Show skill details
  runbox skill show runbox-cli

  # Export a skill with installation guides
  runbox skill export runbox-cli --output ./my-skill

OUTPUT STRUCTURE:
  my-skill/
  ├── SKILL.md              # The skill content
  ├── INSTALL.md            # Unified install guide
  ├── install/
  │   ├── claude-code.md
  │   ├── opencode.md
  │   ├── gemini.md
  │   ├── codex.md
  │   └── cursor.md
  └── install.sh            # Auto-install script

SUPPORTED PLATFORMS:
  - Claude Code  (~/.claude/skills/)
  - OpenCode     (~/.opencode/skills/)
  - Gemini CLI   (project GEMINI.md)
  - Codex        (AGENTS.md)
  - Cursor       (~/.cursor/rules/)")]
    Skill {
        #[command(subcommand)]
        command: SkillCommands,
    },
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
  runbox template create my_template.json --local
NOTE: Use 'runbox validate' to check the JSON before creating.")]
    Create {
        /// Path to template JSON file
        path: String,
        /// Save in project-local .runbox storage
        #[arg(long, conflicts_with = "global")]
        local: bool,
        /// Save in global storage (default)
        #[arg(long, conflicts_with = "local")]
        global: bool,
    },
    /// Delete a registered template
    #[command(after_help = "\
EXAMPLES:
  runbox template delete tpl_hello
  runbox template delete tpl_hello --local
  runbox template delete hello    # short ID prefix works")]
    Delete {
        /// Template ID (or short ID prefix)
        template_id: String,
        /// Delete from project-local .runbox storage
        #[arg(long, conflicts_with = "global")]
        local: bool,
        /// Delete from global storage (default)
        #[arg(long, conflicts_with = "local")]
        global: bool,
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

    /// Show playlist items (all playlists if none specified)
    #[command(after_help = "\
EXAMPLES:
  # Show ALL items from ALL playlists (flattened view)
  runbox playlist show

  # Show items from a specific playlist
  runbox playlist show pl_daily

  # JSON output for specific playlist
  runbox playlist show pl_daily --json

OUTPUT (flattened - no playlist specified):
  PLAYLIST  IDX  SHORT     TEMPLATE        LABEL
  daily     0    a1b2c3d4  tpl_echo        Echo Hello
  daily     1    f5e6d7c8  tpl_train       Train Model
  weekly    0    90ab12cd  tpl_backup      Backup Data

  Run with: runbox playlist run <SHORT>

OUTPUT (specific playlist):
  Playlist: pl_daily (Daily Tasks)

  IDX  SHORT     TEMPLATE        LABEL
  0    a1b2c3d4  tpl_echo        Echo Hello
  1    f5e6d7c8  tpl_train       Train Model

  Run with: runbox playlist run <SHORT> or runbox playlist run daily <IDX|SHORT>")]
    Show {
        /// Playlist ID (optional - if omitted, shows all playlists)
        playlist_id: Option<String>,
        /// Output raw JSON instead of table view
        #[arg(long)]
        json: bool,
    },

    /// Register a new playlist from a JSON file
    #[command(after_help = "\
EXAMPLES:
  runbox playlist create my_playlist.json
  runbox playlist create my_playlist.json --local

NOTE: Use 'runbox validate' to check the JSON before creating.")]
    Create {
        /// Path to playlist JSON file
        path: String,
        /// Save in project-local .runbox storage
        #[arg(long, conflicts_with = "global")]
        local: bool,
        /// Save in global storage (default)
        #[arg(long, conflicts_with = "local")]
        global: bool,
    },

    /// Add a template to a playlist
    #[command(after_help = "\
EXAMPLES:
  # Add with auto-generated label
  runbox playlist add pl_daily tpl_backup

  # Add with custom label
  runbox playlist add pl_daily tpl_backup --label 'Backup Data'

  # Modify a local playlist
  runbox playlist add pl_daily tpl_backup --local")]
    Add {
        /// Playlist ID (or short ID prefix)
        playlist_id: String,
        /// Template ID to add (or short ID prefix)
        template_id: String,
        /// Display label for this item in the playlist
        #[arg(short, long)]
        label: Option<String>,
        /// Modify a project-local playlist in .runbox
        #[arg(long, conflicts_with = "global")]
        local: bool,
        /// Modify a global playlist (default)
        #[arg(long, conflicts_with = "local")]
        global: bool,
    },

    /// Remove a template from a playlist by ID or index
    #[command(after_help = "\
EXAMPLES:
  # Remove by template ID
  runbox playlist remove pl_daily tpl_backup

  # Remove by index (0-based)
  runbox playlist remove pl_daily 0    # remove first item
  runbox playlist remove pl_daily 2    # remove third item

  # Modify a local playlist
  runbox playlist remove pl_daily tpl_backup --local")]
    Remove {
        /// Playlist ID (or short ID prefix)
        playlist_id: String,
        /// Template ID or index (0-based) to remove
        template_or_index: String,
        /// Modify a project-local playlist in .runbox
        #[arg(long, conflicts_with = "global")]
        local: bool,
        /// Modify a global playlist (default)
        #[arg(long, conflicts_with = "local")]
        global: bool,
    },

    /// Run a template from a playlist by global short ID or playlist + index/short ID
    #[command(after_help = "\
EXAMPLES:
  # Run by GLOBAL short ID (from 'playlist show' without args)
  runbox playlist run a1b2c3d4

  # Run by playlist + index
  runbox playlist run pl_daily 0

  # Run by playlist + short ID
  runbox playlist run pl_daily a1b2

  # With bindings and runtime options
  runbox playlist run a1b2 --binding epochs=10 --runtime tmux

  # Dry run to see what would be executed
  runbox playlist run a1b2 --dry-run

NOTES:
  - Use 'runbox playlist show' to see all items with globally unique short IDs
  - Use 'runbox playlist show <playlist>' to see items in a specific playlist
  - Short ID prefix matching is supported (e.g., 'a1' matches 'a1b2c3d4')")]
    Run {
        /// Global short ID, OR playlist ID when used with <item>
        selector: String,
        /// Item index or short ID within playlist (optional - if omitted, selector is treated as global short ID)
        item: Option<String>,
        /// Variable bindings (key=value) for template
        #[arg(short, long)]
        binding: Vec<String>,
        /// Runtime environment (bg, background, tmux, zellij)
        #[arg(short, long, default_value = "bg")]
        runtime: RuntimeType,
        /// Show what would be executed without running
        #[arg(long)]
        dry_run: bool,
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

#[derive(Subcommand)]
enum CreateCommands {
    /// Create a record from JSON input
    #[command(after_help = "\
EXAMPLES:
  # Create from stdin
  cat record.json | runbox create record

  # Create from file
  runbox create record --from-file record.json

  # Minimal record (ID auto-generated)
  echo '{\"command\":{\"argv\":[\"echo\"],\"cwd\":\".\"}}' | runbox create record")]
    Record {
        #[arg(long, value_name = "FILE")]
        from_file: Option<String>,
    },
}

#[derive(Subcommand)]
enum SkillCommands {
    List,
    Show {
        skill_name: String,
    },
    Export {
        skill_name: String,
        #[arg(short, long)]
        output: Option<PathBuf>,
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
            target,
            template,
            replay,
            binding,
            runtime,
            dry_run,
            timeout,
            env_vars,
            cwd,
            no_git,
            command,
        } => {
            // Priority: explicit flags > target short ID > direct command
            if let Some(tpl_id) = template {
                cmd_run_template(&storage, &tpl_id, binding, runtime, dry_run)
            } else if let Some(run_id) = replay {
                cmd_run_replay(&storage, &run_id, runtime, dry_run)
            } else if let Some(short_id) = target {
                cmd_run_unified(&storage, &short_id, binding, runtime, dry_run)
            } else if !command.is_empty() {
                cmd_run_direct(
                    &storage, command, runtime, dry_run, timeout, env_vars, cwd, no_git,
                )
            } else {
                anyhow::bail!("Either a short ID, --template, --replay, or a command (after --) is required.\n\nUsage:\n  runbox run <short_id>                    # unified resolution\n  runbox run --template <id> [--binding key=value]\n  runbox run --replay <id>\n  runbox run [OPTIONS] -- <command...>")
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
        Commands::List {
            r#type,
            playlist,
            repo,
            all_repos,
            tag,
            limit,
            json,
            short,
            verbose,
            where_clause,
            local,
            global,
        } => cmd_list(
            &storage,
            r#type,
            playlist,
            repo,
            all_repos,
            tag,
            limit,
            json,
            short,
            verbose,
            where_clause,
            local,
            global,
        ),
        Commands::Query { sql, json } => cmd_query(&storage, &sql, json),
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
            TemplateCommands::Create {
                path,
                local,
                global,
            } => cmd_template_create(&storage, &path, mutation_scope(local, global)),
            TemplateCommands::Delete {
                template_id,
                local,
                global,
            } => cmd_template_delete(&storage, &template_id, mutation_scope(local, global)),
        },
        Commands::Playlist { command } => match command {
            PlaylistCommands::List => cmd_playlist_list(&storage),
            PlaylistCommands::Show { playlist_id, json } => {
                cmd_playlist_show(&storage, playlist_id.as_deref(), json)
            }
            PlaylistCommands::Create {
                path,
                local,
                global,
            } => cmd_playlist_create(&storage, &path, mutation_scope(local, global)),
            PlaylistCommands::Add {
                playlist_id,
                template_id,
                label,
                local,
                global,
            } => cmd_playlist_add(
                &storage,
                &playlist_id,
                &template_id,
                label,
                mutation_scope(local, global),
            ),
            PlaylistCommands::Remove {
                playlist_id,
                template_or_index,
                local,
                global,
            } => cmd_playlist_remove(
                &storage,
                &playlist_id,
                &template_or_index,
                mutation_scope(local, global),
            ),
            PlaylistCommands::Run {
                selector,
                item,
                binding,
                runtime,
                dry_run,
            } => cmd_playlist_run(
                &storage,
                &selector,
                item.as_deref(),
                binding,
                runtime,
                dry_run,
            ),
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
            ignore_patch,
            reuse,
            fresh,
            verbose,
        } => cmd_replay(
            &storage,
            &run_id,
            worktree_dir,
            keep,
            cleanup,
            ignore_patch,
            reuse,
            fresh,
            verbose,
        ),
        Commands::Validate { path } => cmd_validate(&path),
        Commands::Create { command } => match command {
            CreateCommands::Record { from_file } => cmd_create_record(&storage, from_file),
        },
        Commands::Daemon { command } => match command {
            DaemonCommands::Start => cmd_daemon_start(),
            DaemonCommands::Stop => cmd_daemon_stop(),
            DaemonCommands::Status => cmd_daemon_status(),
            DaemonCommands::Ping => cmd_daemon_ping(),
        },
        Commands::Skill { command } => match command {
            SkillCommands::List => cmd_skill_list(),
            SkillCommands::Show { skill_name } => cmd_skill_show(&skill_name),
            SkillCommands::Export { skill_name, output } => cmd_skill_export(&skill_name, output),
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
    let layered = layered_storage(storage)?;
    let resolved_template_id = layered.resolve_template_id(template_id)?;
    let (template, _) = layered.load_template(&resolved_template_id)?;
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

    if matches!(runtime, RuntimeType::Tmux | RuntimeType::Zellij) {
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

    if matches!(runtime, RuntimeType::Tmux | RuntimeType::Zellij) {
        println!("Attach with: runbox attach {}", run.short_id());
    }
    Ok(())
}

// === Unified Run Command ===

/// Display a box showing what is being run
fn display_run_info(
    runnable: &Runnable,
    template: Option<&RunTemplate>,
    replay: Option<&ReplaySpec>,
    replay_created_at: Option<chrono::DateTime<Utc>>,
) {
    let width = 55;
    let border = "─".repeat(width);

    println!("┌{}┐", border);

    match runnable {
        Runnable::Template(id) => {
            println!("│ {:<width$}│", format!("TEMPLATE: {}", id), width = width);
            if let Some(tpl) = template {
                println!(
                    "│ {:<width$}│",
                    format!("Name: {}", tpl.name),
                    width = width
                );
            }
        }
        Runnable::Replay(id) => {
            println!("│ {:<width$}│", format!("REPLAY: {}", id), width = width);
            if let Some(created) = replay_created_at {
                println!(
                    "│ {:<width$}│",
                    format!("Original: {}", created.format("%Y-%m-%d %H:%M:%S")),
                    width = width
                );
            }
        }
        Runnable::PlaylistItem {
            playlist_id,
            index,
            label,
            ..
        } => {
            let label_str = label
                .as_ref()
                .map(|l| format!(" {:?}", l))
                .unwrap_or_default();
            println!(
                "│ {:<width$}│",
                format!("PLAYLIST ITEM: {}[{}]{}", playlist_id, index, label_str),
                width = width
            );
            if let Some(tpl) = template {
                println!(
                    "│ {:<width$}│",
                    format!("Template: {}", tpl.template_id),
                    width = width
                );
            }
        }
    }

    println!("├{}┤", border);

    if let Some(tpl) = template {
        let cmd = tpl.exec.argv.join(" ");
        let cmd_display = if cmd.len() > width - 10 {
            format!("{}...", &cmd[..width - 13])
        } else {
            cmd
        };
        println!(
            "│ {:<width$}│",
            format!("Command: {}", cmd_display),
            width = width
        );
        println!(
            "│ {:<width$}│",
            format!("Cwd:     {}", tpl.exec.cwd),
            width = width
        );
    } else if let Some(replay) = replay {
        let cmd = replay.argv.join(" ");
        let cmd_display = if cmd.len() > width - 10 {
            format!("{}...", &cmd[..width - 13])
        } else {
            cmd
        };
        println!(
            "│ {:<width$}│",
            format!("Command: {}", cmd_display),
            width = width
        );
        println!(
            "│ {:<width$}│",
            format!("Cwd:     {}", replay.cwd),
            width = width
        );
        if !replay.code_state.repo_url.is_empty() {
            println!(
                "│ {:<width$}│",
                format!(
                    "Commit:  {}",
                    replay
                        .code_state
                        .base_commit
                        .get(..8)
                        .unwrap_or(&replay.code_state.base_commit)
                ),
                width = width
            );
        }
    }

    println!("└{}┘", border);
}

fn normalize_replay_input(input: &str) -> String {
    if let Some(rest) = input.strip_prefix("run-") {
        return format!("run_{}", rest);
    }
    if let Some(rest) = input.strip_prefix("rec-") {
        return format!("rec_{}", rest);
    }
    input.to_string()
}

fn resolve_replay_id(storage: &Storage, input: &str) -> Result<String> {
    let normalized_input = normalize_replay_input(input);

    if normalized_input.starts_with("run_") {
        return storage.resolve_run_id(&normalized_input);
    }

    if normalized_input.starts_with("rec_") {
        return storage.resolve_record_id(&normalized_input);
    }

    let short_input = normalized_input.to_lowercase();
    if short_input.is_empty() {
        bail!("Empty input: please provide a short ID or full ID (run_..., rec_...)");
    }

    let mut matches: Vec<String> = storage
        .list_runs(usize::MAX)?
        .into_iter()
        .filter(|run| run.short_id().to_lowercase().starts_with(&short_input))
        .map(|run| run.run_id)
        .collect();

    matches.extend(
        storage
            .list_records(usize::MAX)?
            .into_iter()
            .filter(|record| record.short_id().to_lowercase().starts_with(&short_input))
            .map(|record| record.record_id),
    );

    match matches.len() {
        0 => bail!("No item found matching '{}'", input),
        1 => Ok(matches.remove(0)),
        n => {
            let candidates: Vec<String> = matches
                .iter()
                .map(|id| format!("  - {}", short_id(id)))
                .collect();
            bail!(
                "Ambiguous: {} items match '{}'. Use more characters.\n{}",
                n,
                input,
                candidates.join("\n")
            )
        }
    }
}

fn load_replay_spec(storage: &Storage, input: &str) -> Result<ReplaySpec> {
    let resolved_id = resolve_replay_id(storage, input)?;

    if resolved_id.starts_with("rec_") {
        let record = storage.load_record(&resolved_id)?;
        Ok(ReplaySpec::from(&record))
    } else {
        let run = storage.load_run(&resolved_id)?;
        Ok(ReplaySpec::from(&run))
    }
}

fn replay_created_at(storage: &Storage, replay_id: &str) -> Option<chrono::DateTime<Utc>> {
    if replay_id.starts_with("rec_") {
        storage
            .load_record(replay_id)
            .ok()
            .map(|record| record.created_at)
    } else {
        storage
            .load_run(replay_id)
            .ok()
            .and_then(|run| run.timeline.created_at)
    }
}

fn git_context_for_replay(replay: &ReplaySpec) -> Result<GitContext> {
    let replay_cwd = Path::new(&replay.cwd);
    if replay_cwd.is_absolute() {
        return GitContext::from_path(replay_cwd).with_context(|| {
            format!(
                "Recorded replay cwd is not a git repository: {}",
                replay_cwd.display()
            )
        });
    }

    GitContext::from_current_dir().with_context(|| {
        format!(
            "Replay '{}' only stores a relative cwd ('{}'). Run replay from a checkout of {}.",
            replay.id, replay.cwd, replay.code_state.repo_url
        )
    })
}

fn git_context_for_record_diff(cwd: &str) -> Result<GitContext> {
    GitContext::from_path(Path::new(cwd)).with_context(|| {
        format!(
            "git_state.diff requires command.cwd to point at a git repository: {}",
            cwd
        )
    })
}

/// Unified run command - resolves short ID to any runnable type and executes
fn cmd_run_unified(
    storage: &Storage,
    short_id: &str,
    bindings: Vec<String>,
    runtime: RuntimeType,
    dry_run: bool,
) -> Result<()> {
    let layered = layered_storage(storage)?;
    // Resolve the short ID to a Runnable
    let runnable = layered.resolve_runnable(short_id, 100)?;

    match &runnable {
        Runnable::Template(template_id) => {
            // Load template for display
            let (template, _) = layered.load_template(template_id)?;
            display_run_info(&runnable, Some(&template), None, None);
            println!();
            cmd_run_template(storage, template_id, bindings, runtime, dry_run)
        }
        Runnable::Replay(replay_id) => {
            let replay = load_replay_spec(storage, replay_id)?;
            display_run_info(
                &runnable,
                None,
                Some(&replay),
                replay_created_at(storage, replay_id),
            );
            println!();
            cmd_run_replay(storage, replay_id, runtime, dry_run)
        }
        Runnable::PlaylistItem {
            template_id,
            playlist_id,
            index,
            label,
        } => {
            // Load template for display
            let (template, _) = layered.load_template(template_id)?;
            let runnable_with_info = Runnable::PlaylistItem {
                playlist_id: playlist_id.clone(),
                index: *index,
                template_id: template_id.clone(),
                label: label.clone(),
            };
            display_run_info(&runnable_with_info, Some(&template), None, None);
            println!();
            cmd_run_template(storage, template_id, bindings, runtime, dry_run)
        }
    }
}

/// Run a replay of a previous run or record
fn cmd_run_replay(
    storage: &Storage,
    replay_id: &str,
    runtime: RuntimeType,
    dry_run: bool,
) -> Result<()> {
    let replay = load_replay_spec(storage, replay_id)?;

    if dry_run {
        println!("Dry run - would replay:");
        println!("  Original: {}", replay.id);
        println!("  Command: {:?}", replay.argv);
        println!("  Cwd: {}", replay.cwd);
        println!("  Commit: {}", replay.code_state.base_commit);
        if replay.code_state.patch.is_some() {
            println!("  Patch: yes");
        }
        return Ok(());
    }

    // Create a new run with the same exec and code_state
    let new_run_id = format!("run_{}", uuid::Uuid::new_v4());
    let exec = Exec {
        argv: replay.argv.clone(),
        cwd: replay.cwd.clone(),
        env: replay.env.clone(),
        timeout_sec: replay.timeout_sec,
    };
    let code_state = CodeState {
        repo_url: replay.code_state.repo_url.clone(),
        base_commit: replay.code_state.base_commit.clone(),
        patch: replay.code_state.patch.clone(),
    };

    let mut run = Run::new(exec, code_state);
    run.run_id = new_run_id;
    run.validate()?;

    // Execute the run
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

    println!("Starting replay: {}", run.run_id);
    println!("Original source: {}", replay.id);
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

    if matches!(runtime, RuntimeType::Tmux | RuntimeType::Zellij) {
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

    let rows: Vec<Vec<String>> = runs
        .into_iter()
        .map(|run| {
            let short_id = run.short_id().to_string();
            let runtime_display = if run.runtime.is_empty() {
                "-".to_string()
            } else {
                run.runtime
            };
            vec![
                short_id,
                run.status.to_string(),
                runtime_display,
                run.exec.argv.join(" "),
            ]
        })
        .collect();

    print_table(
        &[
            TableColumn::left_with_max("SHORT ID", 8, 12),
            TableColumn::left_with_max("STATUS", 7, 10),
            TableColumn::left_with_max("RUNTIME", 8, 10),
            TableColumn::expanded("COMMAND", 12),
        ],
        &rows,
    );
    Ok(())
}

// === List Command ===

/// Detect the current repository from the working directory.
/// Supports both regular git repos and worktrees.
fn detect_current_repo() -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["remote", "get-url", "origin"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Some(normalize_repo_url(&url))
}

/// Normalize a git remote URL to "org/repo" format.
/// Handles SSH and HTTPS URLs.
fn normalize_repo_url(url: &str) -> String {
    // Handle various formats:
    // git@github.com:proboscis/runbox.git → proboscis/runbox
    // https://github.com/proboscis/runbox.git → proboscis/runbox
    // https://github.com/proboscis/runbox → proboscis/runbox
    let url = url.trim_end_matches(".git");

    // Check for SSH format: git@host:org/repo
    // SSH format has colon NOT followed by // and no :// in the URL before that colon
    if let Some(idx) = url.rfind(':') {
        let after_colon = &url[idx + 1..];
        // SSH format: colon is NOT part of a URL scheme (no :// before)
        // and is NOT followed by // (would indicate a different URL format)
        if !after_colon.starts_with("//") && !url.contains("://") {
            return after_colon.to_string();
        }
    }

    // HTTPS format: https://github.com/org/repo
    // Split by '/' and take last two components
    let parts: Vec<&str> = url.rsplitn(3, '/').collect();
    if parts.len() >= 2 {
        return format!("{}/{}", parts[1], parts[0]);
    }

    url.to_string()
}

/// Check if a repo URL matches a filter.
/// The filter can be:
/// - Full match: "org/repo"
/// - Partial match: "repo" (matches any org)
fn repo_matches(repo_url: &Option<String>, filter: &str) -> bool {
    let Some(url) = repo_url else {
        return false;
    };

    let normalized = normalize_repo_url(url);

    // Full match
    if normalized == filter {
        return true;
    }

    // Partial match (repo name only)
    if let Some(repo_name) = normalized.split('/').last() {
        if repo_name == filter {
            return true;
        }
    }

    false
}

/// Runnable info for JSON serialization
#[derive(serde::Serialize)]
struct RunnableInfo {
    short_id: String,
    #[serde(rename = "type")]
    runnable_type: String,
    source: String,
    name: String,
    tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    repo_url: Option<String>,
}

fn format_tags(tags: &[String]) -> String {
    if tags.is_empty() {
        "-".to_string()
    } else {
        tags.join(",")
    }
}

#[derive(Clone, Copy)]
enum TableAlignment {
    Left,
    Right,
}

#[derive(Clone, Copy)]
struct TableColumn<'a> {
    header: &'a str,
    // Soft target width used for initial layout; columns may shrink below this on narrow terminals.
    preferred_width: usize,
    max_width: Option<usize>,
    expand: bool,
    alignment: TableAlignment,
}

impl<'a> TableColumn<'a> {
    fn left_with_max(header: &'a str, preferred_width: usize, max_width: usize) -> Self {
        Self {
            header,
            preferred_width,
            max_width: Some(max_width),
            expand: false,
            alignment: TableAlignment::Left,
        }
    }

    fn expanded(header: &'a str, preferred_width: usize) -> Self {
        Self {
            header,
            preferred_width,
            max_width: None,
            expand: true,
            alignment: TableAlignment::Left,
        }
    }

    fn right(header: &'a str, preferred_width: usize) -> Self {
        Self {
            header,
            preferred_width,
            max_width: None,
            expand: false,
            alignment: TableAlignment::Right,
        }
    }
}

fn display_width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

fn terminal_width() -> usize {
    const DEFAULT_WIDTH: usize = 100;

    std::env::var("COLUMNS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|width| *width > 0)
        .or_else(|| {
            terminal_size()
                .map(|(Width(width), _)| width as usize)
                .filter(|width| *width > 0)
        })
        .unwrap_or(DEFAULT_WIDTH)
}

fn table_separator_width(column_count: usize, terminal_width: usize) -> usize {
    if column_count > 1 && terminal_width > column_count.saturating_sub(1) {
        1
    } else {
        0
    }
}

fn print_table(columns: &[TableColumn<'_>], rows: &[Vec<String>]) {
    if columns.is_empty() {
        return;
    }

    let terminal_width = terminal_width();
    let separator_width = table_separator_width(columns.len(), terminal_width);
    let separator = if separator_width == 0 { "" } else { " " };
    let widths = compute_table_widths(columns, rows, terminal_width, separator_width);
    println!("{}", format_table_row(columns, &widths, None, separator));
    println!(
        "{}",
        "-".repeat(
            widths.iter().sum::<usize>() + separator_width * columns.len().saturating_sub(1)
        )
    );

    for row in rows {
        println!(
            "{}",
            format_table_row(columns, &widths, Some(row), separator)
        );
    }
}

fn compute_table_widths(
    columns: &[TableColumn<'_>],
    rows: &[Vec<String>],
    terminal_width: usize,
    separator_width: usize,
) -> Vec<usize> {
    let spacing = separator_width * columns.len().saturating_sub(1);
    let available = terminal_width.saturating_sub(spacing);

    let mut widths: Vec<usize> = columns
        .iter()
        .enumerate()
        .map(|(idx, column)| {
            let mut width = rows
                .iter()
                .filter_map(|row| row.get(idx))
                .map(|cell| display_width(cell))
                .max()
                .unwrap_or(0)
                .max(display_width(column.header))
                .max(column.preferred_width)
                .max(1);

            if let Some(max_width) = column.max_width {
                width = width.min(max_width);
            }

            width
        })
        .collect();

    let total: usize = widths.iter().sum();
    if total < available {
        distribute_extra_width(columns, &mut widths, available - total);
    } else if total > available {
        shrink_table_widths(columns, &mut widths, total - available);
    }

    widths
}

fn distribute_extra_width(columns: &[TableColumn<'_>], widths: &mut [usize], mut extra: usize) {
    if extra == 0 || widths.is_empty() {
        return;
    }

    let mut preferred: Vec<usize> = columns
        .iter()
        .enumerate()
        .filter_map(|(idx, column)| column.expand.then_some(idx))
        .collect();

    if preferred.is_empty() {
        preferred.push(widths.len() - 1);
    }

    let all_columns: Vec<usize> = (0..widths.len()).collect();
    let groups = [preferred.as_slice(), all_columns.as_slice()];

    for group in groups {
        while extra > 0 {
            let mut changed = false;

            for &idx in group {
                if let Some(max_width) = columns[idx].max_width {
                    if widths[idx] >= max_width {
                        continue;
                    }
                }

                widths[idx] += 1;
                extra -= 1;
                changed = true;

                if extra == 0 {
                    return;
                }
            }

            if !changed {
                break;
            }
        }
    }
}

fn shrink_table_widths(columns: &[TableColumn<'_>], widths: &mut [usize], mut overflow: usize) {
    if overflow == 0 || widths.is_empty() {
        return;
    }

    let mut preferred: Vec<usize> = columns
        .iter()
        .enumerate()
        .filter_map(|(idx, column)| column.expand.then_some(idx))
        .collect();
    preferred.extend(
        columns
            .iter()
            .enumerate()
            .filter_map(|(idx, column)| (!column.expand).then_some(idx)),
    );

    for floor in [
        columns
            .iter()
            .map(|column| column.preferred_width.max(1))
            .collect::<Vec<_>>(),
        vec![0; columns.len()],
    ] {
        while overflow > 0 {
            let mut changed = false;

            for &idx in &preferred {
                if widths[idx] > floor[idx] {
                    widths[idx] -= 1;
                    overflow -= 1;
                    changed = true;

                    if overflow == 0 {
                        return;
                    }
                }
            }

            if !changed {
                break;
            }
        }
    }
}

fn format_table_row(
    columns: &[TableColumn<'_>],
    widths: &[usize],
    row: Option<&[String]>,
    separator: &str,
) -> String {
    columns
        .iter()
        .enumerate()
        .map(|(idx, column)| {
            let value = row
                .and_then(|row| row.get(idx))
                .map(|value| value.as_str())
                .unwrap_or(column.header);
            format_table_cell(value, widths[idx], column.alignment)
        })
        .collect::<Vec<_>>()
        .join(separator)
}

fn format_table_cell(value: &str, width: usize, alignment: TableAlignment) -> String {
    let truncated = truncate_string(value, width);
    let padding = width.saturating_sub(display_width(&truncated));

    match alignment {
        TableAlignment::Left => format!("{}{}", truncated, " ".repeat(padding)),
        TableAlignment::Right => format!("{}{}", " ".repeat(padding), truncated),
    }
}

/// Safely truncate a string to fit within max_width display cells, adding "..." if truncated.
/// Truncation happens at grapheme-cluster boundaries so combined glyphs stay intact.
fn truncate_string(s: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }

    let current_width = display_width(s);
    if current_width <= max_width {
        return s.to_string();
    }

    let ellipsis = if max_width <= 3 {
        return ".".repeat(max_width);
    } else {
        "..."
    };
    let target_width = max_width - display_width(ellipsis);

    let mut truncated = String::new();
    let mut used_width = 0;

    for grapheme in UnicodeSegmentation::graphemes(s, true) {
        let grapheme_width = UnicodeWidthStr::width(grapheme);
        if used_width + grapheme_width > target_width {
            break;
        }
        truncated.push_str(grapheme);
        used_width += grapheme_width;
    }

    truncated.push_str(ellipsis);
    truncated
}

fn layered_storage(storage: &Storage) -> Result<LayeredStorage> {
    LayeredStorage::with_data_and_state_dirs(
        runbox_core::locate_local_runbox_dir(),
        storage.data_dir().clone(),
        storage.state_dir().clone(),
    )
}

fn mutation_scope(local: bool, _global: bool) -> Scope {
    if local {
        Scope::Local
    } else {
        Scope::Global
    }
}

fn scope_name(scope: Scope) -> &'static str {
    match scope {
        Scope::Local => "local",
        Scope::Global => "global",
    }
}

fn index_scope_filter(local: bool, global: bool) -> Option<Vec<&'static str>> {
    if local {
        Some(vec!["local"])
    } else if global {
        Some(vec!["global"])
    } else {
        None
    }
}

fn index_entity_types(type_filter: Option<&runbox_core::RunnableType>) -> Option<Vec<EntityType>> {
    match type_filter {
        Some(runbox_core::RunnableType::Template) => Some(vec![EntityType::Template]),
        Some(runbox_core::RunnableType::Playlist) => Some(vec![EntityType::Playlist]),
        Some(runbox_core::RunnableType::Replay) => Some(vec![EntityType::Run, EntityType::Record]),
        None => None,
    }
}

fn sync_index(storage: &Storage) -> Result<Index> {
    let layered = layered_storage(storage)?;
    let db_path = storage.state_dir().join("runbox.db");
    let index = Index::open(&db_path)
        .with_context(|| format!("Failed to open index database: {}", db_path.display()))?;

    index.clear_file_index()?;

    index_json_dir(
        &index,
        &storage.data_dir().join("templates"),
        EntityType::Template,
        "global",
    )?;
    index_json_dir(
        &index,
        &storage.data_dir().join("playlists"),
        EntityType::Playlist,
        "global",
    )?;
    index_json_dir(
        &index,
        &storage.data_dir().join("records"),
        EntityType::Record,
        "global",
    )?;
    index_json_dir(
        &index,
        &storage.data_dir().join("runs"),
        EntityType::Run,
        "global",
    )?;

    if let Some(local_dir) = layered.local_dir() {
        index_json_dir(
            &index,
            &local_dir.join("templates"),
            EntityType::Template,
            "local",
        )?;
        index_json_dir(
            &index,
            &local_dir.join("playlists"),
            EntityType::Playlist,
            "local",
        )?;
    }

    Ok(index)
}

fn index_json_dir(index: &Index, dir: &Path, entity_type: EntityType, scope: &str) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }

    let mut entries: Vec<_> = std::fs::read_dir(dir)?
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry
                .path()
                .extension()
                .map(|extension| extension == "json")
                .unwrap_or(false)
        })
        .collect();

    entries.sort_by_key(|entry| entry.path());

    for entry in entries {
        index.index_file_with_scope(&entry.path(), entity_type, scope)?;
    }

    Ok(())
}

fn indexed_entity_scope(entity: &IndexedEntity) -> Scope {
    if entity.scope == "local" {
        Scope::Local
    } else {
        Scope::Global
    }
}

fn indexed_entities_to_runnables(entities: &[IndexedEntity]) -> Result<Vec<(Runnable, Scope)>> {
    let mut runnables = Vec::new();

    for entity in entities {
        let scope = indexed_entity_scope(entity);
        match entity.entity_type {
            EntityType::Template => {
                runnables.push((Runnable::Template(entity.id.clone()), scope));
            }
            EntityType::Run | EntityType::Record => {
                runnables.push((Runnable::Replay(entity.id.clone()), scope));
            }
            EntityType::Playlist => {
                let playlist: Playlist = serde_json::from_str(&entity.json_data)
                    .with_context(|| format!("Failed to parse indexed playlist '{}'", entity.id))?;

                for (index, item) in playlist.items.iter().enumerate() {
                    runnables.push((
                        Runnable::PlaylistItem {
                            playlist_id: playlist.playlist_id.clone(),
                            index,
                            template_id: item.template_id.clone(),
                            label: item.label.clone(),
                        },
                        scope,
                    ));
                }
            }
        }
    }

    Ok(runnables)
}

fn filter_runnables(
    layered: &LayeredStorage,
    runnables: Vec<(Runnable, Scope)>,
    type_filter: Option<&runbox_core::RunnableType>,
    playlist_filter: Option<&str>,
    repo_filter: Option<&str>,
    tag_filter: &[String],
    limit: usize,
) -> Vec<(Runnable, Scope)> {
    runnables
        .into_iter()
        .filter(|(r, _)| {
            if let Some(t) = type_filter {
                if &r.runnable_type() != t {
                    return false;
                }
            }

            if let Some(pl) = playlist_filter {
                match r.playlist_id() {
                    Some(pid) => {
                        let pl_name = pid.trim_start_matches("pl_");
                        if !pl_name.starts_with(pl) && !pid.starts_with(pl) {
                            return false;
                        }
                    }
                    None => return false,
                }
            }

            if let Some(repo) = repo_filter {
                let runnable_repo = layered.get_runnable_repo_url(r);
                if !repo_matches(&runnable_repo, repo) {
                    return false;
                }
            }

            if !tag_filter.is_empty() {
                let runnable_tags = layered.get_runnable_tags(r);
                if !tag_filter
                    .iter()
                    .all(|tag| runnable_tags.iter().any(|runnable_tag| runnable_tag == tag))
                {
                    return false;
                }
            }

            true
        })
        .take(limit)
        .collect()
}

fn emit_runnable_output(
    layered: &LayeredStorage,
    filtered: &[(Runnable, Scope)],
    json_output: bool,
    short_output: bool,
    verbose: bool,
) -> Result<()> {
    if filtered.is_empty() {
        if json_output {
            println!("[]");
        } else if !short_output {
            println!("No runnables found.");
        }
        return Ok(());
    }

    let infos: Vec<RunnableInfo> = filtered
        .iter()
        .map(|(r, _)| {
            let repo_url = if verbose {
                layered.get_runnable_repo_url(r)
            } else {
                None
            };
            let tags = layered.get_runnable_tags(r);
            RunnableInfo {
                short_id: r.short_id(),
                runnable_type: r.type_label().to_string(),
                source: r.source_label(),
                name: layered.get_runnable_display_name(r),
                tags,
                repo_url,
            }
        })
        .collect();

    if json_output {
        println!("{}", serde_json::to_string_pretty(&infos)?);
    } else if short_output {
        for info in &infos {
            println!("{}", info.short_id);
        }
    } else {
        let rows: Vec<Vec<String>> = infos
            .iter()
            .map(|info| {
                let mut row = vec![
                    info.short_id.clone(),
                    info.runnable_type.clone(),
                    info.source.clone(),
                    info.name.clone(),
                    format_tags(&info.tags),
                ];

                if verbose {
                    row.push(info.repo_url.clone().unwrap_or_else(|| "-".to_string()));
                }

                row
            })
            .collect();

        if verbose {
            print_table(
                &[
                    TableColumn::left_with_max("SHORT", 8, 10),
                    TableColumn::left_with_max("TYPE", 8, 10),
                    TableColumn::left_with_max("SOURCE", 10, 16),
                    TableColumn::expanded("NAME", 12),
                    TableColumn::left_with_max("TAGS", 6, 14),
                    TableColumn::expanded("REPO", 14),
                ],
                &rows,
            );
        } else {
            print_table(
                &[
                    TableColumn::left_with_max("SHORT", 8, 10),
                    TableColumn::left_with_max("TYPE", 8, 10),
                    TableColumn::left_with_max("SOURCE", 10, 16),
                    TableColumn::expanded("NAME", 14),
                    TableColumn::expanded("TAGS", 8),
                ],
                &rows,
            );
        }

        let mut type_counts: std::collections::HashMap<&str, usize> =
            std::collections::HashMap::new();
        for (r, _) in filtered {
            *type_counts.entry(r.type_label()).or_insert(0) += 1;
        }

        let summary_parts: Vec<String> = type_counts
            .iter()
            .map(|(t, c)| format!("{} {}s", c, t))
            .collect();

        println!(
            "\n{} runnables ({})",
            filtered.len(),
            summary_parts.join(", ")
        );
    }

    Ok(())
}

fn cmd_list(
    storage: &Storage,
    type_filter: Option<String>,
    playlist_filter: Option<String>,
    repo_arg: Option<String>,
    all_repos: bool,
    tag_filter: Vec<String>,
    limit: usize,
    json_output: bool,
    short_output: bool,
    verbose: bool,
    where_clause: Option<String>,
    local: bool,
    global: bool,
) -> Result<()> {
    use runbox_core::RunnableType;

    // Parse type filter
    let type_filter: Option<RunnableType> = if let Some(ref t) = type_filter {
        Some(t.parse().map_err(|e: String| anyhow::anyhow!("{}", e))?)
    } else if playlist_filter.is_some() {
        // --playlist implies --type playlist
        Some(RunnableType::Playlist)
    } else {
        None
    };

    // Determine repo filter
    let repo_filter: Option<String> = if all_repos {
        // Explicitly show all repos
        None
    } else if let Some(r) = repo_arg {
        if r == "." {
            // "." means current repo
            detect_current_repo()
        } else {
            Some(r)
        }
    } else {
        // Auto-detect from current directory
        detect_current_repo()
    };

    // Show hint about repo filtering
    let show_repo_hint = !all_repos && repo_filter.is_some() && !json_output && !short_output;
    if show_repo_hint {
        if let Some(ref repo) = repo_filter {
            eprintln!(
                "Showing runnables for: {} (use --all-repos to show all)\n",
                repo
            );
        }
    }

    let layered = layered_storage(storage)?;

    // Handle --where-clause: use Index query mode
    if let Some(ref where_cond) = where_clause {
        let index = sync_index(storage)?;
        let entity_types = index_entity_types(type_filter.as_ref());
        let scope_filter = index_scope_filter(local, global);
        let results = index.query(
            entity_types.as_deref(),
            scope_filter.as_deref(),
            Some(where_cond),
            limit.saturating_mul(4).max(limit),
        )?;
        let filtered = filter_runnables(
            &layered,
            indexed_entities_to_runnables(&results)?,
            type_filter.as_ref(),
            playlist_filter.as_deref(),
            repo_filter.as_deref(),
            &tag_filter,
            limit,
        );

        if filtered.is_empty() {
            if json_output {
                println!("[]");
            } else if !short_output {
                println!("No results matching WHERE clause.");
            }
            return Ok(());
        }

        return emit_runnable_output(&layered, &filtered, json_output, short_output, verbose);
    }

    // Get all runnables
    let all_runnables = layered.list_all_runnables_with_scope(limit * 2)?; // Get more to account for filtering

    // Apply filters
    let filtered: Vec<_> = all_runnables
        .into_iter()
        .filter(|(_, scope)| {
            (!local || *scope == Scope::Local) && (!global || *scope == Scope::Global)
        })
        .collect();
    let filtered = filter_runnables(
        &layered,
        filtered,
        type_filter.as_ref(),
        playlist_filter.as_deref(),
        repo_filter.as_deref(),
        &tag_filter,
        limit,
    );

    emit_runnable_output(&layered, &filtered, json_output, short_output, verbose)
}

// === Query Command ===
fn cmd_query(storage: &Storage, sql: &str, json_output: bool) -> Result<()> {
    let index = sync_index(storage)?;

    // Execute the query
    let results = index
        .query_raw(sql)
        .with_context(|| format!("Failed to execute query: {}", sql))?;

    if results.is_empty() {
        if json_output {
            println!("[]");
        } else {
            println!("No results.");
        }
        return Ok(());
    }

    if json_output {
        println!("{}", serde_json::to_string_pretty(&results)?);
    } else {
        // Table output - extract column names from first result
        if let Some(first) = results.first() {
            if let serde_json::Value::Object(obj) = first {
                let cols: Vec<_> = obj.keys().collect();
                let columns: Vec<_> = cols
                    .iter()
                    .map(|col| TableColumn::expanded(col, 8))
                    .collect();
                let rows: Vec<Vec<String>> = results
                    .iter()
                    .filter_map(|row| match row {
                        serde_json::Value::Object(obj) => Some(
                            cols.iter()
                                .map(|col| {
                                    let value = obj.get(*col).unwrap_or(&serde_json::Value::Null);
                                    match value {
                                        serde_json::Value::String(text) => text.clone(),
                                        serde_json::Value::Null => "NULL".to_string(),
                                        _ => value.to_string(),
                                    }
                                })
                                .collect(),
                        ),
                        _ => None,
                    })
                    .collect();

                print_table(&columns, &rows);
            }
        }
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

    if run.runtime != "tmux" && run.runtime != "zellij" {
        bail!(
            "Attach is only supported for tmux/zellij runtime (current: {})",
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
    let layered = layered_storage(storage)?;
    let templates = layered.list_templates()?;
    if templates.is_empty() {
        println!("No templates found.");
        return Ok(());
    }

    let rows: Vec<Vec<String>> = templates
        .into_iter()
        .map(|template| vec![short_id(&template.template_id), template.name])
        .collect();

    print_table(
        &[
            TableColumn::left_with_max("ID", 8, 10),
            TableColumn::expanded("NAME", 12),
        ],
        &rows,
    );
    Ok(())
}
fn cmd_template_show(storage: &Storage, template_id: &str) -> Result<()> {
    let layered = layered_storage(storage)?;
    let resolved_id = layered.resolve_template_id(template_id)?;
    let (template, _) = layered.load_template(&resolved_id)?;
    println!("{}", serde_json::to_string_pretty(&template)?);
    Ok(())
}
fn cmd_template_create(storage: &Storage, path: &str, scope: Scope) -> Result<()> {
    let layered = layered_storage(storage)?;
    let content =
        std::fs::read_to_string(path).with_context(|| format!("Failed to read file: {}", path))?;
    // Validate first
    let validator = Validator::new()?;
    let value: serde_json::Value = serde_json::from_str(&content)?;
    validator.validate_template(&value)?;
    let template: RunTemplate = serde_json::from_str(&content)?;
    let saved_path = layered.save_template(&template, scope)?;
    println!(
        "Template created in {} scope: {}",
        scope_name(scope),
        saved_path.display()
    );
    Ok(())
}
fn cmd_template_delete(storage: &Storage, template_id: &str, scope: Scope) -> Result<()> {
    let layered = layered_storage(storage)?;
    let resolved_id = layered.resolve_template_id_in_scope(template_id, scope)?;
    layered.delete_template_in_scope(&resolved_id, scope)?;
    println!(
        "Template deleted from {} scope: {}",
        scope_name(scope),
        short_id(&resolved_id)
    );
    Ok(())
}
// === Playlist Commands ===
fn cmd_playlist_list(storage: &Storage) -> Result<()> {
    let layered = layered_storage(storage)?;
    let playlists = layered.list_playlists()?;
    if playlists.is_empty() {
        println!("No playlists found.");
        return Ok(());
    }

    let rows: Vec<Vec<String>> = playlists
        .into_iter()
        .map(|playlist| {
            vec![
                short_id(&playlist.playlist_id),
                playlist.name,
                playlist.items.len().to_string(),
            ]
        })
        .collect();

    print_table(
        &[
            TableColumn::left_with_max("ID", 8, 10),
            TableColumn::expanded("NAME", 12),
            TableColumn::right("ITEMS", 5),
        ],
        &rows,
    );
    Ok(())
}
fn cmd_playlist_show(
    storage: &Storage,
    playlist_id: Option<&str>,
    json_output: bool,
) -> Result<()> {
    let layered = layered_storage(storage)?;
    match playlist_id {
        Some(id) => {
            // Show specific playlist
            let resolved_id = layered.resolve_playlist_id(id)?;
            let (playlist, _) = layered.load_playlist(&resolved_id)?;

            if json_output {
                // JSON output (original behavior)
                println!("{}", serde_json::to_string_pretty(&playlist)?);
            } else {
                // Table view with short IDs
                println!("Playlist: {} ({})", playlist.playlist_id, playlist.name);
                println!();
                let rows: Vec<Vec<String>> = playlist
                    .items
                    .iter()
                    .enumerate()
                    .map(|(idx, item)| {
                        vec![
                            idx.to_string(),
                            item.short_id(&playlist.playlist_id, idx),
                            short_id(&item.template_id),
                            item.label.as_deref().unwrap_or("-").to_string(),
                        ]
                    })
                    .collect();

                print_table(
                    &[
                        TableColumn::right("IDX", 3),
                        TableColumn::left_with_max("SHORT", 8, 10),
                        TableColumn::left_with_max("TEMPLATE", 8, 15),
                        TableColumn::expanded("LABEL", 12),
                    ],
                    &rows,
                );

                if !playlist.items.is_empty() {
                    println!();
                    println!(
                        "Run with: runbox playlist run <SHORT> or runbox playlist run {} <IDX|SHORT>",
                        short_id(&playlist.playlist_id)
                    );
                }
            }
        }
        None => {
            // Show flattened view of all playlists
            let playlists = layered.list_playlists()?;

            if playlists.is_empty() {
                println!("No playlists found.");
                return Ok(());
            }

            if json_output {
                // JSON output - array of all playlists
                println!("{}", serde_json::to_string_pretty(&playlists)?);
            } else {
                // Flattened table view
                let mut has_items = false;
                let mut rows = Vec::new();
                for playlist in &playlists {
                    let playlist_short = short_id(&playlist.playlist_id);
                    for (idx, item) in playlist.items.iter().enumerate() {
                        has_items = true;
                        rows.push(vec![
                            playlist_short.clone(),
                            idx.to_string(),
                            item.short_id(&playlist.playlist_id, idx),
                            short_id(&item.template_id),
                            item.label.as_deref().unwrap_or("-").to_string(),
                        ]);
                    }
                }

                print_table(
                    &[
                        TableColumn::left_with_max("PLAYLIST", 8, 10),
                        TableColumn::right("IDX", 3),
                        TableColumn::left_with_max("SHORT", 8, 10),
                        TableColumn::left_with_max("TEMPLATE", 8, 15),
                        TableColumn::expanded("LABEL", 12),
                    ],
                    &rows,
                );

                if has_items {
                    println!();
                    println!("Run with: runbox playlist run <SHORT>");
                } else {
                    println!("(no items in any playlist)");
                }
            }
        }
    }

    Ok(())
}

fn cmd_playlist_create(storage: &Storage, path: &str, scope: Scope) -> Result<()> {
    let layered = layered_storage(storage)?;
    let content =
        std::fs::read_to_string(path).with_context(|| format!("Failed to read file: {}", path))?;
    // Validate first
    let validator = Validator::new()?;
    let value: serde_json::Value = serde_json::from_str(&content)?;
    validator.validate_playlist(&value)?;
    let playlist: Playlist = serde_json::from_str(&content)?;
    let saved_path = layered.save_playlist(&playlist, scope)?;
    println!(
        "Playlist created in {} scope: {}",
        scope_name(scope),
        saved_path.display()
    );
    Ok(())
}
fn cmd_playlist_add(
    storage: &Storage,
    playlist_id: &str,
    template_id: &str,
    label: Option<String>,
    scope: Scope,
) -> Result<()> {
    let layered = layered_storage(storage)?;
    let resolved_playlist_id = layered.resolve_playlist_id_in_scope(playlist_id, scope)?;
    let resolved_template_id = match scope {
        Scope::Local => layered.resolve_template_id(template_id)?,
        Scope::Global => layered.resolve_template_id_in_scope(template_id, Scope::Global)?,
    };
    let mut playlist = layered.load_playlist_in_scope(&resolved_playlist_id, scope)?;
    playlist.items.push(PlaylistItem {
        template_id: resolved_template_id.clone(),
        label,
    });
    layered.save_playlist(&playlist, scope)?;
    println!(
        "Added {} to {} in {} scope",
        short_id(&resolved_template_id),
        short_id(&resolved_playlist_id),
        scope_name(scope)
    );
    Ok(())
}
fn cmd_playlist_remove(
    storage: &Storage,
    playlist_id: &str,
    selector: &str,
    scope: Scope,
) -> Result<()> {
    let layered = layered_storage(storage)?;
    let resolved_playlist_id = layered.resolve_playlist_id_in_scope(playlist_id, scope)?;
    let mut playlist = layered.load_playlist_in_scope(&resolved_playlist_id, scope)?;
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
        layered.save_playlist(&playlist, scope)?;
        println!(
            "Removed {} from {} in {} scope",
            short_id(&removed.template_id),
            short_id(&resolved_playlist_id),
            scope_name(scope)
        );
        return Ok(());
    }
    let resolved_template_id = match scope {
        Scope::Local => layered.resolve_template_id(selector)?,
        Scope::Global => layered.resolve_template_id_in_scope(selector, Scope::Global)?,
    };
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
    layered.save_playlist(&playlist, scope)?;
    println!(
        "Removed {} from {} in {} scope",
        short_id(&resolved_template_id),
        short_id(&resolved_playlist_id),
        scope_name(scope)
    );
    Ok(())
}

fn cmd_playlist_run(
    storage: &Storage,
    selector: &str,
    item_selector: Option<&str>,
    bindings: Vec<String>,
    runtime: RuntimeType,
    dry_run: bool,
) -> Result<()> {
    let layered = layered_storage(storage)?;
    // Determine if we're using global short ID or playlist + item
    let (playlist, item_idx, item) = match item_selector {
        Some(item_sel) => {
            // Two arguments: selector is playlist_id, item_sel is index/short ID
            let resolved_playlist_id = layered.resolve_playlist_id(selector)?;
            let (playlist, _) = layered.load_playlist(&resolved_playlist_id)?;

            let (idx, found_item) = playlist.resolve_item(item_sel).with_context(|| {
                format!(
                    "Item '{}' not found in playlist '{}'. Use index (0, 1, ...) or short ID.",
                    item_sel,
                    short_id(&resolved_playlist_id)
                )
            })?;

            let item = found_item.clone();
            (playlist, idx, item)
        }
        None => {
            // One argument: selector is a global short ID
            // Search across all playlists
            let playlists = layered.list_playlists()?;
            let selector_lower = selector.to_lowercase();

            let mut matches: Vec<(Playlist, usize, PlaylistItem)> = Vec::new();

            for playlist in playlists {
                for (idx, item) in playlist.items.iter().enumerate() {
                    let item_short = item.short_id(&playlist.playlist_id, idx);
                    if item_short.starts_with(&selector_lower) {
                        matches.push((playlist.clone(), idx, item.clone()));
                        break; // Found a match in this playlist, move to next
                    }
                }
            }

            match matches.len() {
                0 => bail!(
                    "No item found matching '{}'. Use 'runbox playlist show' to see available items.",
                    selector
                ),
                1 => {
                    let (playlist, idx, item) = matches.into_iter().next().unwrap();
                    (playlist, idx, item)
                }
                _ => {
                    eprintln!("Ambiguous short ID '{}' matches multiple items:", selector);
                    for (playlist, idx, item) in &matches {
                        let item_short = item.short_id(&playlist.playlist_id, *idx);
                        eprintln!(
                            "  {} in playlist {} (index {})",
                            item_short,
                            short_id(&playlist.playlist_id),
                            idx
                        );
                    }
                    bail!("Use more characters to disambiguate, or specify playlist: runbox playlist run <playlist> <item>");
                }
            }
        }
    };

    let item_short = item.short_id(&playlist.playlist_id, item_idx);

    // Load the template
    let (template, _) = layered.load_template(&item.template_id)?;

    if dry_run {
        println!("Would run template: {}", item.template_id);
        println!(
            "  Playlist: {} ({})",
            short_id(&playlist.playlist_id),
            playlist.name
        );
        println!(
            "  Item: {} (index {}, short ID {})",
            item.label.as_deref().unwrap_or("-"),
            item_idx,
            item_short
        );
        println!("  argv: {:?}", template.exec.argv);
        println!("  cwd: {}", template.exec.cwd);
        println!("  runtime: {}", runtime);
        if !bindings.is_empty() {
            println!("  bindings: {:?}", bindings);
        }
        return Ok(());
    }

    // Delegate to the existing template run logic
    cmd_run_template(storage, &item.template_id, bindings, runtime, dry_run)
}

// === History Commands ===
fn cmd_history(storage: &Storage, limit: usize) -> Result<()> {
    let runs = storage.list_runs(limit)?;
    if runs.is_empty() {
        println!("No runs found.");
        return Ok(());
    }

    let rows: Vec<Vec<String>> = runs
        .into_iter()
        .map(|run| vec![short_id(&run.run_id), run.exec.argv.join(" ")])
        .collect();

    print_table(
        &[
            TableColumn::left_with_max("ID", 8, 10),
            TableColumn::expanded("COMMAND", 12),
        ],
        &rows,
    );
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

    let rows: Vec<Vec<String>> = results
        .into_iter()
        .map(|result| {
            vec![
                result.short_id().to_string(),
                short_id(&result.run_id),
                result.execution.exit_code.to_string(),
                format!("{}ms", result.execution.duration_ms),
                result.artifacts.len().to_string(),
            ]
        })
        .collect();

    print_table(
        &[
            TableColumn::left_with_max("RESULT ID", 8, 12),
            TableColumn::left_with_max("RUN ID", 8, 12),
            TableColumn::right("EXIT", 4),
            TableColumn::right("DURATION", 8),
            TableColumn::right("ARTIFACTS", 9),
        ],
        &rows,
    );
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

    let rows: Vec<Vec<String>> = results
        .into_iter()
        .map(|result| {
            vec![
                result.short_id().to_string(),
                result.execution.exit_code.to_string(),
                format!("{}ms", result.execution.duration_ms),
                result
                    .execution
                    .finished_at
                    .format("%Y-%m-%d %H:%M:%S")
                    .to_string(),
            ]
        })
        .collect();

    print_table(
        &[
            TableColumn::left_with_max("RESULT ID", 8, 12),
            TableColumn::right("EXIT", 4),
            TableColumn::right("DURATION", 8),
            TableColumn::expanded("FINISHED", 19),
        ],
        &rows,
    );
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
    replay_id: &str,
    worktree_dir: Option<PathBuf>,
    keep: bool,
    cleanup: bool,
    ignore_patch: bool,
    reuse: bool,
    fresh: bool,
    verbose: u8,
) -> Result<()> {
    let mut replay = load_replay_spec(storage, replay_id)?;
    let had_patch = replay.code_state.patch.is_some();
    if ignore_patch {
        replay.code_state.patch = None;
    }
    // Initialize git context from current directory
    let git = git_context_for_replay(&replay)?;
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
    println!("Replaying: {}", replay.id);
    println!("Command: {:?}", replay.argv);
    println!("Commit: {}", replay.code_state.base_commit);
    if ignore_patch && had_patch {
        println!("Patch: ignored");
    } else if replay.code_state.patch.is_some() {
        println!("Patch: yes");
    }
    // Restore code state in worktree
    let worktree_result = git.restore_code_state_in_worktree(
        &replay.code_state,
        &replay.id,
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
    let exec_cwd = if Path::new(&replay.cwd).is_absolute() {
        // If cwd is absolute, make it relative to worktree
        PathBuf::from(&replay.cwd)
    } else {
        // Relative path - combine with worktree
        worktree_result.worktree_path.join(&replay.cwd)
    };
    logger.log_vv("exec", &format!("cwd: {}", exec_cwd.display()));
    logger.log_vv("exec", &format!("argv: {:?}", replay.argv));
    if !replay.env.is_empty() {
        logger.log_vvv("exec", &format!("env: {:?}", replay.env));
    }
    // Execute
    println!("\nExecuting...");
    let status = Command::new(&replay.argv[0])
        .args(&replay.argv[1..])
        .current_dir(&exec_cwd)
        .envs(&replay.env)
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

/// Create a record from JSON input
fn cmd_create_record(storage: &Storage, from_file: Option<String>) -> Result<()> {
    use std::io::Read;

    // Read JSON from file or stdin
    let json_str = if let Some(file_path) = from_file {
        std::fs::read_to_string(&file_path)
            .with_context(|| format!("Failed to read file: {}", file_path))?
    } else {
        let mut buffer = String::new();
        std::io::stdin()
            .read_to_string(&mut buffer)
            .context("Failed to read from stdin")?;
        buffer
    };

    // Parse JSON
    let json: serde_json::Value = serde_json::from_str(&json_str).context("Invalid JSON")?;

    // Extract or generate record_id
    let record_id = json
        .get("id")
        .or_else(|| json.get("record_id"))
        .and_then(|v| v.as_str())
        .map(String::from)
        .unwrap_or_else(|| format!("rec_{}", uuid::Uuid::new_v4()));

    // Extract command (required)
    let command = json.get("command").context("Missing 'command' field")?;
    let argv: Vec<String> = command
        .get("argv")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();
    if argv.is_empty() {
        bail!("Command argv must not be empty");
    }
    let cwd = command
        .get("cwd")
        .and_then(|v| v.as_str())
        .unwrap_or(".")
        .to_string();
    let env = command
        .get("env")
        .and_then(|v| v.as_object())
        .map(|obj| {
            obj.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default();

    let record_command = RecordCommand { argv, cwd, env };

    // Extract git_state (optional but recommended)
    let git_state = if let Some(gs) = json.get("git_state") {
        let repo_url = gs
            .get("repo_url")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        let commit = gs
            .get("commit")
            .and_then(|v| v.as_str())
            .unwrap_or(&"0".repeat(40))
            .to_string();
        let patch_ref = gs
            .get("patch_ref")
            .and_then(|v| v.as_str())
            .map(String::from);
        let diff = gs.get("diff").and_then(|v| v.as_str());

        if patch_ref.is_some() && diff.is_some() {
            bail!("git_state.patch_ref and git_state.diff are mutually exclusive");
        }

        let patch_ref = if let Some(diff) = diff {
            Some(
                git_context_for_record_diff(&record_command.cwd)?
                    .create_patch_ref_from_diff(&record_id, diff)?
                    .ref_,
            )
        } else {
            patch_ref
        };

        RecordGitState {
            repo_url,
            commit,
            patch_ref,
        }
    } else {
        // Try to capture current git context
        match GitContext::from_current_dir() {
            Ok(ctx) => RecordGitState {
                repo_url: ctx.get_remote_url().unwrap_or("unknown".to_string()),
                commit: ctx.get_head_commit().unwrap_or("0".repeat(40)),
                patch_ref: None,
            },
            Err(_) => RecordGitState {
                repo_url: "unknown".to_string(),
                commit: "0".repeat(40),
                patch_ref: None,
            },
        }
    };

    // Create the record
    let mut record = Record::with_id(record_id.clone(), git_state, record_command);

    // Extract optional fields
    if let Some(exit_code) = json.get("exit_code").and_then(|v| v.as_i64()) {
        record.exit_code = Some(exit_code as i32);
    }
    if let Some(started_at) = json.get("started_at").and_then(|v| v.as_str()) {
        record.started_at = chrono::DateTime::parse_from_rfc3339(started_at)
            .ok()
            .map(|dt| dt.with_timezone(&chrono::Utc));
    }
    if let Some(ended_at) = json.get("ended_at").and_then(|v| v.as_str()) {
        record.ended_at = chrono::DateTime::parse_from_rfc3339(ended_at)
            .ok()
            .map(|dt| dt.with_timezone(&chrono::Utc));
    }
    if let Some(tags) = json.get("tags").and_then(|v| v.as_array()) {
        record.tags = tags
            .iter()
            .filter_map(|v| v.as_str())
            .map(String::from)
            .collect();
    }
    if let Some(source) = json.get("source").and_then(|v| v.as_str()) {
        record.source = source.to_string();
    } else {
        record.source = "external".to_string();
    }
    if let Some(log_ref) = json.get("log_ref").and_then(|v| v.as_str()) {
        record.log_ref = Some(log_ref.to_string());
    }

    // Validate the record
    record.validate().context("Record validation failed")?;

    // Save the record
    let saved_path = storage.save_record(&record)?;

    println!("Created record: {}", record.record_id);
    println!("  Short ID: {}", record.short_id());
    println!("  Command:  {:?}", record.command.argv);
    println!("  Source:   {}", record.source);
    println!("  Path:     {}", saved_path.display());

    Ok(())
}

fn cmd_skill_list() -> Result<()> {
    let skills = find_skills();

    if skills.is_empty() {
        println!("No skills found.");
        println!();
        println!("Skills are searched in:");
        for platform in Platform::all() {
            if let Some(dir) = platform.skill_dir() {
                println!("  {} - {}", platform.name(), dir.display());
            }
        }
        return Ok(());
    }

    let rows: Vec<Vec<String>> = skills
        .iter()
        .map(|(platform, path, name)| {
            vec![
                name.clone(),
                platform.name().to_string(),
                path.to_string_lossy().to_string(),
            ]
        })
        .collect();

    print_table(
        &[
            TableColumn::left_with_max("NAME", 10, 25),
            TableColumn::left_with_max("PLATFORM", 8, 15),
            TableColumn::expanded("PATH", 16),
        ],
        &rows,
    );

    println!();
    println!("{} skill(s) found", skills.len());

    Ok(())
}

fn cmd_skill_show(skill_name: &str) -> Result<()> {
    let (platform, path) = find_skill_by_name(skill_name)
        .ok_or_else(|| anyhow::anyhow!("Skill not found: {}", skill_name))?;

    let skill = Skill::load(&path)?;

    println!("Skill: {}", skill.metadata.name);
    println!("Platform: {}", platform.name());
    println!("Path: {}", path.display());
    if let Some(ref version) = skill.metadata.version {
        println!("Version: {}", version);
    }
    println!();
    println!("Description:");
    println!("  {}", skill.metadata.description);
    println!();

    if !skill.references.is_empty() {
        println!("References ({}):", skill.references.len());
        for ref_path in &skill.references {
            println!("  - {}", ref_path.display());
        }
        println!();
    }

    if !skill.examples.is_empty() {
        println!("Examples ({}):", skill.examples.len());
        for ex_path in &skill.examples {
            println!("  - {}", ex_path.display());
        }
        println!();
    }

    println!("Content preview (first 20 lines):");
    println!("{}", "─".repeat(60));
    for (i, line) in skill.content.lines().take(20).enumerate() {
        println!("{:3} │ {}", i + 1, line);
    }
    if skill.content.lines().count() > 20 {
        println!("... ({} more lines)", skill.content.lines().count() - 20);
    }

    Ok(())
}

fn cmd_skill_export(skill_name: &str, output: Option<PathBuf>) -> Result<()> {
    let (_platform, path) = find_skill_by_name(skill_name)
        .ok_or_else(|| anyhow::anyhow!("Skill not found: {}", skill_name))?;

    let skill = Skill::load(&path)?;
    let output_dir = output.unwrap_or_else(|| PathBuf::from(skill_name));

    println!("Exporting skill: {}", skill.metadata.name);
    println!("Source: {}", path.display());
    println!("Output: {}", output_dir.display());
    println!();

    let result = skill.export(&output_dir)?;

    println!("Export complete!");
    println!();
    println!("Created files:");
    println!("  SKILL.md           - Main skill file");
    if result.references_count > 0 {
        println!(
            "  references/        - {} reference file(s)",
            result.references_count
        );
    }
    if result.examples_count > 0 {
        println!(
            "  examples/          - {} example file(s)",
            result.examples_count
        );
    }
    println!("  INSTALL.md         - Unified installation guide");
    println!("  install/           - Platform-specific guides");
    println!("    claude-code.md");
    println!("    opencode.md");
    println!("    gemini.md");
    println!("    codex.md");
    println!("    cursor.md");
    println!("  install.sh         - Auto-install script");
    println!();
    println!("To install, run:");
    println!("  cd {} && ./install.sh", output_dir.display());

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_string_respects_full_width_display_cells() {
        let truncated = truncate_string("日本語の幅を確認する", 11);
        assert_eq!(display_width(&truncated), 11);
        assert!(truncated.ends_with("..."));
    }

    #[test]
    fn truncate_string_preserves_grapheme_clusters() {
        let truncated = truncate_string("👩‍💻👩‍💻abc", 6);
        assert_eq!(truncated, "👩‍💻...");
        assert!(display_width(&truncated) <= 6);
    }

    #[test]
    fn compute_table_widths_can_shrink_below_preferred_widths() {
        let columns = [
            TableColumn::left_with_max("SHORT ID", 8, 12),
            TableColumn::left_with_max("STATUS", 7, 10),
            TableColumn::left_with_max("RUNTIME", 8, 10),
            TableColumn::expanded("COMMAND", 12),
        ];
        let rows = vec![vec![
            "deadbeef".to_string(),
            "exited".to_string(),
            "background".to_string(),
            "python -m very.long.module".to_string(),
        ]];

        let widths = compute_table_widths(&columns, &rows, 12, 1);
        assert_eq!(widths.iter().sum::<usize>() + columns.len() - 1, 12);
    }

    #[test]
    fn table_separator_width_drops_spacing_when_terminal_is_too_narrow() {
        assert_eq!(table_separator_width(4, 2), 0);
        assert_eq!(table_separator_width(4, 4), 1);
    }
}
