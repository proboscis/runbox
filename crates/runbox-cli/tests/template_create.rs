use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

/// Helper to create a runbox command with RUNBOX_HOME set to temp directory
fn runbox_cmd(temp_dir: &TempDir) -> Command {
    let mut cmd = Command::cargo_bin("runbox").unwrap();
    cmd.env("RUNBOX_HOME", temp_dir.path());
    cmd
}

/// Valid template JSON for testing
fn valid_template_json(template_id: &str) -> String {
    format!(
        r#"{{
    "template_version": 0,
    "template_id": "{}",
    "name": "Test Template",
    "exec": {{
        "argv": ["echo", "hello"],
        "cwd": "."
    }},
    "code_state": {{
        "repo_url": "git@github.com:org/repo.git"
    }}
}}"#,
        template_id
    )
}

#[test]
fn test_template_create_success() {
    let temp = TempDir::new().unwrap();
    let template_file = temp.path().join("template.json");
    std::fs::write(&template_file, valid_template_json("tpl_test")).unwrap();

    runbox_cmd(&temp)
        .args(["template", "create", template_file.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Template created"));

    // Verify template was saved to storage
    let templates_dir = temp.path().join("templates");
    assert!(templates_dir.exists(), "Templates directory should exist");

    let template_path = templates_dir.join("tpl_test.json");
    assert!(
        template_path.exists(),
        "Template file should exist at {:?}",
        template_path
    );

    // Verify template content
    let saved_content = std::fs::read_to_string(&template_path).unwrap();
    let saved: serde_json::Value = serde_json::from_str(&saved_content).unwrap();
    assert_eq!(saved["template_id"], "tpl_test");
    assert_eq!(saved["name"], "Test Template");
}

#[test]
fn test_template_create_and_list() {
    let temp = TempDir::new().unwrap();
    let template_file = temp.path().join("template.json");
    std::fs::write(&template_file, valid_template_json("tpl_listtest")).unwrap();

    // Create template
    runbox_cmd(&temp)
        .args(["template", "create", template_file.to_str().unwrap()])
        .assert()
        .success();

    // Verify it appears in list (list shows short ID without tpl_ prefix)
    runbox_cmd(&temp)
        .args(["template", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("listtest"));
}

#[test]
fn test_template_create_invalid_missing_fields() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("invalid.json");

    // Missing required fields (only has template_id)
    std::fs::write(&file, r#"{"template_id": "tpl_bad"}"#).unwrap();

    runbox_cmd(&temp)
        .args(["template", "create", file.to_str().unwrap()])
        .assert()
        .failure();
}

#[test]
fn test_template_create_invalid_template_id_pattern() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("bad_id.json");

    // template_id must start with "tpl_"
    let json = r#"{
        "template_version": 0,
        "template_id": "bad_id",
        "name": "Test",
        "exec": {"argv": ["echo"], "cwd": "."},
        "code_state": {"repo_url": "git@github.com:org/repo.git"}
    }"#;
    std::fs::write(&file, json).unwrap();

    runbox_cmd(&temp)
        .args(["template", "create", file.to_str().unwrap()])
        .assert()
        .failure();
}

#[test]
fn test_template_create_file_not_found() {
    let temp = TempDir::new().unwrap();
    let nonexistent = temp.path().join("nonexistent.json");

    runbox_cmd(&temp)
        .args(["template", "create", nonexistent.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Failed to read file"));
}

#[test]
fn test_template_create_invalid_json_syntax() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("syntax_error.json");

    // Invalid JSON syntax
    std::fs::write(&file, r#"{ invalid json }"#).unwrap();

    runbox_cmd(&temp)
        .args(["template", "create", file.to_str().unwrap()])
        .assert()
        .failure();
}

#[test]
fn test_template_create_duplicate_id() {
    let temp = TempDir::new().unwrap();
    let template_file = temp.path().join("template.json");
    std::fs::write(&template_file, valid_template_json("tpl_duplicate")).unwrap();

    // Create first template
    runbox_cmd(&temp)
        .args(["template", "create", template_file.to_str().unwrap()])
        .assert()
        .success();

    // Attempt to create duplicate - should fail
    let duplicate_file = temp.path().join("template2.json");
    std::fs::write(&duplicate_file, valid_template_json("tpl_duplicate")).unwrap();

    runbox_cmd(&temp)
        .args(["template", "create", duplicate_file.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Template already exists"));

    // Verify the original template still exists
    let template_path = temp.path().join("templates").join("tpl_duplicate.json");
    assert!(
        template_path.exists(),
        "Template should still exist after duplicate attempt"
    );
}

#[test]
fn test_template_create_with_bindings() {
    let temp = TempDir::new().unwrap();
    let template_file = temp.path().join("template_bindings.json");

    let json = r#"{
        "template_version": 0,
        "template_id": "tpl_with_bindings",
        "name": "Template with Bindings",
        "exec": {
            "argv": ["echo", "{message}"],
            "cwd": ".",
            "env": {"MY_VAR": "value"},
            "timeout_sec": 60
        },
        "bindings": {
            "defaults": {"message": "hello"},
            "interactive": ["message"]
        },
        "code_state": {
            "repo_url": "git@github.com:org/repo.git"
        }
    }"#;
    std::fs::write(&template_file, json).unwrap();

    runbox_cmd(&temp)
        .args(["template", "create", template_file.to_str().unwrap()])
        .assert()
        .success();

    // Verify bindings were saved
    let template_path = temp.path().join("templates").join("tpl_with_bindings.json");
    let saved_content = std::fs::read_to_string(&template_path).unwrap();
    let saved: serde_json::Value = serde_json::from_str(&saved_content).unwrap();

    assert!(saved["bindings"].is_object());
    assert_eq!(saved["bindings"]["defaults"]["message"], "hello");
}
