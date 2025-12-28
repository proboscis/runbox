use crate::{Playlist, Run, RunStatus, RunTemplate};
use anyhow::{bail, Context, Result};
use chrono::Utc;
use std::fs;
use std::path::PathBuf;

/// Normalize an ID for matching by removing prefix and hyphens, converting to lowercase
fn normalize_for_match(id: &str) -> String {
    id.trim_start_matches("run_")
        .trim_start_matches("tpl_")
        .trim_start_matches("pl_")
        .replace('-', "")
        .to_lowercase()
}

/// Convert a full ID to a short display format (first 8 hex characters)
pub fn short_id(full_id: &str) -> String {
    let hex = normalize_for_match(full_id);
    hex.chars().take(8).collect()
}

/// Storage for runs, templates, and playlists
pub struct Storage {
    base_dir: PathBuf,
}

impl Storage {
    /// Create a new Storage with XDG data directory
    pub fn new() -> Result<Self> {
        let base_dir = dirs::data_dir()
            .context("Could not find data directory")?
            .join("runbox");
        Self::with_base_dir(base_dir)
    }

    /// Create a new Storage with custom base directory
    pub fn with_base_dir(base_dir: PathBuf) -> Result<Self> {
        // Create directories if they don't exist
        fs::create_dir_all(base_dir.join("runs"))?;
        fs::create_dir_all(base_dir.join("templates"))?;
        fs::create_dir_all(base_dir.join("playlists"))?;
        fs::create_dir_all(base_dir.join("logs"))?;

        Ok(Self { base_dir })
    }

    /// Get the base directory
    pub fn base_dir(&self) -> &PathBuf {
        &self.base_dir
    }

    // === Run operations ===

    /// Save a run
    pub fn save_run(&self, run: &Run) -> Result<PathBuf> {
        let path = self.base_dir.join("runs").join(format!("{}.json", run.run_id));
        let json = serde_json::to_string_pretty(run)?;
        fs::write(&path, json)?;
        Ok(path)
    }

    /// Load a run by ID
    pub fn load_run(&self, run_id: &str) -> Result<Run> {
        let path = self.base_dir.join("runs").join(format!("{}.json", run_id));
        let json = fs::read_to_string(&path)
            .with_context(|| format!("Run not found: {}", run_id))?;
        let run: Run = serde_json::from_str(&json)?;
        Ok(run)
    }

    /// List all runs, sorted by modification time (newest first)
    pub fn list_runs(&self, limit: usize) -> Result<Vec<Run>> {
        let runs_dir = self.base_dir.join("runs");
        let mut entries: Vec<_> = fs::read_dir(&runs_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .map(|ext| ext == "json")
                    .unwrap_or(false)
            })
            .collect();

        // Sort by modification time (newest first)
        entries.sort_by(|a, b| {
            let a_time = a.metadata().and_then(|m| m.modified()).ok();
            let b_time = b.metadata().and_then(|m| m.modified()).ok();
            b_time.cmp(&a_time)
        });

        let runs: Vec<Run> = entries
            .into_iter()
            .take(limit)
            .filter_map(|e| fs::read_to_string(e.path()).ok())
            .filter_map(|json| serde_json::from_str(&json).ok())
            .collect();

