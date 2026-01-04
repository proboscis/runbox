use assert_cmd::Command;
use predicates::prelude::*;
use runbox_core::runtime::BackgroundAdapter;
use runbox_core::{CodeState, Exec, Run, RunStatus, RuntimeAdapter, RuntimeHandle, Storage};
use std::collections::HashMap;
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

const BASE_COMMIT: &str = "a1b2c3d4e5f6789012345678901234567890abcd";

fn base_code_state() -> CodeState {
    CodeState {
        repo_url: "git@github.com:org/repo.git".to_string(),
        base_commit: BASE_COMMIT.to_string(),
        patch: None,
    }
}

fn exec_from(argv: &[&str]) -> Exec {
    Exec {
        argv: argv.iter().map(|arg| (*arg).to_string()).collect(),
        cwd: ".".to_string(),
        env: HashMap::new(),
        timeout_sec: 0,
    }
}

fn write_run(temp: &TempDir, run: &Run) {
    let storage = Storage::with_base_dir(temp.path().to_path_buf()).unwrap();
    storage.save_run(run).unwrap();
}

fn dummy_handle() -> RuntimeHandle {
    RuntimeHandle::Background {
        pid: 99999,
        pgid: 99999,
    }
}

fn create_run(temp: &TempDir, run_id: &str, status: RunStatus, handle: Option<RuntimeHandle>) {
    let exec = exec_from(&["echo", "hello"]);
    let mut run = Run::new(exec, base_code_state());
    run.run_id = run_id.to_string();
    run.status = status;
    run.runtime = "background".to_string();
    run.handle = handle;
    write_run(temp, &run);
}

fn spawn_running_process(temp: &TempDir, run_id: &str, argv: &[&str]) -> RuntimeHandle {
    let storage = Storage::with_base_dir(temp.path().to_path_buf()).unwrap();
    let adapter = BackgroundAdapter::without_daemon();
    let exec = exec_from(argv);
    let handle = adapter
        .spawn(&exec, run_id, &storage.log_path(run_id))
        .unwrap();

    let mut run = Run::new(exec, base_code_state());
    run.run_id = run_id.to_string();
    run.status = RunStatus::Running;
    run.runtime = "background".to_string();
    run.handle = Some(handle.clone());
    write_run(temp, &run);

    handle
}

fn wait_for_exit(handle: &RuntimeHandle, timeout: Duration) -> bool {
    let adapter = BackgroundAdapter::without_daemon();
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if !adapter.is_alive(handle) {
            return true;
        }
        thread::sleep(Duration::from_millis(50));
    }
    false
}

struct BackgroundProcessGuard {
    handle: RuntimeHandle,
}

impl BackgroundProcessGuard {
    fn new(handle: RuntimeHandle) -> Self {
        Self { handle }
    }
}

impl Drop for BackgroundProcessGuard {
    fn drop(&mut self) {
        let adapter = BackgroundAdapter::without_daemon();
        if adapter.is_alive(&self.handle) {
            let _ = adapter.stop(&self.handle, true);
            let _ = wait_for_exit(&self.handle, Duration::from_secs(2));
        }
    }
}

#[test]
fn test_stop_not_found() {
    let temp = TempDir::new().unwrap();

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["stop", "nonexistent"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("No run found"));
}

#[test]
fn test_stop_already_exited() {
    let temp = TempDir::new().unwrap();
    let run_id = "run_deadbeef-1234-5678-abcd-ef1234567890";

    // Create a run with "exited" status
    create_run(&temp, run_id, RunStatus::Exited, Some(dummy_handle()));

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["stop", run_id])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not running"));
}

#[test]
fn test_stop_already_killed() {
    let temp = TempDir::new().unwrap();
    let run_id = "run_cafebabe-1234-5678-abcd-ef1234567890";

    // Create a run with "killed" status
    create_run(&temp, run_id, RunStatus::Killed, Some(dummy_handle()));

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["stop", run_id])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not running"));
}

#[test]
fn test_stop_running_process() {
    let temp = TempDir::new().unwrap();
    let run_id = "run_12345678-aaaa-bbbb-cccc-ddddeeeeeeee";

    let handle = spawn_running_process(&temp, run_id, &["sleep", "60"]);
    let _guard = BackgroundProcessGuard::new(handle.clone());

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["stop", run_id])
        .assert()
        .success()
        .stdout(predicate::str::contains("Stopped run"));

    assert!(
        wait_for_exit(&handle, Duration::from_secs(2)),
        "process should stop after runbox stop"
    );

    let storage = Storage::with_base_dir(temp.path().to_path_buf()).unwrap();
    let run = storage.load_run(run_id).unwrap();
    assert_eq!(run.status, RunStatus::Killed);
}

#[test]
fn test_stop_with_short_id() {
    let temp = TempDir::new().unwrap();
    let run_id = "run_abcd1234-aaaa-bbbb-cccc-ddddeeeeeeee";
    let short_id = "abcd1234";

    let handle = spawn_running_process(&temp, run_id, &["sleep", "60"]);
    let _guard = BackgroundProcessGuard::new(handle.clone());

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["stop", short_id])
        .assert()
        .success()
        .stdout(predicate::str::contains("Stopped run"));

    assert!(
        wait_for_exit(&handle, Duration::from_secs(2)),
        "process should stop after runbox stop"
    );
}

#[test]
fn test_stop_force() {
    let temp = TempDir::new().unwrap();
    let run_id = "run_force123-aaaa-bbbb-cccc-ddddeeeeeeee";

    let handle = spawn_running_process(
        &temp,
        run_id,
        &["sh", "-c", "trap '' TERM; sleep 60"],
    );
    let _guard = BackgroundProcessGuard::new(handle.clone());

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["stop", "--force", run_id])
        .assert()
        .success()
        .stdout(predicate::str::contains("Force stopped run"));

    assert!(
        wait_for_exit(&handle, Duration::from_secs(2)),
        "process should stop after runbox stop --force"
    );
}

#[test]
fn test_stop_no_handle() {
    let temp = TempDir::new().unwrap();
    let run_id = "run_nohandle1-aaaa-bbbb-cccc-ddddeeeeeeee";

    // Create a run with "running" status but no handle
    create_run(&temp, run_id, RunStatus::Running, None);

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["stop", run_id])
        .assert()
        .failure()
        .stderr(predicate::str::contains("has no handle"));
}
