use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::process::Command as StdCommand;
use tempfile::TempDir;

/// Helper to set up a git repository for testing
fn setup_git_repo(temp: &TempDir) -> String {
    let repo_path = temp.path().join("repo");
    fs::create_dir_all(&repo_path).unwrap();

    // Initialize git repo
    StdCommand::new("git")
        .current_dir(&repo_path)
        .args(["init"])
        .output()
        .expect("Failed to init git repo");

    // Configure git user
    StdCommand::new("git")
        .current_dir(&repo_path)
        .args(["config", "user.email", "test@test.com"])
        .output()
        .expect("Failed to configure git email");

    StdCommand::new("git")
        .current_dir(&repo_path)
        .args(["config", "user.name", "Test User"])
        .output()
        .expect("Failed to configure git name");

    // Create a file and commit
    fs::write(repo_path.join("README.md"), "# Test Repo\n").unwrap();

    StdCommand::new("git")
        .current_dir(&repo_path)
        .args(["add", "."])
        .output()
        .expect("Failed to git add");

    StdCommand::new("git")
        .current_dir(&repo_path)
        .args(["commit", "-m", "Initial commit"])
        .output()
        .expect("Failed to git commit");

    // Get the commit hash
    let output = StdCommand::new("git")
        .current_dir(&repo_path)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("Failed to get commit hash");

    String::from_utf8(output.stdout)
        .unwrap()
        .trim()
        .to_string()
}

/// Create a run record in the storage
fn create_run_record(temp: &TempDir, run_id: &str, base_commit: &str, cwd: &str) {
    let runs_dir = temp.path().join("runs");
    fs::create_dir_all(&runs_dir).unwrap();

    let run = format!(
        r#"{{
    "run_version": 0,
    "run_id": "{}",
    "exec": {{
        "argv": ["echo", "hello from replay"],
        "cwd": "{}",
        "env": {{}},
        "timeout_sec": 0
    }},
    "code_state": {{
        "repo_url": "file:///test/repo",
        "base_commit": "{}"
    }},
    "status": "exited",
    "runtime": "background",
    "timeline": {{
        "created_at": "2024-01-01T00:00:00Z",
        "started_at": "2024-01-01T00:00:01Z",
        "ended_at": "2024-01-01T00:00:02Z"
    }},
    "exit_code": 0
}}"#,
        run_id, cwd, base_commit
    );

    fs::write(runs_dir.join(format!("{}.json", run_id)), run).unwrap();
}

/// Create required storage directories
fn setup_storage_dirs(temp: &TempDir) {
    fs::create_dir_all(temp.path().join("runs")).unwrap();
    fs::create_dir_all(temp.path().join("templates")).unwrap();
    fs::create_dir_all(temp.path().join("playlists")).unwrap();
    fs::create_dir_all(temp.path().join("logs")).unwrap();
}

// === Happy Path Tests ===

