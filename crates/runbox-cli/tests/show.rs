use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

/// Create a test run JSON directly in the storage directory
fn create_test_run(temp_dir: &TempDir, run_id: &str, status: &str, argv: Vec<&str>) {
    let runs_dir = temp_dir.path().join("runs");
    fs::create_dir_all(&runs_dir).unwrap();

    let run_json = serde_json::json!({
        "run_version": 0,
        "run_id": run_id,
        "exec": {
            "argv": argv,
            "cwd": ".",
            "env": {},
            "timeout_sec": 0
        },
        "code_state": {
            "repo_url": "git@github.com:test/repo.git",
            "base_commit": "a1b2c3d4e5f6789012345678901234567890abcd"
        },
        "status": status,
        "runtime": "background",
        "timeline": {
            "created_at": "2024-01-15T10:00:00Z",
            "started_at": "2024-01-15T10:00:01Z"
        }
    });

    let run_path = runs_dir.join(format!("{}.json", run_id));
    fs::write(&run_path, serde_json::to_string_pretty(&run_json).unwrap()).unwrap();
}

#[test]
fn test_show_run_details() {
    let temp = TempDir::new().unwrap();

    // Setup: create a run with known values
    create_test_run(
        &temp,
        "run_test1234-5678-90ab-cdef-111111111111",
        "running",
        vec!["echo", "hello", "world"],
    );

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["show", "run_test1234-5678-90ab-cdef-111111111111"])
        .assert()
        .success()
        .stdout(predicate::str::contains("run_test1234-5678-90ab-cdef-111111111111"))
        .stdout(predicate::str::contains("running"));
}

#[test]
fn test_show_with_short_id() {
    let temp = TempDir::new().unwrap();

    // Setup: create a run
    create_test_run(
        &temp,
        "run_abcd1234-5678-90ab-cdef-222222222222",
        "exited",
        vec!["python", "script.py"],
    );

    // Use only the first few characters of the run ID (after run_)
    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["show", "abcd12"])
        .assert()
        .success()
        .stdout(predicate::str::contains("run_abcd1234-5678-90ab-cdef-222222222222"))
        .stdout(predicate::str::contains("exited"));
}

#[test]
fn test_show_all_fields() {
    let temp = TempDir::new().unwrap();

    // Setup: create a run with all fields we want to verify
    let runs_dir = temp.path().join("runs");
    fs::create_dir_all(&runs_dir).unwrap();

    let run_json = serde_json::json!({
        "run_version": 0,
        "run_id": "run_fields-test-1234-5678-333333333333",
        "exec": {
            "argv": ["make", "test"],
            "cwd": "/project",
            "env": {"DEBUG": "1"},
            "timeout_sec": 300
        },
        "code_state": {
            "repo_url": "git@github.com:example/project.git",
            "base_commit": "deadbeef1234567890abcdef1234567890abcdef",
            "patch": {
                "ref": "refs/patches/run_fields-test-1234-5678-333333333333",
                "sha256": "abc123"
            }
        },
        "status": "exited",
        "runtime": "tmux",
        "timeline": {
            "created_at": "2024-01-15T10:00:00Z",
            "started_at": "2024-01-15T10:00:01Z",
            "ended_at": "2024-01-15T10:05:00Z"
        },
        "exit_code": 0,
        "reconcile_reason": "manual",
        "log_ref": {
            "path": "/tmp/test.log"
        }
    });

    let run_path = runs_dir.join("run_fields-test-1234-5678-333333333333.json");
    fs::write(&run_path, serde_json::to_string_pretty(&run_json).unwrap()).unwrap();

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["show", "fields"])
        .assert()
        .success()
        // Verify run identification
        .stdout(predicate::str::contains("Run ID:"))
        .stdout(predicate::str::contains("run_fields-test-1234-5678-333333333333"))
        .stdout(predicate::str::contains("Short ID:"))
        // Verify status and runtime
        .stdout(predicate::str::contains("Status:"))
        .stdout(predicate::str::contains("exited"))
        .stdout(predicate::str::contains("Runtime:"))
        .stdout(predicate::str::contains("tmux"))
        // Verify exec fields
        .stdout(predicate::str::contains("Command:"))
        .stdout(predicate::str::contains("make"))
        .stdout(predicate::str::contains("Cwd:"))
        .stdout(predicate::str::contains("Env:"))
        .stdout(predicate::str::contains("DEBUG"))
        // Verify code_state fields
        .stdout(predicate::str::contains("Repo:"))
        .stdout(predicate::str::contains("git@github.com:example/project.git"))
        .stdout(predicate::str::contains("Commit:"))
        .stdout(predicate::str::contains("deadbeef"))
        .stdout(predicate::str::contains("Patch:"))
        .stdout(predicate::str::contains("yes"))
        // Verify timeline
        .stdout(predicate::str::contains("Created:"))
        .stdout(predicate::str::contains("Started:"))
        .stdout(predicate::str::contains("Ended:"))
        // Verify exit code
        .stdout(predicate::str::contains("Exit Code:"))
        .stdout(predicate::str::contains("0"))
        // Verify reconcile reason
        .stdout(predicate::str::contains("Reconcile:"))
        .stdout(predicate::str::contains("manual"))
        // Verify log reference
        .stdout(predicate::str::contains("Log:"));
}

#[test]
fn test_show_not_found() {
    let temp = TempDir::new().unwrap();

    // Ensure the storage directories exist but are empty
    fs::create_dir_all(temp.path().join("runs")).unwrap();
    fs::create_dir_all(temp.path().join("templates")).unwrap();
    fs::create_dir_all(temp.path().join("playlists")).unwrap();
    fs::create_dir_all(temp.path().join("logs")).unwrap();

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["show", "nonexistent"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("No item found"));
}

#[test]
fn test_show_ambiguous_id() {
    let temp = TempDir::new().unwrap();

    // Create two runs with similar IDs (both start with "5aaa")
    create_test_run(
        &temp,
        "run_5aaa1111-1111-1111-1111-111111111111",
        "running",
        vec!["cmd1"],
    );
    create_test_run(
        &temp,
        "run_5aaa2222-2222-2222-2222-222222222222",
        "exited",
        vec!["cmd2"],
    );

    // Using a short prefix that matches both runs should fail
    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["show", "5aaa"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Ambiguous"));
}

#[test]
fn test_show_minimal_run() {
    let temp = TempDir::new().unwrap();

    // Create a minimal run (no optional fields)
    let runs_dir = temp.path().join("runs");
    fs::create_dir_all(&runs_dir).unwrap();

    let run_json = serde_json::json!({
        "run_version": 0,
        "run_id": "run_minimal-1234-5678-9abc-def012345678",
        "exec": {
            "argv": ["ls"],
            "cwd": "."
        },
        "code_state": {
            "repo_url": "git@github.com:test/minimal.git",
            "base_commit": "1111111111111111111111111111111111111111"
        }
    });

    let run_path = runs_dir.join("run_minimal-1234-5678-9abc-def012345678.json");
    fs::write(&run_path, serde_json::to_string_pretty(&run_json).unwrap()).unwrap();

    // Should still work and display available fields
    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["show", "minimal"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Run ID:"))
        .stdout(predicate::str::contains("run_minimal-1234-5678-9abc-def012345678"))
        .stdout(predicate::str::contains("Status:"))
        .stdout(predicate::str::contains("pending")) // default status
        .stdout(predicate::str::contains("Command:"))
        .stdout(predicate::str::contains("ls"))
        .stdout(predicate::str::contains("Repo:"));
}
