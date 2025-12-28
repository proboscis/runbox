<issue>

# Run Execution Management

## 概要

既存の runbox は「再現性」に特化している（code_state + exec → replay）。
本ISSUEでは「実行管理」機能を追加し、実行中のジョブを観測・制御できるようにする。

```
既存: "何を実行したか" の記録（再現性）
追加: "今何が動いているか" の管理（実行管理）
```

## 設計原則

1. **code_state は必須** - runbox の核である再現性は維持
2. **UI と分離** - runbox は "実行と台帳" を担い、表示の都合は外に出す
3. **Registry は JSON** - 1 run = 1 JSON ファイル（既存の Storage パターン）
4. **Runtime Adapter パターン** - runtime 固有ロジックを adapter に分離

---

## アーキテクチャ

### 責務分離

```
┌─────────────────────────────────────────────────────┐
│ runbox core                                         │
│  - 台帳管理（Run CRUD）                              │
│  - ログファイル管理                                   │
│  - 状態遷移（status 更新）                           │
│  - テンプレート解決                                   │
│  - reconcile（状態整合性チェック）                    │
└─────────────────────────────────────────────────────┘
                          │
                          ▼
┌─────────────────────────────────────────────────────┐
│ RuntimeAdapter trait                                │
│  - spawn(): プロセス起動 → RuntimeHandle            │
│  - stop(): 停止                                     │
│  - attach(): 端末接続                               │
│  - is_alive(): 生存確認                             │
└─────────────────────────────────────────────────────┘
         │                │                │
         ▼                ▼                ▼
┌─────────────┐  ┌─────────────┐  ┌─────────────┐
│ Background  │  │    Tmux     │  │   Zellij    │
│   Adapter   │  │   Adapter   │  │   Adapter   │
└─────────────┘  └─────────────┘  └─────────────┘
```

### ディレクトリ構造

```
crates/runbox-core/src/
├── lib.rs
├── run.rs              # Run 構造
├── storage.rs          # JSON 永続化
├── runtime/
│   ├── mod.rs          # RuntimeAdapter trait, RuntimeHandle enum
│   ├── background.rs   # BackgroundAdapter
│   ├── tmux.rs         # TmuxAdapter
│   └── zellij.rs       # ZellijAdapter
```

---

## Run 構造の拡張

### 現行

```rust
struct Run {
    run_id: String,
    exec: Exec,
    code_state: CodeState,
}
```

### 拡張後

```rust
struct Run {
    run_id: String,

    // 既存（必須）
    exec: Exec,
    code_state: CodeState,

    // 実行状態
    status: RunStatus,
    runtime: String,                // "background" | "tmux" | "zellij"
    handle: Option<RuntimeHandle>,  // runtime 固有データ
    log_ref: LogRef,
    timeline: Timeline,
    exit_code: Option<i32>,
}

enum RunStatus {
    Pending,    // 作成済み、未起動
    Running,    // 実行中
    Exited,     // 正常終了（exit_code == 0）
    Failed,     // 異常終了（exit_code != 0）
    Killed,     // 手動停止
    Unknown,    // reconcile で検出された不整合状態
}

struct LogRef {
    path: PathBuf,
}

struct Timeline {
    created_at: DateTime<Utc>,
    started_at: Option<DateTime<Utc>>,
    ended_at: Option<DateTime<Utc>>,
}
```

---

## RuntimeAdapter

### trait 定義

```rust
pub trait RuntimeAdapter: Send + Sync {
    /// runtime 名 ("background", "tmux", "zellij")
    fn name(&self) -> &str;

    /// プロセス起動
    /// - exec: 実行するコマンド
    /// - run_id: 識別子（window名等に使用）
    /// - log_path: stdout/stderr の出力先
    fn spawn(
        &self,
        exec: &Exec,
        run_id: &str,
        log_path: &Path,
    ) -> Result<RuntimeHandle>;

    /// 停止（子プロセス含めて終了）
    fn stop(&self, handle: &RuntimeHandle) -> Result<()>;

    /// attach（端末を奪って接続）
    fn attach(&self, handle: &RuntimeHandle) -> Result<()>;

    /// 生存確認（reconcile 用）
    fn is_alive(&self, handle: &RuntimeHandle) -> bool;
}
```

