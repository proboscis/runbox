# Record/Task Separation Spec v1

## Overview

Run を **Record**（実行記録）と **Task**（ライブプロセス）に分離し、XDG Base Directory に完全準拠、外部ツール統合をサポートする。

## Background

### Current Problem

現在の `Run` は以下の両方を兼ねている：
- 実行の記録（git state, command, result）
- ライブプロセスの管理（attach, stop, logs）

この混同により：
- 外部ツール（doeff）が foreground 実行した結果を記録できない
- `runbox run` の意味論が曖昧（実行者なのか記録者なのか）

### Goals

- **Record**: 再現可能な実行スナップショット（誰が実行しても作れる）
- **Task**: ライブプロセス（runbox が executor の時のみ存在）

---

## Core Entities

```
┌─────────────────────────────────────────────────────────────────┐
│                          Record                                 │
│  (実行記録 - 実行ごとに生成、永続)                               │
│  ├─ id: rec_*                                                  │
│  ├─ git_state (repo, commit, patch)                            │
│  ├─ command (resolved)                                         │
│  ├─ result (exit_code, timestamps)                             │
│  ├─ log_ref                                                    │
│  └─ tags                                                       │
└─────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────┐
│                           Task                                  │
│  (ライブプロセス - 実行中のみ、一時的)                           │
│  ├─ id: task_*                                                 │
│  ├─ record_id: rec_* (紐づく Record)                           │
│  ├─ runtime: bg | tmux                                         │
│  ├─ status: running | exited | failed | killed                 │
│  └─ handle (pid/pgid or tmux session)                          │
└─────────────────────────────────────────────────────────────────┘
```

### Relationship

```
Template ─────┐
              ├──→ runbox run ──→ Task ──→ Record
Playlist ─────┘         │              │
                        │              ↓
doeff run ──────────────┴────→ Record (直接作成)
                                   │
                                   ↓
                          runbox replay ──→ Task + Record
```

---

## Storage Design

### XDG Base Directory (強制)

すべてのプラットフォームで XDG Base Directory Specification に従う。
**macOS の `~/Library/Application Support` は使用しない。**

```bash
# XDG 環境変数 (デフォルト値)
XDG_DATA_HOME    = ~/.local/share      # ユーザーデータ
XDG_CONFIG_HOME  = ~/.config           # 設定ファイル
XDG_STATE_HOME   = ~/.local/state      # 状態・ログ
XDG_CACHE_HOME   = ~/.cache            # キャッシュ
```

### Directory Structure

```
# プロジェクトローカル (git tracked)
<project>/
└── .runbox/
    ├── templates/              # プロジェクト固有テンプレート
    │   └── tpl_*.json
    └── playlists/              # プロジェクト固有プレイリスト
        └── pl_*.json

# グローバル - XDG 準拠
~/.local/share/runbox/          # $XDG_DATA_HOME/runbox
├── templates/                  # グローバルテンプレート (JSON files)
├── playlists/                  # グローバルプレイリスト (JSON files)
└── records/                    # 実行記録 (JSON files, 永続)

~/.local/state/runbox/          # $XDG_STATE_HOME/runbox
├── runbox.db                   # SQLite (index + tasks)
├── tasks/                      # アクティブタスク状態
└── logs/                       # 実行ログ

~/.config/runbox/               # $XDG_CONFIG_HOME/runbox
└── config.toml                 # 設定ファイル

~/.cache/runbox/                # $XDG_CACHE_HOME/runbox
└── (再構築可能なキャッシュ)
```

### Storage Category

| データ種別 | XDG カテゴリ | 保存形式 | git tracked |
|-----------|-------------|---------|-------------|
| Templates (local) | プロジェクト | JSON files | Yes |
| Templates (global) | DATA | JSON files | No |
| Playlists (local) | プロジェクト | JSON files | Yes |
| Playlists (global) | DATA | JSON files | No |
| Records | DATA | JSON files | No |
| Tasks | STATE | SQLite | No |
| Logs | STATE | Files | No |
| Config | CONFIG | TOML | No |

### Rust Implementation

```rust
use std::env;
use std::path::PathBuf;

fn xdg_data_home() -> PathBuf {
    env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            dirs::home_dir().unwrap().join(".local/share")
        })
}

fn xdg_state_home() -> PathBuf {
    env::var("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            dirs::home_dir().unwrap().join(".local/state")
        })
}

fn xdg_config_home() -> PathBuf {
    env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            dirs::home_dir().unwrap().join(".config")
        })
}

fn xdg_cache_home() -> PathBuf {
    env::var("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            dirs::home_dir().unwrap().join(".cache")
        })
}

// NEVER use ~/Library/Application Support on macOS
```

---

## Hybrid Storage: Files + SQLite Index

Template/Playlist/Record は JSON ファイルとして保存し、SQLite でインデックスする。

### Behavior

1. 起動時に JSON ファイルをスキャン
2. SQLite の index テーブルに登録 (file_path, mtime でキャッシュ)
3. クエリは SQLite 経由
4. ファイル編集 → 次回起動で再インデックス

