use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

#[test]
fn test_playlist_list_empty() {
    let temp = TempDir::new().unwrap();

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["playlist", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No playlists found."));
}

#[test]
fn test_playlist_list_with_playlists() {
    let temp = TempDir::new().unwrap();

    // Create playlists directory
    let playlists_dir = temp.path().join("playlists");
    fs::create_dir_all(&playlists_dir).unwrap();

    // Create a playlist file
    let playlist = r#"{
        "playlist_id": "pl_a1b2c3d4-e5f6-7890-abcd-ef1234567890",
        "name": "Daily Tasks",
        "items": [
            {
                "template_id": "tpl_deadbeef-1234-5678-abcd-ef1234567890",
                "label": "Build"
            }
        ]
    }"#;

    fs::write(
        playlists_dir.join("pl_a1b2c3d4-e5f6-7890-abcd-ef1234567890.json"),
        playlist,
    )
    .unwrap();

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["playlist", "list"])
        .assert()
        .success()
        // Verify short ID (first 8 hex chars of playlist_id)
        .stdout(predicate::str::contains("a1b2c3d4"))
        // Verify playlist name
        .stdout(predicate::str::contains("Daily Tasks"));
}

#[test]
fn test_playlist_list_output_format() {
    let temp = TempDir::new().unwrap();

    // Create playlists directory
    let playlists_dir = temp.path().join("playlists");
    fs::create_dir_all(&playlists_dir).unwrap();

    // Create a playlist with multiple items
    let playlist = r#"{
        "playlist_id": "pl_deadbeef-1234-5678-abcd-ef1234567890",
        "name": "My Test Playlist",
        "items": [
            {
                "template_id": "tpl_11111111-1234-5678-abcd-ef1234567890"
            },
            {
                "template_id": "tpl_22222222-1234-5678-abcd-ef1234567890",
                "label": "Second"
            }
        ]
    }"#;

    fs::write(
        playlists_dir.join("pl_deadbeef-1234-5678-abcd-ef1234567890.json"),
        playlist,
    )
    .unwrap();

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["playlist", "list"])
        .assert()
        .success()
        // Verify table headers
        .stdout(predicate::str::contains("ID"))
        .stdout(predicate::str::contains("NAME"))
        .stdout(predicate::str::contains("ITEMS"))
        // Verify playlist_id short form (first 8 hex chars)
        .stdout(predicate::str::contains("deadbeef"))
        // Verify name
        .stdout(predicate::str::contains("My Test Playlist"))
        // Verify item count
        .stdout(predicate::str::contains("2"));
}
