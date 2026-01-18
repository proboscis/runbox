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
fn test_playlist_show_json() {
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

    let assert = Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["playlist", "show", "pl_test-1234-5678-90ab-cdef12345678", "--json"])
        .assert()
        .success();

    let output = assert.get_output();
    let playlist: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    assert_eq!(playlist["playlist_id"], "pl_test-1234-5678-90ab-cdef12345678");
    assert_eq!(playlist["name"], "Test Playlist");
    let items = playlist["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["template_id"], "tpl_runner-1111-2222-3333-444455556666");
    assert_eq!(items[0]["label"], "Runner Task");
    assert_eq!(items[1]["template_id"], "tpl_eval-aaaa-bbbb-cccc-ddddeeeeffff");
    assert!(items[1]["label"].is_null());
}

#[test]
fn test_playlist_show_table_specific_playlist() {
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
        .stdout(predicate::str::contains("Playlist: pl_test-1234-5678-90ab-cdef12345678 (Test Playlist)"))
        .stdout(predicate::str::contains("IDX"))
        .stdout(predicate::str::contains("SHORT"))
        .stdout(predicate::str::contains("TEMPLATE"))
        .stdout(predicate::str::contains("LABEL"))
        .stdout(predicate::str::contains("Runner Task"))
        .stdout(predicate::str::contains("runbox playlist run"));
}

#[test]
fn test_playlist_show_flattened_all_playlists() {
    let temp = TempDir::new().unwrap();

    // Setup: create multiple playlists
    create_test_playlist(
        &temp,
        "pl_daily",
        "Daily Tasks",
        vec![("tpl_echo", Some("Echo Hello"))],
    );
    create_test_playlist(
        &temp,
        "pl_weekly",
        "Weekly Tasks",
        vec![("tpl_backup", Some("Backup Data"))],
    );

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["playlist", "show"])
        .assert()
        .success()
        .stdout(predicate::str::contains("PLAYLIST"))
        .stdout(predicate::str::contains("IDX"))
        .stdout(predicate::str::contains("SHORT"))
        .stdout(predicate::str::contains("TEMPLATE"))
        .stdout(predicate::str::contains("LABEL"))
        .stdout(predicate::str::contains("Echo Hello"))
        .stdout(predicate::str::contains("Backup Data"))
        .stdout(predicate::str::contains("runbox playlist run <SHORT>"));
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
    let assert = Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["playlist", "show", "abcd12", "--json"])
        .assert()
        .success();

    let output = assert.get_output();
    let playlist: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    assert_eq!(playlist["playlist_id"], "pl_abcd1234-5678-90ab-cdef12345678");
    assert_eq!(playlist["name"], "Short ID Playlist");
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

    // Table view for empty playlist
    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["playlist", "show", "pl_empty-1234-5678-90ab-cdef12345678"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Playlist: pl_empty-1234-5678-90ab-cdef12345678 (Empty Playlist)"))
        .stdout(predicate::str::contains("IDX"));

    // JSON view for empty playlist
    let assert = Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["playlist", "show", "pl_empty-1234-5678-90ab-cdef12345678", "--json"])
        .assert()
        .success();

    let output = assert.get_output();
    let playlist: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    assert_eq!(playlist["playlist_id"], "pl_empty-1234-5678-90ab-cdef12345678");
    assert_eq!(playlist["name"], "Empty Playlist");
    let items = playlist["items"].as_array().unwrap();
    assert!(items.is_empty());
}

#[test]
fn test_playlist_show_no_playlists() {
    let temp = TempDir::new().unwrap();

    // Ensure the storage directories exist but are empty
    fs::create_dir_all(temp.path().join("templates")).unwrap();
    fs::create_dir_all(temp.path().join("runs")).unwrap();
    fs::create_dir_all(temp.path().join("playlists")).unwrap();
    fs::create_dir_all(temp.path().join("logs")).unwrap();

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["playlist", "show"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No playlists found"));
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
