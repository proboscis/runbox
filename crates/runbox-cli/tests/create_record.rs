//! E2E tests for `runbox create record` command
//!
//! Tests external tool integration (Phase 5 of ISSUE-036)

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::process::Command as StdCommand;
use tempfile::TempDir;

/// Helper to create a runbox command with RUNBOX_HOME set to temp directory
fn runbox_cmd(temp_dir: &TempDir) -> Command {
    let mut cmd = Command::cargo_bin("runbox").unwrap();
    cmd.env("RUNBOX_HOME", temp_dir.path());
    cmd
}

fn setup_git_repo(temp_dir: &TempDir) -> (std::path::PathBuf, String) {
    let repo_path = temp_dir.path().join("repo");
    fs::create_dir_all(&repo_path).unwrap();

    StdCommand::new("git")
        .current_dir(&repo_path)
        .args(["init"])
        .output()
        .unwrap();
    StdCommand::new("git")
        .current_dir(&repo_path)
        .args(["config", "user.email", "test@test.com"])
        .output()
        .unwrap();
    StdCommand::new("git")
        .current_dir(&repo_path)
        .args(["config", "user.name", "Test User"])
        .output()
        .unwrap();

    fs::write(repo_path.join("README.md"), "# Test Repo\n").unwrap();
    StdCommand::new("git")
        .current_dir(&repo_path)
        .args(["add", "."])
        .output()
        .unwrap();
    StdCommand::new("git")
        .current_dir(&repo_path)
        .args(["commit", "-m", "Initial commit"])
        .output()
        .unwrap();

    let output = StdCommand::new("git")
        .current_dir(&repo_path)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();

    let commit = String::from_utf8(output.stdout).unwrap().trim().to_string();
    (repo_path, commit)
}

/// Minimal valid record JSON
fn minimal_record_json() -> String {
    r#"{"command": {"argv": ["echo", "hello"], "cwd": "."}}"#.to_string()
}

/// Full record JSON with all fields
fn full_record_json(record_id: &str) -> String {
    format!(
        r#"{{
    "id": "{}",
    "git_state": {{
        "repo_url": "git@github.com:org/repo.git",
        "commit": "a1b2c3d4e5f6789012345678901234567890abcd"
    }},
    "command": {{
        "argv": ["python", "train.py", "--epochs", "10"],
        "cwd": "src",
        "env": {{"CUDA_VISIBLE_DEVICES": "0"}}
    }},
    "exit_code": 0,
    "started_at": "2025-01-19T10:00:00Z",
    "ended_at": "2025-01-19T10:05:00Z",
    "tags": ["ml", "training"],
    "source": "doeff"
}}"#,
        record_id
    )
}

