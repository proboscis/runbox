use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

/// Create a test playlist JSON directly in the storage directory
fn create_test_playlist(temp_dir: &TempDir, playlist_id: &str, name: &str, items: Vec<(&str, Option<&str>)>) {
    let playlists_dir = temp_dir.path().join("playlists");
    fs::create_dir_all(&playlists_dir).unwrap();

    let items_json: Vec<serde_json::Value> = items
        .into_iter()
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
    fs::write(&playlist_path, serde_json::to_string_pretty(&playlist_json).unwrap()).unwrap();
}

#[test]
fn test_playlist_show() {
    let temp = TempDir::new().unwrap();

    // Setup: create a playlist with items
    create_test_playlist(
        &temp,
        "pl_test-1234-5678-90ab-cdef12345678",
        "Test Playlist",
        vec![
            ("tpl_runner-1111-2222-3333-444455556666", Some("Runner Task")),
            ("tpl_eval-aaaa-bbbb-cccc-ddddeeeeffff", None),
        ],
    );

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["playlist", "show", "pl_test-1234-5678-90ab-cdef12345678"])
        .assert()
        .success()
        .stdout(predicate::str::contains("pl_test-1234-5678-90ab-cdef12345678"))
        .stdout(predicate::str::contains("Test Playlist"))
        .stdout(predicate::str::contains("tpl_runner-1111-2222-3333-444455556666"))
        .stdout(predicate::str::contains("Runner Task"))
        .stdout(predicate::str::contains("tpl_eval-aaaa-bbbb-cccc-ddddeeeeffff"));
}

#[test]
fn test_playlist_show_with_short_id() {
    let temp = TempDir::new().unwrap();

    // Setup: create a playlist
    create_test_playlist(
        &temp,
        "pl_abcd1234-5678-90ab-cdef12345678",
        "Short ID Playlist",
        vec![("tpl_test-1111-2222-3333-444455556666", None)],
    );

    // Use only the first few characters of the playlist ID (without pl_ prefix)
    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["playlist", "show", "abcd12"])
        .assert()
        .success()
        .stdout(predicate::str::contains("pl_abcd1234-5678-90ab-cdef12345678"))
        .stdout(predicate::str::contains("Short ID Playlist"));
}

#[test]
fn test_playlist_show_empty_playlist() {
    let temp = TempDir::new().unwrap();

    // Setup: create an empty playlist
    create_test_playlist(
        &temp,
        "pl_empty-1234-5678-90ab-cdef12345678",
        "Empty Playlist",
        vec![],
    );

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["playlist", "show", "pl_empty-1234-5678-90ab-cdef12345678"])
        .assert()
        .success()
        .stdout(predicate::str::contains("pl_empty-1234-5678-90ab-cdef12345678"))
        .stdout(predicate::str::contains("Empty Playlist"))
        .stdout(predicate::str::contains("\"items\": []"));
}

#[test]
fn test_playlist_show_not_found() {
    let temp = TempDir::new().unwrap();

    // Ensure the storage directories exist but are empty
    fs::create_dir_all(temp.path().join("templates")).unwrap();
    fs::create_dir_all(temp.path().join("runs")).unwrap();
    fs::create_dir_all(temp.path().join("playlists")).unwrap();
    fs::create_dir_all(temp.path().join("logs")).unwrap();

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["playlist", "show", "nonexistent"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("No item found"));
}
