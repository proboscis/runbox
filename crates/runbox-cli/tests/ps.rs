use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

/// Helper to create a run JSON file in the runs directory
fn create_run_file(temp: &TempDir, run_id: &str, status: &str, command: &[&str], runtime: &str) {
    let runs_dir = temp.path().join("runs");
    fs::create_dir_all(&runs_dir).unwrap();

    let argv_json: Vec<String> = command.iter().map(|s| format!("\"{}\"", s)).collect();
    let run_json = format!(
        r#"{{
    "run_version": 0,
    "run_id": "{}",
    "exec": {{
        "argv": [{}],
        "cwd": ".",
        "env": {{}},
        "timeout_sec": 0
    }},
    "code_state": {{
        "repo_url": "git@github.com:org/repo.git",
        "base_commit": "a1b2c3d4e5f6789012345678901234567890abcd"
    }},
    "status": "{}",
    "runtime": "{}"
}}"#,
        run_id,
        argv_json.join(", "),
        status,
        runtime
    );

    fs::write(runs_dir.join(format!("{}.json", run_id)), run_json).unwrap();
}

#[test]
fn test_ps_empty() {
    let temp = TempDir::new().unwrap();

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["ps"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No runs found."));
}

#[test]
fn test_ps_with_runs() {
    let temp = TempDir::new().unwrap();

    // Create a run
    create_run_file(
        &temp,
        "run_550e8400-e29b-41d4-a716-446655440000",
        "exited",
        &["echo", "hello"],
        "background",
    );

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["ps"])
        .assert()
        .success()
        // Verify short ID is shown (first 8 chars of UUID)
        .stdout(predicate::str::contains("550e8400"))
        // Verify status
        .stdout(predicate::str::contains("exited"))
        // Verify command is shown
        .stdout(predicate::str::contains("echo hello"));
}

#[test]
fn test_ps_status_filter() {
    let temp = TempDir::new().unwrap();

    // Create runs with different statuses
    create_run_file(
        &temp,
        "run_aaaa0000-e29b-41d4-a716-446655440000",
        "exited",
        &["echo", "one"],
        "background",
    );
    create_run_file(
        &temp,
        "run_bbbb0000-e29b-41d4-a716-446655440000",
        "pending",
        &["echo", "two"],
        "background",
    );

    // Filter by exited status
    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["ps", "--status", "exited"])
        .assert()
        .success()
        // Should show exited run
        .stdout(predicate::str::contains("aaaa0000"))
        .stdout(predicate::str::contains("exited"))
        // Should NOT show pending run
        .stdout(predicate::str::contains("bbbb0000").not());

    // Filter by pending status
    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["ps", "--status", "pending"])
        .assert()
        .success()
        // Should show pending run
        .stdout(predicate::str::contains("bbbb0000"))
        .stdout(predicate::str::contains("pending"))
        // Should NOT show exited run
        .stdout(predicate::str::contains("aaaa0000").not());
}

#[test]
fn test_ps_limit() {
    let temp = TempDir::new().unwrap();

    // Create multiple runs
    for i in 0..5 {
        create_run_file(
            &temp,
            &format!("run_{:08x}-e29b-41d4-a716-446655440000", i),
            "exited",
            &["echo", &format!("run{}", i)],
            "background",
        );
        // Add a small delay to ensure different modification times
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    // Limit to 2 results
    let output = Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["ps", "--limit", "2"])
        .assert()
        .success();

    // Count the number of data rows (excluding header and separator)
    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    let data_lines: Vec<&str> = stdout
        .lines()
        .filter(|line| !line.starts_with("SHORT") && !line.starts_with("-"))
        .filter(|line| !line.is_empty())
        .collect();

    assert_eq!(
        data_lines.len(),
        2,
        "Expected 2 runs but got {}: {:?}",
        data_lines.len(),
        data_lines
    );
}

#[test]
fn test_ps_output_format() {
    let temp = TempDir::new().unwrap();

    // Use "exited" status since "running" would be reconciled to "unknown"
    // when there's no live process
    create_run_file(
        &temp,
        "run_deadbeef-1234-5678-abcd-ef1234567890",
        "exited",
        &["python", "-m", "test"],
        "tmux",
    );

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["ps"])
        .assert()
        .success()
        // Verify table headers
        .stdout(predicate::str::contains("SHORT ID"))
        .stdout(predicate::str::contains("STATUS"))
        .stdout(predicate::str::contains("RUNTIME"))
        .stdout(predicate::str::contains("COMMAND"))
        // Verify separator line
        .stdout(predicate::str::contains("---"))
        // Verify data
        .stdout(predicate::str::contains("deadbeef"))
        .stdout(predicate::str::contains("exited"))
        .stdout(predicate::str::contains("tmux"))
        .stdout(predicate::str::contains("python -m test"));
}

