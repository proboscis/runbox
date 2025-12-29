<issue>

# Background Process Exit Status Capture (Daemon Architecture)

## Problem

When running `runbox run -t <template>` with background runtime, the process exit status is never captured. All completed runs end up with `unknown` status instead of `exited` or `failed`.

### Evidence

```bash
$ runbox run -t echo
Starting run: run_bd93ac29-...
...

$ runbox ps
SHORT ID     STATUS     RUNTIME    COMMAND
bd93ac29     unknown    background echo hello world

$ runbox show bd93ac29
Status:     unknown
Reconcile:  process 25946 not found
```

### Root Cause Analysis

**Original code** (`crates/runbox-core/src/runtime/background.rs`):
```rust
let child = cmd.spawn()?;
std::mem::forget(child);  // Exit is never captured
```

**Why thread-based solution doesn't work:**

The naive fix of spawning a thread to wait:
```rust
std::thread::spawn(move || {
    child.wait();  // Thread waits for exit
    update_run_on_exit(...);
});
```

**Fails because:**
1. `runbox run` is designed to exit immediately after spawning
2. When CLI process exits, all its threads are terminated
3. For any process that outlives the CLI, exit status is never captured

**Why double-fork doesn't work:**
```rust
if libc::fork() == 0 {
    setsid();
    libc::waitpid(pid, ...);  // ECHILD! - not our child
}
```
- `waitpid()` only works on **your own children**
- The command was spawned by CLI, not by the forked daemon
- When CLI exits, command is re-parented to init, not to our daemon

---

## Solution: runbox daemon

A long-running daemon process that:
1. Spawns and owns background processes
2. Waits for exit and captures status
3. Updates storage with exit code
4. Survives CLI exit

### Architecture

```
┌─────────────┐    Unix Socket (spawn request)    ┌─────────────────┐
│  runbox CLI │ ────────────────────────────────► │  runbox daemon  │
│             │ ◄──────────────────────────────── │  (auto-started) │
│             │    (pid response)                 │                 │
└──────┬──────┘                                   └────────┬────────┘
       │                                                   │
       │ read                                         wait/update
       ▼                                                   ▼
┌──────────────────────────────────────────────────────────────────┐
│                         SQLite Database                          │
│  runs, templates, playlists (replaces JSON files)                │
└──────────────────────────────────────────────────────────────────┘
```

### Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Storage | **SQLite** | ACID transactions, better concurrent access, single file |
| Daemon lifecycle | **Auto-start** | User should never know daemon exists |
| IPC mechanism | **Unix socket** | Synchronous request-response, battle-tested, used by Docker/PostgreSQL/systemd |
| Socket path | `$XDG_RUNTIME_DIR/runbox/daemon.sock` | Standard location for runtime files |

---

## Required Changes

### 1. New Crate: `runbox-daemon`

Create `crates/runbox-daemon/` with:

```
crates/runbox-daemon/
├── Cargo.toml
└── src/
    ├── main.rs          # Daemon entry point
    ├── server.rs        # Unix socket server
    ├── process_manager.rs  # Spawn, wait, reap processes
    └── protocol.rs      # CLI <-> Daemon message types
```

### 2. Protocol Messages

```rust
// CLI -> Daemon
enum Request {
    Spawn {
        run_id: String,
        exec: Exec,
        log_path: PathBuf,
    },
    Stop {
        run_id: String,
        force: bool,
    },
    Status {
        run_id: String,
    },
    Shutdown,
}

// Daemon -> CLI
enum Response {
    Spawned { pid: u32, pgid: u32 },
    Stopped,
    Status { alive: bool, exit_code: Option<i32> },
    Error { message: String },
}
```

### 3. Daemon Process Manager

```rust
struct ProcessManager {
    /// Map of run_id -> (Child, JoinHandle for wait thread)
    processes: HashMap<String, ManagedProcess>,
}

impl ProcessManager {
    fn spawn(&mut self, run_id: &str, exec: &Exec, log_path: &Path) -> Result<(u32, u32)> {
        let child = Command::new(&exec.argv[0])
            .args(&exec.argv[1..])
            // ... setup ...
            .spawn()?;

        let pid = child.id();
        let pgid = pid;

        // Spawn wait thread (lives as long as daemon)
        let run_id_owned = run_id.to_string();
        let handle = std::thread::spawn(move || {
            let status = child.wait();
            let exit_code = extract_exit_code(&status);
            update_run_on_exit(&run_id_owned, exit_code);
        });

        self.processes.insert(run_id.to_string(), ManagedProcess { pid, pgid, handle });
        Ok((pid, pgid))
    }
}
```

### 4. CLI Changes

Modify `BackgroundAdapter` to communicate with daemon:

```rust
impl RuntimeAdapter for BackgroundAdapter {
    fn spawn(&self, exec: &Exec, run_id: &str, log_path: &Path) -> Result<RuntimeHandle> {
        let client = DaemonClient::connect()?;
        let response = client.send(Request::Spawn {
            run_id: run_id.to_string(),
            exec: exec.clone(),
            log_path: log_path.to_path_buf(),
        })?;

        match response {
            Response::Spawned { pid, pgid } => Ok(RuntimeHandle::Background { pid, pgid }),
            Response::Error { message } => bail!("Daemon error: {}", message),
            _ => bail!("Unexpected response"),
        }
    }
}
```

