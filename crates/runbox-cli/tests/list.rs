//! Integration tests for `runbox list` command

use std::fs;
use std::process::Command;
use tempfile::tempdir;

fn runbox(args: &[&str], home: &std::path::Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_runbox"))
        .args(args)
        .env("RUNBOX_HOME", home)
        .output()
        .expect("failed to execute runbox")
}

fn setup_home() -> tempfile::TempDir {
    let home = tempdir().unwrap();
    fs::create_dir_all(home.path().join("templates")).unwrap();
    fs::create_dir_all(home.path().join("runs")).unwrap();
    fs::create_dir_all(home.path().join("playlists")).unwrap();
    home
}

fn create_template(home: &std::path::Path, id: &str, name: &str) {
    let template = serde_json::json!({
        "template_version": 0,
        "template_id": id,
        "name": name,
        "exec": {
            "argv": ["echo", "hello"],
            "cwd": ".",
            "env": {},
            "timeout_sec": 0
        },
        "code_state": {
            "repo_url": "git@github.com:test/repo.git"
        }
    });
    let path = home.join("templates").join(format!("{}.json", id));
    fs::write(&path, serde_json::to_string_pretty(&template).unwrap()).unwrap();
}

fn create_run(home: &std::path::Path, id: &str) {
    let run = serde_json::json!({
        "run_version": 0,
        "run_id": id,
        "exec": {
            "argv": ["echo", "test"],
            "cwd": ".",
            "env": {},
            "timeout_sec": 0
        },
        "code_state": {
            "repo_url": "git@github.com:test/repo.git",
            "base_commit": "a1b2c3d4e5f6789012345678901234567890abcd"
        },
        "status": "exited",
        "runtime": "background"
    });
    let path = home.join("runs").join(format!("{}.json", id));
    fs::write(&path, serde_json::to_string_pretty(&run).unwrap()).unwrap();
}

fn create_playlist(home: &std::path::Path, id: &str, name: &str, items: &[(&str, Option<&str>)]) {
    let items_json: Vec<serde_json::Value> = items
        .iter()
        .map(|(tpl_id, label)| {
            if let Some(lbl) = label {
                serde_json::json!({"template_id": tpl_id, "label": lbl})
            } else {
                serde_json::json!({"template_id": tpl_id})
            }
        })
        .collect();
    
    let playlist = serde_json::json!({
        "playlist_id": id,
        "name": name,
        "items": items_json
    });
    let path = home.join("playlists").join(format!("{}.json", id));
    fs::write(&path, serde_json::to_string_pretty(&playlist).unwrap()).unwrap();
}

#[test]
fn test_list_empty() {
    let home = setup_home();
    let output = runbox(&["list", "--all-repos"], home.path());
    
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("No runnables found"));
}

#[test]
fn test_list_templates_only() {
    let home = setup_home();
    create_template(home.path(), "tpl_echo", "Echo Command");
    create_template(home.path(), "tpl_train", "Train Model");
    
    let output = runbox(&["list", "--type", "template", "--all-repos"], home.path());
    
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("template"));
    assert!(stdout.contains("Echo Command"));
    assert!(stdout.contains("Train Model"));
    assert!(stdout.contains("2 runnables"));
}

#[test]
fn test_list_replays_only() {
    let home = setup_home();
    create_run(home.path(), "run_550e8400-e29b-41d4-a716-446655440000");
    create_run(home.path(), "run_a1b2c3d4-e5f6-7890-abcd-ef1234567890");
    
    let output = runbox(&["list", "--type", "replay", "--all-repos"], home.path());
    
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("replay"));
    assert!(stdout.contains("550e8400"));
    assert!(stdout.contains("a1b2c3d4"));
    assert!(stdout.contains("2 runnables"));
}

#[test]
fn test_list_playlist_items() {
    let home = setup_home();
    create_template(home.path(), "tpl_echo", "Echo Command");
    create_playlist(home.path(), "pl_daily", "Daily Tasks", &[
        ("tpl_echo", Some("Morning Echo")),
        ("tpl_echo", Some("Evening Echo")),
    ]);
    
    let output = runbox(&["list", "--type", "playlist", "--all-repos"], home.path());
    
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("playlist"));
    assert!(stdout.contains("daily[0]"));
    assert!(stdout.contains("daily[1]"));
    assert!(stdout.contains("Morning Echo"));
    assert!(stdout.contains("Evening Echo"));
    assert!(stdout.contains("2 runnables"));
}

#[test]
fn test_list_all_types() {
    let home = setup_home();
    create_template(home.path(), "tpl_echo", "Echo Command");
    create_run(home.path(), "run_550e8400-e29b-41d4-a716-446655440000");
    create_playlist(home.path(), "pl_daily", "Daily Tasks", &[
        ("tpl_echo", Some("Morning Echo")),
    ]);
    
    let output = runbox(&["list", "--all-repos"], home.path());
    
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("template"));
    assert!(stdout.contains("replay"));
    assert!(stdout.contains("playlist"));
    assert!(stdout.contains("3 runnables"));
}

#[test]
fn test_list_filter_by_playlist() {
    let home = setup_home();
    create_template(home.path(), "tpl_echo", "Echo Command");
    create_playlist(home.path(), "pl_daily", "Daily Tasks", &[
        ("tpl_echo", Some("Morning Echo")),
    ]);
    create_playlist(home.path(), "pl_weekly", "Weekly Tasks", &[
        ("tpl_echo", Some("Weekly Report")),
    ]);
    
    let output = runbox(&["list", "--playlist", "daily", "--all-repos"], home.path());
    
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Morning Echo"));
    assert!(!stdout.contains("Weekly Report"));
    assert!(stdout.contains("1 runnables"));
}

#[test]
fn test_list_json_output() {
    let home = setup_home();
    create_template(home.path(), "tpl_echo", "Echo Command");
    
    let output = runbox(&["list", "--json", "--all-repos"], home.path());
    
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    
    // Should be valid JSON
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("Should be valid JSON");
    assert!(json.is_array());
    let arr = json.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["type"], "template");
    assert_eq!(arr[0]["name"], "Echo Command");
}

#[test]
fn test_list_short_output() {
    let home = setup_home();
    create_template(home.path(), "tpl_echo", "Echo Command");
    create_template(home.path(), "tpl_train", "Train Model");
    
    let output = runbox(&["list", "--short", "--all-repos"], home.path());
    
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    
    // Should have exactly 2 lines, each with 8-char hex short ID
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines.len(), 2);
    for line in lines {
        assert_eq!(line.len(), 8);
        assert!(line.chars().all(|c| c.is_ascii_hexdigit()));
    }
}

#[test]
fn test_list_limit() {
    let home = setup_home();
    create_template(home.path(), "tpl_a", "Template A");
    create_template(home.path(), "tpl_b", "Template B");
    create_template(home.path(), "tpl_c", "Template C");
    
    let output = runbox(&["list", "--limit", "2", "--all-repos"], home.path());
    
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("2 runnables"));
}

#[test]
fn test_list_verbose_shows_repo() {
    let home = setup_home();
    create_template(home.path(), "tpl_echo", "Echo Command");
    
    let output = runbox(&["list", "--verbose", "--all-repos"], home.path());
    
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("REPO"));
    assert!(stdout.contains("test/repo.git") || stdout.contains("..."));
}

#[test]
fn test_list_invalid_type() {
    let home = setup_home();
    
    let output = runbox(&["list", "--type", "invalid", "--all-repos"], home.path());
    
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Invalid runnable type") || stderr.contains("invalid"));
}
