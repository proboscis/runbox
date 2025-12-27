---
type: issue
id: ISSUE-004
title: Worktree-based Replay with Layered Configuration
status: open
summary: Replay should use git worktree instead of checkout-in-place, with configurable worktree directory
---

# Worktree-based Replay with Layered Configuration

## 概要

現在の `runbox replay` は現在のリポジトリで直接 `git checkout` を行うため、作業ツリーが破壊される。
安全な再現実行のため、git worktree を使用した隔離環境での実行に変更する。

## 問題点

現在の実装 (`git.rs:277-287`):
```rust
pub fn restore_code_state(&self, code_state: &CodeState) -> Result<()> {
    self.checkout(&code_state.base_commit)?;  // ← 破壊的
    if let Some(patch) = &code_state.patch {
        self.apply_patch(&patch.ref_)?;
    }
    Ok(())
}
```

これは spec (`specs/run.md:199-214`) に違反:
```bash
# Spec says:
git clone {repo_url} workdir  # ← 別ディレクトリに隔離
cd workdir
git checkout {base_commit}
```

## 解決策

### 1. Worktree ベースの Replay

```bash
runbox replay <run_id>
# 1. Check if worktree with same commit exists → reuse
# 2. Otherwise create: git worktree add <dir> <base_commit>
# 3. Apply patch if present
# 4. Execute in worktree
# 5. Optionally cleanup or keep for inspection
```

### 2. Layered Configuration (優先順位順)

| Priority | Source | Example |
|----------|--------|---------|
| 1 | CLI flag | `--worktree-dir /path` |
| 2 | Git config (local) | `git config --local runbox.worktreeDir /path` |
| 3 | Global config | `~/.config/runbox/config.toml` |
| 4 | Default | `.git-worktrees/replay/` in repo root |

### 3. Verbose Logging

```bash
# -v: Show config resolution
runbox replay <run_id> -v
# [config] worktree_dir: /Users/me/worktrees (from: git config)

# -vv: Show all layers checked
runbox replay <run_id> -vv
# [config] checking CLI flag --worktree-dir: not set
# [config] checking git config runbox.worktreeDir: /Users/me/worktrees
# [config] → using: /Users/me/worktrees (source: git config)
# [worktree] checking existing worktrees for commit a1b2c3d4...
# [worktree] no match, creating new at /Users/me/worktrees/run_abc123
# [git] checkout base_commit: a1b2c3d4e5f6...
# [git] applying patch: refs/patches/run_abc123
# [exec] cwd: /Users/me/worktrees/run_abc123/.
# [exec] argv: ["echo", "hello world"]
# [exec] exit_code: 0

# -vvv: Debug level (git commands, timing, etc.)
```

---

## CLI Changes

```bash
# New flags
runbox replay <run_id> [OPTIONS]

OPTIONS:
  --worktree-dir <PATH>   Override worktree directory
  --keep                  Keep worktree after execution (default: keep)
  --cleanup               Remove worktree after execution
  --reuse                 Reuse existing worktree if commit matches (default)
  --fresh                 Always create fresh worktree
  -v, --verbose           Verbose output (can be repeated: -v, -vv, -vvv)
```

---

## Config File Format

### Git config (per-repo, user-specific)
```bash
git config --local runbox.worktreeDir "/Users/me/worktrees/myproject"
git config --local runbox.worktreeCleanup "false"
git config --local runbox.verbosity "1"
```

### Global config (`~/.config/runbox/config.toml`)
```toml
[replay]
worktree_dir = "~/.runbox/worktrees"  # default base directory
cleanup = false
reuse = true

[logging]
verbosity = 0  # 0=normal, 1=-v, 2=-vv, 3=-vvv
```

---

## Acceptance Criteria

- [ ] `runbox replay` uses git worktree instead of checkout-in-place
- [ ] Worktree directory configurable via CLI flag
- [ ] Worktree directory configurable via git config (local)
- [ ] Worktree directory configurable via global config file
- [ ] Default worktree location: `.git-worktrees/replay/<run_id>/`
- [ ] Reuse existing worktree if same base_commit (configurable)
- [ ] `-v` shows which config source was used
- [ ] `-vv` shows all config layers checked
- [ ] `-vvv` shows debug-level details (git commands, etc.)
- [ ] Worktree cleanup option (--keep / --cleanup)

---

## Implementation Notes

### Config Resolution Order
```rust
fn resolve_worktree_dir(cli: &Cli, repo: &Repository) -> PathBuf {
    // 1. CLI flag (highest priority)
    if let Some(dir) = &cli.worktree_dir {
        log_v!("worktree_dir: {} (from: CLI flag)", dir);
        return dir.clone();
    }

    // 2. Git config (local)
    if let Ok(dir) = repo.config()?.get_string("runbox.worktreeDir") {
        log_v!("worktree_dir: {} (from: git config)", dir);
        return PathBuf::from(dir);
    }

    // 3. Global config
    if let Some(dir) = global_config().replay.worktree_dir {
        log_v!("worktree_dir: {} (from: global config)", dir);
        return dir;
    }

    // 4. Default
    let default = repo.workdir().join(".git-worktrees/replay");
    log_v!("worktree_dir: {} (from: default)", default);
    default
}
```

### Worktree Naming
```
<worktree_dir>/<run_id>/
# Example: /Users/me/worktrees/run_550e8400-e29b-41d4-a716-446655440000/
```

---

## 関連

- ISSUE-002: Rust CLI Implementation (implements current replay)
- specs/run.md: Reproduce section (lines 199-214)