### RuntimeHandle

```rust
/// Runtime 固有のデータ（シリアライズ可能）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum RuntimeHandle {
    Background {
        pid: u32,
        pgid: u32,
    },
    Tmux {
        session: String,
        window: String,
    },
    Zellij {
        session: String,
        tab: String,
    },
}
```

---

## Runtime 実装

### BackgroundAdapter

```rust
impl RuntimeAdapter for BackgroundAdapter {
    fn name(&self) -> &str { "background" }

    fn spawn(&self, exec: &Exec, run_id: &str, log_path: &Path) -> Result<RuntimeHandle> {
        let log_file = File::create(log_path)?;

        let child = Command::new(&exec.argv[0])
            .args(&exec.argv[1..])
            .current_dir(&exec.cwd)
            .envs(&exec.env)
            .stdout(Stdio::from(log_file.try_clone()?))
            .stderr(Stdio::from(log_file))
            .process_group(0)  // 新しい pgid を作成
            .spawn()?;

        let pid = child.id();
        let pgid = pid;  // process_group(0) なら pid == pgid

        // 別スレッドで wait して終了を検知
        let run_id = run_id.to_string();
        std::thread::spawn(move || {
            let status = child.wait();
            let exit_code = status.map(|s| s.code().unwrap_or(-1)).unwrap_or(-1);
            // _on-exit 相当の処理
            if let Err(e) = update_run_on_exit(&run_id, exit_code) {
                eprintln!("Failed to update run status: {}", e);
            }
        });

        Ok(RuntimeHandle::Background { pid, pgid })
    }

    fn stop(&self, handle: &RuntimeHandle) -> Result<()> {
        if let RuntimeHandle::Background { pgid, .. } = handle {
            unsafe {
                libc::killpg(*pgid as i32, libc::SIGTERM);
            }
        }
        Ok(())
    }

    fn attach(&self, _handle: &RuntimeHandle) -> Result<()> {
        bail!("Background runtime does not support attach")
    }

    fn is_alive(&self, handle: &RuntimeHandle) -> bool {
        if let RuntimeHandle::Background { pid, .. } = handle {
            // /proc/{pid} が存在するか、または kill -0 で確認
            Path::new(&format!("/proc/{}", pid)).exists()
                || unsafe { libc::kill(*pid as i32, 0) == 0 }
        } else {
            false
        }
    }
}
```

### TmuxAdapter

