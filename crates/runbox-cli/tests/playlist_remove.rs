use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

/// Helper to create a runbox command with RUNBOX_HOME set to temp directory
fn runbox_cmd(temp_dir: &TempDir) -> Command {
    let mut cmd = Command::cargo_bin("runbox").unwrap();
    cmd.env("RUNBOX_HOME", temp_dir.path());
    cmd
}

/// Create a test playlist JSON directly in the storage directory
fn create_test_playlist(
    temp_dir: &TempDir,
    playlist_id: &str,
    name: &str,
    items: &[(&str, Option<&str>)],
) {
    let playlists_dir = temp_dir.path().join("playlists");
    fs::create_dir_all(&playlists_dir).unwrap();

    let items_json: Vec<serde_json::Value> = items
        .iter()
        .map(|(template_id, label)| {
            let mut item = serde_json::json!({
                "template_id": template_id
            });
            if let Some(l) = label {
                item["label"] = serde_json::Value::String(l.to_string());
            }
            item
        })
        .collect();

    let playlist_json = serde_json::json!({
        "playlist_id": playlist_id,
        "name": name,
        "items": items_json
    });

    let playlist_path = playlists_dir.join(format!("{}.json", playlist_id));
    fs::write(
        &playlist_path,
        serde_json::to_string_pretty(&playlist_json).unwrap(),
    )
    .unwrap();
}

/// Create a test template JSON directly in the storage directory
fn create_test_template(temp_dir: &TempDir, template_id: &str, name: &str) {
    let templates_dir = temp_dir.path().join("templates");
    fs::create_dir_all(&templates_dir).unwrap();

    let template_json = serde_json::json!({
        "template_version": 0,
        "template_id": template_id,
        "name": name,
        "exec": {
            "argv": ["echo", "hello"],
            "cwd": ".",
            "env": {},
            "timeout_sec": 0
        },
        "bindings": {
            "defaults": {},
            "interactive": []
        },
        "code_state": {
            "repo_url": "git@github.com:test/repo.git"
        }
    });

    let template_path = templates_dir.join(format!("{}.json", template_id));
    fs::write(
        &template_path,
        serde_json::to_string_pretty(&template_json).unwrap(),
    )
    .unwrap();
}

