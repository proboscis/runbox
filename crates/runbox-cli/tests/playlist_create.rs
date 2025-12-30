use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

/// Helper to create a runbox command with RUNBOX_HOME set to temp directory
fn runbox_cmd(temp_dir: &TempDir) -> Command {
    let mut cmd = Command::cargo_bin("runbox").unwrap();
    cmd.env("RUNBOX_HOME", temp_dir.path());
    cmd
}

/// Valid playlist JSON for testing
fn valid_playlist_json(playlist_id: &str) -> String {
    format!(
        r#"{{
    "playlist_id": "{}",
    "name": "Test Playlist",
    "items": []
}}"#,
        playlist_id
    )
}

/// Valid playlist JSON with items
fn valid_playlist_with_items_json(playlist_id: &str) -> String {
    format!(
        r#"{{
    "playlist_id": "{}",
    "name": "Test Playlist with Items",
    "items": [
        {{"template_id": "tpl_runner"}},
        {{"template_id": "tpl_eval", "label": "Evaluation"}}
    ]
}}"#,
        playlist_id
    )
}

#[test]
fn test_playlist_create_success() {
    let temp = TempDir::new().unwrap();
    let playlist_file = temp.path().join("playlist.json");
    std::fs::write(&playlist_file, valid_playlist_json("pl_test")).unwrap();

    runbox_cmd(&temp)
        .args(["playlist", "create", playlist_file.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Playlist created"));

    // Verify playlist was saved to storage
    let playlists_dir = temp.path().join("playlists");
    assert!(playlists_dir.exists(), "Playlists directory should exist");

    let playlist_path = playlists_dir.join("pl_test.json");
    assert!(
        playlist_path.exists(),
        "Playlist file should exist at {:?}",
        playlist_path
    );

    // Verify playlist content
    let saved_content = std::fs::read_to_string(&playlist_path).unwrap();
    let saved: serde_json::Value = serde_json::from_str(&saved_content).unwrap();
    assert_eq!(saved["playlist_id"], "pl_test");
    assert_eq!(saved["name"], "Test Playlist");
    assert!(saved["items"].is_array());
}

#[test]
fn test_playlist_create_with_items() {
    let temp = TempDir::new().unwrap();
    let playlist_file = temp.path().join("playlist.json");
    std::fs::write(
        &playlist_file,
        valid_playlist_with_items_json("pl_with_items"),
    )
    .unwrap();

    runbox_cmd(&temp)
        .args(["playlist", "create", playlist_file.to_str().unwrap()])
        .assert()
        .success();

    // Verify items were saved
    let playlist_path = temp.path().join("playlists").join("pl_with_items.json");
    let saved_content = std::fs::read_to_string(&playlist_path).unwrap();
    let saved: serde_json::Value = serde_json::from_str(&saved_content).unwrap();

    assert_eq!(saved["items"].as_array().unwrap().len(), 2);
    assert_eq!(saved["items"][0]["template_id"], "tpl_runner");
    assert_eq!(saved["items"][1]["template_id"], "tpl_eval");
    assert_eq!(saved["items"][1]["label"], "Evaluation");
}

#[test]
fn test_playlist_create_and_list() {
    let temp = TempDir::new().unwrap();
    let playlist_file = temp.path().join("playlist.json");
    std::fs::write(&playlist_file, valid_playlist_json("pl_listtest")).unwrap();

    // Create playlist
    runbox_cmd(&temp)
        .args(["playlist", "create", playlist_file.to_str().unwrap()])
        .assert()
        .success();

    // Verify it appears in list (list shows short ID without pl_ prefix)
    runbox_cmd(&temp)
        .args(["playlist", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("listtest"));
}

#[test]
fn test_playlist_create_invalid_missing_fields() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("invalid.json");

    // Missing required 'name' field (only has playlist_id)
    std::fs::write(&file, r#"{"playlist_id": "pl_bad"}"#).unwrap();

    runbox_cmd(&temp)
        .args(["playlist", "create", file.to_str().unwrap()])
        .assert()
        .failure();
}

#[test]
fn test_playlist_create_invalid_playlist_id_pattern() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("bad_id.json");

    // playlist_id must start with "pl_"
    let json = r#"{
        "playlist_id": "bad_id",
        "name": "Test Playlist",
        "items": []
    }"#;
    std::fs::write(&file, json).unwrap();

    runbox_cmd(&temp)
        .args(["playlist", "create", file.to_str().unwrap()])
        .assert()
        .failure();
}

#[test]
fn test_playlist_create_file_not_found() {
    let temp = TempDir::new().unwrap();
    let nonexistent = temp.path().join("nonexistent.json");

    runbox_cmd(&temp)
        .args(["playlist", "create", nonexistent.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Failed to read file"));
}

#[test]
fn test_playlist_create_invalid_json_syntax() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("syntax_error.json");

    // Invalid JSON syntax
    std::fs::write(&file, r#"{ invalid json }"#).unwrap();

    runbox_cmd(&temp)
        .args(["playlist", "create", file.to_str().unwrap()])
        .assert()
        .failure();
}

#[test]
fn test_playlist_create_duplicate_id() {
    let temp = TempDir::new().unwrap();
    let playlist_file = temp.path().join("playlist.json");
    std::fs::write(&playlist_file, valid_playlist_json("pl_duplicate")).unwrap();

    // Create first playlist
    runbox_cmd(&temp)
        .args(["playlist", "create", playlist_file.to_str().unwrap()])
        .assert()
        .success();

    // Attempt to create duplicate - current implementation overwrites
    let duplicate_file = temp.path().join("playlist2.json");
    std::fs::write(&duplicate_file, valid_playlist_json("pl_duplicate")).unwrap();

    // Try creating again with same ID (current implementation overwrites)
    runbox_cmd(&temp)
        .args(["playlist", "create", duplicate_file.to_str().unwrap()])
        .assert()
        .success();

    // Verify the playlist still exists (either overwritten or rejected)
    let playlist_path = temp.path().join("playlists").join("pl_duplicate.json");
    assert!(
        playlist_path.exists(),
        "Playlist should still exist after duplicate attempt"
    );
}
