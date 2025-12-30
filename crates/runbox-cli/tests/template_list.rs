use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

#[test]
fn test_template_list_empty() {
    let temp = TempDir::new().unwrap();

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["template", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No templates found."));
}

#[test]
fn test_template_list_with_templates() {
    let temp = TempDir::new().unwrap();

    // Create templates directory
    let templates_dir = temp.path().join("templates");
    fs::create_dir_all(&templates_dir).unwrap();

    // Create a template file
    let template = r#"{
        "template_version": 0,
        "template_id": "tpl_a1b2c3d4-e5f6-7890-abcd-ef1234567890",
        "name": "Test Template",
        "exec": {
            "argv": ["echo", "hello"],
            "cwd": ".",
            "env": {},
            "timeout_sec": 0
        },
        "code_state": {
            "repo_url": "git@github.com:org/repo.git"
        }
    }"#;

    fs::write(
        templates_dir.join("tpl_a1b2c3d4-e5f6-7890-abcd-ef1234567890.json"),
        template,
    )
    .unwrap();

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["template", "list"])
        .assert()
        .success()
        // Verify short ID (first 8 hex chars of template_id)
        .stdout(predicate::str::contains("a1b2c3d4"))
        // Verify template name
        .stdout(predicate::str::contains("Test Template"));
}

#[test]
fn test_template_list_output_format() {
    let temp = TempDir::new().unwrap();

    // Create templates directory
    let templates_dir = temp.path().join("templates");
    fs::create_dir_all(&templates_dir).unwrap();

    // Create a template file
    let template = r#"{
        "template_version": 0,
        "template_id": "tpl_deadbeef-1234-5678-abcd-ef1234567890",
        "name": "My Test",
        "exec": {
            "argv": ["python", "-m", "test"],
            "cwd": ".",
            "env": {},
            "timeout_sec": 0
        },
        "code_state": {
            "repo_url": "git@github.com:org/repo.git"
        }
    }"#;

    fs::write(
        templates_dir.join("tpl_deadbeef-1234-5678-abcd-ef1234567890.json"),
        template,
    )
    .unwrap();

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["template", "list"])
        .assert()
        .success()
        // Verify table headers
        .stdout(predicate::str::contains("ID"))
        .stdout(predicate::str::contains("NAME"))
        // Verify template_id short form (first 8 hex chars)
        .stdout(predicate::str::contains("deadbeef"))
        // Verify name
        .stdout(predicate::str::contains("My Test"));
}
