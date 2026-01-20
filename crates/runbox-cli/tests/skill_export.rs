use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::tempdir;
use std::fs;

#[test]
fn test_skill_list_help() {
    let mut cmd = Command::cargo_bin("runbox").unwrap();
    cmd.arg("skill").arg("list").arg("--help");
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("List all available skills"));
}

#[test]
fn test_skill_export_help() {
    let mut cmd = Command::cargo_bin("runbox").unwrap();
    cmd.arg("skill").arg("export").arg("--help");
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("Export a skill with platform-specific installation guides"));
}

#[test]
fn test_skill_export_missing_skill() {
    let tmp = tempdir().unwrap();
    let output = tmp.path().join("output");
    
    let mut cmd = Command::cargo_bin("runbox").unwrap();
    cmd.arg("skill")
        .arg("export")
        .arg("nonexistent-skill-12345")
        .arg("--output")
        .arg(&output);
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("Skill not found"));
}

#[test]
fn test_skill_show_missing_skill() {
    let mut cmd = Command::cargo_bin("runbox").unwrap();
    cmd.arg("skill")
        .arg("show")
        .arg("nonexistent-skill-12345");
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("Skill not found"));
}