### 5. Daemon Lifecycle

**Decision: Auto-start** (user never knows daemon exists)

**Auto-start behavior:**
1. CLI attempts to connect to daemon socket
2. If connection fails (daemon not running):
   - CLI spawns daemon as background process
   - Daemon daemonizes (double-fork, setsid)
   - CLI retries connection (with backoff)
3. CLI sends spawn request, receives PID response

**Daemon process:**
- Writes PID to `$XDG_RUNTIME_DIR/runbox/daemon.pid`
- Logs to `$XDG_DATA_HOME/runbox/daemon.log`
- Handles SIGTERM for graceful shutdown
- Auto-exits after idle timeout (e.g., 1 hour with no managed processes)

**Hidden CLI commands (for debugging only):**
```bash
runbox daemon start   # Manual start (foreground)
runbox daemon stop    # Graceful shutdown
runbox daemon status  # Check if running
```

**On daemon restart:**
- Read all runs with `Running` status from SQLite
- Check if PIDs still exist (`kill -0`)
- Mark dead processes as `Unknown` with reason "daemon restarted, process not found"

### 6. Storage: SQLite Migration

**Decision: SQLite** (replaces JSON files)

**Schema:**
```sql
CREATE TABLE runs (
    run_id TEXT PRIMARY KEY,
    status TEXT NOT NULL,  -- 'pending', 'running', 'exited', 'failed', 'unknown'
    exit_code INTEGER,
    pid INTEGER,
    pgid INTEGER,
    exec_json TEXT NOT NULL,  -- JSON blob of Exec struct
    code_state_json TEXT NOT NULL,  -- JSON blob of CodeState
    log_path TEXT,
    runtime TEXT NOT NULL,  -- 'background', 'tmux'
    created_at TEXT NOT NULL,
    started_at TEXT,
    ended_at TEXT,
    unknown_reason TEXT
);

CREATE TABLE templates (
    name TEXT PRIMARY KEY,
    template_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE playlists (
    name TEXT PRIMARY KEY,
    playlist_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX idx_runs_status ON runs(status);
CREATE INDEX idx_runs_created_at ON runs(created_at);
```

**Location:** `$XDG_DATA_HOME/runbox/runbox.db` (typically `~/.local/share/runbox/runbox.db`)

---

## Test Coverage Requirements

| Component | Required Tests |
|-----------|---------------|
| Daemon spawn/wait | 3+ tests |
| CLI-Daemon protocol | 3+ tests |
| Daemon restart reconcile | 2+ tests |
| Exit code capture (success/failure/signal) | 3+ tests |
| CAS status updates | 3+ tests |

### Required Test Cases

```rust
#[test]
fn test_daemon_spawn_and_exit_capture() {
    // Start daemon, spawn "true", verify Exited with exit_code=0
}

#[test]
fn test_daemon_spawn_failure_capture() {
    // Start daemon, spawn "false", verify Failed with exit_code=1
}

#[test]
fn test_daemon_signal_capture() {
    // Start daemon, spawn "sleep 60", send SIGTERM, verify exit_code=143
}

#[test]
fn test_daemon_survives_cli_exit() {
    // Start daemon, spawn long process, CLI exits, process still managed
}

#[test]
fn test_daemon_restart_reconcile() {
    // Daemon restarts, checks stale PIDs, marks dead as Unknown
}
```

---

## Acceptance Criteria

- [ ] `runbox-daemon` crate created
- [ ] Daemon spawns processes and captures exit status correctly
- [ ] CLI communicates with daemon via Unix socket
- [ ] Exit code captured for: success (0), failure (non-zero), signal (128+N)
- [ ] Daemon restart reconciles stale processes
- [ ] All new tests pass
- [ ] Existing tests continue to pass
- [ ] **Codex review score >= 80**

---

## Workflow

### Step 1: Design Review ✅

Decisions confirmed:
1. ✅ Storage: SQLite
2. ✅ Daemon auto-start (user never knows)
3. ✅ Socket path: `$XDG_RUNTIME_DIR/runbox/daemon.sock`

### Step 2: Implement Daemon

1. Create `runbox-daemon` crate
2. Implement Unix socket server
3. Implement process manager with wait threads
4. Add daemon CLI commands

### Step 3: Update CLI

1. Modify `BackgroundAdapter` to use daemon
2. Add daemon start/stop/status commands
3. Auto-start daemon if not running (optional)

### Step 4: Test & Review

```bash
cargo test --workspace
```

Request Codex review with score >= 80.

---

## Related

- ISSUE-005: Run Execution Management (original spec)
- ISSUE-006: Short ID Support
- `crates/runbox-core/src/runtime/background.rs`
- `crates/runbox-core/src/storage.rs`

## Supersedes

This issue supersedes the previous thread-based approach which was found to be fundamentally flawed (threads die with CLI process).

</issue>

Instructions:
- Implement the changes described in the issue above
- Run tests to verify your changes work correctly
- When complete, create a pull request targeting `main`:
  - Title should summarize the change
  - Body should reference issue: ISSUE-007
  - Include a summary of changes made
