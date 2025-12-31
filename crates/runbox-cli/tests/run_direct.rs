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

/// Create a minimal git repository with origin remote
fn init_git_repo(path: &std::path::Path) -> std::io::Result<()> {
    // Initialize git repo
    StdCommand::new("git")
        .current_dir(path)
        .args(["init"])
        .output()?;

    // Configure git user for commits
    StdCommand::new("git")
        .current_dir(path)
        .args(["config", "user.email", "test@example.com"])
        .output()?;

    StdCommand::new("git")
        .current_dir(path)
        .args(["config", "user.name", "Test User"])
        .output()?;

    // Create a file and commit
    fs::write(path.join("README.md"), "# Test")?;

    StdCommand::new("git")
        .current_dir(path)
        .args(["add", "."])
        .output()?;

    StdCommand::new("git")
        .current_dir(path)
        .args(["commit", "-m", "Initial commit"])
        .output()?;

    // Add origin remote (doesn't need to exist for our tests)
    StdCommand::new("git")
        .current_dir(path)
        .args(["remote", "add", "origin", "git@github.com:test/repo.git"])
        .output()?;

    Ok(())
}

// =============================================================================
// Direct Command Execution - Happy Path Tests
// =============================================================================

#[test]
fn test_run_direct_simple_command_dry_run() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path()).unwrap();

    runbox_cmd(&temp)
        .current_dir(temp.path())
        .args(["run", "--dry-run", "--", "echo", "hello"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Dry run"))
        .stdout(predicate::str::contains("echo"))
        .stdout(predicate::str::contains("hello"))
        .stdout(predicate::str::contains("\"source\": \"direct\""));
}

#[test]
fn test_run_direct_with_timeout_dry_run() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path()).unwrap();

    runbox_cmd(&temp)
        .current_dir(temp.path())
        .args(["run", "--timeout", "300", "--dry-run", "--", "sleep", "10"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Dry run"))
        .stdout(predicate::str::contains("\"timeout_sec\": 300"));
}

#[test]
fn test_run_direct_with_env_vars_dry_run() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path()).unwrap();

    runbox_cmd(&temp)
        .current_dir(temp.path())
        .args([
            "run",
            "--env",
            "MY_VAR=my_value",
            "--env",
            "ANOTHER=another_value",
            "--dry-run",
            "--",
            "echo",
            "test",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Dry run"))
        .stdout(predicate::str::contains("MY_VAR"))
        .stdout(predicate::str::contains("my_value"))
        .stdout(predicate::str::contains("ANOTHER"))
        .stdout(predicate::str::contains("another_value"));
}

#[test]
fn test_run_direct_with_cwd_dry_run() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path()).unwrap();

    // Create a subdirectory
    let subdir = temp.path().join("subdir");
    fs::create_dir(&subdir).unwrap();

    runbox_cmd(&temp)
        .current_dir(temp.path())
        .args([
            "run",
            "--cwd",
            subdir.to_str().unwrap(),
            "--dry-run",
            "--",
            "pwd",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Dry run"))
        .stdout(predicate::str::contains("subdir"));
}

#[test]
fn test_run_direct_with_no_git_dry_run() {
    let temp = TempDir::new().unwrap();
    // Note: NOT initializing git repo

    runbox_cmd(&temp)
        .current_dir(temp.path())
        .args(["run", "--no-git", "--dry-run", "--", "echo", "hello"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Dry run"))
        .stdout(predicate::str::contains("echo"))
        // Should have placeholder code_state when --no-git is used
        .stdout(predicate::str::contains("\"repo_url\": \"none\""));
}

#[test]
fn test_run_direct_with_bg_runtime_dry_run() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path()).unwrap();

    runbox_cmd(&temp)
        .current_dir(temp.path())
        .args(["run", "--runtime", "bg", "--dry-run", "--", "echo", "test"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Dry run"));
}

#[test]
fn test_run_direct_with_tmux_runtime_dry_run() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path()).unwrap();

    runbox_cmd(&temp)
        .current_dir(temp.path())
        .args(["run", "--runtime", "tmux", "--dry-run", "--", "echo", "test"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Dry run"));
}

#[test]
fn test_run_direct_complex_command_dry_run() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path()).unwrap();

    runbox_cmd(&temp)
        .current_dir(temp.path())
        .args([
            "run",
            "--dry-run",
            "--",
            "python",
            "train.py",
            "--epochs",
            "10",
            "--lr",
            "0.001",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Dry run"))
        .stdout(predicate::str::contains("python"))
        .stdout(predicate::str::contains("train.py"))
        .stdout(predicate::str::contains("--epochs"))
        .stdout(predicate::str::contains("10"));
}

// =============================================================================
// Log Command (Alias for Direct Execution) - Happy Path Tests
// =============================================================================

#[test]
fn test_log_simple_command_dry_run() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path()).unwrap();

    runbox_cmd(&temp)
        .current_dir(temp.path())
        .args(["log", "--dry-run", "--", "echo", "hello"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Dry run"))
        .stdout(predicate::str::contains("echo"))
        .stdout(predicate::str::contains("hello"))
        .stdout(predicate::str::contains("\"source\": \"direct\""));
}

#[test]
fn test_log_with_all_options_dry_run() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path()).unwrap();

    runbox_cmd(&temp)
        .current_dir(temp.path())
        .args([
            "log",
            "--runtime",
            "bg",
            "--timeout",
            "60",
            "--env",
            "TEST=1",
            "--dry-run",
            "--",
            "make",
            "test",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Dry run"))
        .stdout(predicate::str::contains("make"))
        .stdout(predicate::str::contains("test"))
        .stdout(predicate::str::contains("TEST"));
}

#[test]
fn test_log_with_no_git_dry_run() {
    let temp = TempDir::new().unwrap();
    // Note: NOT initializing git repo

    runbox_cmd(&temp)
        .current_dir(temp.path())
        .args(["log", "--no-git", "--dry-run", "--", "echo", "no-git-test"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Dry run"))
        .stdout(predicate::str::contains("no-git-test"));
}

// =============================================================================
// Error Path Tests
// =============================================================================

#[test]
fn test_run_no_template_no_command_error() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path()).unwrap();

    runbox_cmd(&temp)
        .current_dir(temp.path())
        .args(["run"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--template").or(predicate::str::contains("command")));
}

#[test]
fn test_run_direct_not_in_git_repo_without_no_git_flag() {
    let temp = TempDir::new().unwrap();
    // Note: NOT initializing git repo

    runbox_cmd(&temp)
        .current_dir(temp.path())
        .args(["run", "--dry-run", "--", "echo", "hello"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("git").or(predicate::str::contains("repository")));
}

#[test]
fn test_run_direct_invalid_env_var_format() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path()).unwrap();

    runbox_cmd(&temp)
        .current_dir(temp.path())
        .args(["run", "--env", "INVALID_FORMAT", "--dry-run", "--", "echo", "test"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("KEY=VALUE").or(predicate::str::contains("Invalid")));
}

#[test]
fn test_run_direct_invalid_runtime() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path()).unwrap();

    runbox_cmd(&temp)
        .current_dir(temp.path())
        .args(["run", "--runtime", "invalid", "--dry-run", "--", "echo", "test"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid").or(predicate::str::contains("possible values")));
}

#[test]
fn test_log_no_command_error() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path()).unwrap();

    runbox_cmd(&temp)
        .current_dir(temp.path())
        .args(["log", "--"])
        .assert()
        .failure();
}

// =============================================================================
// Edge Cases
// =============================================================================

#[test]
fn test_run_direct_with_special_characters_in_command_dry_run() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path()).unwrap();

    runbox_cmd(&temp)
        .current_dir(temp.path())
        .args(["run", "--dry-run", "--", "echo", "hello world", "foo=bar"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Dry run"))
        .stdout(predicate::str::contains("hello world"))
        .stdout(predicate::str::contains("foo=bar"));
}

#[test]
fn test_run_direct_combined_with_template_flag() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path()).unwrap();

    // When both --template and -- command are provided, template takes precedence
    // This should fail because template doesn't exist
    runbox_cmd(&temp)
        .current_dir(temp.path())
        .args(["run", "--template", "tpl_nonexistent", "--", "echo", "test"])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("not found")
                .or(predicate::str::contains("No template"))
                .or(predicate::str::contains("No item found")),
        );
}

#[test]
fn test_run_direct_env_var_with_special_chars_dry_run() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path()).unwrap();

    runbox_cmd(&temp)
        .current_dir(temp.path())
        .args([
            "run",
            "--env",
            "PATH_VAR=/usr/bin:/usr/local/bin",
            "--env",
            "QUOTED=hello world",
            "--dry-run",
            "--",
            "echo",
            "test",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("/usr/bin:/usr/local/bin"))
        .stdout(predicate::str::contains("hello world"));
}

#[test]
fn test_run_direct_single_word_command_dry_run() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path()).unwrap();

    runbox_cmd(&temp)
        .current_dir(temp.path())
        .args(["run", "--dry-run", "--", "ls"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Dry run"))
        .stdout(predicate::str::contains("ls"));
}