```rust
pub struct TmuxAdapter {
    session_name: String,  // デフォルト: "runbox"
}

impl RuntimeAdapter for TmuxAdapter {
    fn name(&self) -> &str { "tmux" }

    fn spawn(&self, exec: &Exec, run_id: &str, log_path: &Path) -> Result<RuntimeHandle> {
        // 1. session 存在確認、なければ作成
        let has_session = Command::new("tmux")
            .args(["has-session", "-t", &self.session_name])
            .status()?
            .success();

        if !has_session {
            Command::new("tmux")
                .args(["new-session", "-d", "-s", &self.session_name])
                .status()?;
        }

        // 2. window 名（short_id を使用）
        let window_name = short_id(run_id);

        // 3. コマンド構築（stdout/stderr を直接ファイルへ）
        let cmd = format!(
            "exec {} > {} 2>&1",
            shell_escape_argv(&exec.argv),
            log_path.display()
        );

        // 4. 新しい window で実行
        Command::new("tmux")
            .args([
                "new-window",
                "-t", &self.session_name,
                "-n", &window_name,
                &cmd,
            ])
            .current_dir(&exec.cwd)
            .envs(&exec.env)
            .status()?;

        Ok(RuntimeHandle::Tmux {
            session: self.session_name.clone(),
            window: window_name,
        })
    }

    fn stop(&self, handle: &RuntimeHandle) -> Result<()> {
        if let RuntimeHandle::Tmux { session, window } = handle {
            Command::new("tmux")
                .args(["kill-window", "-t", &format!("{}:{}", session, window)])
                .status()?;
        }
        Ok(())
    }

    fn attach(&self, handle: &RuntimeHandle) -> Result<()> {
        if let RuntimeHandle::Tmux { session, window } = handle {
            // window を選択
            Command::new("tmux")
                .args(["select-window", "-t", &format!("{}:{}", session, window)])
                .status()?;

            // attach or switch-client
            if std::env::var("TMUX").is_ok() {
                Command::new("tmux")
                    .args(["switch-client", "-t", session])
                    .exec();
            } else {
                Command::new("tmux")
                    .args(["attach", "-t", session])
                    .exec();
            }
        }
        Ok(())
    }

    fn is_alive(&self, handle: &RuntimeHandle) -> bool {
        if let RuntimeHandle::Tmux { session, window } = handle {
            Command::new("tmux")
                .args(["has-session", "-t", &format!("{}:{}", session, window)])
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        } else {
            false
        }
    }
}
```

### ZellijAdapter

```rust
pub struct ZellijAdapter {
    session_name: String,
}

impl RuntimeAdapter for ZellijAdapter {
    fn name(&self) -> &str { "zellij" }

    fn spawn(&self, exec: &Exec, run_id: &str, log_path: &Path) -> Result<RuntimeHandle> {
        // TODO: zellij の実装
        // - session 存在確認
        // - new-tab でコマンド実行
        // - tab 名は short_id(run_id)
        todo!("Zellij support")
    }

    fn stop(&self, handle: &RuntimeHandle) -> Result<()> {
        // zellij action close-tab が使えるか調査中
        // 使えない場合は、tab 内のプロセスを kill
        todo!()
    }

    fn attach(&self, handle: &RuntimeHandle) -> Result<()> {
        if let RuntimeHandle::Zellij { session, .. } = handle {
            Command::new("zellij")
                .args(["attach", session])
                .exec();
        }
        Ok(())
    }

    fn is_alive(&self, handle: &RuntimeHandle) -> bool {
        if let RuntimeHandle::Zellij { session, .. } = handle {
            // zellij list-sessions で確認
            Command::new("zellij")
                .args(["list-sessions"])
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).contains(session))
                .unwrap_or(false)
        } else {
            false
        }
    }
}
```

---

## Reconcile（状態整合性チェック）

### 概要

`_on-exit` が呼ばれない場合（tmux window を手動で閉じた、runbox が異常終了等）に
status が `Running` のまま残る問題を解決する。

### 実装

```rust
/// ps 実行時に自動で呼ばれる
pub fn reconcile_runs(storage: &Storage) -> Result<()> {
    let runs = storage.list_runs(usize::MAX)?;
    let adapters = create_adapters();  // HashMap<String, Box<dyn RuntimeAdapter>>

    for run in runs {
        if run.status != RunStatus::Running {
            continue;
        }

        let Some(ref handle) = run.handle else {
            // handle がない Running は異常
            update_status(storage, &run.run_id, RunStatus::Unknown)?;
            continue;
        };

        let Some(adapter) = adapters.get(&run.runtime) else {
            continue;
        };

        if !adapter.is_alive(handle) {
            update_status(storage, &run.run_id, RunStatus::Unknown)?;
        }
    }

    Ok(())
}

fn update_status(storage: &Storage, run_id: &str, status: RunStatus) -> Result<()> {
    let mut run = storage.load_run(run_id)?;
    run.status = status;
    run.timeline.ended_at = Some(Utc::now());
    storage.save_run(&run)?;
    Ok(())
}
```

### 呼び出しタイミング

