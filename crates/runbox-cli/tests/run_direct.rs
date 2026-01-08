use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::process::Command as StdCommand;
use std::thread;
use std::time::Duration;
use tempfile::TempDir;

fn runbox_cmd(temp_dir: &TempDir) -> Command {
    let mut cmd = Command::cargo_bin("runbox").unwrap();
    cmd.env("RUNBOX_HOME", temp_dir.path());
    cmd
}

fn init_git_repo(path: &std::path::Path) -> std::io::Result<()> {
    StdCommand::new("git")
        .current_dir(path)
        .args(["init"])
        .output()?;

    StdCommand::new("git")
        .current_dir(path)
        .args(["config", "user.email", "test@example.com"])
        .output()?;

    StdCommand::new("git")
        .current_dir(path)
        .args(["config", "user.name", "Test User"])
        .output()?;

    fs::write(path.join("README.md"), "# Test")?;

    StdCommand::new("git")
        .current_dir(path)
        .args(["add", "."])
        .output()?;

    StdCommand::new("git")
        .current_dir(path)
        .args(["commit", "-m", "Initial commit"])
        .output()?;

    StdCommand::new("git")
        .current_dir(path)
        .args(["remote", "add", "origin", "git@github.com:test/repo.git"])
        .output()?;

    Ok(())
}

fn wait_for_run_completion(temp: &TempDir, short_id: &str, max_wait_ms: u64) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed().as_millis() < max_wait_ms as u128 {
        let output = runbox_cmd(temp)
            .args(["ps"])
            .output()
            .expect("Failed to run ps");

        let stdout = String::from_utf8_lossy(&output.stdout);

        for line in stdout.lines() {
            if line.contains(short_id) {
                if line.contains("exited") || line.contains("failed") || line.contains("unknown") {
                    return true;
                }
            }
        }
        thread::sleep(Duration::from_millis(100));
    }
    false
}

fn get_run_id_from_output(output: &str) -> Option<String> {
    for line in output.lines() {
        if line.contains("Short ID:") {
            return line.split(':').nth(1).map(|s| s.trim().to_string());
        }
    }
    None
}

// =============================================================================
// Actual Execution Tests
// =============================================================================

#[test]
fn test_direct_run_executes_and_appears_in_ps() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path()).unwrap();

    let output = runbox_cmd(&temp)
        .current_dir(temp.path())
        .args(["run", "--", "echo", "hello"])
        .output()
        .expect("Failed to execute command");

    assert!(output.status.success(), "run command failed: {:?}", output);

    let stdout = String::from_utf8_lossy(&output.stdout);
    let short_id = get_run_id_from_output(&stdout).expect("Could not find short ID in output");

    assert!(
        wait_for_run_completion(&temp, &short_id, 5000),
        "Run did not complete within timeout"
    );

    runbox_cmd(&temp)
        .args(["ps"])
        .assert()
        .success()
        .stdout(predicate::str::contains(&short_id))
        .stdout(predicate::str::contains("echo hello"));
}

#[test]
fn test_direct_run_completes_and_shows_status() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path()).unwrap();

    let output = runbox_cmd(&temp)
        .current_dir(temp.path())
        .args(["run", "--", "true"])
        .output()
        .expect("Failed to execute command");

    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    let short_id = get_run_id_from_output(&stdout).expect("Could not find short ID");

    assert!(wait_for_run_completion(&temp, &short_id, 5000));

    runbox_cmd(&temp)
        .args(["show", &short_id])
        .assert()
        .success()
        .stdout(predicate::str::contains("Run ID:"))
        .stdout(predicate::str::contains("Status:"));
}

#[test]
fn test_direct_run_failing_command_completes() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path()).unwrap();

    let output = runbox_cmd(&temp)
        .current_dir(temp.path())
        .args(["run", "--", "false"])
        .output()
        .expect("Failed to execute command");

    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    let short_id = get_run_id_from_output(&stdout).expect("Could not find short ID");

    assert!(wait_for_run_completion(&temp, &short_id, 5000));

    runbox_cmd(&temp)
        .args(["ps"])
        .assert()
        .success()
        .stdout(predicate::str::contains(&short_id));
}

#[test]
fn test_direct_run_logs_capture_stdout() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path()).unwrap();

    let output = runbox_cmd(&temp)
        .current_dir(temp.path())
        .args(["run", "--", "echo", "captured_output_12345"])
        .output()
        .expect("Failed to execute command");

    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    let short_id = get_run_id_from_output(&stdout).expect("Could not find short ID");

    assert!(wait_for_run_completion(&temp, &short_id, 5000));

    thread::sleep(Duration::from_millis(200));

    runbox_cmd(&temp)
        .args(["logs", &short_id])
        .assert()
        .success()
        .stdout(predicate::str::contains("captured_output_12345"));
}