#[test]
fn test_ps_reconcile_marks_stale_running_as_unknown() {
    let temp = TempDir::new().unwrap();

    // Create a run with "running" status but with a non-existent PID
    let runs_dir = temp.path().join("runs");
    fs::create_dir_all(&runs_dir).unwrap();

    // Use a PID that definitely doesn't exist (99999999)
    let run_json = r#"{
    "run_version": 0,
    "run_id": "run_cafebabe-1234-5678-abcd-ef1234567890",
    "exec": {
        "argv": ["sleep", "1000"],
        "cwd": ".",
        "env": {},
        "timeout_sec": 0
    },
    "code_state": {
        "repo_url": "git@github.com:org/repo.git",
        "base_commit": "a1b2c3d4e5f6789012345678901234567890abcd"
    },
    "status": "running",
    "runtime": "background",
    "handle": {
        "type": "Background",
        "pid": 99999999,
        "pgid": 99999999
    }
}"#;

    fs::write(
        runs_dir.join("run_cafebabe-1234-5678-abcd-ef1234567890.json"),
        run_json,
    )
    .unwrap();

    // Run ps - this should trigger reconciliation
    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["ps"])
        .assert()
        .success()
        // After reconciliation, status should be "unknown" not "running"
        .stdout(predicate::str::contains("unknown"))
        .stdout(predicate::str::contains("cafebabe"));

    // Verify the run file was updated
    let run_content =
        fs::read_to_string(runs_dir.join("run_cafebabe-1234-5678-abcd-ef1234567890.json")).unwrap();
    assert!(
        run_content.contains("\"status\": \"unknown\""),
        "Run status should be updated to unknown in the file"
    );
}

#[test]
fn test_ps_multiple_runs_sorted() {
    let temp = TempDir::new().unwrap();

    // Create runs with specific file modification order
    // Note: list_runs sorts by modification time, newest first
    create_run_file(
        &temp,
        "run_11110000-e29b-41d4-a716-446655440000",
        "exited",
        &["echo", "first"],
        "background",
    );
    std::thread::sleep(std::time::Duration::from_millis(50));

    create_run_file(
        &temp,
        "run_22220000-e29b-41d4-a716-446655440000",
        "pending",
        &["echo", "second"],
        "background",
    );
    std::thread::sleep(std::time::Duration::from_millis(50));

    create_run_file(
        &temp,
        "run_33330000-e29b-41d4-a716-446655440000",
        "running",
        &["echo", "third"],
        "tmux",
    );

    let output = Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["ps"])
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);

    // All three runs should be present
    assert!(stdout.contains("11110000"));
    assert!(stdout.contains("22220000"));
    assert!(stdout.contains("33330000"));

    // Verify newest (33330000) appears before oldest (11110000)
    let pos_third = stdout.find("33330000").unwrap();
    let pos_first = stdout.find("11110000").unwrap();
    assert!(
        pos_third < pos_first,
        "Newest run should appear first (sorted by modification time)"
    );
}

#[test]
fn test_ps_command_truncation() {
    let temp = TempDir::new().unwrap();

    // Create a run with a very long command
    create_run_file(
        &temp,
        "run_abcd0000-e29b-41d4-a716-446655440000",
        "exited",
        &[
            "python",
            "-c",
            "print('this is a very long command that should be truncated')",
        ],
        "background",
    );

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["ps"])
        .assert()
        .success()
        // Command should be truncated with "..."
        .stdout(predicate::str::contains("..."))
        .stdout(predicate::str::contains("abcd0000"));
}

#[test]
fn test_ps_empty_runtime() {
    let temp = TempDir::new().unwrap();

    // Create a run with empty runtime (legacy run)
    create_run_file(
        &temp,
        "run_fefe0000-e29b-41d4-a716-446655440000",
        "exited",
        &["echo", "legacy"],
        "", // empty runtime
    );

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["ps"])
        .assert()
        .success()
        // Empty runtime should show as "-"
        .stdout(predicate::str::contains("-"))
        .stdout(predicate::str::contains("fefe0000"));
}
