use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

/// Helper to create a runbox command with RUNBOX_HOME set to temp directory
fn runbox_cmd(temp_dir: &TempDir) -> Command {
    let mut cmd = Command::cargo_bin("runbox").unwrap();
    cmd.env("RUNBOX_HOME", temp_dir.path());
    cmd
}

#[test]
fn test_skill_list() {
    let temp = TempDir::new().unwrap();

    runbox_cmd(&temp)
        .args(["skill", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("runbox-cli"))
        .stdout(predicate::str::contains("skill(s) available"));
}

#[test]
fn test_skill_export_success() {
    let temp = TempDir::new().unwrap();
    let output_dir = temp.path().join("exported");

    runbox_cmd(&temp)
        .args(["skill", "export", "runbox-cli", "--output", output_dir.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("SKILL.md"))
        .stdout(predicate::str::contains("INSTALL.md"))
        .stdout(predicate::str::contains("install.sh"));

    // Verify files were created
    assert!(output_dir.join("SKILL.md").exists(), "SKILL.md should exist");
    assert!(output_dir.join("INSTALL.md").exists(), "INSTALL.md should exist");
    assert!(output_dir.join("install.sh").exists(), "install.sh should exist");
    assert!(output_dir.join("install").join("claude-code.md").exists(), "claude-code.md should exist");
    assert!(output_dir.join("install").join("opencode.md").exists(), "opencode.md should exist");
    assert!(output_dir.join("install").join("gemini.md").exists(), "gemini.md should exist");
    assert!(output_dir.join("install").join("codex.md").exists(), "codex.md should exist");
    assert!(output_dir.join("install").join("cursor.md").exists(), "cursor.md should exist");
}

#[test]
fn test_skill_export_default_output() {
    let temp = TempDir::new().unwrap();

    // Run from temp directory
    runbox_cmd(&temp)
        .current_dir(temp.path())
        .args(["skill", "export", "runbox-cli"])
        .assert()
        .success();

    // Should create directory with skill name
    assert!(temp.path().join("runbox-cli").join("SKILL.md").exists());
    assert!(temp.path().join("runbox-cli").join("INSTALL.md").exists());
}

#[test]
fn test_skill_export_not_found() {
    let temp = TempDir::new().unwrap();

    runbox_cmd(&temp)
        .args(["skill", "export", "nonexistent-skill"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));
}

#[test]
fn test_skill_export_install_script_executable() {
    let temp = TempDir::new().unwrap();
    let output_dir = temp.path().join("exported");

    runbox_cmd(&temp)
        .args(["skill", "export", "runbox-cli", "--output", output_dir.to_str().unwrap()])
        .assert()
        .success();

    let script_path = output_dir.join("install.sh");
    
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let metadata = std::fs::metadata(&script_path).unwrap();
        let mode = metadata.permissions().mode();
        // Check executable bit is set
        assert!(mode & 0o111 != 0, "install.sh should be executable");
    }
}

#[test]
fn test_skill_export_content_includes_skill() {
    let temp = TempDir::new().unwrap();
    let output_dir = temp.path().join("exported");

    runbox_cmd(&temp)
        .args(["skill", "export", "runbox-cli", "--output", output_dir.to_str().unwrap()])
        .assert()
        .success();

    // Read SKILL.md and verify it has content
    let skill_content = std::fs::read_to_string(output_dir.join("SKILL.md")).unwrap();
    assert!(skill_content.contains("runbox"), "SKILL.md should contain runbox documentation");
    
    // Read INSTALL.md and verify platform instructions
    let install_content = std::fs::read_to_string(output_dir.join("INSTALL.md")).unwrap();
    assert!(install_content.contains("Claude Code"), "INSTALL.md should mention Claude Code");
    assert!(install_content.contains("OpenCode"), "INSTALL.md should mention OpenCode");
    assert!(install_content.contains("Cursor"), "INSTALL.md should mention Cursor");
}
