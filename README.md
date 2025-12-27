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

```bash
# Run from template
runbox run --template tpl_my_task --binding key=value

# List templates
runbox template list

# Replay a previous run
runbox replay run_550e8400-...
```

## Documentation

- [Run Spec](./specs/run.md)
- [CLI Reference](./docs/cli.md)

## License

MIT
