# Runbox CLI Skill for OpenCode/Claude

This skill teaches AI assistants how to use runbox CLI commands for reproducible command execution.

## Trigger Phrases

This skill should be used when the user asks to:
- "run a template", "create a run template", "manage playlists"
- "replay a run", "show run history", "validate runbox JSON"
- "configure runbox", "set worktree directory", "runbox verbose output"
- "list running jobs", "stop a run", "view logs", "attach to tmux"
- "runbox ps", "runbox logs", "runbox stop", "runbox attach"

Or mentions:
- runbox CLI commands
- reproducible execution
- code state capture
- worktree-based replay
- run execution management

---

## Core Concepts

### Run
A **Run** is a fully-resolved, reproducible execution record containing:
- **exec**: Command to execute (argv, cwd, env, timeout_sec)
- **code_state**: Git state (repo_url, base_commit, optional patch for uncommitted changes)
- **run_id**: Unique identifier (format: `run_<uuid>`)
- **status**: Execution state (`pending`, `running`, `exited`, `failed`, `killed`, `unknown`)
- **runtime**: Execution backend ("background" or "tmux")
- **timeline**: Timestamps (created_at, started_at, ended_at)

### RunTemplate
A **RunTemplate** is a parameterized blueprint for creating Runs:
- **exec**: Command with `{variable}` placeholders in argv/env
- **bindings**: Variable resolution (defaults, interactive prompts)
- **code_state**: Repository URL (commit captured at runtime)
- **template_id**: Must start with `tpl_` prefix

### Playlist
A **Playlist** is a collection of templates for batch execution:
- **items**: List of template references with optional labels
- **playlist_id**: Must start with `pl_` prefix

### Short ID Support
All IDs support git-style prefix matching:
- Full ID: `run_550e8400-e29b-41d4-a716-446655440000` -> Short: `550e8400` or `550e`
- Template: `tpl_plc_runner` -> Short: `plc_runner` or `plc`
- Playlist: `pl_daily_tasks` -> Short: `daily_tasks` or `daily`

---

## CLI Commands

### Direct Execution

Run any command directly and capture git context:

```bash
# Execute a command directly (-- separates runbox args from command)
runbox run -- echo "Hello, World!"
runbox run -- python train.py --epochs 10
runbox run -- make test

# Using the 'log' alias (same as run)
runbox log -- npm run build

# With options
runbox run --runtime tmux -- python debug.py      # Interactive tmux session
runbox run --timeout 3600 -- ./long_job.sh        # Set timeout (seconds)
runbox run --env CUDA_VISIBLE_DEVICES=0 -- python train.py  # Environment vars
runbox run --cwd /path/to/project -- npm test     # Working directory
runbox run --no-git -- echo "skip git capture"    # Skip git context
runbox run --dry-run -- python train.py           # Preview without executing
```

### Template-Based Execution

Run from pre-defined templates with variable bindings:

```bash
# Run with default bindings
runbox run --template tpl_train_model

# Override specific bindings
runbox run --template tpl_train_model --binding epochs=200 --binding gpu=1

# Short flags
runbox run -t tpl_runner -b i=42 -b name=experiment

# Dry run (preview)
runbox run -t tpl_runner --dry-run

# Select runtime
runbox run -t tpl_runner --runtime bg      # Background (default)
runbox run -t tpl_runner --runtime tmux    # Tmux window
```

### Monitoring Runs

```bash
# List runs (recent and running)
runbox ps

# Filter by status
runbox ps --status running
runbox ps --status running,failed
runbox ps --status exited

# Limit results
runbox ps --limit 5

# Show run details (short ID supported)
runbox show <run_id>
runbox show 550e8400     # Short ID
```

### Viewing Logs

```bash
# Show all logs
runbox logs <run_id>

# Follow logs (like tail -f)
runbox logs <run_id> --follow
runbox logs -f <run_id>

# Show last N lines
runbox logs <run_id> --lines 50
```

### Stopping Runs

```bash
# Graceful stop (SIGTERM)
runbox stop <run_id>

# Force stop (SIGKILL)
runbox stop <run_id> --force
runbox stop --force <run_id>
```

### Attaching to Tmux Sessions

```bash
# Attach to a tmux run (tmux runtime only)
runbox attach <run_id>
```

### Run History

```bash
# Show recent runs (default: 10)
runbox history

# Show more runs
runbox history --limit 50
```

### Template Management

```bash
# List all templates
runbox template list

# Show template details (short ID supported)
runbox template show <template_id>
runbox template show plc    # Matches tpl_plc_runner

# Create template from JSON file
runbox template create path/to/template.json

# Delete a template
runbox template delete <template_id>
```

### Playlist Management

```bash
# List all playlists
runbox playlist list

# Show playlist contents (short ID supported)
runbox playlist show <playlist_id>
runbox playlist show daily    # Matches pl_daily_tasks

# Create playlist from JSON
runbox playlist create path/to/playlist.json

# Add template to playlist (with optional label)
runbox playlist add <playlist_id> <template_id> --label "My Label"

# Remove template from playlist
runbox playlist remove <playlist_id> <template_id>
```

### Replay (Reproduce Past Runs)

Replay uses git worktrees for isolated execution without affecting working directory:

```bash
# Basic replay
runbox replay <run_id>
runbox replay 550e    # Short ID

# With options
runbox replay <run_id> --worktree-dir /tmp/replay  # Custom worktree location
runbox replay <run_id> --keep                       # Keep worktree after (default)
runbox replay <run_id> --cleanup                    # Remove worktree after
runbox replay <run_id> --reuse                      # Reuse existing worktree (default)
runbox replay <run_id> --fresh                      # Always create fresh worktree

# Verbose output
runbox replay <run_id> -v      # Level 1
runbox replay <run_id> -vv     # Level 2  
runbox replay <run_id> -vvv    # Level 3
```

### Validation

```bash
# Validate any runbox JSON file (auto-detects type by ID prefix)
runbox validate path/to/file.json
```

### Daemon Management

```bash
# Check daemon status
runbox daemon status

# Start/stop daemon
runbox daemon start
runbox daemon stop

# Ping daemon
runbox daemon ping
```

---

## Run Status Values

| Status | Description |
|--------|-------------|
| `pending` | Created but not yet started |
| `running` | Currently executing |
| `exited` | Completed successfully (exit_code == 0) |
| `failed` | Completed with error (exit_code != 0) |
| `killed` | Manually stopped via `runbox stop` |
| `unknown` | Lost track of process |

---

## Variable Binding Resolution

When running a template, variables resolve in priority order:

1. **Provided** (`--binding key=value`) - Highest priority
2. **Interactive** - Prompt user if variable is in `bindings.interactive`
3. **Defaults** - Use value from `bindings.defaults`

If a variable has no resolution path, the run fails with an error.

---

## Code State Capture

When executing `runbox run`:

1. Captures current HEAD commit as `base_commit`
2. If uncommitted changes exist:
   - Creates a patch
   - Stores as local git ref `refs/patches/<run_id>`
   - Records patch ref and SHA256 in `code_state.patch`
3. Saves complete Run JSON to storage

This enables exact reproduction via `runbox replay`.

---

## Storage Locations

Runbox stores data in XDG directories:

```
~/.local/share/runbox/
├── runs/           # Run records (JSON)
├── templates/      # Template definitions (JSON)
├── playlists/      # Playlist definitions (JSON)
└── logs/           # Execution logs
```

Override with: `export RUNBOX_HOME=/custom/path`

---

## JSON Schemas

### RunTemplate Example

```json
{
  "template_id": "tpl_train_model",
  "name": "Train ML Model",
  "exec": {
    "argv": ["python", "-m", "trainer", "--epochs", "{epochs}", "--lr", "{lr}"],
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
      "lr": "0.001",
      "gpu": "0"
    },
    "interactive": ["epochs"]
  },
  "code_state": {
    "repo_url": "git@github.com:org/repo.git"
  }
}
```

### Playlist Example

```json
{
  "playlist_id": "pl_daily",
  "name": "Daily Tasks",
  "items": [
    {"template_id": "tpl_sync_data", "label": "Sync Data"},
    {"template_id": "tpl_train_model", "label": "Train Model"},
    {"template_id": "tpl_evaluate", "label": "Evaluate"}
  ]
}
```

---

## Common Workflows

### Quick Ad-hoc Execution

```bash
# Run a quick test and track it
runbox run -- pytest tests/ -v

# Execute with specific environment
runbox run --env CUDA_VISIBLE_DEVICES=1 -- python train.py

# Interactive debugging in tmux
runbox run --runtime tmux -- python -m pdb script.py

# Check results
runbox ps
runbox logs <run_id>

# Later, reproduce exactly
runbox replay <run_id>
```

### Create and Run a Template

```bash
# 1. Create template JSON
cat > my_template.json << 'EOF'
{
  "template_id": "tpl_experiment",
  "name": "Run Experiment",
  "exec": {
    "argv": ["python", "experiment.py", "--seed", "{seed}"],
    "cwd": ".",
    "env": {"PYTHONUNBUFFERED": "1"},
    "timeout_sec": 7200
  },
  "bindings": {
    "defaults": {"seed": "42"},
    "interactive": ["seed"]
  },
  "code_state": {"repo_url": "git@github.com:org/repo.git"}
}
EOF

# 2. Register template
runbox template create my_template.json

# 3. Run it
runbox run -t tpl_experiment
runbox run -t tpl_experiment --binding seed=123
```

### Batch Experiments

```bash
# Run multiple experiments with different seeds
for seed in 1 2 3 4 5; do
  runbox run -t tpl_experiment --binding seed=$seed --runtime bg
done

# Monitor all runs
watch runbox ps

# Replay the best result
runbox replay <run_id_of_best>
```

### Reproduce Past Work

```bash
# Find the run
runbox history --limit 20
runbox show <run_id>

# Replay in isolated worktree
runbox replay <run_id>
```

---

## Configuration

### Git Config (per-repo or global)

```bash
# Repository-level
git config runbox.worktree-dir /path/to/worktrees

# Global
git config --global runbox.worktree-dir ~/runbox-worktrees
```

### Global Config File

```bash
mkdir -p ~/.config/runbox
cat > ~/.config/runbox/config.toml << 'EOF'
[replay]
worktree_dir = "~/.runbox/worktrees"
cleanup = false
reuse = true
EOF
```

---

## Troubleshooting

### Logs Not Appearing
1. Check if run is still `pending`: `runbox show <run_id>`
2. For tmux runs, use `runbox attach` instead

### Short ID Ambiguity
```
Error: Ambiguous: 2 items match '5a'. Use more characters.
```
Solution: Use more characters to disambiguate.

### Worktree Issues
```bash
# List existing worktrees
git worktree list

# Remove stale worktrees
git worktree prune
```

---

## Installation

See [README.md](../README.md) and [Tutorial](./tutorial.md) for installation and complete documentation.
