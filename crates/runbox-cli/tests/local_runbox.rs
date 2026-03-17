use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::Path;
use std::process::Command as StdCommand;
use tempfile::TempDir;

fn init_git_repo(path: &Path) {
    StdCommand::new("git")
        .current_dir(path)
        .args(["init"])
        .output()
        .unwrap();

    StdCommand::new("git")
        .current_dir(path)
        .args(["config", "user.email", "test@example.com"])
        .output()
        .unwrap();

    StdCommand::new("git")
        .current_dir(path)
        .args(["config", "user.name", "Test User"])
        .output()
        .unwrap();

    fs::write(path.join("README.md"), "# Test\n").unwrap();

    StdCommand::new("git")
        .current_dir(path)
        .args(["add", "."])
        .output()
        .unwrap();

    StdCommand::new("git")
        .current_dir(path)
        .args(["commit", "-m", "Initial commit"])
        .output()
        .unwrap();

    StdCommand::new("git")
        .current_dir(path)
        .args([
            "remote",
            "add",
            "origin",
            "git@github.com:test/local-runbox.git",
        ])
        .output()
        .unwrap();
}

fn create_local_template(project: &Path, template_id: &str, name: &str, argv: &[&str]) {
    let templates_dir = project.join(".runbox").join("templates");
    fs::create_dir_all(&templates_dir).unwrap();

    let template = serde_json::json!({
        "template_version": 0,
        "template_id": template_id,
        "name": name,
        "exec": {
            "argv": argv,
            "cwd": ".",
            "env": {},
            "timeout_sec": 60
        },
        "code_state": {
            "repo_url": "git@github.com:test/local-runbox.git"
        }
    });

    fs::write(
        templates_dir.join(format!("{}.json", template_id)),
        serde_json::to_string_pretty(&template).unwrap(),
    )
    .unwrap();
}

fn create_global_template(home: &Path, template_id: &str, name: &str) {
    let templates_dir = home.join("templates");
    fs::create_dir_all(&templates_dir).unwrap();

    let template = serde_json::json!({
        "template_version": 0,
        "template_id": template_id,
        "name": name,
        "exec": {
            "argv": ["echo", "global-template"],
            "cwd": ".",
            "env": {},
            "timeout_sec": 60
        },
        "code_state": {
            "repo_url": "git@github.com:test/global-runbox.git"
        }
    });

    fs::write(
        templates_dir.join(format!("{}.json", template_id)),
        serde_json::to_string_pretty(&template).unwrap(),
    )
    .unwrap();
}

fn create_global_playlist(
    home: &Path,
    playlist_id: &str,
    name: &str,
    items: &[(&str, Option<&str>)],
) {
    let playlists_dir = home.join("playlists");
    fs::create_dir_all(&playlists_dir).unwrap();

    let items_json: Vec<_> = items
        .iter()
        .map(|(template_id, label)| {
            if let Some(label) = label {
                serde_json::json!({
                    "template_id": template_id,
                    "label": label,
                })
            } else {
                serde_json::json!({
                    "template_id": template_id,
                })
            }
        })
        .collect();

    let playlist = serde_json::json!({
        "playlist_id": playlist_id,
        "name": name,
        "items": items_json,
    });

    fs::write(
        playlists_dir.join(format!("{}.json", playlist_id)),
        serde_json::to_string_pretty(&playlist).unwrap(),
    )
    .unwrap();
}

fn write_template_file(path: &Path, template_id: &str, name: &str, argv: &[&str]) {
    let template = serde_json::json!({
        "template_version": 0,
        "template_id": template_id,
        "name": name,
        "exec": {
            "argv": argv,
            "cwd": ".",
            "env": {},
            "timeout_sec": 60
        },
        "code_state": {
            "repo_url": "git@github.com:test/local-runbox.git"
        }
    });

    fs::write(path, serde_json::to_string_pretty(&template).unwrap()).unwrap();
}

fn write_playlist_file(path: &Path, playlist_id: &str, name: &str) {
    let playlist = serde_json::json!({
        "playlist_id": playlist_id,
        "name": name,
        "items": [],
    });

    fs::write(path, serde_json::to_string_pretty(&playlist).unwrap()).unwrap();
}

fn load_playlist_item_count(path: &Path) -> usize {
    let content = fs::read_to_string(path).unwrap();
    let playlist: serde_json::Value = serde_json::from_str(&content).unwrap();
    playlist["items"].as_array().unwrap().len()
}

