# Runbox Tutorial

Runbox is a reproducible command execution system. It captures command executions with full git context, stores them for later reference, and allows you to replay them in isolated git worktrees with the exact same code state.

## Table of Contents

1. [Installation](#installation)
2. [Quick Start](#quick-start)
3. [Core Concepts](#core-concepts)
4. [Templates](#templates)
5. [Running Commands](#running-commands)
6. [Monitoring and Logs](#monitoring-and-logs)
7. [Playlists](#playlists)
8. [Replay](#replay)
9. [Configuration](#configuration)
10. [Troubleshooting](#troubleshooting)

---

## Installation

Build from source:

```bash
cd runbox
cargo build --release

# Add to PATH
export PATH="$PWD/target/release:$PATH"
```

Verify installation:

```bash
runbox --help
```

---

## Quick Start

### 1. Create a Template

Create a file `my_template.json`:

```json
{
  "template_id": "tpl_hello",
  "name": "Hello World",
  "exec": {
    "argv": ["echo", "Hello, {name}!"],
    "cwd": ".",
    "env": {},
    "timeout_sec": 60
  },
  "bindings": {
    "defaults": {
      "name": "World"
    },
    "interactive": []
  },
  "code_state": {
    "repo_url": "git@github.com:your/repo.git"
  }
}
```

Register it:

```bash
runbox template create my_template.json
```

### 2. Run the Template

```bash
# Use default binding
runbox run --template tpl_hello

# Override the binding
runbox run --template tpl_hello --binding name=Runbox
```

### 3. Check Status

```bash
runbox ps
```

### 4. View Logs

```bash
runbox logs <run_id>
```

---

## Core Concepts

### Run

A **Run** is a fully-resolved execution record. It contains:

- **exec**: The command to execute (argv, cwd, env, timeout)
- **code_state**: Git context (commit + optional patch for uncommitted changes)
- **status**: Current state (pending, running, exited, failed, killed)
- **timeline**: Timestamps for creation, start, and end

Runs are immutable records of what was executed and in what state.

### RunTemplate

A **RunTemplate** is a parametrized blueprint for creating Runs. It supports:

- **Template variables**: `{variable_name}` syntax in argv and env values
- **Default bindings**: Fallback values for variables
- **Interactive bindings**: Variables to prompt the user for at runtime

### Playlist

A **Playlist** is a collection of templates for batch execution or organization.

### Runtimes

Runbox supports multiple execution runtimes:

| Runtime | Description | Use Case |
|---------|-------------|----------|
| `background` (or `bg`) | Detached background process | Long-running tasks, scripts |
| `tmux` | Tmux window session | Interactive debugging |

---

## Templates

### Creating Templates

Templates use JSON format with the following structure:

```json
{
  "template_id": "tpl_<unique_name>",
  "name": "Human Readable Name",
  "exec": {
    "argv": ["command", "arg1", "{variable}"],
    "cwd": ".",
    "env": {
      "ENV_VAR": "value",
      "DYNAMIC_VAR": "{another_variable}"
    },
    "timeout_sec": 3600
  },
  "bindings": {
    "defaults": {
      "variable": "default_value",
      "another_variable": "default"
    },
    "interactive": ["variable"]
  },
  "code_state": {
    "repo_url": "git@github.com:org/repo.git"
  }
}
```

**Rules:**
- `template_id` must start with `tpl_`
- Variables use `{name}` syntax
- `interactive` variables will prompt the user if not provided via `--binding`

### Managing Templates

```bash
# List all templates
runbox template list

# Show template details
runbox template show tpl_hello

# Create from JSON file
runbox template create path/to/template.json

# Delete a template
runbox template delete tpl_hello
```

### Example: ML Training Template

```json
{
  "template_id": "tpl_train_model",
  "name": "Train ML Model",
  "exec": {
    "argv": [
      "uv", "run", "python", "-m", "trainer",
      "--epochs", "{epochs}",
      "--lr", "{learning_rate}",
      "--model", "{model}"
    ],
    "cwd": ".",
    "env": {
      "CUDA_VISIBLE_DEVICES": "{gpu}",
      "WANDB_PROJECT": "my-project"
    },
    "timeout_sec": 86400
  },
  "bindings": {
    "defaults": {
      "epochs": "100",
      "learning_rate": "0.001",
      "model": "resnet50",
      "gpu": "0"
    },
    "interactive": ["epochs", "model"]
  },
  "code_state": {
    "repo_url": "git@github.com:org/ml-repo.git"
  }
}
```

---

## Running Commands

### Basic Execution

```bash
# Run with defaults
runbox run --template tpl_train_model

# Override specific bindings
runbox run --template tpl_train_model --binding epochs=200 --binding gpu=1

# Multiple bindings
runbox run --template tpl_train_model \
  --binding epochs=50 \
  --binding learning_rate=0.0001 \
  --binding model=vit_base
```

### Runtime Options

```bash
# Background execution (default)
runbox run --template tpl_hello --runtime bg

# Tmux session (interactive)
runbox run --template tpl_hello --runtime tmux
```

### Dry Run

Preview what would be executed without actually running:

```bash
runbox run --template tpl_train_model --binding epochs=10 --dry-run
```

---

## Monitoring and Logs

### List Runs

```bash
# List recent runs
runbox ps

# Filter by status
runbox ps --status running
runbox ps --status exited

# Limit results
runbox ps --limit 5
```

### View Run Details

```bash
# Full details
runbox show <run_id>

# Short IDs work too (first 8 characters)
runbox show 550e8400
```

### View Logs

```bash
# Show all logs
runbox logs <run_id>

# Follow logs (like tail -f)
runbox logs <run_id> --follow

# Show last N lines
runbox logs <run_id> --lines 50
```

### Stop a Run

```bash
# Graceful stop (SIGTERM)
runbox stop <run_id>

# Force stop (SIGKILL)
runbox stop <run_id> --force
```

### Attach to Tmux Session

```bash
runbox attach <run_id>
```

### View History

```bash
# Recent run history
runbox history

# Limit results
runbox history --limit 20
```

---

## Playlists

Playlists organize multiple templates for batch workflows.

### Creating a Playlist

Create `daily_tasks.json`:

```json
{
  "playlist_id": "pl_daily",
  "name": "Daily Tasks",
  "items": [
    {"template_id": "tpl_sync_data", "label": "Sync Data"},
    {"template_id": "tpl_train_model", "label": "Train Model"},
    {"template_id": "tpl_evaluate", "label": "Evaluate Results"}
  ]
}
```

Register it:

```bash
runbox playlist create daily_tasks.json
```

### Managing Playlists

```bash
# List all playlists
runbox playlist list

# Show playlist details
runbox playlist show pl_daily

# Add a template to playlist
runbox playlist add pl_daily tpl_backup --label "Backup Data"

# Remove a template from playlist
runbox playlist remove pl_daily tpl_backup
```

---

## Replay

Replay allows you to re-execute a previous run in an isolated git worktree with the exact same code state.

### Basic Replay

```bash
runbox replay <run_id>
```

This will:
1. Create a git worktree at the original commit
2. Apply any uncommitted changes (patch) if present
3. Execute the same command

### Replay Options

```bash
# Specify worktree directory
runbox replay <run_id> --worktree-dir /tmp/replay

# Keep worktree after completion (default)
runbox replay <run_id> --keep

# Clean up worktree after completion
runbox replay <run_id> --cleanup

# Reuse existing worktree if commit matches (default)
runbox replay <run_id> --reuse

# Always create fresh worktree
runbox replay <run_id> --fresh

# Verbose output
runbox replay <run_id> -v      # Level 1
runbox replay <run_id> -vv     # Level 2
runbox replay <run_id> -vvv    # Level 3
```

### How Code State is Captured

When you run a command, Runbox captures:

1. **base_commit**: The current HEAD commit hash
2. **patch** (optional): If there are uncommitted changes, they're saved as a git patch

This allows exact reproduction of the code state, even with uncommitted changes.

---

## Configuration

### Storage Location

By default, Runbox stores data in:

```
~/.local/share/runbox/
├── runs/           # Run records (JSON)
├── templates/      # Template definitions (JSON)
├── playlists/      # Playlist definitions (JSON)
└── logs/           # Execution logs
```

Override with environment variable:

```bash
export RUNBOX_HOME=/custom/path
```

### Git Configuration

Configure default worktree directory:

```bash
# Repository-level
git config runbox.worktree-dir /path/to/worktrees

# Global
git config --global runbox.worktree-dir ~/runbox-worktrees
```

---

## Troubleshooting

### Validate JSON Files

Check if your template/run/playlist JSON is valid:

```bash
runbox validate path/to/file.json
```

Runbox auto-detects the type based on the ID field prefix.

### Short ID Ambiguity

If a short ID matches multiple items:

```
Error: Ambiguous: 2 items match '5a'. Use more characters.
  - 5aaa0000
  - 5abb1234
```

Solution: Use more characters to disambiguate.

### Daemon Issues

For background processes, Runbox uses a daemon to track exit status:

```bash
# Check daemon status
runbox daemon status

# Restart daemon
runbox daemon stop
runbox daemon start

# Ping daemon
runbox daemon ping
```

### Common Status Values

| Status | Meaning |
|--------|---------|
| `pending` | Created but not yet started |
| `running` | Currently executing |
| `exited` | Completed with exit code 0 |
| `failed` | Completed with non-zero exit code |
| `killed` | Terminated by user (SIGTERM/SIGKILL) |
| `unknown` | Status cannot be determined |

### Logs Not Appearing

1. Check if the run is still in `pending` status
2. Verify the log file exists: `runbox show <run_id>` shows the log path
3. For tmux runs, use `runbox attach` instead

### Worktree Issues

If replay fails with worktree errors:

```bash
# List existing worktrees
git worktree list

# Remove stale worktrees
git worktree prune
```

---

## Examples

### Scientific Computing Workflow

```bash
# Create experiment template
cat > experiment.json << 'EOF'
{
  "template_id": "tpl_experiment",
  "name": "Run Experiment",
  "exec": {
    "argv": ["python", "experiment.py", "--seed", "{seed}", "--config", "{config}"],
    "cwd": ".",
    "env": {"PYTHONUNBUFFERED": "1"},
    "timeout_sec": 7200
  },
  "bindings": {
    "defaults": {"seed": "42", "config": "default.yaml"},
    "interactive": ["seed"]
  },
  "code_state": {"repo_url": "git@github.com:lab/experiments.git"}
}
EOF

runbox template create experiment.json

# Run multiple experiments
for seed in 1 2 3 4 5; do
  runbox run --template tpl_experiment --binding seed=$seed --runtime bg
done

# Monitor all runs
watch runbox ps

# Later, replay a successful run
runbox replay <run_id_of_best_result>
```

### CI/CD Integration

```bash
# Run tests with captured state
runbox run --template tpl_test_suite --runtime bg

# Wait and check result
run_id=$(runbox ps --limit 1 | tail -1 | awk '{print $1}')
runbox logs $run_id --follow

# If tests pass, the code state is reproducible
runbox show $run_id
```

---

## Summary

| Command | Purpose |
|---------|---------|
| `runbox run` | Execute a template |
| `runbox ps` | List runs |
| `runbox stop` | Stop a run |
| `runbox logs` | View run logs |
| `runbox attach` | Attach to tmux session |
| `runbox show` | Show run details |
| `runbox history` | Show run history |
| `runbox replay` | Replay a run |
| `runbox template *` | Manage templates |
| `runbox playlist *` | Manage playlists |
| `runbox validate` | Validate JSON files |
| `runbox daemon *` | Manage daemon |

For more details on any command:

```bash
runbox <command> --help
```
