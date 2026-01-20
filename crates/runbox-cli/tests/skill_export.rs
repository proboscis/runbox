use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn runbox_cmd(temp_dir: &TempDir) -> Command {
    let mut cmd = Command::cargo_bin("runbox").unwrap();
    cmd.env("RUNBOX_HOME", temp_dir.path());
    cmd
}

#[test]
fn test_skill_list_help() {
    let temp = TempDir::new().unwrap();
    runbox_cmd(&temp)
        .args(["skill", "list", "--help"])
        .assert()
        .success();
}

#[test]
fn test_skill_show_help() {
    let temp = TempDir::new().unwrap();
    runbox_cmd(&temp)
        .args(["skill", "show", "--help"])
        .assert()
        .success();
}

#[test]
fn test_skill_export_help() {
    let temp = TempDir::new().unwrap();
    runbox_cmd(&temp)
        .args(["skill", "export", "--help"])
        .assert()
        .success();
}

#[test]
fn test_skill_show_not_found() {
    let temp = TempDir::new().unwrap();
    runbox_cmd(&temp)
        .args(["skill", "show", "nonexistent-skill-12345"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Skill not found"));
}

#[test]
fn test_skill_export_not_found() {
    let temp = TempDir::new().unwrap();
    let output = temp.path().join("output");

    runbox_cmd(&temp)
        .args(["skill", "export", "nonexistent-skill-12345", "--output"])
        .arg(&output)
        .assert()
        .failure()
        .stderr(predicate::str::contains("Skill not found"));
}

#[test]
fn test_skill_list_runs() {
    let temp = TempDir::new().unwrap();
    runbox_cmd(&temp).args(["skill", "list"]).assert().success();
}
