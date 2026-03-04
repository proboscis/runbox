use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

/// Create a minimal run JSON for testing
fn create_run_json(run_id: &str, log_path: Option<&str>) -> String {
    let log_ref = if let Some(path) = log_path {
        format!(r#""log_ref": {{"path": "{}"}}"#, path)
    } else {
        String::new()
    };

    let log_ref_field = if log_ref.is_empty() {
        String::new()
    } else {
        format!(",\n    {}", log_ref)
    };

    format!(
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
    "status": "exited"{}
}}"#,
        run_id, log_ref_field
    )
}

#[test]
fn test_logs_shows_content() {
    let temp = TempDir::new().unwrap();

    // Create required directories
    let runs_dir = temp.path().join("runs");
    let logs_dir = temp.path().join("logs");
    fs::create_dir_all(&runs_dir).unwrap();
    fs::create_dir_all(&logs_dir).unwrap();

    let run_id = "run_550e8400-e29b-41d4-a716-446655440000";
    let log_path = logs_dir.join(format!("{}.log", run_id));

    // Create run file with log_ref pointing to the log file
    let run_json = create_run_json(run_id, Some(log_path.to_str().unwrap()));
    fs::write(runs_dir.join(format!("{}.json", run_id)), run_json).unwrap();

    // Create log file with known content
    let log_content = "Line 1: Hello World\nLine 2: Test output\nLine 3: Final line";
    fs::write(&log_path, log_content).unwrap();

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["logs", run_id])
        .assert()
        .success()
        .stdout(predicate::str::contains("Hello World"))
        .stdout(predicate::str::contains("Test output"))
        .stdout(predicate::str::contains("Final line"));
}

#[test]
fn test_logs_with_short_id() {
    let temp = TempDir::new().unwrap();

    // Create required directories
    let runs_dir = temp.path().join("runs");
    let logs_dir = temp.path().join("logs");
    fs::create_dir_all(&runs_dir).unwrap();
    fs::create_dir_all(&logs_dir).unwrap();

    let run_id = "run_abcd1234-e5f6-7890-abcd-ef1234567890";
    let short_id = "abcd1234"; // First 8 hex chars after "run_"
    let log_path = logs_dir.join(format!("{}.log", run_id));

    // Create run file
    let run_json = create_run_json(run_id, Some(log_path.to_str().unwrap()));
    fs::write(runs_dir.join(format!("{}.json", run_id)), run_json).unwrap();

    // Create log file with known content
    let log_content = "Short ID test log content";
    fs::write(&log_path, log_content).unwrap();

    // Use short ID to access logs
    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["logs", short_id])
        .assert()
        .success()
        .stdout(predicate::str::contains("Short ID test log content"));
}

#[test]
fn test_logs_run_not_found() {
    let temp = TempDir::new().unwrap();

    // Create required directories (empty)
    fs::create_dir_all(temp.path().join("runs")).unwrap();
    fs::create_dir_all(temp.path().join("logs")).unwrap();

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["logs", "nonexistent"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("No run found matching"));
}

#[test]
fn test_logs_log_file_not_found() {
    let temp = TempDir::new().unwrap();

    // Create required directories
    let runs_dir = temp.path().join("runs");
    let logs_dir = temp.path().join("logs");
    fs::create_dir_all(&runs_dir).unwrap();
    fs::create_dir_all(&logs_dir).unwrap();

    let run_id = "run_deadbeef-1234-5678-abcd-ef1234567890";
    let log_path = logs_dir.join(format!("{}.log", run_id));

    // Create run file with log_ref, but don't create the actual log file
    let run_json = create_run_json(run_id, Some(log_path.to_str().unwrap()));
    fs::write(runs_dir.join(format!("{}.json", run_id)), run_json).unwrap();

    // Log file doesn't exist - should fail
    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["logs", run_id])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Log file not found"));
}

#[test]
fn test_logs_with_lines_option() {
    let temp = TempDir::new().unwrap();

    // Create required directories
    let runs_dir = temp.path().join("runs");
    let logs_dir = temp.path().join("logs");
    fs::create_dir_all(&runs_dir).unwrap();
    fs::create_dir_all(&logs_dir).unwrap();

    let run_id = "run_11112222-3333-4444-5555-666677778888";
    let log_path = logs_dir.join(format!("{}.log", run_id));

    // Create run file
    let run_json = create_run_json(run_id, Some(log_path.to_str().unwrap()));
    fs::write(runs_dir.join(format!("{}.json", run_id)), run_json).unwrap();

    // Create log file with multiple lines
    let log_content = "Line 1\nLine 2\nLine 3\nLine 4\nLine 5";
    fs::write(&log_path, log_content).unwrap();

    // Request only last 2 lines
    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["logs", "--lines", "2", run_id])
        .assert()
        .success()
        .stdout(predicate::str::contains("Line 4"))
        .stdout(predicate::str::contains("Line 5"))
        // Should NOT contain first lines
        .stdout(predicate::str::contains("Line 1").not());
}
