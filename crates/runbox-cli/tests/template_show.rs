use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

/// Create a test template JSON directly in the storage directory
fn create_test_template(temp_dir: &TempDir, template_id: &str, name: &str) {
    let templates_dir = temp_dir.path().join("templates");
    fs::create_dir_all(&templates_dir).unwrap();

    let template_json = serde_json::json!({
        "template_version": 0,
        "template_id": template_id,
        "name": name,
        "exec": {
            "argv": ["echo", "hello", "{name}"],
            "cwd": ".",
            "env": {},
            "timeout_sec": 0
        },
        "bindings": {
            "defaults": {
                "name": "world"
            },
            "interactive": []
        },
        "code_state": {
            "repo_url": "git@github.com:test/repo.git"
        }
    });

    let template_path = templates_dir.join(format!("{}.json", template_id));
    fs::write(&template_path, serde_json::to_string_pretty(&template_json).unwrap()).unwrap();
}

#[test]
fn test_template_show() {
    let temp = TempDir::new().unwrap();

    // Setup: create a template
    create_test_template(&temp, "tpl_test-1234-5678-90ab-cdef12345678", "Test Template");

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["template", "show", "tpl_test-1234-5678-90ab-cdef12345678"])
        .assert()
        .success()
        .stdout(predicate::str::contains("tpl_test-1234-5678-90ab-cdef12345678"))
        .stdout(predicate::str::contains("Test Template"))
        .stdout(predicate::str::contains("echo"))
        .stdout(predicate::str::contains("repo_url"));
}

#[test]
fn test_template_show_with_short_id() {
    let temp = TempDir::new().unwrap();

    // Setup: create a template
    create_test_template(&temp, "tpl_abcd1234-5678-90ab-cdef12345678", "Short ID Template");

    // Use only the first few characters of the template ID (without tpl_ prefix)
    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["template", "show", "abcd12"])
        .assert()
        .success()
        .stdout(predicate::str::contains("tpl_abcd1234-5678-90ab-cdef12345678"))
        .stdout(predicate::str::contains("Short ID Template"));
}

#[test]
fn test_template_show_all_fields() {
    let temp = TempDir::new().unwrap();

    // Setup: create a template with all fields
    create_test_template(&temp, "tpl_fields-test-1234-5678-90abcdef1234", "All Fields Template");

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["template", "show", "fields"])
        .assert()
        .success()
        // Verify exec fields
        .stdout(predicate::str::contains("exec"))
        .stdout(predicate::str::contains("argv"))
        .stdout(predicate::str::contains("cwd"))
        // Verify bindings fields
        .stdout(predicate::str::contains("bindings"))
        .stdout(predicate::str::contains("defaults"))
        // Verify code_state fields
        .stdout(predicate::str::contains("code_state"))
        .stdout(predicate::str::contains("repo_url"));
}

#[test]
fn test_template_show_not_found() {
    let temp = TempDir::new().unwrap();

    // Ensure the templates directory exists but is empty
    fs::create_dir_all(temp.path().join("templates")).unwrap();
    fs::create_dir_all(temp.path().join("runs")).unwrap();
    fs::create_dir_all(temp.path().join("playlists")).unwrap();
    fs::create_dir_all(temp.path().join("logs")).unwrap();

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["template", "show", "nonexistent"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("No item found"));
}
