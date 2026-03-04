use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

/// Create a test template JSON directly in the storage directory
fn create_test_template(temp_dir: &TempDir, template_id: &str, name: &str) {
    let templates_dir = temp_dir.path().join("templates");
    fs::create_dir_all(&templates_dir).unwrap();

    let template_json = serde_json::json!({
        "template_id": template_id,
        "template_version": 0,
        "name": name,
        "exec": {
            "argv": ["echo", "Hello from", name],
            "cwd": ".",
            "env": {},
            "timeout_sec": 60
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

/// Create a test playlist JSON directly in the storage directory
fn create_test_playlist(
    temp_dir: &TempDir,
    playlist_id: &str,
    name: &str,
    items: Vec<(&str, Option<&str>)>,
) {
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
    fs::write(
        &playlist_path,
        serde_json::to_string_pretty(&playlist_json).unwrap(),
    )
    .unwrap();
}

/// Helper to extract short ID from playlist show output
fn extract_short_id(output: &str) -> Option<String> {
    // Find the data line (after the header line with dashes)
    output
        .lines()
        .skip_while(|line| !line.starts_with("---"))
        .skip(1) // Skip the dashes line
        .next()
        .and_then(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            // For flattened: PLAYLIST IDX SHORT TEMPLATE LABEL -> parts[2]
            // For specific: IDX SHORT TEMPLATE LABEL -> parts[1]
            if parts.len() >= 3 && parts[0].chars().all(|c| c.is_ascii_hexdigit()) {
                // Flattened view: first column is PLAYLIST (hex)
                parts.get(2).map(|s| s.to_string())
            } else {
                // Specific playlist view
                parts.get(1).map(|s| s.to_string())
            }
        })
}

#[test]
fn test_playlist_run_dry_run_by_index() {
    let temp = TempDir::new().unwrap();

    // Create template and playlist
    create_test_template(&temp, "tpl_echo", "Echo Template");
    create_test_playlist(
        &temp,
        "pl_daily",
        "Daily Tasks",
        vec![("tpl_echo", Some("Echo Hello"))],
    );

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["playlist", "run", "pl_daily", "0", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Would run template: tpl_echo"))
        .stdout(predicate::str::contains("Echo Hello"))
        .stdout(predicate::str::contains("index 0"));
}

#[test]
fn test_playlist_run_dry_run_by_global_short_id() {
    let temp = TempDir::new().unwrap();

    // Create template and playlist
    create_test_template(&temp, "tpl_echo", "Echo Template");
    create_test_playlist(
        &temp,
        "pl_daily",
        "Daily Tasks",
        vec![("tpl_echo", Some("Echo Hello"))],
    );

    // First, get the short ID from playlist show (flattened view)
    let show_output = Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["playlist", "show"])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&show_output.stdout);

    // Extract the short ID from the flattened table output
    // Format: PLAYLIST  IDX  SHORT     TEMPLATE        LABEL
    let lines: Vec<&str> = stdout.lines().collect();
    let short_id: Option<String> = lines
        .iter()
        .skip_while(|line| !line.starts_with("---"))
        .skip(1)
        .next()
        .and_then(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            // parts[0] = PLAYLIST, parts[1] = IDX, parts[2] = SHORT
            parts.get(2).map(|s| s.to_string())
        });

    let short_id = short_id.expect(&format!(
        "Could not extract short ID from output:\n{}",
        stdout
    ));
    assert!(
        short_id.chars().all(|c| c.is_ascii_hexdigit()),
        "Short ID should be hex: {}",
        short_id
    );

    // Run using the GLOBAL short ID (one argument, not playlist + item)
    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["playlist", "run", &short_id, "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Would run template: tpl_echo"))
        .stdout(predicate::str::contains("daily")); // Should show playlist info
}

#[test]
fn test_playlist_run_invalid_index() {
    let temp = TempDir::new().unwrap();

    // Create template and playlist
    create_test_template(&temp, "tpl_echo", "Echo Template");
    create_test_playlist(
        &temp,
        "pl_daily",
        "Daily Tasks",
        vec![("tpl_echo", Some("Echo Hello"))],
    );

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["playlist", "run", "pl_daily", "99", "--dry-run"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));
}

#[test]
fn test_playlist_run_invalid_global_short_id() {
    let temp = TempDir::new().unwrap();

    // Create template and playlist
    create_test_template(&temp, "tpl_echo", "Echo Template");
    create_test_playlist(
        &temp,
        "pl_daily",
        "Daily Tasks",
        vec![("tpl_echo", Some("Echo Hello"))],
    );

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["playlist", "run", "zzzzzzzz", "--dry-run"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("No item found"));
}

#[test]
fn test_playlist_run_playlist_not_found() {
    let temp = TempDir::new().unwrap();

    // Ensure the storage directories exist but are empty
    fs::create_dir_all(temp.path().join("templates")).unwrap();
    fs::create_dir_all(temp.path().join("runs")).unwrap();
    fs::create_dir_all(temp.path().join("playlists")).unwrap();
    fs::create_dir_all(temp.path().join("logs")).unwrap();

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["playlist", "run", "nonexistent", "0", "--dry-run"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("No item found"));
}

#[test]
fn test_playlist_run_short_id_prefix_match() {
    let temp = TempDir::new().unwrap();

    // Create template and playlist
    create_test_template(&temp, "tpl_echo", "Echo Template");
    create_test_playlist(
        &temp,
        "pl_daily",
        "Daily Tasks",
        vec![("tpl_echo", Some("Echo Hello"))],
    );

    // Get the full short ID first from flattened view
    let show_output = Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["playlist", "show"])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&show_output.stdout);

    let lines: Vec<&str> = stdout.lines().collect();
    let short_id: Option<String> = lines
        .iter()
        .skip_while(|line| !line.starts_with("---"))
        .skip(1)
        .next()
        .and_then(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            parts.get(2).map(|s| s.to_string())
        });

    let short_id = short_id.expect(&format!(
        "Could not extract short ID from output:\n{}",
        stdout
    ));
    assert!(
        short_id.len() >= 4,
        "Short ID too short for prefix test: {}",
        short_id
    );

    // Use only first 4 characters (prefix match)
    let prefix = &short_id[..4];

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["playlist", "run", prefix, "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Would run template: tpl_echo"));
}

#[test]
fn test_playlist_run_multiple_playlists_global_short_id() {
    let temp = TempDir::new().unwrap();

    // Create templates
    create_test_template(&temp, "tpl_echo", "Echo Template");
    create_test_template(&temp, "tpl_backup", "Backup Template");

    // Create multiple playlists
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

    // Get the short ID of the second playlist's item
    let show_output = Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["playlist", "show"])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&show_output.stdout);

    // Find the line with "Backup Data"
    let lines: Vec<&str> = stdout.lines().collect();
    let backup_line = lines
        .iter()
        .find(|line| line.contains("Backup Data"))
        .expect("Could not find Backup Data line");

    let parts: Vec<&str> = backup_line.split_whitespace().collect();
    let short_id = parts.get(2).expect("Could not get short ID").to_string();

    // Run using global short ID
    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["playlist", "run", &short_id, "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Would run template: tpl_backup"))
        .stdout(predicate::str::contains("weekly")); // Should show it's from weekly playlist
}
