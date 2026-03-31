use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

#[test]
fn test_attach_not_found() {
    let temp = TempDir::new().unwrap();
    let runs_dir = temp.path().join("runs");
    fs::create_dir_all(&runs_dir).unwrap();

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["attach", "nonexistent"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found").or(predicate::str::contains("No run found")));
}

#[test]
fn test_attach_background_not_supported() {
    let temp = TempDir::new().unwrap();

    // Create runs directory
    let runs_dir = temp.path().join("runs");
    fs::create_dir_all(&runs_dir).unwrap();

    // Create a run with background runtime
    let run = r#"{
        "run_version": 0,
        "run_id": "run_a1b2c3d4-e5f6-7890-abcd-ef1234567890",
        "exec": {
            "argv": ["echo", "hello"],
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
            "pid": 12345,
            "pgid": 12345
        }
    }"#;

    fs::write(
        runs_dir.join("run_a1b2c3d4-e5f6-7890-abcd-ef1234567890.json"),
        run,
    )
    .unwrap();

    // Try to attach to background run - should fail
    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["attach", "a1b2c3d4"])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("not support")
                .or(predicate::str::contains("only supported for tmux/zellij")),
        );
}

#[test]
fn test_attach_no_runtime_not_supported() {
    let temp = TempDir::new().unwrap();

    // Create runs directory
    let runs_dir = temp.path().join("runs");
    fs::create_dir_all(&runs_dir).unwrap();

    // Create a run with empty runtime (legacy/default state)
    let run = r#"{
        "run_version": 0,
        "run_id": "run_b2c3d4e5-f678-9012-bcde-f12345678901",
        "exec": {
            "argv": ["echo", "hello"],
            "cwd": ".",
            "env": {},
            "timeout_sec": 0
        },
        "code_state": {
            "repo_url": "git@github.com:org/repo.git",
            "base_commit": "a1b2c3d4e5f6789012345678901234567890abcd"
        },
        "status": "pending",
        "runtime": ""
    }"#;

    fs::write(
        runs_dir.join("run_b2c3d4e5-f678-9012-bcde-f12345678901.json"),
        run,
    )
    .unwrap();

    // Try to attach to run with no runtime - should fail
    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["attach", "b2c3d4e5"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("only supported for tmux/zellij"));
}

/// Feature-gated test for tmux attach functionality.
/// This test requires tmux to be installed and available.
/// Run with: cargo test --features tmux-tests
#[cfg(feature = "tmux-tests")]
#[test]
fn test_attach_tmux() {
    use std::process::Command as StdCommand;

    // Check if tmux is available
    let tmux_check = StdCommand::new("tmux").arg("-V").output();
    if tmux_check.is_err() || !tmux_check.unwrap().status.success() {
        eprintln!("Skipping tmux test: tmux not available");
        return;
    }

    let temp = TempDir::new().unwrap();

    // Create runs directory
    let runs_dir = temp.path().join("runs");
    fs::create_dir_all(&runs_dir).unwrap();

    // Create a unique session name for this test
    let session_name = format!("runbox_test_{}", std::process::id());

    // Start a tmux session in detached mode
    let session_result = StdCommand::new("tmux")
        .args([
            "new-session",
            "-d",
            "-s",
            &session_name,
            "-n",
            "test_window",
        ])
        .status();

    if session_result.is_err() || !session_result.unwrap().success() {
        eprintln!("Failed to create tmux session");
        return;
    }

    // Create a run with tmux runtime
    let run = format!(
        r#"{{
        "run_version": 0,
        "run_id": "run_c3d4e5f6-7890-1234-cdef-123456789012",
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
        "status": "running",
        "runtime": "tmux",
        "handle": {{
            "type": "Tmux",
            "session": "{}",
            "window": "test_window"
        }}
    }}"#,
        session_name
    );

    fs::write(
        runs_dir.join("run_c3d4e5f6-7890-1234-cdef-123456789012.json"),
        run,
    )
    .unwrap();

    // The attach command will fail because stdout is not a TTY,
    // but we can verify it gets past validation and reaches tmux.
    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .env_remove("TMUX")
        .args(["attach", "c3d4e5f6"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("terminal"));

    // Clean up tmux session
    let _ = StdCommand::new("tmux")
        .args(["kill-session", "-t", &session_name])
        .status();
}

/// Feature-gated test for zellij attach functionality.
/// This test requires zellij to be installed and available.
/// Run with: cargo test --features zellij-tests
#[cfg(feature = "zellij-tests")]
#[test]
fn test_attach_zellij() {
    use std::process::Command as StdCommand;

    let zellij_check = StdCommand::new("zellij").arg("--version").output();
    if zellij_check.is_err() || !zellij_check.unwrap().status.success() {
        eprintln!("Skipping zellij test: zellij not available");
        return;
    }

    let temp = TempDir::new().unwrap();
    let runs_dir = temp.path().join("runs");
    fs::create_dir_all(&runs_dir).unwrap();

    let session_name = format!("runbox_test_{}", std::process::id());
    let tab_name = "test_tab";

    let session_result = StdCommand::new("zellij")
        .args(["attach", &session_name, "--create-background"])
        .status();

    if session_result.is_err() || !session_result.unwrap().success() {
        eprintln!("Failed to create zellij session");
        return;
    }

    let tab_result = StdCommand::new("zellij")
        .args([
            "--session",
            &session_name,
            "action",
            "new-tab",
            "--name",
            tab_name,
        ])
        .status();

    if tab_result.is_err() || !tab_result.unwrap().success() {
        eprintln!("Failed to create zellij tab");
        let _ = StdCommand::new("zellij")
            .args(["kill-session", &session_name])
            .status();
        return;
    }

    let run = format!(
        r#"{{
        "run_version": 0,
        "run_id": "run_d4e5f607-8901-2345-def0-123456789012",
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
        "status": "running",
        "runtime": "zellij",
        "handle": {{
            "type": "Zellij",
            "session": "{}",
            "tab": "{}"
        }}
    }}"#,
        session_name, tab_name
    );

    fs::write(
        runs_dir.join("run_d4e5f607-8901-2345-def0-123456789012.json"),
        run,
    )
    .unwrap();

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .env_remove("ZELLIJ")
        .args(["attach", "d4e5f607"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("terminal"));

    let _ = StdCommand::new("zellij")
        .args(["kill-session", &session_name])
        .status();
}