        Ok(runs)
    }

    /// Delete a run by ID
    pub fn delete_run(&self, run_id: &str) -> Result<()> {
        let path = self.base_dir.join("runs").join(format!("{}.json", run_id));
        fs::remove_file(&path).with_context(|| format!("Run not found: {}", run_id))?;
        Ok(())
    }

    /// Transition run status to Running (atomic, CAS-style)
    ///
    /// Only updates if the current status is Pending. This prevents overwriting
    /// terminal states (Exited, Failed, Killed, Unknown) that might have been
    /// set by the exit-watching thread or reconcile.
    ///
    /// Returns Ok(true) if status was updated, Ok(false) if skipped due to CAS.
    pub fn transition_to_running(&self, run: &mut Run) -> Result<bool> {
        // Re-read current state
        let current = self.load_run(&run.run_id)?;

        // Only transition from Pending to Running
        if current.status != RunStatus::Pending {
            // Status already changed - don't overwrite
            // Update local copy with current state
            *run = current;
            return Ok(false);
        }

        // Safe to update
        run.status = RunStatus::Running;
        run.timeline.started_at = Some(Utc::now());
        self.save_run(run)?;
        Ok(true)
    }

    /// Get the log path for a run
    pub fn log_path(&self, run_id: &str) -> PathBuf {
        self.base_dir.join("logs").join(format!("{}.log", run_id))
    }

    /// Get the logs directory
    pub fn logs_dir(&self) -> PathBuf {
        self.base_dir.join("logs")
    }

    // === Template operations ===

    /// Save a template
    pub fn save_template(&self, template: &RunTemplate) -> Result<PathBuf> {
        let path = self
            .base_dir
            .join("templates")
            .join(format!("{}.json", template.template_id));
        let json = serde_json::to_string_pretty(template)?;
        fs::write(&path, json)?;
        Ok(path)
    }

    /// Load a template by ID
    pub fn load_template(&self, template_id: &str) -> Result<RunTemplate> {
        let path = self
            .base_dir
            .join("templates")
            .join(format!("{}.json", template_id));
        let json = fs::read_to_string(&path)
            .with_context(|| format!("Template not found: {}", template_id))?;
        let template: RunTemplate = serde_json::from_str(&json)?;
        Ok(template)
    }

    /// List all templates
    pub fn list_templates(&self) -> Result<Vec<RunTemplate>> {
        let templates_dir = self.base_dir.join("templates");
        let templates: Vec<RunTemplate> = fs::read_dir(&templates_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .map(|ext| ext == "json")
                    .unwrap_or(false)
            })
            .filter_map(|e| fs::read_to_string(e.path()).ok())
            .filter_map(|json| serde_json::from_str(&json).ok())
            .collect();
        Ok(templates)
    }

    /// Delete a template by ID
    pub fn delete_template(&self, template_id: &str) -> Result<()> {
        let path = self
            .base_dir
            .join("templates")
            .join(format!("{}.json", template_id));
        fs::remove_file(&path).with_context(|| format!("Template not found: {}", template_id))?;
        Ok(())
    }

    // === Playlist operations ===

    /// Save a playlist
    pub fn save_playlist(&self, playlist: &Playlist) -> Result<PathBuf> {
        let path = self
            .base_dir
            .join("playlists")
            .join(format!("{}.json", playlist.playlist_id));
        let json = serde_json::to_string_pretty(playlist)?;
        fs::write(&path, json)?;
        Ok(path)
    }

    /// Load a playlist by ID
    pub fn load_playlist(&self, playlist_id: &str) -> Result<Playlist> {
        let path = self
            .base_dir
            .join("playlists")
            .join(format!("{}.json", playlist_id));
        let json = fs::read_to_string(&path)
            .with_context(|| format!("Playlist not found: {}", playlist_id))?;
        let playlist: Playlist = serde_json::from_str(&json)?;
        Ok(playlist)
    }

    /// List all playlists
    pub fn list_playlists(&self) -> Result<Vec<Playlist>> {
        let playlists_dir = self.base_dir.join("playlists");
        let playlists: Vec<Playlist> = fs::read_dir(&playlists_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .map(|ext| ext == "json")
                    .unwrap_or(false)
            })
            .filter_map(|e| fs::read_to_string(e.path()).ok())
            .filter_map(|json| serde_json::from_str(&json).ok())
            .collect();
        Ok(playlists)
    }

    /// Delete a playlist by ID
    pub fn delete_playlist(&self, playlist_id: &str) -> Result<()> {
        let path = self
            .base_dir
            .join("playlists")
            .join(format!("{}.json", playlist_id));
        fs::remove_file(&path).with_context(|| format!("Playlist not found: {}", playlist_id))?;
        Ok(())
    }

    // === ID Resolution ===

    /// Resolve a run ID (supports short IDs)
    pub fn resolve_run_id(&self, input: &str) -> Result<String> {
        let runs = self.list_runs(usize::MAX)?;
        resolve_id_from_items(&runs, input, |r| &r.run_id)
    }

    /// Resolve a template ID (supports short IDs)
    pub fn resolve_template_id(&self, input: &str) -> Result<String> {
        let templates = self.list_templates()?;
        resolve_id_from_items(&templates, input, |t| &t.template_id)
    }

    /// Resolve a playlist ID (supports short IDs)
    pub fn resolve_playlist_id(&self, input: &str) -> Result<String> {
        let playlists = self.list_playlists()?;
        resolve_id_from_items(&playlists, input, |p| &p.playlist_id)
    }
}