fn create_local_playlist(
    project: &Path,
    playlist_id: &str,
    name: &str,
    items: &[(&str, Option<&str>)],
) {
    let playlists_dir = project.join(".runbox").join("playlists");
    fs::create_dir_all(&playlists_dir).unwrap();

    let items_json: Vec<_> = items
        .iter()
        .map(|(template_id, label)| {
            if let Some(label) = label {
                serde_json::json!({
                    "template_id": template_id,
                    "label": label,
                })
            } else {
                serde_json::json!({
                    "template_id": template_id,
                })
            }
        })
        .collect();

    let playlist = serde_json::json!({
        "playlist_id": playlist_id,
        "name": name,
        "items": items_json,
    });

    fs::write(
        playlists_dir.join(format!("{}.json", playlist_id)),
        serde_json::to_string_pretty(&playlist).unwrap(),
    )
    .unwrap();
}

#[test]
fn test_template_list_detects_local_runbox_templates() {
    let home = TempDir::new().unwrap();
    let project = TempDir::new().unwrap();
    let nested = project.path().join("nested").join("deeper");
    fs::create_dir_all(&nested).unwrap();

    create_local_template(
        project.path(),
        "tpl_abcd1234-5678-90ab-cdef12345678",
        "Local Template",
        &["echo", "from-local-template-list"],
    );

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", home.path())
        .current_dir(&nested)
        .args(["template", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Local Template"))
        .stdout(predicate::str::contains("abcd1234"));
}

#[test]
fn test_run_dry_run_uses_local_runbox_template() {
    let home = TempDir::new().unwrap();
    let project = TempDir::new().unwrap();
    init_git_repo(project.path());

    create_local_template(
        project.path(),
        "tpl_abcd1234-5678-90ab-cdef12345678",
        "Local Dry Run Template",
        &["echo", "from-local-run"],
    );

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", home.path())
        .current_dir(project.path())
        .args(["run", "-t", "abcd1234", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Dry run"))
        .stdout(predicate::str::contains("from-local-run"));
}

#[test]
fn test_run_unified_dry_run_uses_local_runbox_template() {
    let home = TempDir::new().unwrap();
    let project = TempDir::new().unwrap();
    init_git_repo(project.path());

    create_local_template(
        project.path(),
        "tpl_abcd1234-5678-90ab-cdef12345678",
        "Local Unified Template",
        &["echo", "from-local-unified-run"],
    );

    let template_short =
        runbox_core::Runnable::Template("tpl_abcd1234-5678-90ab-cdef12345678".to_string())
            .short_id();

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", home.path())
        .current_dir(project.path())
        .args(["run", &template_short, "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Local Unified Template"))
        .stdout(predicate::str::contains("from-local-unified-run"));
}

#[test]
fn test_template_create_defaults_to_global_scope_even_with_local_runbox() {
    let home = TempDir::new().unwrap();
    let project = TempDir::new().unwrap();
    fs::create_dir_all(project.path().join(".runbox")).unwrap();
    let template_file = project.path().join("template.json");
    write_template_file(
        &template_file,
        "tpl_scope-default-1234",
        "Scoped Template",
        &["echo", "scoped-default"],
    );

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", home.path())
        .current_dir(project.path())
        .args(["template", "create", template_file.to_str().unwrap()])
        .assert()
        .success();

    assert!(home
        .path()
        .join("templates")
        .join("tpl_scope-default-1234.json")
        .exists());
    assert!(!project
        .path()
        .join(".runbox")
        .join("templates")
        .join("tpl_scope-default-1234.json")
        .exists());
}

#[test]
fn test_template_create_local_flag_writes_local_runbox() {
    let home = TempDir::new().unwrap();
    let project = TempDir::new().unwrap();
    fs::create_dir_all(project.path().join(".runbox")).unwrap();
    let template_file = project.path().join("template.json");
    write_template_file(
        &template_file,
        "tpl_scope-local-1234",
        "Scoped Local Template",
        &["echo", "scoped-local"],
    );

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", home.path())
        .current_dir(project.path())
        .args([
            "template",
            "create",
            template_file.to_str().unwrap(),
            "--local",
        ])
        .assert()
        .success();

    assert!(project
        .path()
        .join(".runbox")
        .join("templates")
        .join("tpl_scope-local-1234.json")
        .exists());
    assert!(!home
        .path()
        .join("templates")
        .join("tpl_scope-local-1234.json")
        .exists());
}

#[test]
fn test_template_delete_defaults_to_global_scope_with_local_shadow() {
    let home = TempDir::new().unwrap();
    let project = TempDir::new().unwrap();
    fs::create_dir_all(project.path().join(".runbox")).unwrap();

    create_global_template(home.path(), "tpl_shadowed-1234", "Global Shadowed Template");
    create_local_template(
        project.path(),
        "tpl_shadowed-1234",
        "Local Shadowed Template",
        &["echo", "local-shadowed"],
    );

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", home.path())
        .current_dir(project.path())
        .args(["template", "delete", "tpl_shadowed-1234"])
        .assert()
        .success();

    assert!(!home
        .path()
        .join("templates")
        .join("tpl_shadowed-1234.json")
        .exists());
    assert!(project
        .path()
        .join(".runbox")
        .join("templates")
        .join("tpl_shadowed-1234.json")
        .exists());
}

#[test]
fn test_template_delete_local_flag_targets_local_scope() {
    let home = TempDir::new().unwrap();
    let project = TempDir::new().unwrap();
    fs::create_dir_all(project.path().join(".runbox")).unwrap();

    create_global_template(
        home.path(),
        "tpl_shadowed-local-1234",
        "Global Shadowed Template",
    );
    create_local_template(
        project.path(),
        "tpl_shadowed-local-1234",
        "Local Shadowed Template",
        &["echo", "local-shadowed"],
    );

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", home.path())
        .current_dir(project.path())
        .args(["template", "delete", "tpl_shadowed-local-1234", "--local"])
        .assert()
        .success();

    assert!(home
        .path()
        .join("templates")
        .join("tpl_shadowed-local-1234.json")
        .exists());
    assert!(!project
        .path()
        .join(".runbox")
        .join("templates")
        .join("tpl_shadowed-local-1234.json")
        .exists());
}

#[test]
fn test_playlist_show_detects_local_runbox_playlists() {
    let home = TempDir::new().unwrap();
    let project = TempDir::new().unwrap();
    let nested = project.path().join("nested");
    fs::create_dir_all(&nested).unwrap();

    create_local_template(
        project.path(),
        "tpl_local-template",
        "Local Playlist Template",
        &["echo", "playlist-local-template"],
    );
    create_local_playlist(
        project.path(),
        "pl_abcd1234-5678-90ab-cdef12345678",
        "Local Playlist",
        &[("tpl_local-template", Some("Local Playlist Item"))],
    );

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", home.path())
        .current_dir(&nested)
        .args(["playlist", "show", "abcd1234"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Local Playlist"))
        .stdout(predicate::str::contains("Local Playlist Item"));
}

#[test]
fn test_playlist_create_defaults_to_global_scope_even_with_local_runbox() {
    let home = TempDir::new().unwrap();
    let project = TempDir::new().unwrap();
    fs::create_dir_all(project.path().join(".runbox")).unwrap();
    let playlist_file = project.path().join("playlist.json");
    write_playlist_file(&playlist_file, "pl_scope-default-1234", "Scoped Playlist");

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", home.path())
        .current_dir(project.path())
        .args(["playlist", "create", playlist_file.to_str().unwrap()])
        .assert()
        .success();

    assert!(home
        .path()
        .join("playlists")
        .join("pl_scope-default-1234.json")
        .exists());
    assert!(!project
        .path()
        .join(".runbox")
        .join("playlists")
        .join("pl_scope-default-1234.json")
        .exists());
}

#[test]
fn test_playlist_create_local_flag_writes_local_runbox() {
    let home = TempDir::new().unwrap();
    let project = TempDir::new().unwrap();
    fs::create_dir_all(project.path().join(".runbox")).unwrap();
    let playlist_file = project.path().join("playlist.json");
    write_playlist_file(
        &playlist_file,
        "pl_scope-local-1234",
        "Scoped Local Playlist",
    );

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", home.path())
        .current_dir(project.path())
        .args([
            "playlist",
            "create",
            playlist_file.to_str().unwrap(),
            "--local",
        ])
        .assert()
        .success();

    assert!(project
        .path()
        .join(".runbox")
        .join("playlists")
        .join("pl_scope-local-1234.json")
        .exists());
    assert!(!home
        .path()
        .join("playlists")
        .join("pl_scope-local-1234.json")
        .exists());
}

#[test]
fn test_playlist_run_dry_run_uses_local_runbox_playlist() {
    let home = TempDir::new().unwrap();
    let project = TempDir::new().unwrap();
    init_git_repo(project.path());

    create_local_template(
        project.path(),
        "tpl_local-template",
        "Local Playlist Template",
        &["echo", "playlist-local-template"],
    );
    create_local_playlist(
        project.path(),
        "pl_abcd1234-5678-90ab-cdef12345678",
        "Local Playlist",
        &[("tpl_local-template", Some("Local Playlist Item"))],
    );

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", home.path())
        .current_dir(project.path())
        .args(["playlist", "run", "abcd1234", "0", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Would run template: tpl_local-template",
        ))
        .stdout(predicate::str::contains("Local Playlist"))
        .stdout(predicate::str::contains("Local Playlist Item"));
}

#[test]
fn test_playlist_add_local_flag_updates_local_playlist_only() {
    let home = TempDir::new().unwrap();
    let project = TempDir::new().unwrap();
    fs::create_dir_all(project.path().join(".runbox")).unwrap();

    create_global_template(home.path(), "tpl_shared-1234", "Shared Template");
    create_global_playlist(home.path(), "pl_shared-1234", "Global Playlist", &[]);
    create_local_playlist(project.path(), "pl_shared-1234", "Local Playlist", &[]);

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", home.path())
        .current_dir(project.path())
        .args([
            "playlist",
            "add",
            "pl_shared-1234",
            "tpl_shared-1234",
            "--local",
        ])
        .assert()
        .success();

    assert_eq!(
        load_playlist_item_count(
            &project
                .path()
                .join(".runbox")
                .join("playlists")
                .join("pl_shared-1234.json"),
        ),
        1
    );
    assert_eq!(
        load_playlist_item_count(&home.path().join("playlists").join("pl_shared-1234.json")),
        0
    );
}

#[test]
fn test_playlist_remove_defaults_to_global_scope_with_local_shadow() {
    let home = TempDir::new().unwrap();
    let project = TempDir::new().unwrap();
    fs::create_dir_all(project.path().join(".runbox")).unwrap();

    create_global_template(home.path(), "tpl_shared-remove-1234", "Shared Template");
    create_global_playlist(
        home.path(),
        "pl_shared-remove-1234",
        "Global Playlist",
        &[("tpl_shared-remove-1234", Some("Global Item"))],
    );
    create_local_playlist(
        project.path(),
        "pl_shared-remove-1234",
        "Local Playlist",
        &[("tpl_shared-remove-1234", Some("Local Item"))],
    );

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", home.path())
        .current_dir(project.path())
        .args([
            "playlist",
            "remove",
            "pl_shared-remove-1234",
            "tpl_shared-remove-1234",
        ])
        .assert()
        .success();

    assert_eq!(
        load_playlist_item_count(
            &home
                .path()
                .join("playlists")
                .join("pl_shared-remove-1234.json")
        ),
        0
    );
    assert_eq!(
        load_playlist_item_count(
            &project
                .path()
                .join(".runbox")
                .join("playlists")
                .join("pl_shared-remove-1234.json"),
        ),
        1
    );
}

#[test]
fn test_list_local_filter_only_shows_local_runbox_items() {
    let home = TempDir::new().unwrap();
    let project = TempDir::new().unwrap();

    create_global_template(
        home.path(),
        "tpl_global1234-5678-90ab-cdef12345678",
        "Global Template",
    );
    create_local_template(
        project.path(),
        "tpl_local1234-5678-90ab-cdef12345678",
        "Local Template",
        &["echo", "local-template-only"],
    );

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", home.path())
        .current_dir(project.path())
        .args(["list", "--all-repos", "--local"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Local Template"))
        .stdout(predicate::str::contains("1 runnables"))
        .stdout(predicate::str::contains("Global Template").not());
}

#[test]
fn test_list_where_detects_local_runbox_templates() {
    let home = TempDir::new().unwrap();
    let project = TempDir::new().unwrap();
    let nested = project.path().join("nested");
    fs::create_dir_all(&nested).unwrap();

    create_local_template(
        project.path(),
        "tpl_where-1234-5678-90ab-cdef12345678",
        "Where Local Template",
        &["echo", "from-where-local"],
    );

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", home.path())
        .current_dir(&nested)
        .args([
            "list",
            "--where-clause",
            "name = 'Where Local Template'",
            "--all-repos",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Where Local Template"));
}

#[test]
fn test_list_where_local_filter_only_shows_local_items() {
    let home = TempDir::new().unwrap();
    let project = TempDir::new().unwrap();

    create_global_template(
        home.path(),
        "tpl_where-global-1234",
        "Where Global Template",
    );
    create_local_template(
        project.path(),
        "tpl_where-local-1234",
        "Where Local Template",
        &["echo", "from-where-local"],
    );

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", home.path())
        .current_dir(project.path())
        .args(["list", "--where-clause", "1 = 1", "--all-repos", "--local"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Where Local Template"))
        .stdout(predicate::str::contains("Where Global Template").not());
}

#[test]
fn test_query_sql_can_see_local_scope() {
    let home = TempDir::new().unwrap();
    let project = TempDir::new().unwrap();
    let nested = project.path().join("nested");
    fs::create_dir_all(&nested).unwrap();

    create_local_template(
        project.path(),
        "tpl_query-local-1234",
        "Query Local Template",
        &["echo", "from-query-local"],
    );

    Command::cargo_bin("runbox")
        .unwrap()
        .env("RUNBOX_HOME", home.path())
        .current_dir(&nested)
        .args([
            "query",
            "SELECT id, scope FROM file_index WHERE scope = 'local'",
            "--json",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("tpl_query-local-1234"))
        .stdout(predicate::str::contains("\"scope\": \"local\""));
}
