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
    fs::write(&template_path, serde_json::to_string_pretty(&template_json).unwrap()).unwrap();
}

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
fn test_playlist_run_dry_run_by_short_id() {
    let temp = TempDir::new().unwrap();

    // Create template and playlist
    create_test_template(&temp, "tpl_echo", "Echo Template");
    create_test_playlist(
        &temp,
        "pl_daily",
        "Daily Tasks",
        vec![("tpl_echo", Some("Echo Hello"))],
    );

    // First, get the short ID from playlist show
    let show_output = Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["playlist", "show", "pl_daily"])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&show_output.stdout);
    
    // Find the data line (after the header line with dashes)
    // The table format is:
    // IDX  SHORT     TEMPLATE        LABEL
    // ------------------------------------------------------------
    // 0    a1b2c3d4  echo            Echo Hello
    let lines: Vec<&str> = stdout.lines().collect();
    let short_id: Option<String> = lines.iter()
        .skip_while(|line| !line.starts_with("---"))
        .skip(1)  // Skip the dashes line
        .next()
        .and_then(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            // parts[0] = IDX, parts[1] = SHORT
            parts.get(1).map(|s| s.to_string())
        });

    let short_id = short_id.expect(&format!("Could not extract short ID from output:\n{}", stdout));
    assert!(short_id.chars().all(|c| c.is_ascii_hexdigit()), "Short ID should be hex: {}", short_id);

    // Run using the short ID
    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["playlist", "run", "pl_daily", &short_id, "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Would run template: tpl_echo"));
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
fn test_playlist_run_invalid_short_id() {
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
        .args(["playlist", "run", "pl_daily", "zzzzzzzz", "--dry-run"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));
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

    // Get the full short ID first
    let show_output = Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["playlist", "show", "pl_daily"])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&show_output.stdout);
    
    // Find the data line (after the header line with dashes)
    let lines: Vec<&str> = stdout.lines().collect();
    let short_id: Option<String> = lines.iter()
        .skip_while(|line| !line.starts_with("---"))
        .skip(1)  // Skip the dashes line
        .next()
        .and_then(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            parts.get(1).map(|s| s.to_string())
        });

    let short_id = short_id.expect(&format!("Could not extract short ID from output:\n{}", stdout));
    assert!(short_id.len() >= 4, "Short ID too short for prefix test: {}", short_id);

    // Use only first 4 characters (prefix match)
    let prefix = &short_id[..4];

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", temp.path())
        .args(["playlist", "run", "pl_daily", prefix, "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Would run template: tpl_echo"));
}