/// Update run status when process exits
///
/// Uses CAS-style update: only updates if status is Running.
/// This prevents race conditions where a run is manually killed
/// but the exit-watcher thread still tries to update the status.
///
/// Note: For fast-exiting processes, the CLI must use `Storage::transition_to_running()`
/// instead of directly setting status to Running to prevent overwriting exit status.
pub fn update_run_on_exit(run_id: &str, exit_code: i32) -> Result<()> {
    let storage = Storage::new()?;
    update_run_on_exit_with_storage(&storage, run_id, exit_code)
}

/// Update run status when process exits (with custom storage)
///
/// Uses CAS-style update: only updates if status is Running.
/// This variant accepts a Storage instance for testability.
///
/// The CAS includes a re-read verification before save to minimize race windows.
/// If a concurrent process (like cmd_stop) changes status between our read and
/// save, the verification will detect this and abort the write.
///
/// For fast-exiting processes where the exit-watcher might run before the CLI
/// sets status to Running, the CLI should use `Storage::transition_to_running()`
/// which re-reads state before writing and prevents overwriting terminal states.
pub fn update_run_on_exit_with_storage(
    storage: &Storage,
    run_id: &str,
    exit_code: i32,
) -> Result<()> {
    let run = storage.load_run(run_id)?;

    // CAS: Only update if currently Running
    // If status is still Pending, the CLI hasn't transitioned yet, and will
    // call transition_to_running which will detect the terminal state.
    // If status is Killed/Unknown/Exited/Failed, another process already updated.
    if run.status != RunStatus::Running {
        return Ok(());
    }

    // Prepare the update
    let mut updated_run = run.clone();
    updated_run.status = if exit_code == 0 {
        RunStatus::Exited
    } else {
        RunStatus::Failed
    };
    updated_run.exit_code = Some(exit_code);

    // Don't overwrite ended_at if already set
    if updated_run.timeline.ended_at.is_none() {
        updated_run.timeline.ended_at = Some(Utc::now());
    }

    // Re-read to verify status hasn't changed (reduces race window)
    let current = storage.load_run(run_id)?;
    if current.status != RunStatus::Running {
        // Status changed concurrently (e.g., by cmd_stop setting Killed)
        // Don't overwrite - the other operation takes precedence
        return Ok(());
    }

    storage.save_run(&updated_run)?;
    Ok(())
}

