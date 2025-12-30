use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::process::Command as StdCommand;
use tempfile::TempDir;

/// Helper to create a runbox command with RUNBOX_HOME set to temp directory
fn runbox_cmd(temp_dir: &TempDir) -> Command {
    let mut cmd = Command::cargo_bin("runbox").unwrap();
    cmd.env("RUNBOX_HOME", temp_dir.path());
    cmd
}

/// Create a minimal git repository with origin remote
fn init_git_repo(path: &std::path::Path) -> std::io::Result<()> {
    // Initialize git repo
    StdCommand::new("git")
        .current_dir(path)
        .args(["init"])
        .output()?;

    // Configure git user for commits
    StdCommand::new("git")
        .current_dir(path)
        .args(["config", "user.email", "test@example.com"])
        .output()?;

    StdCommand::new("git")
        .current_dir(path)
        .args(["config", "user.name", "Test User"])
        .output()?;

    // Create a file and commit
    fs::write(path.join("README.md"), "# Test")?;

    StdCommand::new("git")
        .current_dir(path)
        .args(["add", "."])
        .output()?;

    StdCommand::new("git")
        .current_dir(path)
        .args(["commit", "-m", "Initial commit"])
        .output()?;

    // Add origin remote (doesn't need to exist for our tests)
    StdCommand::new("git")
        .current_dir(path)
        .args(["remote", "add", "origin", "git@github.com:test/repo.git"])
        .output()?;

    Ok(())
}

/// Create a template file in storage and return its ID
fn create_template(temp: &TempDir, template_id: &str, template_json: &str) {
    let templates_dir = temp.path().join("templates");
    fs::create_dir_all(&templates_dir).unwrap();
    fs::write(templates_dir.join(format!("{}.json", template_id)), template_json).unwrap();
}

/// Template JSON without bindings (uses literal args)
fn simple_template_json(template_id: &str) -> String {
    format!(
        r#"{{
    "template_version": 0,
    "template_id": "{}",
    "name": "Simple Test Template",
    "exec": {{
        "argv": ["echo", "hello"],
        "cwd": ".",
        "env": {{}},
        "timeout_sec": 60
    }},
    "code_state": {{
        "repo_url": "git@github.com:test/repo.git"
    }}
}}"#,
        template_id
    )
}

/// Template JSON with bindings that have defaults
fn template_with_defaults_json(template_id: &str) -> String {
    format!(
        r#"{{
    "template_version": 0,
    "template_id": "{}",
    "name": "Template with Defaults",
    "exec": {{
        "argv": ["echo", "{{{{message}}}}"],
        "cwd": ".",
        "env": {{}},
        "timeout_sec": 60
    }},
    "bindings": {{
        "defaults": {{"message": "default_msg"}},
        "interactive": []
    }},
    "code_state": {{
        "repo_url": "git@github.com:test/repo.git"
    }}
}}"#,
        template_id
    )
}

/// Template JSON with required binding (no default, no interactive)
fn template_with_required_binding_json(template_id: &str) -> String {
    format!(
        r#"{{
    "template_version": 0,
    "template_id": "{}",
    "name": "Template with Required Binding",
    "exec": {{
        "argv": ["echo", "{{{{required_var}}}}"],
        "cwd": ".",
        "env": {{}},
        "timeout_sec": 60
    }},
    "code_state": {{
        "repo_url": "git@github.com:test/repo.git"
    }}
}}"#,
        template_id
    )
}

// =============================================================================
// Happy Path Tests
// =============================================================================

#[test]
fn test_run_with_simple_template_dry_run() {
    let temp = TempDir::new().unwrap();

    // Initialize git repo in current directory context
    init_git_repo(temp.path()).unwrap();

    // Create template in storage
    create_template(&temp, "tpl_simple", &simple_template_json("tpl_simple"));

    runbox_cmd(&temp)
        .current_dir(temp.path())
        .args(["run", "-t", "tpl_simple", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Dry run"))
        .stdout(predicate::str::contains("echo"))
        .stdout(predicate::str::contains("hello"));
}

#[test]
fn test_run_with_default_bindings_dry_run() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path()).unwrap();

    create_template(
        &temp,
        "tpl_defaults",
        &template_with_defaults_json("tpl_defaults"),
    );

    runbox_cmd(&temp)
        .current_dir(temp.path())
        .args(["run", "-t", "tpl_defaults", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Dry run"))
        // The default value should be substituted in the output
        .stdout(predicate::str::contains("default_msg"));
}

#[test]
fn test_run_with_provided_binding_dry_run() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path()).unwrap();

    create_template(
        &temp,
        "tpl_binding",
        &template_with_defaults_json("tpl_binding"),
    );

    runbox_cmd(&temp)
        .current_dir(temp.path())
        .args([
            "run",
            "-t",
            "tpl_binding",
            "--binding",
            "message=custom_value",
            "--dry-run",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Dry run"))
        // The provided value should override the default
        .stdout(predicate::str::contains("custom_value"));
}

#[test]
fn test_run_with_short_template_id_dry_run() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path()).unwrap();

    create_template(
        &temp,
        "tpl_abcd1234-5678-9abc-def0-123456789abc",
        &simple_template_json("tpl_abcd1234-5678-9abc-def0-123456789abc"),
    );

    // Use short ID (first 8 hex chars)
    runbox_cmd(&temp)
        .current_dir(temp.path())
        .args(["run", "-t", "abcd1234", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Dry run"));
}

#[test]
fn test_run_with_bg_runtime_dry_run() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path()).unwrap();

    create_template(&temp, "tpl_bg", &simple_template_json("tpl_bg"));

    runbox_cmd(&temp)
        .current_dir(temp.path())
        .args(["run", "-t", "tpl_bg", "--runtime", "bg", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Dry run"));
}

#[test]
fn test_run_with_background_runtime_alias_dry_run() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path()).unwrap();

    create_template(&temp, "tpl_background", &simple_template_json("tpl_background"));

    runbox_cmd(&temp)
        .current_dir(temp.path())
        .args(["run", "-t", "tpl_background", "--runtime", "background", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Dry run"));
}

#[test]
fn test_run_with_tmux_runtime_dry_run() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path()).unwrap();

    create_template(&temp, "tpl_tmux", &simple_template_json("tpl_tmux"));

    runbox_cmd(&temp)
        .current_dir(temp.path())
        .args(["run", "-t", "tpl_tmux", "--runtime", "tmux", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Dry run"));
}

// =============================================================================
// Error Path Tests
// =============================================================================

#[test]
fn test_run_template_not_found() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path()).unwrap();

    runbox_cmd(&temp)
        .current_dir(temp.path())
        .args(["run", "-t", "nonexistent"])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("not found")
                .or(predicate::str::contains("No template"))
                .or(predicate::str::contains("No item found")),
        );
}

