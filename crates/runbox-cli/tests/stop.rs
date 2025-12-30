use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

/// Create a run JSON file with the given status and optional handle
fn create_run(temp: &TempDir, run_id: &str, status: &str, with_handle: bool) {
    let runs_dir = temp.path().join("runs");
    fs::create_dir_all(&runs_dir).unwrap();

    let handle = if with_handle {
        r#",
        "handle": {
            "type": "Background",
            "pid": 99999,
            "pgid": 99999
        }"#
    } else {
        ""
    };

    let run = format!(
        r#"{{
        "run_version": 0,
        "run_id": "{}",
        "exec": {{
            "argv": ["echo", "hello"],
            "cwd": ".",
            "env": {{}},
            "timeout_sec": 0
        }},
        "code_state": {{
            "repo_url": "git@github.com:org/repo.git",
            "base_commit": "a1b2c3d4e5f6789012345678901234567890abcd"
        }},
        "status": "{}",
        "runtime": "background"{}
    }}"#,
        run_id, status, handle
    );

    fs::write(runs_dir.join(format!("{}.json", run_id)), run).unwrap();
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
    create_run(&temp, run_id, "exited", true);

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
    create_run(&temp, run_id, "killed", true);

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

    // Create a run with "running" status and a handle
    // The PID doesn't need to exist - the stop command will succeed even if the process
    // is already gone (ESRCH is handled gracefully)
    create_run(&temp, run_id, "running", true);

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["stop", run_id])
        .assert()
        .success()
        .stdout(predicate::str::contains("Stopped run"));
}

#[test]
fn test_stop_with_short_id() {
    let temp = TempDir::new().unwrap();
    let run_id = "run_abcd1234-aaaa-bbbb-cccc-ddddeeeeeeee";
    let short_id = "abcd1234";

    // Create a run with "running" status
    create_run(&temp, run_id, "running", true);

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["stop", short_id])
        .assert()
        .success()
        .stdout(predicate::str::contains("Stopped run"));
}

#[test]
fn test_stop_force() {
    let temp = TempDir::new().unwrap();
    let run_id = "run_force123-aaaa-bbbb-cccc-ddddeeeeeeee";

    // Create a run with "running" status
    create_run(&temp, run_id, "running", true);

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["stop", "--force", run_id])
        .assert()
        .success()
        .stdout(predicate::str::contains("Force stopped run"));
}

#[test]
fn test_stop_no_handle() {
    let temp = TempDir::new().unwrap();
    let run_id = "run_nohandle1-aaaa-bbbb-cccc-ddddeeeeeeee";

    // Create a run with "running" status but no handle
    create_run(&temp, run_id, "running", false);

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["stop", run_id])
        .assert()
        .failure()
        .stderr(predicate::str::contains("has no handle"));
}