/// Load a playlist from storage and return its items
fn load_playlist_items(temp_dir: &TempDir, playlist_id: &str) -> Vec<String> {
    let playlist_path = temp_dir
        .path()
        .join("playlists")
        .join(format!("{}.json", playlist_id));
    let content = fs::read_to_string(&playlist_path).unwrap();
    let playlist: serde_json::Value = serde_json::from_str(&content).unwrap();
    playlist["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["template_id"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn test_playlist_remove_by_template_id() {
    let temp = TempDir::new().unwrap();

    // Setup: create a template and a playlist containing it
    create_test_template(
        &temp,
        "tpl_to-remove-1234-5678-90ab-cdef12345678",
        "Template to Remove",
    );
    create_test_template(
        &temp,
        "tpl_to-keep-1234-5678-90ab-cdef12345678",
        "Template to Keep",
    );
    create_test_playlist(
        &temp,
        "pl_test-remove-1234",
        "Test Playlist",
        &[
            (
                "tpl_to-remove-1234-5678-90ab-cdef12345678",
                Some("Remove Me"),
            ),
            ("tpl_to-keep-1234-5678-90ab-cdef12345678", Some("Keep Me")),
        ],
    );

    // Remove the template from the playlist
    runbox_cmd(&temp)
        .args([
            "playlist",
            "remove",
            "pl_test-remove-1234",
            "tpl_to-remove-1234-5678-90ab-cdef12345678",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed"));

    // Verify the item was removed
    let items = load_playlist_items(&temp, "pl_test-remove-1234");
    assert_eq!(items.len(), 1, "Playlist should have 1 item after removal");
    assert_eq!(items[0], "tpl_to-keep-1234-5678-90ab-cdef12345678");
}

#[test]
fn test_playlist_remove_by_index() {
    let temp = TempDir::new().unwrap();

    create_test_playlist(
        &temp,
        "pl_index-1234",
        "Index Playlist",
        &[
            ("tpl_first-1234-5678-90ab-cdef12345678", None),
            ("tpl_second-1234-5678-90ab-cdef12345678", None),
        ],
    );

    runbox_cmd(&temp)
        .args(["playlist", "remove", "pl_index-1234", "0"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed"));

    let items = load_playlist_items(&temp, "pl_index-1234");
    assert_eq!(items.len(), 1, "Playlist should have 1 item after removal");
    assert_eq!(items[0], "tpl_second-1234-5678-90ab-cdef12345678");
}

#[test]
fn test_playlist_remove_with_short_ids() {
    let temp = TempDir::new().unwrap();

    // Setup: create a template and a playlist containing it
    create_test_template(&temp, "tpl_shortid-1234-5678-90ab-cdef12345678", "Template");
    create_test_playlist(
        &temp,
        "pl_shortpl-1234-5678",
        "Test Playlist",
        &[("tpl_shortid-1234-5678-90ab-cdef12345678", None)],
    );

    // Remove using short IDs (without prefixes)
    runbox_cmd(&temp)
        .args(["playlist", "remove", "shortpl", "shortid"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed"));

    // Verify the item was removed
    let items = load_playlist_items(&temp, "pl_shortpl-1234-5678");
    assert!(items.is_empty(), "Playlist should be empty after removal");
}

#[test]
fn test_playlist_remove_index_out_of_bounds() {
    let temp = TempDir::new().unwrap();

    create_test_playlist(
        &temp,
        "pl_index-oob-1234",
        "Index Playlist",
        &[("tpl_only-1234-5678-90ab-cdef12345678", None)],
    );

    runbox_cmd(&temp)
        .args(["playlist", "remove", "pl_index-oob-1234", "999"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("out of bounds"));

    let items = load_playlist_items(&temp, "pl_index-oob-1234");
    assert_eq!(items.len(), 1, "Playlist should still have 1 item");
}

#[test]
fn test_playlist_remove_not_found() {
    let temp = TempDir::new().unwrap();

    // Ensure the directories exist but are empty
    fs::create_dir_all(temp.path().join("templates")).unwrap();
    fs::create_dir_all(temp.path().join("playlists")).unwrap();
    fs::create_dir_all(temp.path().join("runs")).unwrap();
    fs::create_dir_all(temp.path().join("logs")).unwrap();

    // Create a template so the template resolution succeeds
    create_test_template(&temp, "tpl_exists-1234-5678-90ab-cdef12345678", "Exists");

    // Try to remove from a nonexistent playlist
    runbox_cmd(&temp)
        .args([
            "playlist",
            "remove",
            "nonexistent",
            "tpl_exists-1234-5678-90ab-cdef12345678",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("No item found"));
}

#[test]
fn test_playlist_remove_template_not_in_playlist() {
    let temp = TempDir::new().unwrap();

    // Setup: create templates and a playlist that doesn't contain the template to remove
    create_test_template(
        &temp,
        "tpl_in-playlist-1234-5678-90ab-cdef12345678",
        "In Playlist",
    );
    create_test_template(
        &temp,
        "tpl_not-in-playlist-1234-5678-90ab-cdef12345678",
        "Not In Playlist",
    );
    create_test_playlist(
        &temp,
        "pl_has-item-1234-5678",
        "Test Playlist",
        &[("tpl_in-playlist-1234-5678-90ab-cdef12345678", None)],
    );

    // Try to remove a template that is not in the playlist
    runbox_cmd(&temp)
        .args([
            "playlist",
            "remove",
            "pl_has-item-1234-5678",
            "tpl_not-in-playlist-1234-5678-90ab-cdef12345678",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found in playlist"));

    // Verify original item is still there
    let items = load_playlist_items(&temp, "pl_has-item-1234-5678");
    assert_eq!(items.len(), 1, "Playlist should still have 1 item");
}

#[test]
fn test_playlist_remove_empty_playlist() {
    let temp = TempDir::new().unwrap();

    // Setup: create a template and an empty playlist
    create_test_template(
        &temp,
        "tpl_orphan-1234-5678-90ab-cdef12345678",
        "Orphan Template",
    );
    create_test_playlist(&temp, "pl_empty-1234-5678", "Empty Playlist", &[]);

    // Try to remove a template from an empty playlist
    runbox_cmd(&temp)
        .args([
            "playlist",
            "remove",
            "pl_empty-1234-5678",
            "tpl_orphan-1234-5678-90ab-cdef12345678",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found in playlist"));
}
