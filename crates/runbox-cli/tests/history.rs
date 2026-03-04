use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

#[test]
fn test_history_empty() {
    let temp = TempDir::new().unwrap();

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["history"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No runs found."));
}

#[test]
fn test_history_with_runs() {
    let temp = TempDir::new().unwrap();

    // Create runs directory
    let runs_dir = temp.path().join("runs");
    fs::create_dir_all(&runs_dir).unwrap();

    // Create a run file
    let run = r#"{
        "run_version": 0,
        "run_id": "run_a1b2c3d4-e5f6-7890-abcd-ef1234567890",
        "exec": {
            "argv": ["echo", "hello", "world"],
            "cwd": ".",
            "env": {},
            "timeout_sec": 0
        },
        "code_state": {
            "repo_url": "git@github.com:org/repo.git",
            "base_commit": "abcd1234567890abcdef1234567890abcdef1234"
        },
        "status": "exited",
        "runtime": "background",
        "timeline": {}
    }"#;

    fs::write(
        runs_dir.join("run_a1b2c3d4-e5f6-7890-abcd-ef1234567890.json"),
        run,
    )
    .unwrap();

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["history"])
        .assert()
        .success()
        // Verify short ID (first 8 hex chars of run_id)
        .stdout(predicate::str::contains("a1b2c3d4"))
        // Verify command is shown
        .stdout(predicate::str::contains("echo hello world"));
}

#[test]
fn test_history_output_format() {
    let temp = TempDir::new().unwrap();

    // Create runs directory
    let runs_dir = temp.path().join("runs");
    fs::create_dir_all(&runs_dir).unwrap();

    // Create a run file
    let run = r#"{
        "run_version": 0,
        "run_id": "run_deadbeef-1234-5678-abcd-ef1234567890",
        "exec": {
            "argv": ["python", "-m", "pytest"],
            "cwd": ".",
            "env": {},
            "timeout_sec": 0
        },
        "code_state": {
            "repo_url": "git@github.com:org/repo.git",
            "base_commit": "1234567890abcdef1234567890abcdef12345678"
        },
        "status": "exited",
        "runtime": "background",
        "timeline": {}
    }"#;

    fs::write(
        runs_dir.join("run_deadbeef-1234-5678-abcd-ef1234567890.json"),
        run,
    )
    .unwrap();

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["history"])
        .assert()
        .success()
        // Verify table headers
        .stdout(predicate::str::contains("ID"))
        .stdout(predicate::str::contains("COMMAND"))
        // Verify run_id short form (first 8 hex chars)
        .stdout(predicate::str::contains("deadbeef"))
        // Verify command
        .stdout(predicate::str::contains("python -m pytest"));
}

#[test]
fn test_history_limit() {
    let temp = TempDir::new().unwrap();

    // Create runs directory
    let runs_dir = temp.path().join("runs");
    fs::create_dir_all(&runs_dir).unwrap();

    // Create 10 run files
    for i in 0..10 {
        let run = format!(
            r#"{{
            "run_version": 0,
            "run_id": "run_{:08x}-0000-0000-0000-000000000000",
            "exec": {{
                "argv": ["echo", "run{}"],
                "cwd": ".",
                "env": {{}},
                "timeout_sec": 0
            }},
            "code_state": {{
                "repo_url": "git@github.com:org/repo.git",
                "base_commit": "1234567890abcdef1234567890abcdef12345678"
            }},
            "status": "exited",
            "runtime": "background",
            "timeline": {{}}
        }}"#,
            i, i
        );

        fs::write(
            runs_dir.join(format!("run_{:08x}-0000-0000-0000-000000000000.json", i)),
            run,
        )
        .unwrap();
    }

    // Test with limit of 5
    let output = Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["history", "-n", "5"])
        .assert()
        .success();

    // Count the number of data rows (excluding header and separator)
    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    let data_lines: Vec<&str> = stdout
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with("ID") && !line.starts_with('-'))
        .collect();

    assert_eq!(
        data_lines.len(),
        5,
        "Expected 5 entries, got {}:\n{}",
        data_lines.len(),
        stdout
    );
}

#[test]
fn test_history_chronological_order() {
    let temp = TempDir::new().unwrap();

    // Create runs directory
    let runs_dir = temp.path().join("runs");
    fs::create_dir_all(&runs_dir).unwrap();

    // Create runs with distinct IDs - file modification time determines order
    let run1 = r#"{
        "run_version": 0,
        "run_id": "run_11111111-0000-0000-0000-000000000000",
        "exec": {
            "argv": ["echo", "first"],
            "cwd": ".",
            "env": {},
            "timeout_sec": 0
        },
        "code_state": {
            "repo_url": "git@github.com:org/repo.git",
            "base_commit": "1234567890abcdef1234567890abcdef12345678"
        },
        "status": "exited",
        "runtime": "background",
        "timeline": {}
    }"#;

    let run2 = r#"{
        "run_version": 0,
        "run_id": "run_22222222-0000-0000-0000-000000000000",
        "exec": {
            "argv": ["echo", "second"],
            "cwd": ".",
            "env": {},
            "timeout_sec": 0
        },
        "code_state": {
            "repo_url": "git@github.com:org/repo.git",
            "base_commit": "1234567890abcdef1234567890abcdef12345678"
        },
        "status": "exited",
        "runtime": "background",
        "timeline": {}
    }"#;

    // Write first run
    fs::write(
        runs_dir.join("run_11111111-0000-0000-0000-000000000000.json"),
        run1,
    )
    .unwrap();

    // Small delay to ensure different modification times
    std::thread::sleep(std::time::Duration::from_millis(50));

    // Write second run (newer)
    fs::write(
        runs_dir.join("run_22222222-0000-0000-0000-000000000000.json"),
        run2,
    )
    .unwrap();

    let output = Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["history"])
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);

    // Newer run (22222222) should appear before older run (11111111)
    let pos_newer = stdout.find("22222222").expect("Should find newer run");
    let pos_older = stdout.find("11111111").expect("Should find older run");

    assert!(
        pos_newer < pos_older,
        "Newer run should appear first (sorted by modification time, newest first)"
    );
}
