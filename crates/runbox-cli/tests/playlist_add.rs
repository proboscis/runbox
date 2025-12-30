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

/// Valid playlist JSON for testing
fn valid_playlist_json(playlist_id: &str, name: &str) -> String {
    format!(
        r#"{{
    "playlist_id": "{}",
    "name": "{}",
    "items": []
}}"#,
        playlist_id, name
    )
}

/// Setup helper: create a template and playlist in storage
fn setup_template_and_playlist(temp: &TempDir, template_id: &str, playlist_id: &str) {
    let template_file = temp.path().join("template.json");
    std::fs::write(&template_file, valid_template_json(template_id)).unwrap();

    runbox_cmd(temp)
        .args(["template", "create", template_file.to_str().unwrap()])
        .assert()
        .success();

    let playlist_file = temp.path().join("playlist.json");
    std::fs::write(&playlist_file, valid_playlist_json(playlist_id, "Test Playlist")).unwrap();

    runbox_cmd(temp)
        .args(["playlist", "create", playlist_file.to_str().unwrap()])
        .assert()
        .success();
}

#[test]
fn test_playlist_add() {
    let temp = TempDir::new().unwrap();
    setup_template_and_playlist(&temp, "tpl_test", "pl_test");

    runbox_cmd(&temp)
        .args(["playlist", "add", "pl_test", "tpl_test"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Added"));

    // Verify item was added by loading the playlist
    let playlist_path = temp.path().join("playlists").join("pl_test.json");
    let content = std::fs::read_to_string(&playlist_path).unwrap();
    let playlist: serde_json::Value = serde_json::from_str(&content).unwrap();

    assert_eq!(playlist["items"].as_array().unwrap().len(), 1);
    assert_eq!(playlist["items"][0]["template_id"], "tpl_test");
    assert!(playlist["items"][0]["label"].is_null());
}

#[test]
fn test_playlist_add_with_label() {
    let temp = TempDir::new().unwrap();
    setup_template_and_playlist(&temp, "tpl_labeled", "pl_labeled");

    runbox_cmd(&temp)
        .args([
            "playlist",
            "add",
            "pl_labeled",
            "tpl_labeled",
            "--label",
            "My Custom Run",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Added"));

    // Verify item was added with label
    let playlist_path = temp.path().join("playlists").join("pl_labeled.json");
    let content = std::fs::read_to_string(&playlist_path).unwrap();
    let playlist: serde_json::Value = serde_json::from_str(&content).unwrap();

    assert_eq!(playlist["items"].as_array().unwrap().len(), 1);
    assert_eq!(playlist["items"][0]["template_id"], "tpl_labeled");
    assert_eq!(playlist["items"][0]["label"], "My Custom Run");
}

#[test]
fn test_playlist_add_multiple_items() {
    let temp = TempDir::new().unwrap();

    // Create playlist
    let playlist_file = temp.path().join("playlist.json");
    std::fs::write(
        &playlist_file,
        valid_playlist_json("pl_multi", "Multi Playlist"),
    )
    .unwrap();
    runbox_cmd(&temp)
        .args(["playlist", "create", playlist_file.to_str().unwrap()])
        .assert()
        .success();

    // Create two templates
    let template1_file = temp.path().join("template1.json");
    std::fs::write(&template1_file, valid_template_json("tpl_first")).unwrap();
    runbox_cmd(&temp)
        .args(["template", "create", template1_file.to_str().unwrap()])
        .assert()
        .success();

    let template2_file = temp.path().join("template2.json");
    std::fs::write(&template2_file, valid_template_json("tpl_second")).unwrap();
    runbox_cmd(&temp)
        .args(["template", "create", template2_file.to_str().unwrap()])
        .assert()
        .success();

    // Add both templates to playlist
    runbox_cmd(&temp)
        .args(["playlist", "add", "pl_multi", "tpl_first"])
        .assert()
        .success();

    runbox_cmd(&temp)
        .args(["playlist", "add", "pl_multi", "tpl_second", "--label", "Second"])
        .assert()
        .success();

    // Verify both items were added
    let playlist_path = temp.path().join("playlists").join("pl_multi.json");
    let content = std::fs::read_to_string(&playlist_path).unwrap();
    let playlist: serde_json::Value = serde_json::from_str(&content).unwrap();

    let items = playlist["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["template_id"], "tpl_first");
    assert_eq!(items[1]["template_id"], "tpl_second");
    assert_eq!(items[1]["label"], "Second");
}

#[test]
fn test_playlist_add_playlist_not_found() {
    let temp = TempDir::new().unwrap();

    // Create only a template, no playlist
    let template_file = temp.path().join("template.json");
    std::fs::write(&template_file, valid_template_json("tpl_orphan")).unwrap();
    runbox_cmd(&temp)
        .args(["template", "create", template_file.to_str().unwrap()])
        .assert()
        .success();

    runbox_cmd(&temp)
        .args(["playlist", "add", "nonexistent", "tpl_orphan"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("No item found"));
}

#[test]
fn test_playlist_add_template_not_found() {
    let temp = TempDir::new().unwrap();

    // Create only a playlist, no template
    let playlist_file = temp.path().join("playlist.json");
    std::fs::write(
        &playlist_file,
        valid_playlist_json("pl_lonely", "Lonely Playlist"),
    )
    .unwrap();
    runbox_cmd(&temp)
        .args(["playlist", "create", playlist_file.to_str().unwrap()])
        .assert()
        .success();

    runbox_cmd(&temp)
        .args(["playlist", "add", "pl_lonely", "nonexistent"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("No item found"));
}

#[test]
fn test_playlist_add_with_short_id() {
    let temp = TempDir::new().unwrap();
    setup_template_and_playlist(&temp, "tpl_shortid", "pl_shortid");

    // Use short IDs (without pl_ and tpl_ prefixes)
    runbox_cmd(&temp)
        .args(["playlist", "add", "shortid", "shortid"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Added"));

    // Verify item was added
    let playlist_path = temp.path().join("playlists").join("pl_shortid.json");
    let content = std::fs::read_to_string(&playlist_path).unwrap();
    let playlist: serde_json::Value = serde_json::from_str(&content).unwrap();

    assert_eq!(playlist["items"].as_array().unwrap().len(), 1);
}
