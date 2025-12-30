use assert_cmd::Command;
use predicates::prelude::*;
use std::path::PathBuf;
use tempfile::TempDir;

/// Get the path to the fixtures directory
fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

// === Happy Path Tests ===

#[test]
fn test_validate_valid_run() {
    let fixture = fixtures_dir().join("valid_run.json");

    Command::cargo_bin("runbox")
        .unwrap()
        .args(["validate", fixture.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Run"));
}

#[test]
fn test_validate_valid_template() {
    let fixture = fixtures_dir().join("valid_template.json");

    Command::cargo_bin("runbox")
        .unwrap()
        .args(["validate", fixture.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("RunTemplate"));
}

#[test]
fn test_validate_valid_playlist() {
    let fixture = fixtures_dir().join("valid_playlist.json");

    Command::cargo_bin("runbox")
        .unwrap()
        .args(["validate", fixture.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Playlist"));
}

#[test]
fn test_validate_valid_run_inline() {
    let temp = TempDir::new().unwrap();
    let run_file = temp.path().join("run.json");
    std::fs::write(
        &run_file,
        r#"{
        "run_version": 0,
        "run_id": "run_550e8400-e29b-41d4-a716-446655440000",
        "exec": {"argv": ["echo", "test"], "cwd": "."},
        "code_state": {
            "repo_url": "git@github.com:org/repo.git",
            "base_commit": "a1b2c3d4e5f6789012345678901234567890abcd"
        }
    }"#,
    )
    .unwrap();

    Command::cargo_bin("runbox")
        .unwrap()
        .args(["validate", run_file.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Run"));
}

#[test]
fn test_validate_valid_template_inline() {
    let temp = TempDir::new().unwrap();
    let template_file = temp.path().join("template.json");
    std::fs::write(
        &template_file,
        r#"{
        "template_version": 0,
        "template_id": "tpl_test-template-id",
        "name": "Test Template",
        "exec": {"argv": ["echo", "hello"], "cwd": "."},
        "code_state": {"repo_url": "git@github.com:org/repo.git"}
    }"#,
    )
    .unwrap();

    Command::cargo_bin("runbox")
        .unwrap()
        .args(["validate", template_file.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("RunTemplate"));
}

#[test]
fn test_validate_valid_playlist_inline() {
    let temp = TempDir::new().unwrap();
    let playlist_file = temp.path().join("playlist.json");
    std::fs::write(
        &playlist_file,
        r#"{
        "playlist_id": "pl_test",
        "name": "Test Playlist",
        "items": []
    }"#,
    )
    .unwrap();

    Command::cargo_bin("runbox")
        .unwrap()
        .args(["validate", playlist_file.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Playlist"));
}

// === Error Path Tests ===

#[test]
fn test_validate_invalid_json() {
    let fixture = fixtures_dir().join("invalid_run.json");

    Command::cargo_bin("runbox")
        .unwrap()
        .args(["validate", fixture.to_str().unwrap()])
        .assert()
        .failure();
}

#[test]
fn test_validate_file_not_found() {
    Command::cargo_bin("runbox")
        .unwrap()
        .args(["validate", "/nonexistent/file.json"])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("not found")
                .or(predicate::str::contains("No such file"))
                .or(predicate::str::contains("Failed to read")),
        );
}

#[test]
fn test_validate_malformed_json() {
    let fixture = fixtures_dir().join("malformed.json");

    Command::cargo_bin("runbox")
        .unwrap()
        .args(["validate", fixture.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Invalid JSON"));
}

#[test]
fn test_validate_invalid_missing_required_fields() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("invalid.json");

    // Run with missing required fields (exec and code_state)
    std::fs::write(
        &file,
        r#"{"run_id": "run_550e8400-e29b-41d4-a716-446655440000"}"#,
    )
    .unwrap();

    Command::cargo_bin("runbox")
        .unwrap()
        .args(["validate", file.to_str().unwrap()])
        .assert()
        .failure();
}

#[test]
fn test_validate_invalid_run_id_pattern() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("bad_run_id.json");

    // run_id doesn't match required pattern
    std::fs::write(
        &file,
        r#"{
        "run_id": "bad_id",
        "exec": {"argv": ["echo"], "cwd": "."},
        "code_state": {
            "repo_url": "git@github.com:org/repo.git",
            "base_commit": "a1b2c3d4e5f6789012345678901234567890abcd"
        }
    }"#,
    )
    .unwrap();

    Command::cargo_bin("runbox")
        .unwrap()
        .args(["validate", file.to_str().unwrap()])
        .assert()
        .failure();
}

#[test]
fn test_validate_invalid_template_id_pattern() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("bad_template_id.json");

    // template_id doesn't start with "tpl_"
    std::fs::write(
        &file,
        r#"{
        "template_version": 0,
        "template_id": "bad_id",
        "name": "Test",
        "exec": {"cwd": "."},
        "code_state": {"repo_url": "git@github.com:org/repo.git"}
    }"#,
    )
    .unwrap();

    Command::cargo_bin("runbox")
        .unwrap()
        .args(["validate", file.to_str().unwrap()])
        .assert()
        .failure();
}

#[test]
fn test_validate_invalid_playlist_id_pattern() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("bad_playlist_id.json");

    // playlist_id doesn't start with "pl_"
    std::fs::write(
        &file,
        r#"{
        "playlist_id": "bad_id",
        "name": "Test Playlist"
    }"#,
    )
    .unwrap();

    Command::cargo_bin("runbox")
        .unwrap()
        .args(["validate", file.to_str().unwrap()])
        .assert()
        .failure();
}

#[test]
fn test_validate_unknown_type() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("unknown.json");

    // JSON without run_id, template_id, or playlist_id
    std::fs::write(&file, r#"{"name": "something", "value": 123}"#).unwrap();

    Command::cargo_bin("runbox")
        .unwrap()
        .args(["validate", file.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Could not determine JSON type"));
}

#[test]
fn test_validate_output_shows_file_path() {
    let fixture = fixtures_dir().join("valid_run.json");

    Command::cargo_bin("runbox")
        .unwrap()
        .args(["validate", fixture.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("valid_run.json"));
}