#[test]
fn test_create_record_from_stdin_minimal() {
    let temp = TempDir::new().unwrap();

    runbox_cmd(&temp)
        .args(["create", "record"])
        .write_stdin(minimal_record_json())
        .assert()
        .success()
        .stdout(predicate::str::contains("Created record: rec_"))
        .stdout(predicate::str::contains("Short ID:"))
        .stdout(predicate::str::contains("Command:"))
        .stdout(predicate::str::contains("Source:   external"));

    // Verify record was saved
    let records_dir = temp.path().join("records");
    assert!(records_dir.exists(), "Records directory should exist");

    let records: Vec<_> = std::fs::read_dir(&records_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert_eq!(records.len(), 1, "Should have exactly one record");
}

#[test]
fn test_create_record_from_file() {
    let temp = TempDir::new().unwrap();
    let record_file = temp.path().join("record.json");
    std::fs::write(&record_file, full_record_json("rec_test-from-file")).unwrap();

    runbox_cmd(&temp)
        .args([
            "create",
            "record",
            "--from-file",
            record_file.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Created record: rec_test-from-file",
        ))
        .stdout(predicate::str::contains("Source:   doeff"));

    // Verify the specific file was created
    let record_path = temp.path().join("records").join("rec_test-from-file.json");
    assert!(record_path.exists(), "Record file should exist");

    // Verify content
    let saved = std::fs::read_to_string(&record_path).unwrap();
    let json: serde_json::Value = serde_json::from_str(&saved).unwrap();
    assert_eq!(json["record_id"], "rec_test-from-file");
    assert_eq!(json["source"], "doeff");
    assert_eq!(json["exit_code"], 0);
    assert_eq!(json["tags"], serde_json::json!(["ml", "training"]));
}

#[test]
fn test_create_record_auto_generates_id() {
    let temp = TempDir::new().unwrap();

    // Record without explicit ID
    let json_no_id = r#"{"command": {"argv": ["echo"], "cwd": "."}}"#;

    let output = runbox_cmd(&temp)
        .args(["create", "record"])
        .write_stdin(json_no_id)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let stdout = String::from_utf8(output).unwrap();
    assert!(
        stdout.contains("Created record: rec_"),
        "Should auto-generate rec_ ID"
    );

    // Extract the generated ID and verify it's a valid UUID format
    let id_line = stdout
        .lines()
        .find(|l| l.contains("Created record:"))
        .unwrap();
    let id = id_line.split("Created record: ").nth(1).unwrap().trim();
    assert!(id.starts_with("rec_"), "ID should start with rec_");
    assert!(id.len() > 10, "ID should be long enough to contain UUID");
}

#[test]
fn test_create_record_with_explicit_id() {
    let temp = TempDir::new().unwrap();

    runbox_cmd(&temp)
        .args(["create", "record"])
        .write_stdin(full_record_json("rec_my-custom-id"))
        .assert()
        .success()
        .stdout(predicate::str::contains("Created record: rec_my-custom-id"));

    // Verify exact file was created
    let record_path = temp.path().join("records").join("rec_my-custom-id.json");
    assert!(record_path.exists());
}

#[test]
fn test_create_record_missing_command_fails() {
    let temp = TempDir::new().unwrap();

    // Missing command field
    let invalid_json = r#"{"git_state": {"repo_url": "test", "commit": "abc"}}"#;

    runbox_cmd(&temp)
        .args(["create", "record"])
        .write_stdin(invalid_json)
        .assert()
        .failure()
        .stderr(predicate::str::contains("Missing 'command' field"));
}

#[test]
fn test_create_record_empty_argv_fails() {
    let temp = TempDir::new().unwrap();

    // Empty argv
    let invalid_json = r#"{"command": {"argv": [], "cwd": "."}}"#;

    runbox_cmd(&temp)
        .args(["create", "record"])
        .write_stdin(invalid_json)
        .assert()
        .failure()
        .stderr(predicate::str::contains("argv must not be empty"));
}

#[test]
fn test_create_record_invalid_json_fails() {
    let temp = TempDir::new().unwrap();

    runbox_cmd(&temp)
        .args(["create", "record"])
        .write_stdin("not valid json")
        .assert()
        .failure()
        .stderr(predicate::str::contains("Invalid JSON"));
}

#[test]
fn test_create_record_file_not_found_fails() {
    let temp = TempDir::new().unwrap();

    runbox_cmd(&temp)
        .args(["create", "record", "--from-file", "/nonexistent/file.json"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Failed to read file"));
}

#[test]
fn test_create_record_with_tags() {
    let temp = TempDir::new().unwrap();

    let json_with_tags = r#"{
        "command": {"argv": ["echo"], "cwd": "."},
        "tags": ["test", "ci", "automated"]
    }"#;

    runbox_cmd(&temp)
        .args(["create", "record"])
        .write_stdin(json_with_tags)
        .assert()
        .success();

    // Verify tags were saved
    let records_dir = temp.path().join("records");
    let record_file = std::fs::read_dir(&records_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .next()
        .unwrap();

    let content = std::fs::read_to_string(record_file.path()).unwrap();
    let json: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert_eq!(json["tags"], serde_json::json!(["test", "ci", "automated"]));
}

#[test]
fn test_create_record_preserves_env_vars() {
    let temp = TempDir::new().unwrap();

    let json_with_env = r#"{
        "command": {
            "argv": ["python", "script.py"],
            "cwd": ".",
            "env": {"FOO": "bar", "DEBUG": "true"}
        }
    }"#;

    runbox_cmd(&temp)
        .args(["create", "record"])
        .write_stdin(json_with_env)
        .assert()
        .success();

    // Verify env was saved
    let records_dir = temp.path().join("records");
    let record_file = std::fs::read_dir(&records_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .next()
        .unwrap();

    let content = std::fs::read_to_string(record_file.path()).unwrap();
    let json: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert_eq!(json["command"]["env"]["FOO"], "bar");
    assert_eq!(json["command"]["env"]["DEBUG"], "true");
}

#[test]
fn test_create_record_materializes_git_diff_into_patch_ref() {
    let temp = TempDir::new().unwrap();
    let (repo_path, commit) = setup_git_repo(&temp);

    fs::write(
        repo_path.join("README.md"),
        "# Test Repo\npatched from diff\n",
    )
    .unwrap();
    let diff_output = StdCommand::new("git")
        .current_dir(&repo_path)
        .args(["diff", "HEAD"])
        .output()
        .unwrap();
    let diff = String::from_utf8(diff_output.stdout)
        .unwrap()
        .trim_end_matches('\n')
        .to_string();
    StdCommand::new("git")
        .current_dir(&repo_path)
        .args(["checkout", "--", "README.md"])
        .output()
        .unwrap();

    let input = serde_json::json!({
        "id": "rec_from-diff",
        "git_state": {
            "repo_url": "git@github.com:org/repo.git",
            "commit": commit,
            "diff": diff,
        },
        "command": {
            "argv": ["cat", "README.md"],
            "cwd": ".",
        },
        "source": "doeff",
    });

    runbox_cmd(&temp)
        .current_dir(&repo_path)
        .args(["create", "record"])
        .write_stdin(input.to_string())
        .assert()
        .success()
        .stdout(predicate::str::contains("Created record: rec_from-diff"));

    let record_path = temp.path().join("records").join("rec_from-diff.json");
    let saved = fs::read_to_string(&record_path).unwrap();
    let json: serde_json::Value = serde_json::from_str(&saved).unwrap();
    assert_eq!(json["record_id"], "rec_from-diff");
    assert_eq!(json["git_state"]["patch_ref"], "refs/patches/rec_from-diff");
    assert!(json["git_state"].get("diff").is_none());

    let worktree_dir = temp.path().join("worktrees");
    fs::create_dir_all(&worktree_dir).unwrap();

    runbox_cmd(&temp)
        .current_dir(&repo_path)
        .args([
            "replay",
            "rec_from-diff",
            "--worktree-dir",
            worktree_dir.to_str().unwrap(),
            "--keep",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("patched from diff"));
}

#[test]
fn test_create_record_diff_fails_when_command_cwd_is_not_git_repo() {
    let temp = TempDir::new().unwrap();
    let non_repo_dir = temp.path().join("not_a_repo");
    fs::create_dir_all(&non_repo_dir).unwrap();

    let input = serde_json::json!({
        "id": "rec_bad-diff",
        "git_state": {
            "commit": "a1b2c3d4e5f6789012345678901234567890abcd",
            "diff": "diff --git a/README.md b/README.md\n",
        },
        "command": {
            "argv": ["echo", "hello"],
            "cwd": non_repo_dir,
        },
    });

    runbox_cmd(&temp)
        .args(["create", "record"])
        .write_stdin(input.to_string())
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "git_state.diff requires command.cwd to point at a git repository",
        ));
}
