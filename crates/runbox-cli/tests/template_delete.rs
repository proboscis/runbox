use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

/// Helper to create a template JSON file in the storage directory
fn setup_template(temp: &TempDir, template_id: &str) {
    let templates_dir = temp.path().join("templates");
    fs::create_dir_all(&templates_dir).unwrap();

    let template_json = serde_json::json!({
        "template_version": 0,
        "template_id": template_id,
        "name": "Test Template",
        "exec": {
            "argv": ["echo", "hello"],
            "cwd": "."
        },
        "code_state": {
            "repo_url": "git@github.com:org/repo.git"
        }
    });

    let template_path = templates_dir.join(format!("{}.json", template_id));
    fs::write(
        &template_path,
        serde_json::to_string_pretty(&template_json).unwrap(),
    )
    .unwrap();
}

/// Helper to check if a template exists in storage
fn template_exists(temp: &TempDir, template_id: &str) -> bool {
    let template_path = temp
        .path()
        .join("templates")
        .join(format!("{}.json", template_id));
    template_path.exists()
}

#[test]
fn test_template_delete() {
    let temp = TempDir::new().unwrap();
    let template_id = "tpl_12345678-1234-5678-abcd-ef1234567890";

    // Setup: create a template
    setup_template(&temp, template_id);
    assert!(
        template_exists(&temp, template_id),
        "Template should exist before deletion"
    );

    // Run delete command
    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["template", "delete", template_id])
        .assert()
        .success()
        .stdout(predicate::str::contains("Template deleted"));

    // Verify template no longer exists
    assert!(
        !template_exists(&temp, template_id),
        "Template should not exist after deletion"
    );
}

#[test]
fn test_template_delete_short_id() {
    let temp = TempDir::new().unwrap();
    let template_id = "tpl_abcdef12-3456-7890-abcd-ef1234567890";

    // Setup: create a template
    setup_template(&temp, template_id);
    assert!(
        template_exists(&temp, template_id),
        "Template should exist before deletion"
    );

    // Run delete command with short ID
    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["template", "delete", "abcdef12"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Template deleted"));

    // Verify template no longer exists
    assert!(
        !template_exists(&temp, template_id),
        "Template should not exist after deletion"
    );
}

#[test]
fn test_template_delete_not_found() {
    let temp = TempDir::new().unwrap();

    // Ensure the templates directory exists (Storage::new creates it)
    fs::create_dir_all(temp.path().join("templates")).unwrap();

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["template", "delete", "nonexistent"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("No item found"));
}