#[test]
fn test_direct_run_show_displays_details() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path()).unwrap();

    let output = runbox_cmd(&temp)
        .current_dir(temp.path())
        .args(["run", "--", "echo", "show_test"])
        .output()
        .expect("Failed to execute command");

    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    let short_id = get_run_id_from_output(&stdout).expect("Could not find short ID");

    assert!(wait_for_run_completion(&temp, &short_id, 5000));

    runbox_cmd(&temp)
        .args(["show", &short_id])
        .assert()
        .success()
        .stdout(predicate::str::contains("Run ID:"))
        .stdout(predicate::str::contains("Command:"))
        .stdout(predicate::str::contains("echo"))
        .stdout(predicate::str::contains("show_test"))
        .stdout(predicate::str::contains("Repo:"))
        .stdout(predicate::str::contains("Commit:"));
}

#[test]
fn test_direct_run_with_env_vars_passes_to_command() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path()).unwrap();

    let output = runbox_cmd(&temp)
        .current_dir(temp.path())
        .args([
            "run",
            "--env",
            "MY_TEST_VAR=test_value_xyz",
            "--",
            "sh",
            "-c",
            "echo $MY_TEST_VAR",
        ])
        .output()
        .expect("Failed to execute command");

    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    let short_id = get_run_id_from_output(&stdout).expect("Could not find short ID");

    assert!(wait_for_run_completion(&temp, &short_id, 5000));

    thread::sleep(Duration::from_millis(200));

    runbox_cmd(&temp)
        .args(["logs", &short_id])
        .assert()
        .success()
        .stdout(predicate::str::contains("test_value_xyz"));
}

#[test]
fn test_log_command_executes_like_run() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path()).unwrap();

    let output = runbox_cmd(&temp)
        .current_dir(temp.path())
        .args(["log", "--", "echo", "log_command_test"])
        .output()
        .expect("Failed to execute command");

    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    let short_id = get_run_id_from_output(&stdout).expect("Could not find short ID");

    assert!(wait_for_run_completion(&temp, &short_id, 5000));

    runbox_cmd(&temp)
        .args(["ps"])
        .assert()
        .success()
        .stdout(predicate::str::contains(&short_id))
        .stdout(predicate::str::contains("echo log_command"));
}

#[test]
fn test_direct_run_with_no_git_executes() {
    let temp = TempDir::new().unwrap();
    // NOT initializing git repo

    let output = runbox_cmd(&temp)
        .current_dir(temp.path())
        .args(["run", "--no-git", "--", "echo", "no_git_test"])
        .output()
        .expect("Failed to execute command");

    assert!(
        output.status.success(),
        "Command should succeed with --no-git"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let short_id = get_run_id_from_output(&stdout).expect("Could not find short ID");

    assert!(wait_for_run_completion(&temp, &short_id, 5000));

    runbox_cmd(&temp)
        .args(["ps"])
        .assert()
        .success()
        .stdout(predicate::str::contains(&short_id));
}

#[test]
fn test_direct_run_git_context_captured() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path()).unwrap();

    let output = runbox_cmd(&temp)
        .current_dir(temp.path())
        .args(["run", "--", "echo", "git_context_test"])
        .output()
        .expect("Failed to execute command");

    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    let short_id = get_run_id_from_output(&stdout).expect("Could not find short ID");

    assert!(wait_for_run_completion(&temp, &short_id, 5000));

    runbox_cmd(&temp)
        .args(["show", &short_id])
        .assert()
        .success()
        .stdout(predicate::str::contains("Repo:"))
        .stdout(predicate::str::contains("git@github.com:test/repo.git"))
        .stdout(predicate::str::contains("Commit:"));
}

#[test]
fn test_direct_run_multiple_runs_listed() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path()).unwrap();

    let mut short_ids = Vec::new();

    for i in 1..=3 {
        let output = runbox_cmd(&temp)
            .current_dir(temp.path())
            .args(["run", "--", "echo", &format!("run_{}", i)])
            .output()
            .expect("Failed to execute command");

        assert!(output.status.success());

        let stdout = String::from_utf8_lossy(&output.stdout);
        let short_id = get_run_id_from_output(&stdout).expect("Could not find short ID");
        short_ids.push(short_id);

        thread::sleep(Duration::from_millis(50));
    }

    for short_id in &short_ids {
        assert!(
            wait_for_run_completion(&temp, short_id, 5000),
            "Run {} did not complete",
            short_id
        );
    }

    let ps_output = runbox_cmd(&temp)
        .args(["ps"])
        .output()
        .expect("Failed to run ps");

    let stdout = String::from_utf8_lossy(&ps_output.stdout);

    for short_id in &short_ids {
        assert!(
            stdout.contains(short_id),
            "ps output should contain run {}",
            short_id
        );
    }
}

#[test]
fn test_direct_run_history_shows_runs() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path()).unwrap();

    let output = runbox_cmd(&temp)
        .current_dir(temp.path())
        .args(["run", "--", "echo", "history_test"])
        .output()
        .expect("Failed to execute command");

    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    let short_id = get_run_id_from_output(&stdout).expect("Could not find short ID");

    assert!(wait_for_run_completion(&temp, &short_id, 5000));

    runbox_cmd(&temp)
        .args(["history"])
        .assert()
        .success()
        .stdout(predicate::str::contains(&short_id))
        .stdout(predicate::str::contains("echo history_test"));
}