- `runbox ps` 実行時（軽量チェック）
- `runbox doctor`（候補）で重いチェック

---

## JSON 構造

```json
{
  "run_id": "run_550e8400-e29b-41d4",
  "exec": {
    "argv": ["python", "train.py", "--lr", "0.001"],
    "cwd": "/path/to/project",
    "env": {"CUDA_VISIBLE_DEVICES": "0"}
  },
  "code_state": {
    "repo_url": "git@github.com:user/repo.git",
    "base_commit": "abc123def456",
    "patch": null
  },
  "status": "running",
  "runtime": "tmux",
  "handle": {
    "type": "Tmux",
    "session": "runbox",
    "window": "550e8400"
  },
  "log_ref": {
    "path": "/Users/me/.local/share/runbox/logs/run_550e8400.log"
  },
  "timeline": {
    "created_at": "2025-01-01T00:00:00Z",
    "started_at": "2025-01-01T00:00:01Z",
    "ended_at": null
  },
  "exit_code": null
}
```

---

## CLI インターフェース

### MVP（必須）

| コマンド | 説明 |
|----------|------|
| `runbox run -t <template> [--runtime bg\|tmux]` | 起動 |
| `runbox ps` | 実行中/最近の一覧（+ reconcile） |
| `runbox show <run_id>` | 詳細表示 |
| `runbox logs <run_id>` | ログ表示 |
| `runbox logs -f <run_id>` | ログ tail |
| `runbox stop <run_id>` | 停止 |

```bash
# バックグラウンドで起動（デフォルト）
runbox run -t my_template
runbox run -t my_template --runtime bg

# tmux で起動
runbox run -t my_template --runtime tmux
```

### 推奨

| コマンド | 説明 |
|----------|------|
| `runbox attach <run_id>` | 端末接続（tmux/zellij のみ） |
| `runbox ps --status running` | フィルタ |
| `runbox recent [N]` | 最近 N 件 |

### 候補

| コマンド | 説明 |
|----------|------|
| `runbox doctor` | 重い整合性チェック |
| `runbox clean` | 古いログ/台帳の掃除 |

---

## 境界線（絶対に守る）

runbox は以下を **しない**:
- どの pane に出すか決める
- パネルを開く/閉じる
- レイアウトを作る/切り替える
- 一覧 UI（カーソル移動でプレビュー）を持つ

これらは別レイヤー（runbox-ui 等）の責務。

---

## Acceptance Criteria

### MVP

- [ ] Run 構造に status, runtime, handle, log_ref, timeline, exit_code を追加
- [ ] RuntimeAdapter trait を定義
- [ ] BackgroundAdapter を実装（pid/pgid、子プロセスごと stop）
- [ ] TmuxAdapter を実装（session/window 管理）
- [ ] `runbox run --runtime bg|tmux` で起動
- [ ] `runbox ps` で一覧表示（+ reconcile）
- [ ] `runbox stop <run_id>` で停止
- [ ] `runbox logs <run_id>` / `runbox logs -f` でログ表示
- [ ] `runbox show` が新しいフィールドを表示

### 推奨

- [ ] `runbox attach <run_id>` で tmux に接続
- [ ] RunStatus::Unknown の reconcile 検出
- [ ] `runbox ps --status running` フィルタ

### 候補

- [ ] ZellijAdapter を実装
- [ ] `runbox doctor` で重いチェック
- [ ] `runbox clean` で古いログ削除

---

## 関連

- ISSUE-001: Run/RunTemplate/Playlist Spec（基盤）
- ISSUE-002: Rust CLI Implementation（現行実装）
- ISSUE-004: Worktree-based Replay（replay の改善）
- ISSUE-006: Short ID Support（短縮ID）

</issue>

Instructions:
- Implement the changes described in the issue above
- Run tests to verify your changes work correctly
- When complete, create a pull request targeting `main`:
  - Title should summarize the change
  - Body should reference issue: ISSUE-005
  - Include a summary of changes made
