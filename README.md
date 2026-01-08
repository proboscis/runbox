# runbox

Reproducible command execution system.

## Overview

runbox provides a framework for capturing, storing, and reproducing command executions with full git context (commit + uncommitted changes).

## Core Concepts

- **Run**: A fully-resolved, reproducible execution record
- **RunTemplate**: A template for creating Runs with variable bindings
- **Playlist**: A collection of RunTemplate references

## Installation

```bash
# CLI
cargo install runbox

# Python
pip install runbox
```

## Usage

### Direct Execution

Run any command directly and capture git context:

```bash
# Execute a command directly
runbox run -- echo "Hello, World!"
runbox run -- python train.py --epochs 10
runbox run -- make test

# Using the 'log' alias
runbox log -- npm run build

# With options
runbox run --runtime tmux -- python debug.py
runbox run --timeout 3600 -- ./long_job.sh
runbox run --env CUDA_VISIBLE_DEVICES=0 -- python train.py
```

### Template-Based Execution

Create reusable templates with variable bindings:

```bash
# Run from template
runbox run --template tpl_my_task --binding key=value

# List templates
runbox template list
```

### Monitoring and Replay

```bash
# Check running processes
runbox ps

# View logs
runbox logs <run_id>

# Replay a previous run with exact code state
runbox replay <run_id>
```

## Documentation

- [Tutorial](./docs/tutorial.md)
- [Run Spec](./specs/run.md)

## License

MIT