### SQLite Schema

```sql
-- ファイルインデックス (templates, playlists, records を統合)
CREATE TABLE file_index (
    id TEXT PRIMARY KEY,
    type TEXT NOT NULL,         -- 'template' | 'playlist' | 'record'
    file_path TEXT NOT NULL UNIQUE,
    file_mtime INTEGER,
    scope TEXT NOT NULL,        -- 'local' | 'global'
    command TEXT,
    bindings JSON,
    items JSON,
    git_state JSON,
    exit_code INTEGER,
    started_at DATETIME,
    ended_at DATETIME,
    log_ref TEXT,
    tags JSON,
    created_at DATETIME,
    indexed_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

-- Tasks (SQLite only, not file-backed)
CREATE TABLE tasks (
    id TEXT PRIMARY KEY,
    record_id TEXT,
    runtime TEXT NOT NULL,
    status TEXT NOT NULL,
    handle JSON,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

-- Unified view for queries
CREATE VIEW entities AS
    SELECT type, id, command as name, tags, scope, created_at, 
           CASE type 
               WHEN 'record' THEN CASE WHEN exit_code = 0 THEN 'exited' WHEN exit_code IS NOT NULL THEN 'failed' ELSE 'pending' END
               ELSE NULL 
           END as status,
           exit_code 
    FROM file_index
    UNION ALL
    SELECT 'task' as type, t.id, f.command as name, NULL as tags, NULL as scope, 
           t.created_at, t.status, NULL as exit_code 
    FROM tasks t LEFT JOIN file_index f ON t.record_id = f.id;
```

---

## CLI Changes

### Query Commands

```bash
# 基本クエリ
runbox list                           # デフォルト: runnables
runbox list --all                     # 全エンティティ

# Type フィルタ (複数指定 = OR)
runbox list --type task
runbox list --type template --type playlist
runbox list -t task -t record

# Tag フィルタ (複数指定 = AND)
runbox list --tag ml --tag gpu

# Status フィルタ
runbox list --status running
runbox list --status exited --status failed

# Scope フィルタ
runbox list --local              # プロジェクトローカルのみ
runbox list --global             # グローバルのみ

# WHERE 句 (複雑なクエリ)
runbox list --where "status='running' AND created_at > datetime('now', '-1 hour')"
runbox list --where "tags LIKE '%ml%' OR tags LIKE '%gpu%'"

# フル SQL
runbox query "SELECT id, command, exit_code FROM file_index WHERE type='record' AND exit_code != 0 LIMIT 10"
```

### Mutation Commands

```bash
# Record 作成 (外部ツール用)
runbox create record < record.json
runbox create record --from-file record.json

# Scope 指定
runbox create template foo.json              # デフォルト: ローカル
runbox create template foo.json --global     # グローバル
```

---

## doeff Integration

### Foreground Execution (Record only)

```bash
# doeff が自前で実行、Record だけ作成
doeff run --program X
  ↓
  1. git state をキャプチャ
  2. doeff が foreground で実行
  3. 終了後: runbox create record < record.json
```

### Detached Execution (Task + Record)

```bash
# runbox 経由で実行
doeff run --program X --detach
  ↓
  runbox run --runtime bg -- uv run doeff run --program X
```

### Record JSON Format

```json
{
  "id": "rec_550e8400-e29b-41d4-a716-446655440000",
  "git_state": {
    "repo_url": "git@github.com:user/repo.git",
    "commit": "abc123def456...",
    "patch_ref": "refs/patches/rec_550e8400..."
  },
  "command": "uv run doeff run --program my.module.program",
  "exit_code": 0,
  "started_at": "2025-01-19T10:00:00Z",
  "ended_at": "2025-01-19T10:05:30Z",
  "log_ref": "~/.local/state/runbox/logs/rec_550e8400.log",
  "tags": ["doeff", "ml"]
}
```

---

## Migration

### From Current Storage

1. Detect existing data at `~/Library/Application Support/runbox/`
2. Move to XDG paths
3. Split `runs/` into `records/` (DATA) and active tasks (STATE)

### Backward Compatibility

```bash
# Existing commands work as aliases
runbox ps       → runbox list --type task --status active
runbox history  → runbox list --type record --sort recent
runbox replay   → runbox run <record_id>
```

---

## ID Format

| Entity | Prefix | Format | Example |
|--------|--------|--------|---------|
| Template | `tpl_` | `tpl_<name>` | `tpl_daily_build` |
| Playlist | `pl_` | `pl_<name>` | `pl_morning_tasks` |
| Record | `rec_` | `rec_<uuid>` | `rec_550e8400-e29b-...` |
| Task | `task_` | `task_<uuid>` | `task_a1b2c3d4-...` |

---

## References

- [run.md](./run.md) - Original Run spec
- ISSUE-033: Unified Runnable with hex short IDs
- ISSUE-034: Unified runbox list for all runnables