#[test]
fn test_run_missing_required_binding() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path()).unwrap();

    create_template(
        &temp,
        "tpl_required",
        &template_with_required_binding_json("tpl_required"),
    );

    runbox_cmd(&temp)
        .current_dir(temp.path())
        .args(["run", "-t", "tpl_required", "--dry-run"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Missing binding").or(predicate::str::contains("required_var")));
}

#[test]
fn test_run_invalid_runtime() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path()).unwrap();

    create_template(&temp, "tpl_invalid_rt", &simple_template_json("tpl_invalid_rt"));

    // Clap should reject invalid runtime values
    runbox_cmd(&temp)
        .current_dir(temp.path())
        .args(["run", "-t", "tpl_invalid_rt", "--runtime", "invalid_runtime"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid").or(predicate::str::contains("possible values")));
}

#[test]
fn test_run_not_in_git_repo() {
    let temp = TempDir::new().unwrap();
    // Note: NOT initializing git repo

    create_template(&temp, "tpl_nogit", &simple_template_json("tpl_nogit"));

    runbox_cmd(&temp)
        .current_dir(temp.path())
        .args(["run", "-t", "tpl_nogit", "--dry-run"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("git").or(predicate::str::contains("repository")));
}

// =============================================================================
// Edge Cases and Additional Tests
// =============================================================================

#[test]
fn test_run_multiple_bindings_dry_run() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path()).unwrap();

    // Template with multiple variables
    let template_json = r#"{
        "template_version": 0,
        "template_id": "tpl_multi",
        "name": "Multi-binding Template",
        "exec": {
            "argv": ["echo", "{arg1}", "{arg2}"],
            "cwd": ".",
            "env": {},
            "timeout_sec": 60
        },
        "bindings": {
            "defaults": {"arg1": "default1", "arg2": "default2"},
            "interactive": []
        },
        "code_state": {
            "repo_url": "git@github.com:test/repo.git"
        }
    }"#;

    create_template(&temp, "tpl_multi", template_json);

    runbox_cmd(&temp)
        .current_dir(temp.path())
        .args([
            "run",
            "-t",
            "tpl_multi",
            "--binding",
            "arg1=value1",
            "--binding",
            "arg2=value2",
            "--dry-run",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("value1"))
        .stdout(predicate::str::contains("value2"));
}

#[test]
fn test_run_partial_binding_override_dry_run() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path()).unwrap();

    // Template with multiple variables with defaults
    let template_json = r#"{
        "template_version": 0,
        "template_id": "tpl_partial",
        "name": "Partial Override Template",
        "exec": {
            "argv": ["echo", "{a}", "{b}"],
            "cwd": ".",
            "env": {},
            "timeout_sec": 60
        },
        "bindings": {
            "defaults": {"a": "default_a", "b": "default_b"},
            "interactive": []
        },
        "code_state": {
            "repo_url": "git@github.com:test/repo.git"
        }
    }"#;

    create_template(&temp, "tpl_partial", template_json);

    // Only override one binding
    runbox_cmd(&temp)
        .current_dir(temp.path())
        .args([
            "run",
            "-t",
            "tpl_partial",
            "--binding",
            "a=overridden",
            "--dry-run",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("overridden"))
        .stdout(predicate::str::contains("default_b"));
}

#[test]
fn test_run_env_vars_in_template_dry_run() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path()).unwrap();

    // Template with environment variables
    let template_json = r#"{
        "template_version": 0,
        "template_id": "tpl_env",
        "name": "Env Template",
        "exec": {
            "argv": ["echo", "test"],
            "cwd": ".",
            "env": {"MY_VAR": "my_value", "ANOTHER": "another_value"},
            "timeout_sec": 60
        },
        "code_state": {
            "repo_url": "git@github.com:test/repo.git"
        }
    }"#;

    create_template(&temp, "tpl_env", template_json);

    runbox_cmd(&temp)
        .current_dir(temp.path())
        .args(["run", "-t", "tpl_env", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("MY_VAR"))
        .stdout(predicate::str::contains("my_value"));
}
