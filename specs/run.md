# Run Spec v0

## Overview

runbox provides a framework for capturing, storing, and reproducing command executions with full git context.

## Design Principles

### Purity
Run contains **only** information needed for reproduction:
- `exec`: What to execute and how
- `code_state`: Which code state to run against

External concerns are handled by upper layers:
- tags, notes → Experiment layer
- jump links → Jump layer
- execution results → RunResult

### Reproducibility
All fields are **fully resolved**:
- No template variables (`{var}` forbidden)
- No interactive input (resolved before Run creation)
- No derived values (concrete values only)

---

## Layer Structure

```
┌─────────────────────────────────────────┐
│  Experiment Layer                       │
│  - hypothesis, tags, notes              │
│  - control/treatment assignment         │
│  - references: [run_id, ...]            │
└──────────────┬──────────────────────────┘
               │
┌──────────────▼──────────────────────────┐
│  Playlist Layer (view)                  │
│  - List of RunTemplate references       │
└──────────────┬──────────────────────────┘
               │
┌──────────────▼──────────────────────────┐
│  RunTemplate Layer                      │
│  - Unresolved bindings                  │
│  - code_state (commit TBD)              │
└──────────────┬──────────────────────────┘
               │ context capture
┌──────────────▼──────────────────────────┐
│  Run Layer (pure, reproducible)         │
│  - exec (resolved)                      │
│  - code_state (checkpoint)              │
└─────────────────────────────────────────┘
```

---

## Schema

See [run.cue](./run.cue) for CUE definitions.

---

## Examples

### Run (from direct execution)

Created by: `runbox run -- echo hello`

```json
{
  "run_version": 0,
  "run_id": "run_617c2725-692e-4cdf-9336-85a526ad8415",

  "exec": {
    "argv": ["echo", "hello"],
    "cwd": ".",
    "env": {},
    "timeout_sec": 0
  },

  "code_state": {
    "repo_url": "git@github.com:org/repo.git",
    "base_commit": "a1b2c3d4e5f6789012345678901234567890abcd"
  }
}
```

### Run (from template)

Created by: `runbox run --template tpl_plc_runner --binding i=7`

```json
{
  "run_version": 0,
  "run_id": "run_550e8400-e29b-41d4-a716-446655440000",

  "exec": {
    "argv": ["uv", "run", "python", "-m", "plc.runner", "--i", "7"],
    "cwd": ".",
    "env": {
      "WANDB_DIR": "./outputs",
      "CUDA_VISIBLE_DEVICES": "0"
    },
    "timeout_sec": 3600
  },

  "code_state": {
    "repo_url": "git@github.com:org/repo.git",
    "base_commit": "a1b2c3d4e5f6789012345678901234567890abcd",
    "patch": {
      "ref": "refs/patches/run_550e8400-e29b-41d4-a716-446655440000",
      "sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    }
  }
}
```

### RunTemplate
```json
{
  "template_version": 0,
  "template_id": "tpl_plc_runner",
  "name": "PLC Runner",

  "exec": {
    "argv": ["uv", "run", "python", "-m", "plc.runner", "--i", "{i}"],
    "cwd": ".",
    "env": {
      "WANDB_DIR": "./outputs"
    },
    "timeout_sec": 3600
  },

  "bindings": {
    "defaults": {"i": 0},
    "interactive": ["i"]
  },

  "code_state": {
    "repo_url": "git@github.com:org/repo.git"
  }
}
```

### Playlist
```json
{
  "playlist_id": "pl_daily",
  "name": "Daily Tasks",
  "items": [
    {"template_id": "tpl_plc_runner", "label": "PLC Runner (default)"},
    {"template_id": "tpl_eval_all"},
    {"template_id": "tpl_sync_data"}
  ]
}
```

---

## Invariants

### Run
| Field | Constraint |
|-------|------------|
| `run_version` | `0` (fixed) |
| `run_id` | UUID v4 with `run_` prefix |
| `exec.argv` | Non-empty array, fully resolved, no template variables |
| `exec.cwd` | Relative path from repo root |
| `exec.env` | All string values (path interpretation is caller's responsibility) |
| `exec.timeout_sec` | 0 = unlimited |
| `code_state.repo_url` | Cloneable URL |
| `code_state.base_commit` | Full SHA (40 chars) |
| `code_state.patch` | Optional, omit if no diff |
| `code_state.patch.ref` | `refs/patches/{run_id}` format |
| `code_state.patch.sha256` | SHA-256 of patch content |

### RunTemplate
| Field | Constraint |
|-------|------------|
| `template_id` | `tpl_` prefix |
| `exec.argv` | Template variables `{var}` allowed |
| `bindings.interactive` | List of variable names to prompt user |

### Playlist
| Field | Constraint |
|-------|------------|
| `playlist_id` | `pl_` prefix |
| `items[].template_id` | Must reference existing RunTemplate |

---

## Storage

XDG Base Directory Specification:

```
~/.local/share/runbox/
├── runs/
│   └── run_550e8400-....json
├── templates/
│   └── tpl_plc_runner.json
└── playlists/
    └── pl_daily.json
```

---

## Patch Storage Strategy

Git refs for persistent patch storage. **Patch push is mandatory** (prerequisite for Run creation).

### Save
```bash
# 1. Save diff as blob
git diff > /tmp/patch.diff
BLOB_SHA=$(git hash-object -w /tmp/patch.diff)

# 2. Create ref
git update-ref refs/patches/{run_id} $BLOB_SHA

# 3. Push to remote (ensures reproducibility)
git push origin refs/patches/{run_id}
```

### Reproduce
```bash
# 1. Clone & checkout
git clone {repo_url} workdir
cd workdir
git checkout {base_commit}

# 2. Fetch & apply patch
git fetch origin {patch.ref}
git cat-file -p FETCH_HEAD > /tmp/patch.diff
git apply /tmp/patch.diff

# 3. Execute
cd {exec.cwd}
env {exec.env} {exec.argv}
```

---

## Execution Flow

### Direct Execution Flow

```
1. User runs: runbox run -- <command>
2. Parse command after --
3. Capture context:
   - base_commit = git rev-parse HEAD
   - patch = git diff (if dirty)
   - push patch to refs/patches/{run_id}
4. Generate Run (with UUID)
5. Save Run to ~/.local/share/runbox/runs/
6. Execute
```

### Template Execution Flow

```
1. User clicks playlist item or runs: runbox run --template <id>
2. Resolve template_id → RunTemplate
3. Resolve bindings:
   - Apply defaults
   - Prompt user for interactive bindings
   - Expand template variables in argv/env
4. Capture context:
   - base_commit = git rev-parse HEAD
   - patch = git diff (if dirty)
   - push patch to refs/patches/{run_id}
5. Generate Run (with UUID)
6. Save Run to ~/.local/share/runbox/runs/
7. Execute
```

---

## Validation

Generate JSON Schema from CUE for cross-language validation:

```bash
cue export --out openapi run.cue > run.schema.json
```

| Language | Library |
|----------|---------|
| Rust | `jsonschema` |
| Go | `gojsonschema` |
| Lua | `lua-resty-jsonschema` |
| Python | `jsonschema` |