#[test]
fn test_replay_creates_worktree_and_executes() {
    let temp = TempDir::new().unwrap();
    let repo_path = temp.path().join("repo");

    // Set up git repo and get commit hash
    let commit_hash = setup_git_repo(&temp);
    setup_storage_dirs(&temp);

    // Create run record
    let run_id = "run_a1b2c3d4-e5f6-7890-abcd-ef1234567890";
    create_run_record(&temp, run_id, &commit_hash, ".");

    // Create worktree directory
    let worktree_dir = temp.path().join("worktrees");
    fs::create_dir_all(&worktree_dir).unwrap();

    // Run replay command
    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .current_dir(&repo_path)
        .args([
            "replay",
            "a1b2c3d4",
            "--worktree-dir",
            worktree_dir.to_str().unwrap(),
            "--keep",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Replaying:"))
        .stdout(predicate::str::contains("hello from replay"))
        .stdout(predicate::str::contains("Replay completed successfully"));

    // Verify worktree was created
    let worktree_path = worktree_dir.join(run_id);
    assert!(worktree_path.exists(), "Worktree should exist at {:?}", worktree_path);
    assert!(worktree_path.join("README.md").exists(), "README.md should exist in worktree");
}

#[test]
fn test_replay_with_keep_flag_preserves_worktree() {
    let temp = TempDir::new().unwrap();
    let repo_path = temp.path().join("repo");

    // Set up git repo
    let commit_hash = setup_git_repo(&temp);
    setup_storage_dirs(&temp);

    // Create run record
    let run_id = "run_deadbeef-1234-5678-abcd-ef1234567890";
    create_run_record(&temp, run_id, &commit_hash, ".");

    // Create worktree directory
    let worktree_dir = temp.path().join("worktrees");
    fs::create_dir_all(&worktree_dir).unwrap();

    // Run replay with --keep flag
    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .current_dir(&repo_path)
        .args([
            "replay",
            "deadbeef",
            "--worktree-dir",
            worktree_dir.to_str().unwrap(),
            "--keep",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Worktree kept at:"));

    // Verify worktree still exists after command finishes
    let worktree_path = worktree_dir.join(run_id);
    assert!(worktree_path.exists(), "Worktree should be preserved with --keep");
}

#[test]
fn test_replay_with_cleanup_removes_worktree() {
    let temp = TempDir::new().unwrap();
    let repo_path = temp.path().join("repo");

    // Set up git repo
    let commit_hash = setup_git_repo(&temp);
    setup_storage_dirs(&temp);

    // Create run record
    let run_id = "run_cafebabe-1234-5678-abcd-ef1234567890";
    create_run_record(&temp, run_id, &commit_hash, ".");

    // Create worktree directory
    let worktree_dir = temp.path().join("worktrees");
    fs::create_dir_all(&worktree_dir).unwrap();

    // Run replay with --cleanup flag
    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .current_dir(&repo_path)
        .args([
            "replay",
            "cafebabe",
            "--worktree-dir",
            worktree_dir.to_str().unwrap(),
            "--cleanup",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Cleaning up worktree"))
        .stdout(predicate::str::contains("Worktree removed"));

    // Verify worktree was removed
    let worktree_path = worktree_dir.join(run_id);
    assert!(!worktree_path.exists(), "Worktree should be removed with --cleanup");
}

#[test]
fn test_replay_with_fresh_creates_new_worktree() {
    let temp = TempDir::new().unwrap();
    let repo_path = temp.path().join("repo");

    // Set up git repo
    let commit_hash = setup_git_repo(&temp);
    setup_storage_dirs(&temp);

    // Create run record
    let run_id = "run_12345678-abcd-ef12-3456-789012345678";
    create_run_record(&temp, run_id, &commit_hash, ".");

    // Create worktree directory
    let worktree_dir = temp.path().join("worktrees");
    fs::create_dir_all(&worktree_dir).unwrap();

    // Run replay with --fresh flag
    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .current_dir(&repo_path)
        .args([
            "replay",
            "12345678",
            "--worktree-dir",
            worktree_dir.to_str().unwrap(),
            "--fresh",
            "--keep",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Created worktree:"));

    // Verify worktree was created
    let worktree_path = worktree_dir.join(run_id);
    assert!(worktree_path.exists(), "Worktree should be created with --fresh");
}

// === Error Path Tests ===

#[test]
fn test_replay_run_not_found() {
    let temp = TempDir::new().unwrap();
    let repo_path = temp.path().join("repo");

    // Set up git repo but no run record
    setup_git_repo(&temp);
    setup_storage_dirs(&temp);

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .current_dir(&repo_path)
        .args(["replay", "nonexistent"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("No item found"));
}

#[test]
fn test_replay_not_in_git_repo() {
    let temp = TempDir::new().unwrap();
    setup_storage_dirs(&temp);

    // Create a non-git directory
    let non_git_dir = temp.path().join("not_a_repo");
    fs::create_dir_all(&non_git_dir).unwrap();

    // Create a run record (even though we won't be able to use it)
    let run_id = "run_abcd1234-0000-0000-0000-000000000000";
    create_run_record(&temp, run_id, "a1b2c3d4e5f6789012345678901234567890abcd", ".");

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .current_dir(&non_git_dir)
        .args(["replay", "abcd1234"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Not a git repository"));
}

#[test]
fn test_replay_invalid_commit() {
    let temp = TempDir::new().unwrap();
    let repo_path = temp.path().join("repo");

    // Set up git repo with a different commit
    setup_git_repo(&temp);
    setup_storage_dirs(&temp);

    // Create run record with a commit that doesn't exist
    let run_id = "run_badcomit-0000-0000-0000-000000000000";
    let fake_commit = "0000000000000000000000000000000000000000";
    create_run_record(&temp, run_id, fake_commit, ".");

    // Create worktree directory
    let worktree_dir = temp.path().join("worktrees");
    fs::create_dir_all(&worktree_dir).unwrap();

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .current_dir(&repo_path)
        .args([
            "replay",
            "badcomit",
            "--worktree-dir",
            worktree_dir.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Failed to create worktree"));
}

// === Code State Restoration Tests ===

#[test]
fn test_replay_restores_correct_commit() {
    let temp = TempDir::new().unwrap();
    let repo_path = temp.path().join("repo");
    fs::create_dir_all(&repo_path).unwrap();

    // Initialize git repo
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

    // Create first commit
    fs::write(repo_path.join("file.txt"), "version 1").unwrap();
    StdCommand::new("git")
        .current_dir(&repo_path)
        .args(["add", "."])
        .output()
        .unwrap();
    StdCommand::new("git")
        .current_dir(&repo_path)
        .args(["commit", "-m", "First commit"])
        .output()
        .unwrap();

    // Get first commit hash
    let output = StdCommand::new("git")
        .current_dir(&repo_path)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    let first_commit = String::from_utf8(output.stdout).unwrap().trim().to_string();

    // Create second commit with different content
    fs::write(repo_path.join("file.txt"), "version 2").unwrap();
    StdCommand::new("git")
        .current_dir(&repo_path)
        .args(["add", "."])
        .output()
        .unwrap();
    StdCommand::new("git")
        .current_dir(&repo_path)
        .args(["commit", "-m", "Second commit"])
        .output()
        .unwrap();

    setup_storage_dirs(&temp);

    // Create run record pointing to FIRST commit
    // The command will output the content of file.txt
    let run_id = "run_commitck-0000-0000-0000-000000000000";
    let runs_dir = temp.path().join("runs");
    let run = format!(
        r#"{{
    "run_version": 0,
    "run_id": "{}",
    "exec": {{
        "argv": ["cat", "file.txt"],
        "cwd": ".",
        "env": {{}},
        "timeout_sec": 0
    }},
    "code_state": {{
        "repo_url": "file:///test/repo",
        "base_commit": "{}"
    }},
    "status": "exited",
    "runtime": "background",
    "timeline": {{}},
    "exit_code": 0
}}"#,
        run_id, first_commit
    );
    fs::write(runs_dir.join(format!("{}.json", run_id)), run).unwrap();

    // Create worktree directory
    let worktree_dir = temp.path().join("worktrees");
    fs::create_dir_all(&worktree_dir).unwrap();

    // Run replay - should checkout first commit and show "version 1"
    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .current_dir(&repo_path)
        .args([
            "replay",
            "commitck",
            "--worktree-dir",
            worktree_dir.to_str().unwrap(),
            "--keep",
        ])
        .assert()
        .success()
        // Should see "version 1" (from first commit), not "version 2" (current HEAD)
        .stdout(predicate::str::contains("version 1"));
}

#[test]
fn test_replay_uses_short_id() {
    let temp = TempDir::new().unwrap();
    let repo_path = temp.path().join("repo");

    // Set up git repo
    let commit_hash = setup_git_repo(&temp);
    setup_storage_dirs(&temp);

    // Create run record with full UUID
    let run_id = "run_87654321-fedc-ba98-7654-321fedcba987";
    create_run_record(&temp, run_id, &commit_hash, ".");

    // Create worktree directory
    let worktree_dir = temp.path().join("worktrees");
    fs::create_dir_all(&worktree_dir).unwrap();

    // Use short ID (first 4 characters)
    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .current_dir(&repo_path)
        .args([
            "replay",
            "8765",
            "--worktree-dir",
            worktree_dir.to_str().unwrap(),
            "--keep",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(run_id));
}

#[test]
fn test_replay_verbose_output() {
    let temp = TempDir::new().unwrap();
    let repo_path = temp.path().join("repo");

    // Set up git repo
    let commit_hash = setup_git_repo(&temp);
    setup_storage_dirs(&temp);

    // Create run record
    let run_id = "run_verbose0-1234-5678-abcd-ef1234567890";
    create_run_record(&temp, run_id, &commit_hash, ".");

    // Create worktree directory
    let worktree_dir = temp.path().join("worktrees");
    fs::create_dir_all(&worktree_dir).unwrap();

    // Run with verbose flag
    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .current_dir(&repo_path)
        .args([
            "replay",
            "verbose0",
            "--worktree-dir",
            worktree_dir.to_str().unwrap(),
            "--keep",
            "-v",
        ])
        .assert()
        .success()
        // Verbose output goes to stderr
        .stderr(predicate::str::contains("[config]"));
}