/// Generic ID resolution from a list of items
fn resolve_id_from_items<T, F>(items: &[T], input: &str, get_id: F) -> Result<String>
where
    F: Fn(&T) -> &str,
{
    // Check for exact match first
    for item in items {
        if get_id(item) == input {
            return Ok(get_id(item).to_string());
        }
    }

    // Normalize input for prefix matching
    let normalized_input = normalize_for_match(input);

    // Find prefix matches
    let matches: Vec<&T> = items
        .iter()
        .filter(|item| {
            let id_normalized = normalize_for_match(get_id(item));
            id_normalized.starts_with(&normalized_input)
        })
        .collect();

    match matches.len() {
        0 => bail!("No item found matching '{}'", input),
        1 => Ok(get_id(matches[0]).to_string()),
        n => {
            let candidates: Vec<String> = matches
                .iter()
                .map(|m| format!("  - {}", short_id(get_id(m))))
                .collect();
            bail!(
                "Ambiguous: {} items match '{}'. Use more characters.\n{}",
                n,
                input,
                candidates.join("\n")
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CodeState, Exec, RunStatus};
    use chrono::Utc;
    use std::collections::HashMap;
    use tempfile::tempdir;

    #[test]
    fn test_run_storage() {
        let dir = tempdir().unwrap();
        let storage = Storage::with_base_dir(dir.path().to_path_buf()).unwrap();

        let run = Run::new(
            Exec {
                argv: vec!["echo".to_string(), "hello".to_string()],
                cwd: ".".to_string(),
                env: HashMap::new(),
                timeout_sec: 0,
            },
            CodeState {
                repo_url: "git@github.com:org/repo.git".to_string(),
                base_commit: "a1b2c3d4e5f6789012345678901234567890abcd".to_string(),
                patch: None,
            },
        );

        // Save
        storage.save_run(&run).unwrap();

        // Load
        let loaded = storage.load_run(&run.run_id).unwrap();
        assert_eq!(loaded.run_id, run.run_id);

        // List
        let runs = storage.list_runs(10).unwrap();
        assert_eq!(runs.len(), 1);

        // Delete
        storage.delete_run(&run.run_id).unwrap();
        let runs = storage.list_runs(10).unwrap();
        assert_eq!(runs.len(), 0);
    }

    #[test]
    fn test_short_id() {
        // Test with run_id prefix
        assert_eq!(short_id("run_550e8400-e29b-41d4-a716-446655440000"), "550e8400");
        // Test with template_id prefix
        assert_eq!(short_id("tpl_a1b2c3d4-e5f6-7890-abcd-ef1234567890"), "a1b2c3d4");
        // Test with playlist_id prefix
        assert_eq!(short_id("pl_def45678-90ab-cdef-1234-567890abcdef"), "def45678");
        // Test without prefix (should still work)
        assert_eq!(short_id("550e8400-e29b-41d4-a716-446655440000"), "550e8400");
        // Test short input
        assert_eq!(short_id("run_abc"), "abc");
    }

    #[test]
    fn test_normalize_for_match() {
        // Test with prefixes
        assert_eq!(normalize_for_match("run_550e8400-e29b"), "550e8400e29b");
        assert_eq!(normalize_for_match("tpl_A1B2C3D4"), "a1b2c3d4");
        assert_eq!(normalize_for_match("pl_DEF-456"), "def456");
        // Test without prefix
        assert_eq!(normalize_for_match("550e8400"), "550e8400");
        // Test uppercase
        assert_eq!(normalize_for_match("550E8400"), "550e8400");
    }

    #[test]
    fn test_resolve_run_id() {
        let dir = tempdir().unwrap();
        let storage = Storage::with_base_dir(dir.path().to_path_buf()).unwrap();

        // Create two runs with different IDs
        let mut run1 = Run::new(
            Exec {
                argv: vec!["echo".to_string(), "hello".to_string()],
                cwd: ".".to_string(),
                env: HashMap::new(),
                timeout_sec: 0,
            },
            CodeState {
                repo_url: "git@github.com:org/repo.git".to_string(),
                base_commit: "a1b2c3d4e5f6789012345678901234567890abcd".to_string(),
                patch: None,
            },
        );
        run1.run_id = "run_550e8400-e29b-41d4-a716-446655440000".to_string();

        let mut run2 = Run::new(
            Exec {
                argv: vec!["echo".to_string(), "world".to_string()],
                cwd: ".".to_string(),
                env: HashMap::new(),
                timeout_sec: 0,
            },
            CodeState {
                repo_url: "git@github.com:org/repo.git".to_string(),
                base_commit: "b2c3d4e5f6789012345678901234567890abcdef".to_string(),
                patch: None,
            },
        );
        run2.run_id = "run_a1b2c3d4-e5f6-7890-abcd-ef1234567890".to_string();

        storage.save_run(&run1).unwrap();
        storage.save_run(&run2).unwrap();

        // Test exact match
        let resolved = storage.resolve_run_id(&run1.run_id).unwrap();
        assert_eq!(resolved, run1.run_id);

        // Test short prefix match
        let resolved = storage.resolve_run_id("550e").unwrap();
        assert_eq!(resolved, run1.run_id);

        // Test prefix without "run_"
        let resolved = storage.resolve_run_id("a1b2").unwrap();
        assert_eq!(resolved, run2.run_id);

        // Test not found
        let result = storage.resolve_run_id("xyz");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No item found"));
    }

    #[test]
    fn test_resolve_ambiguous_id() {
        let dir = tempdir().unwrap();
        let storage = Storage::with_base_dir(dir.path().to_path_buf()).unwrap();

        // Create two runs with similar prefixes
        let mut run1 = Run::new(
            Exec {
                argv: vec!["echo".to_string(), "hello".to_string()],
                cwd: ".".to_string(),
                env: HashMap::new(),
                timeout_sec: 0,
            },
            CodeState {
                repo_url: "git@github.com:org/repo.git".to_string(),
                base_commit: "a1b2c3d4e5f6789012345678901234567890abcd".to_string(),
                patch: None,
            },
        );
        run1.run_id = "run_5aaa0000-e29b-41d4-a716-446655440000".to_string();

        let mut run2 = Run::new(
            Exec {
                argv: vec!["echo".to_string(), "world".to_string()],
                cwd: ".".to_string(),
                env: HashMap::new(),
                timeout_sec: 0,
            },
            CodeState {
                repo_url: "git@github.com:org/repo.git".to_string(),
                base_commit: "b2c3d4e5f6789012345678901234567890abcdef".to_string(),
                patch: None,
            },
        );
        run2.run_id = "run_5bbb0000-e5f6-7890-abcd-ef1234567890".to_string();

        storage.save_run(&run1).unwrap();
        storage.save_run(&run2).unwrap();

        // Test ambiguous prefix - should fail with helpful error
        let result = storage.resolve_run_id("5");
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Ambiguous"));
        assert!(err_msg.contains("2 items match"));
        assert!(err_msg.contains("5aaa0000"));
        assert!(err_msg.contains("5bbb0000"));

        // More specific prefix should work
        let resolved = storage.resolve_run_id("5aaa").unwrap();
        assert_eq!(resolved, run1.run_id);
    }

    // === Tests for update_run_on_exit ===

    fn create_test_run(storage: &Storage, run_id: &str, status: RunStatus) -> Run {
        let mut run = Run::new(
            Exec {
                argv: vec!["echo".to_string(), "test".to_string()],
                cwd: ".".to_string(),
                env: HashMap::new(),
                timeout_sec: 0,
            },
            CodeState {
                repo_url: "git@github.com:org/repo.git".to_string(),
                base_commit: "a1b2c3d4e5f6789012345678901234567890abcd".to_string(),
                patch: None,
            },
        );
        run.run_id = run_id.to_string();
        run.status = status;
        storage.save_run(&run).unwrap();
        run
    }

    #[test]
    fn test_update_run_on_exit_success() {
        let dir = tempdir().unwrap();
        let storage = Storage::with_base_dir(dir.path().to_path_buf()).unwrap();

        // Create a run in Running status
        let run_id = "run_test-success-0000-0000-000000000000";
        create_test_run(&storage, run_id, RunStatus::Running);

        // Update with exit code 0 (success)
        update_run_on_exit_with_storage(&storage, run_id, 0).unwrap();

        // Verify status changed to Exited
        let updated = storage.load_run(run_id).unwrap();
        assert_eq!(updated.status, RunStatus::Exited);
        assert_eq!(updated.exit_code, Some(0));
        assert!(updated.timeline.ended_at.is_some());
    }

    #[test]
    fn test_update_run_on_exit_failure() {
        let dir = tempdir().unwrap();
        let storage = Storage::with_base_dir(dir.path().to_path_buf()).unwrap();

        // Create a run in Running status
        let run_id = "run_test-failure-0000-0000-000000000000";
        create_test_run(&storage, run_id, RunStatus::Running);

        // Update with exit code 1 (failure)
        update_run_on_exit_with_storage(&storage, run_id, 1).unwrap();

        // Verify status changed to Failed
        let updated = storage.load_run(run_id).unwrap();
        assert_eq!(updated.status, RunStatus::Failed);
        assert_eq!(updated.exit_code, Some(1));
        assert!(updated.timeline.ended_at.is_some());
    }

    #[test]
    fn test_update_run_on_exit_cas_pending() {
        let dir = tempdir().unwrap();
        let storage = Storage::with_base_dir(dir.path().to_path_buf()).unwrap();

        // Create a run in Pending status
        let run_id = "run_test-pending-0000-0000-000000000000";
        create_test_run(&storage, run_id, RunStatus::Pending);

        // Try to update - should be a no-op due to CAS (only updates from Running)
        update_run_on_exit_with_storage(&storage, run_id, 0).unwrap();

        // Verify status is still Pending (not changed)
        // For fast-exit handling, CLI should use transition_to_running()
        let updated = storage.load_run(run_id).unwrap();
        assert_eq!(updated.status, RunStatus::Pending);
        assert_eq!(updated.exit_code, None);
    }

    #[test]
    fn test_update_run_on_exit_cas_killed() {
        let dir = tempdir().unwrap();
        let storage = Storage::with_base_dir(dir.path().to_path_buf()).unwrap();

        // Create a run that's already Killed
        let run_id = "run_test-killed-0000-0000-000000000000";
        create_test_run(&storage, run_id, RunStatus::Killed);

        // Try to update - should be a no-op due to CAS
        update_run_on_exit_with_storage(&storage, run_id, 0).unwrap();

        // Verify status is still Killed (not changed to Exited)
        let updated = storage.load_run(run_id).unwrap();
        assert_eq!(updated.status, RunStatus::Killed);
        assert_eq!(updated.exit_code, None); // Should not be set
    }

    #[test]
    fn test_update_run_on_exit_cas_unknown() {
        let dir = tempdir().unwrap();
        let storage = Storage::with_base_dir(dir.path().to_path_buf()).unwrap();

        // Create a run that's Unknown
        let run_id = "run_test-unknown-0000-0000-000000000000";
        create_test_run(&storage, run_id, RunStatus::Unknown);

        // Try to update - should be a no-op due to CAS
        update_run_on_exit_with_storage(&storage, run_id, 0).unwrap();

        // Verify status is still Unknown
        let updated = storage.load_run(run_id).unwrap();
        assert_eq!(updated.status, RunStatus::Unknown);
    }

    #[test]
    fn test_update_run_on_exit_preserves_ended_at() {
        let dir = tempdir().unwrap();
        let storage = Storage::with_base_dir(dir.path().to_path_buf()).unwrap();

        // Create a run with ended_at already set
        let run_id = "run_test-endtime-0000-0000-000000000000";
        let original_ended_at = Utc::now() - chrono::Duration::hours(1);
        let mut run = Run::new(
            Exec {
                argv: vec!["echo".to_string(), "test".to_string()],
                cwd: ".".to_string(),
                env: HashMap::new(),
                timeout_sec: 0,
            },
            CodeState {
                repo_url: "git@github.com:org/repo.git".to_string(),
                base_commit: "a1b2c3d4e5f6789012345678901234567890abcd".to_string(),
                patch: None,
            },
        );
        run.run_id = run_id.to_string();
        run.status = RunStatus::Running;
        run.timeline.ended_at = Some(original_ended_at);
        storage.save_run(&run).unwrap();

        // Update with exit code 0
        update_run_on_exit_with_storage(&storage, run_id, 0).unwrap();

        // Verify ended_at was NOT overwritten
        let updated = storage.load_run(run_id).unwrap();
        assert_eq!(updated.status, RunStatus::Exited);
        assert_eq!(
            updated.timeline.ended_at.unwrap().timestamp(),
            original_ended_at.timestamp()
        );
    }

    #[test]
    fn test_update_run_on_exit_nonzero_codes() {
        let dir = tempdir().unwrap();
        let storage = Storage::with_base_dir(dir.path().to_path_buf()).unwrap();

        // Test various non-zero exit codes
        for exit_code in [-1i32, 2, 127, 255] {
            let run_id = format!("run_test-code{}-0000-0000-00000000", exit_code.abs());
            create_test_run(&storage, &run_id, RunStatus::Running);

            update_run_on_exit_with_storage(&storage, &run_id, exit_code).unwrap();

            let updated = storage.load_run(&run_id).unwrap();
            assert_eq!(updated.status, RunStatus::Failed);
            assert_eq!(updated.exit_code, Some(exit_code));
        }
    }

    // === Tests for transition_to_running ===

    #[test]
    fn test_transition_to_running_from_pending() {
        let dir = tempdir().unwrap();
        let storage = Storage::with_base_dir(dir.path().to_path_buf()).unwrap();

        let run_id = "run_test-trans-0000-0000-000000000000";
        let mut run = create_test_run(&storage, run_id, RunStatus::Pending);

        // Transition should succeed
        let result = storage.transition_to_running(&mut run).unwrap();
        assert!(result, "Transition from Pending should succeed");
        assert_eq!(run.status, RunStatus::Running);
        assert!(run.timeline.started_at.is_some());

        // Verify in storage
        let loaded = storage.load_run(run_id).unwrap();
        assert_eq!(loaded.status, RunStatus::Running);
    }

    #[test]
    fn test_transition_to_running_after_exited() {
        let dir = tempdir().unwrap();
        let storage = Storage::with_base_dir(dir.path().to_path_buf()).unwrap();

        // Create run as Running, then update to Exited
        let run_id = "run_test-exited-0000-0000-000000000000";
        let mut run = create_test_run(&storage, run_id, RunStatus::Running);

        // Simulate exit-watcher updating status from Running to Exited
        update_run_on_exit_with_storage(&storage, run_id, 0).unwrap();

        // Verify status is now Exited
        let loaded = storage.load_run(run_id).unwrap();
        assert_eq!(loaded.status, RunStatus::Exited);

        // Now try to transition to Running - should fail (not Pending)
        run.status = RunStatus::Pending; // Reset local copy to simulate stale state
        let result = storage.transition_to_running(&mut run).unwrap();
        assert!(!result, "Transition should fail when already Exited");
        assert_eq!(run.status, RunStatus::Exited, "Run should be updated with current state");
    }

    #[test]
    fn test_fast_exit_scenario() {
        // This test documents the expected behavior for fast-exiting processes
        // where the process exits before CLI can transition to Running.
        // The exit-watcher will not update (CAS fails on Pending), but
        // transition_to_running will succeed and then reconcile will later
        // detect the process is gone and set status to Unknown.
        let dir = tempdir().unwrap();
        let storage = Storage::with_base_dir(dir.path().to_path_buf()).unwrap();

        let run_id = "run_test-fast-0000-0000-000000000000";
        let mut run = create_test_run(&storage, run_id, RunStatus::Pending);

        // Fast exit: process exits while status is still Pending
        // Exit-watcher tries to update but CAS fails (only updates from Running)
        update_run_on_exit_with_storage(&storage, run_id, 0).unwrap();
        let loaded = storage.load_run(run_id).unwrap();
        assert_eq!(loaded.status, RunStatus::Pending, "CAS should prevent update from Pending");

        // CLI then transitions to Running
        let result = storage.transition_to_running(&mut run).unwrap();
        assert!(result, "Transition should succeed from Pending");
        assert_eq!(run.status, RunStatus::Running);

        // At this point, the exit code is lost, but reconcile will detect
        // the dead process and set status to Unknown
        // (This is the documented limitation of the current approach)
    }

    #[test]
    fn test_transition_to_running_already_running() {
        let dir = tempdir().unwrap();
        let storage = Storage::with_base_dir(dir.path().to_path_buf()).unwrap();

        let run_id = "run_test-running-0000-0000-00000000";
        let mut run = create_test_run(&storage, run_id, RunStatus::Running);

        // Try to transition - should fail (not Pending)
        let result = storage.transition_to_running(&mut run).unwrap();
        assert!(!result, "Transition should fail when already Running");
    }
}
