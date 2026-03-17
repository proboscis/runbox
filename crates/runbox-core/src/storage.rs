use crate::xdg::{legacy_macos_dir, runbox_data_dir, runbox_state_dir};
use crate::{Playlist, Run, RunResult, RunStatus, RunTemplate};
use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;

pub(crate) fn normalize_for_match(id: &str) -> String {
    id.trim_start_matches("run_")
        .trim_start_matches("run-")
        .trim_start_matches("tpl_")
        .trim_start_matches("tpl-")
        .trim_start_matches("pl_")
        .trim_start_matches("pl-")
        .trim_start_matches("result_")
        .trim_start_matches("result-")
        .trim_start_matches("rec_")
        .trim_start_matches("rec-")
        .trim_start_matches("task_")
        .trim_start_matches("task-")
        .replace('-', "")
        .to_lowercase()
}

/// Convert a full ID to a short display format (first 8 hex characters)
pub fn short_id(full_id: &str) -> String {
    let hex = normalize_for_match(full_id);
    hex.chars().take(8).collect()
}

/// Storage for runs, templates, and playlists
///
/// Uses XDG Base Directory Specification for storage:
/// - Data (templates, playlists, records): $XDG_DATA_HOME/runbox/
/// - State (logs, sqlite): $XDG_STATE_HOME/runbox/
///
/// On macOS, this intentionally uses ~/.local/share and ~/.local/state
/// rather than ~/Library/Application Support for cross-platform consistency.
pub struct Storage {
    /// Base directory for data files (templates, playlists, runs/records)
    data_dir: PathBuf,
    /// Base directory for state files (logs)
    state_dir: PathBuf,
}

impl Storage {
    /// Create a new Storage with XDG-compliant paths
    ///
    /// Uses RUNBOX_HOME environment variable if set, otherwise uses XDG paths.
    /// If legacy macOS storage exists, migrates data automatically.
    pub fn new() -> Result<Self> {
        // Check for RUNBOX_HOME override (for testing and custom setups)
        if let Ok(home) = std::env::var("RUNBOX_HOME") {
            let base_dir = PathBuf::from(home);
            return Self::with_unified_base_dir(base_dir);
        }

        // Use XDG paths
        let data_dir = runbox_data_dir();
        let state_dir = runbox_state_dir();

        // Check for legacy macOS data and migrate if needed
        if let Some(legacy_dir) = legacy_macos_dir() {
            if let Err(e) = Self::migrate_from_legacy(&legacy_dir, &data_dir, &state_dir) {
                eprintln!("[runbox] Migration warning: {}", e);
            }
        }

        Self::with_data_and_state_dirs(data_dir, state_dir)
    }

    /// Create storage with unified base dir (legacy mode, for RUNBOX_HOME and tests)
    pub fn with_unified_base_dir(base_dir: PathBuf) -> Result<Self> {
        // In legacy mode, data and state are in the same directory
        Self::with_data_and_state_dirs(base_dir.clone(), base_dir)
    }

    /// Alias for backward compatibility
    pub fn with_base_dir(base_dir: PathBuf) -> Result<Self> {
        Self::with_unified_base_dir(base_dir)
    }

    /// Create storage with separate data and state directories (XDG mode)
    pub fn with_data_and_state_dirs(data_dir: PathBuf, state_dir: PathBuf) -> Result<Self> {
        // Create data directories
        fs::create_dir_all(data_dir.join("runs"))?;
        fs::create_dir_all(data_dir.join("templates"))?;
        fs::create_dir_all(data_dir.join("playlists"))?;
        fs::create_dir_all(data_dir.join("results"))?;
        fs::create_dir_all(data_dir.join("blobs"))?;

        // Create state directories
        fs::create_dir_all(state_dir.join("logs"))?;

        Ok(Self {
            data_dir,
            state_dir,
        })
    }

    /// Migrate data from legacy macOS path to XDG paths
    fn migrate_from_legacy(
        legacy_dir: &PathBuf,
        data_dir: &PathBuf,
        state_dir: &PathBuf,
    ) -> Result<()> {
        eprintln!(
            "[runbox] Migrating from legacy storage: {}",
            legacy_dir.display()
        );
        eprintln!("[runbox]   → Data: {}", data_dir.display());
        eprintln!("[runbox]   → State: {}", state_dir.display());

        // Create target directories
        fs::create_dir_all(data_dir)?;
        fs::create_dir_all(state_dir)?;

        // Data directories to migrate
        let data_subdirs = ["runs", "templates", "playlists", "results", "blobs"];
        for subdir in &data_subdirs {
            let src = legacy_dir.join(subdir);
            let dst = data_dir.join(subdir);
            if src.exists() && src.is_dir() {
                if let Err(e) = Self::migrate_directory(&src, &dst) {
                    eprintln!("[runbox] Warning: failed to migrate {}: {}", subdir, e);
                }
            }
        }

        // State directories to migrate
        let state_subdirs = ["logs"];
        for subdir in &state_subdirs {
            let src = legacy_dir.join(subdir);
            let dst = state_dir.join(subdir);
            if src.exists() && src.is_dir() {
                if let Err(e) = Self::migrate_directory(&src, &dst) {
                    eprintln!("[runbox] Warning: failed to migrate {}: {}", subdir, e);
                }
            }
        }

        // Check if legacy directory is now empty and remove it
        match Self::is_dir_empty(legacy_dir) {
            Ok(true) => {
                eprintln!("[runbox] Removing empty legacy directory");
                let _ = fs::remove_dir_all(legacy_dir);
            }
            Ok(false) => {
                eprintln!(
                    "[runbox] Legacy directory not empty, keeping: {}",
                    legacy_dir.display()
                );
            }
            Err(_) => {
                // Directory might already be gone or inaccessible, ignore
            }
        }

        eprintln!("[runbox] Migration complete");
        Ok(())
    }
    fn migrate_directory(src: &PathBuf, dst: &PathBuf) -> Result<()> {
        fs::create_dir_all(dst)?;

        for entry in fs::read_dir(src)? {
            let entry = entry?;
            let src_path = entry.path();
            let dst_path = dst.join(entry.file_name());

            // Skip if destination already exists (don't overwrite)
            if dst_path.exists() {
                continue;
            }

            // Try to rename (move) first - fastest on same filesystem
            if fs::rename(&src_path, &dst_path).is_err() {
                // If rename fails (cross-filesystem), copy then delete
                if src_path.is_dir() {
                    Self::copy_dir_recursive(&src_path, &dst_path)?;
                } else {
                    fs::copy(&src_path, &dst_path)?;
                }
                if src_path.is_dir() {
                    fs::remove_dir_all(&src_path)?;
                } else {
                    fs::remove_file(&src_path)?;
                }
            }
        }

        // Try to remove source directory if empty
        let _ = fs::remove_dir(src);

        Ok(())
    }

    /// Copy directory recursively
    fn copy_dir_recursive(src: &PathBuf, dst: &PathBuf) -> Result<()> {
        fs::create_dir_all(dst)?;
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            let src_path = entry.path();
            let dst_path = dst.join(entry.file_name());

            if src_path.is_dir() {
                Self::copy_dir_recursive(&src_path, &dst_path)?;
            } else {
                fs::copy(&src_path, &dst_path)?;
            }
        }
        Ok(())
    }

    /// Check if a directory is empty
    fn is_dir_empty(dir: &PathBuf) -> Result<bool> {
        Ok(fs::read_dir(dir)?.next().is_none())
    }

    /// Get the base directory (for backward compatibility, returns data_dir)
    pub fn base_dir(&self) -> &PathBuf {
        &self.data_dir
    }

    /// Get the data directory
    pub fn data_dir(&self) -> &PathBuf {
        &self.data_dir
    }

    /// Get the state directory
    pub fn state_dir(&self) -> &PathBuf {
        &self.state_dir
    }
    // === Run operations ===

    /// Save a run (with atomic write via rename)
    pub fn save_run(&self, run: &Run) -> Result<PathBuf> {
        let path = self
            .data_dir
            .join("runs")
            .join(format!("{}.json", run.run_id));
        let temp_path = self
            .data_dir
            .join("runs")
            .join(format!("{}.json.tmp", run.run_id));

        let json = serde_json::to_string_pretty(run)?;

        // Write to temp file first
        let mut file = File::create(&temp_path)?;
        file.write_all(json.as_bytes())?;
        file.sync_all()?; // Ensure data is flushed to disk
        drop(file);

        // Atomic rename
        fs::rename(&temp_path, &path)?;

        Ok(path)
    }

    /// Save a run only if current status is one of the expected statuses
    /// Returns Ok(true) if saved, Ok(false) if status didn't match
    /// The update_fn is called with the current run to allow merging fields
    pub fn save_run_if_status_with<F>(
        &self,
        run_id: &str,
        expected_statuses: &[RunStatus],
        update_fn: F,
    ) -> Result<bool>
    where
        F: FnOnce(&mut Run),
    {
        let path = self.data_dir.join("runs").join(format!("{}.json", run_id));
        let lock_path = self
            .data_dir
            .join("runs")
            .join(format!("{}.json.lock", run_id));

        // Acquire exclusive lock
        let lock_file = File::create(&lock_path)?;
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            let fd = lock_file.as_raw_fd();
            // LOCK_EX = exclusive lock, blocking
            let result = unsafe { libc::flock(fd, libc::LOCK_EX) };
            if result != 0 {
                bail!("Failed to acquire lock on {}", lock_path.display());
            }
        }

        // Read current state while holding lock
        if !path.exists() {
            // File doesn't exist, can't update
            #[cfg(unix)]
            {
                use std::os::unix::io::AsRawFd;
                let fd = lock_file.as_raw_fd();
                unsafe { libc::flock(fd, libc::LOCK_UN) };
            }
            bail!("Run not found: {}", run_id);
        }

        let current_json = fs::read_to_string(&path)?;
        let mut current_run: Run = serde_json::from_str(&current_json)?;

        if !expected_statuses.contains(&current_run.status) {
            // Status doesn't match, don't save
            #[cfg(unix)]
            {
                use std::os::unix::io::AsRawFd;
                let fd = lock_file.as_raw_fd();
                unsafe { libc::flock(fd, libc::LOCK_UN) };
            }
            return Ok(false);
        }

        // Apply updates to the fresh copy
        update_fn(&mut current_run);

        // Write atomically
        let temp_path = self
            .data_dir
            .join("runs")
            .join(format!("{}.json.tmp", run_id));
        let json = serde_json::to_string_pretty(&current_run)?;

        let mut file = File::create(&temp_path)?;
        file.write_all(json.as_bytes())?;
        file.sync_all()?;
        drop(file);

        fs::rename(&temp_path, &path)?;

        // Release lock (automatically released when lock_file is dropped, but explicit is clearer)
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            let fd = lock_file.as_raw_fd();
            unsafe { libc::flock(fd, libc::LOCK_UN) };
        }

        Ok(true)
    }

    /// Simple version that just replaces the run if status matches
    pub fn save_run_if_status(&self, run: &Run, expected_statuses: &[RunStatus]) -> Result<bool> {
        let run_clone = run.clone();
        self.save_run_if_status_with(&run.run_id, expected_statuses, |current| {
            *current = run_clone;
        })
    }

    /// Load a run by ID
    pub fn load_run(&self, run_id: &str) -> Result<Run> {
        let path = self.data_dir.join("runs").join(format!("{}.json", run_id));
        let json =
            fs::read_to_string(&path).with_context(|| format!("Run not found: {}", run_id))?;
        let run: Run = serde_json::from_str(&json)?;
        Ok(run)
    }

    /// List all runs, sorted by modification time (newest first)
    pub fn list_runs(&self, limit: usize) -> Result<Vec<Run>> {
        let runs_dir = self.data_dir.join("runs");
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
        let path = self.data_dir.join("runs").join(format!("{}.json", run_id));
        fs::remove_file(&path).with_context(|| format!("Run not found: {}", run_id))?;
        Ok(())
    }

    /// Get the log path for a run
    pub fn log_path(&self, run_id: &str) -> PathBuf {
        self.state_dir.join("logs").join(format!("{}.log", run_id))
    }

    /// Get the logs directory
    pub fn logs_dir(&self) -> PathBuf {
        self.state_dir.join("logs")
    }

    // === Record operations ===

    /// Save a record (with atomic write via rename)
    pub fn save_record(&self, record: &crate::Record) -> Result<PathBuf> {
        let records_dir = self.data_dir.join("records");
        fs::create_dir_all(&records_dir)?;

        let path = records_dir.join(format!("{}.json", record.record_id));
        let temp_path = records_dir.join(format!("{}.json.tmp", record.record_id));

        let json = serde_json::to_string_pretty(record)?;

        let mut file = File::create(&temp_path)?;
        file.write_all(json.as_bytes())?;
        file.sync_all()?;
        drop(file);

        fs::rename(&temp_path, &path)?;

        Ok(path)
    }

    /// Load a record by ID
    pub fn load_record(&self, record_id: &str) -> Result<crate::Record> {
        let path = self
            .data_dir
            .join("records")
            .join(format!("{}.json", record_id));
        let json = fs::read_to_string(&path)
            .with_context(|| format!("Record not found: {}", record_id))?;
        let record: crate::Record = serde_json::from_str(&json)?;
        Ok(record)
    }

    /// List all records, sorted by creation time (newest first)
    pub fn list_records(&self, limit: usize) -> Result<Vec<crate::Record>> {
        let records_dir = self.data_dir.join("records");
        if !records_dir.exists() {
            return Ok(Vec::new());
        }

        let mut entries: Vec<_> = fs::read_dir(&records_dir)?
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

        let records: Vec<crate::Record> = entries
            .into_iter()
            .take(limit)
            .filter_map(|e| fs::read_to_string(e.path()).ok())
            .filter_map(|json| serde_json::from_str(&json).ok())
            .collect();

        Ok(records)
    }

    /// Resolve a record ID (supports short IDs)
    pub fn resolve_record_id(&self, input: &str) -> Result<String> {
        let records = self.list_records(usize::MAX)?;
        resolve_id_from_items(&records, input, |r| &r.record_id)
    }

    // === Template operations ===

    /// Save a template
    pub fn save_template(&self, template: &RunTemplate) -> Result<PathBuf> {
        let path = self
            .base_dir()
            .join("templates")
            .join(format!("{}.json", template.template_id));
        if path.exists() {
            bail!("Template already exists: {}", template.template_id);
        }
        let json = serde_json::to_string_pretty(template)?;
        fs::write(&path, json)?;
        Ok(path)
    }

    /// Load a template by ID
    pub fn load_template(&self, template_id: &str) -> Result<RunTemplate> {
        let path = self
            .base_dir()
            .join("templates")
            .join(format!("{}.json", template_id));
        let json = fs::read_to_string(&path)
            .with_context(|| format!("Template not found: {}", template_id))?;
        let template: RunTemplate = serde_json::from_str(&json)?;
        Ok(template)
    }

    /// List all templates
    pub fn list_templates(&self) -> Result<Vec<RunTemplate>> {
        let templates_dir = self.data_dir.join("templates");
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
            .base_dir()
            .join("templates")
            .join(format!("{}.json", template_id));
        fs::remove_file(&path).with_context(|| format!("Template not found: {}", template_id))?;
        Ok(())
    }

    // === Playlist operations ===

    /// Save a playlist
    pub fn save_playlist(&self, playlist: &Playlist) -> Result<PathBuf> {
        let path = self
            .base_dir()
            .join("playlists")
            .join(format!("{}.json", playlist.playlist_id));
        let json = serde_json::to_string_pretty(playlist)?;
        fs::write(&path, json)?;
        Ok(path)
    }

    /// Load a playlist by ID
    pub fn load_playlist(&self, playlist_id: &str) -> Result<Playlist> {
        let path = self
            .base_dir()
            .join("playlists")
            .join(format!("{}.json", playlist_id));
        let json = fs::read_to_string(&path)
            .with_context(|| format!("Playlist not found: {}", playlist_id))?;
        let playlist: Playlist = serde_json::from_str(&json)?;
        Ok(playlist)
    }

    /// List all playlists
    pub fn list_playlists(&self) -> Result<Vec<Playlist>> {
        let playlists_dir = self.data_dir.join("playlists");
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

    pub fn delete_playlist(&self, playlist_id: &str) -> Result<()> {
        let path = self
            .base_dir()
            .join("playlists")
            .join(format!("{}.json", playlist_id));
        fs::remove_file(&path).with_context(|| format!("Playlist not found: {}", playlist_id))?;
        Ok(())
    }

    // === Result operations ===

    pub fn save_result(&self, result: &RunResult) -> Result<PathBuf> {
        let path = self
            .base_dir()
            .join("results")
            .join(format!("{}.json", result.result_id));
        let temp_path = self
            .base_dir()
            .join("results")
            .join(format!("{}.json.tmp", result.result_id));

        let json = serde_json::to_string_pretty(result)?;

        let mut file = File::create(&temp_path)?;
        file.write_all(json.as_bytes())?;
        file.sync_all()?;
        drop(file);

        fs::rename(&temp_path, &path)?;

        Ok(path)
    }

    pub fn load_result(&self, result_id: &str) -> Result<RunResult> {
        let path = self
            .base_dir()
            .join("results")
            .join(format!("{}.json", result_id));
        let json = fs::read_to_string(&path)
            .with_context(|| format!("Result not found: {}", result_id))?;
        let result: RunResult = serde_json::from_str(&json)?;
        Ok(result)
    }

    pub fn list_results(&self, limit: usize) -> Result<Vec<RunResult>> {
        let results_dir = self.data_dir.join("results");
        let mut entries: Vec<_> = fs::read_dir(&results_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .map(|ext| ext == "json")
                    .unwrap_or(false)
            })
            .collect();

        entries.sort_by(|a, b| {
            let a_time = a.metadata().and_then(|m| m.modified()).ok();
            let b_time = b.metadata().and_then(|m| m.modified()).ok();
            b_time.cmp(&a_time)
        });

        let results: Vec<RunResult> = entries
            .into_iter()
            .take(limit)
            .filter_map(|e| fs::read_to_string(e.path()).ok())
            .filter_map(|json| serde_json::from_str(&json).ok())
            .collect();

        Ok(results)
    }

    pub fn list_results_for_run(&self, run_id: &str) -> Result<Vec<RunResult>> {
        let all_results = self.list_results(usize::MAX)?;
        Ok(all_results
            .into_iter()
            .filter(|r| r.run_id == run_id)
            .collect())
    }

    pub fn delete_result(&self, result_id: &str) -> Result<()> {
        let path = self
            .base_dir()
            .join("results")
            .join(format!("{}.json", result_id));
        fs::remove_file(&path).with_context(|| format!("Result not found: {}", result_id))?;
        Ok(())
    }

    // === Blob operations ===

    pub fn save_blob(&self, content: &[u8]) -> Result<String> {
        let mut hasher = Sha256::new();
        hasher.update(content);
        let hash = format!("{:x}", hasher.finalize());
        let blob_ref = format!("blobs/{}", hash);

        let blob_path = self.data_dir.join("blobs").join(&hash);

        if !blob_path.exists() {
            let temp_path = self.data_dir.join("blobs").join(format!("{}.tmp", hash));
            let mut file = File::create(&temp_path)?;
            file.write_all(content)?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temp_path, &blob_path)?;
        }

        Ok(blob_ref)
    }

    pub fn load_blob(&self, blob_ref: &str) -> Result<Vec<u8>> {
        let hash = blob_ref.trim_start_matches("blobs/");
        let blob_path = self.data_dir.join("blobs").join(hash);
        let content =
            fs::read(&blob_path).with_context(|| format!("Blob not found: {}", blob_ref))?;
        Ok(content)
    }

    pub fn blob_exists(&self, blob_ref: &str) -> bool {
        let hash = blob_ref.trim_start_matches("blobs/");
        self.data_dir.join("blobs").join(hash).exists()
    }

    pub fn blobs_dir(&self) -> PathBuf {
        self.data_dir.join("blobs")
    }

    pub fn results_dir(&self) -> PathBuf {
        self.data_dir.join("results")
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

    pub fn resolve_playlist_id(&self, input: &str) -> Result<String> {
        let playlists = self.list_playlists()?;
        resolve_id_from_items(&playlists, input, |p| &p.playlist_id)
    }

    pub fn resolve_result_id(&self, input: &str) -> Result<String> {
        let results = self.list_results(usize::MAX)?;
        resolve_id_from_items(&results, input, |r| &r.result_id)
    }

    // === Unified Runnable Resolution ===

    /// Resolve any runnable by short ID prefix or full ID.
    ///
    /// This searches across all runnable types (templates, runs/records for replay, playlist items)
    /// and returns the matching runnable or an error if not found or ambiguous.
    ///
    /// # Arguments
    /// * `input` - A short ID prefix (hex string) or full ID (tpl_..., run_..., rec_...)
    /// * `limit` - Maximum number of runs to search (for replay matching)
    ///
    /// # Returns
    /// * `Ok(Runnable)` - Single matching runnable
    /// * `Err` - No match, ambiguous matches, or invalid input
    ///
    /// # Full ID Support
    /// If input starts with `tpl_`, it's treated as a template ID.
    /// If input starts with `run_` or `rec_`, it's treated as a replay ID.
    /// Otherwise, it's treated as a short ID prefix to search.
    pub fn resolve_runnable(&self, input: &str, limit: usize) -> Result<crate::Runnable> {
        let templates = self.list_templates()?;
        let runs = self.list_runs(limit)?;
        let records = self.list_records(limit)?;
        let playlists = self.list_playlists()?;
        resolve_runnable_from_items(input, &templates, &runs, &records, &playlists)
    }

    // === List All Runnables ===

    /// List all runnables (templates, replays, playlist items) with optional filtering.
    ///
    /// # Arguments
    /// * `replay_limit` - Maximum number of recent runs to include for replays
    ///
    /// # Returns
    /// A vector of all Runnables: templates first, then replays, then playlist items
    pub fn list_all_runnables(&self, replay_limit: usize) -> Result<Vec<crate::Runnable>> {
        let templates = self.list_templates()?;
        let runs = self.list_runs(replay_limit)?;
        let playlists = self.list_playlists()?;
        Ok(runnables_from_items(&templates, &runs, &playlists))
    }

    /// Get the repo URL for a runnable.
    ///
    /// - For Template: returns the template's code_state.repo_url
    /// - For Replay: returns the run's code_state.repo_url
    /// - For PlaylistItem: returns the referenced template's code_state.repo_url
    pub fn get_runnable_repo_url(&self, runnable: &crate::Runnable) -> Option<String> {
        runnable_repo_url_with(
            runnable,
            |id| self.load_template(id),
            |id| self.load_run(id),
            |id| self.load_record(id),
        )
    }

    /// Get a display-friendly name for a runnable.
    ///
    /// - For Template: returns the template's name
    /// - For Replay: returns the command (first part of argv)
    /// - For PlaylistItem: returns the label or template name
    pub fn get_runnable_display_name(&self, runnable: &crate::Runnable) -> String {
        runnable_display_name_with(
            runnable,
            |id| self.load_template(id),
            |id| self.load_run(id),
            |id| self.load_record(id),
        )
    }
}

pub(crate) fn runnables_from_items(
    templates: &[RunTemplate],
    runs: &[Run],
    playlists: &[Playlist],
) -> Vec<crate::Runnable> {
    use crate::Runnable;

    let mut runnables = Vec::new();

    for template in templates {
        runnables.push(Runnable::Template(template.template_id.clone()));
    }

    for run in runs {
        runnables.push(Runnable::Replay(run.run_id.clone()));
    }

    for playlist in playlists {
        for (index, item) in playlist.items.iter().enumerate() {
            runnables.push(Runnable::PlaylistItem {
                playlist_id: playlist.playlist_id.clone(),
                index,
                template_id: item.template_id.clone(),
                label: item.label.clone(),
            });
        }
    }

    runnables
}

pub(crate) fn resolve_runnable_from_items(
    input: &str,
    templates: &[RunTemplate],
    runs: &[Run],
    records: &[crate::Record],
    playlists: &[Playlist],
) -> Result<crate::Runnable> {
    use crate::runnable::{format_ambiguous_matches, Runnable, RunnableMatch};

    if input.is_empty() {
        bail!("Empty input: please provide a short ID or full ID (tpl_..., run_..., rec_...)");
    }

    if input.starts_with("tpl_") {
        let resolved_id =
            resolve_id_from_items(templates, input, |template| &template.template_id)?;
        return Ok(Runnable::Template(resolved_id));
    }

    if input.starts_with("run_") || input.starts_with("run-") {
        let resolved_id = resolve_id_from_items(runs, input, |run| &run.run_id)?;
        return Ok(Runnable::Replay(resolved_id));
    }

    if input.starts_with("rec_") || input.starts_with("rec-") {
        let resolved_id = resolve_id_from_items(records, input, |record| &record.record_id)?;
        return Ok(Runnable::Replay(resolved_id));
    }

    if input.starts_with("pl_") {
        let resolved_id =
            resolve_id_from_items(playlists, input, |playlist| &playlist.playlist_id)?;
        let playlist = playlists
            .iter()
            .find(|playlist| playlist.playlist_id == resolved_id)
            .expect("resolved playlist should exist");
        if playlist.items.is_empty() {
            bail!("Playlist '{}' has no items", resolved_id);
        }
        let item = &playlist.items[0];
        return Ok(Runnable::PlaylistItem {
            playlist_id: resolved_id,
            index: 0,
            template_id: item.template_id.clone(),
            label: item.label.clone(),
        });
    }

    let input_lower = input.to_lowercase();
    if !input_lower.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!(
            "Invalid short ID '{}': must be hexadecimal or a full ID (tpl_..., run_..., rec_...)",
            input
        );
    }

    let mut matches: Vec<RunnableMatch> = Vec::new();

    for template in templates {
        let runnable = Runnable::Template(template.template_id.clone());
        if runnable.short_id().starts_with(&input_lower) {
            matches.push(RunnableMatch::from_runnable(runnable));
        }
    }

    for run in runs {
        let runnable = Runnable::Replay(run.run_id.clone());
        if runnable.short_id().starts_with(&input_lower) {
            matches.push(RunnableMatch::from_runnable(runnable));
        }
    }

    for record in records {
        let runnable = Runnable::Replay(record.record_id.clone());
        if runnable.short_id().starts_with(&input_lower) {
            matches.push(RunnableMatch::from_runnable(runnable));
        }
    }

    for playlist in playlists {
        for (index, item) in playlist.items.iter().enumerate() {
            let runnable = Runnable::PlaylistItem {
                playlist_id: playlist.playlist_id.clone(),
                index,
                template_id: item.template_id.clone(),
                label: item.label.clone(),
            };
            if runnable.short_id().starts_with(&input_lower) {
                matches.push(RunnableMatch::from_runnable(runnable));
            }
        }
    }

    match matches.len() {
        0 => bail!("No runnable found matching '{}'", input),
        1 => Ok(matches.remove(0).runnable),
        n => bail!(
            "Ambiguous short ID \"{}\" matches {} items:{}",
            input,
            n,
            format_ambiguous_matches(&matches)
        ),
    }
}

pub(crate) fn runnable_repo_url_with<LoadTemplate, LoadRun, LoadRecord>(
    runnable: &crate::Runnable,
    load_template: LoadTemplate,
    load_run: LoadRun,
    load_record: LoadRecord,
) -> Option<String>
where
    LoadTemplate: Fn(&str) -> Result<RunTemplate>,
    LoadRun: Fn(&str) -> Result<Run>,
    LoadRecord: Fn(&str) -> Result<crate::Record>,
{
    match runnable {
        crate::Runnable::Template(id) => load_template(id)
            .ok()
            .map(|template| template.code_state.repo_url),
        crate::Runnable::Replay(id) => load_run(id)
            .ok()
            .map(|run| run.code_state.repo_url)
            .or_else(|| load_record(id).ok().map(|record| record.git_state.repo_url)),
        crate::Runnable::PlaylistItem { template_id, .. } => load_template(template_id)
            .ok()
            .map(|template| template.code_state.repo_url),
    }
}

pub(crate) fn runnable_display_name_with<LoadTemplate, LoadRun, LoadRecord>(
    runnable: &crate::Runnable,
    load_template: LoadTemplate,
    load_run: LoadRun,
    load_record: LoadRecord,
) -> String
where
    LoadTemplate: Fn(&str) -> Result<RunTemplate>,
    LoadRun: Fn(&str) -> Result<Run>,
    LoadRecord: Fn(&str) -> Result<crate::Record>,
{
    match runnable {
        crate::Runnable::Template(id) => load_template(id)
            .map(|template| template.name)
            .unwrap_or_else(|_| id.clone()),
        crate::Runnable::Replay(id) => load_run(id)
            .map(|run| run.exec.argv.join(" "))
            .or_else(|_| load_record(id).map(|record| record.command.argv.join(" ")))
            .unwrap_or_else(|_| id.clone()),
        crate::Runnable::PlaylistItem {
            label, template_id, ..
        } => {
            if let Some(label) = label {
                label.clone()
            } else {
                load_template(template_id)
                    .map(|template| template.name)
                    .unwrap_or_else(|_| template_id.clone())
            }
        }
    }
}

/// Generic ID resolution from a list of items
pub(crate) fn resolve_id_from_items<T, F>(items: &[T], input: &str, get_id: F) -> Result<String>
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
    use crate::{CodeState, Exec};
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
        assert_eq!(
            short_id("run_550e8400-e29b-41d4-a716-446655440000"),
            "550e8400"
        );
        // Test with template_id prefix
        assert_eq!(
            short_id("tpl_a1b2c3d4-e5f6-7890-abcd-ef1234567890"),
            "a1b2c3d4"
        );
        // Test with playlist_id prefix
        assert_eq!(
            short_id("pl_def45678-90ab-cdef-1234-567890abcdef"),
            "def45678"
        );
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

    #[test]
    fn test_save_run_if_status_with_match() {
        let dir = tempdir().unwrap();
        let storage = Storage::with_base_dir(dir.path().to_path_buf()).unwrap();

        // Create a run with Running status
        let mut run = Run::new(
            Exec {
                argv: vec!["echo".to_string()],
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
        run.status = crate::RunStatus::Running;
        storage.save_run(&run).unwrap();

        // CAS update should succeed when status matches
        let result = storage
            .save_run_if_status_with(&run.run_id, &[crate::RunStatus::Running], |current| {
                current.status = crate::RunStatus::Exited;
                current.exit_code = Some(0);
            })
            .unwrap();

        assert!(result, "CAS should succeed when status matches");

        // Verify the update was applied
        let loaded = storage.load_run(&run.run_id).unwrap();
        assert_eq!(loaded.status, crate::RunStatus::Exited);
        assert_eq!(loaded.exit_code, Some(0));
    }

    #[test]
    fn test_save_run_if_status_with_mismatch() {
        let dir = tempdir().unwrap();
        let storage = Storage::with_base_dir(dir.path().to_path_buf()).unwrap();

        // Create a run with Exited status
        let mut run = Run::new(
            Exec {
                argv: vec!["echo".to_string()],
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
        run.status = crate::RunStatus::Exited;
        run.exit_code = Some(0);
        storage.save_run(&run).unwrap();

        // CAS update should fail when status doesn't match
        let result = storage
            .save_run_if_status_with(
                &run.run_id,
                &[crate::RunStatus::Running], // Expecting Running but it's Exited
                |current| {
                    current.status = crate::RunStatus::Unknown;
                    current.exit_code = Some(99);
                },
            )
            .unwrap();

        assert!(!result, "CAS should fail when status doesn't match");

        // Verify the run was NOT modified
        let loaded = storage.load_run(&run.run_id).unwrap();
        assert_eq!(loaded.status, crate::RunStatus::Exited);
        assert_eq!(loaded.exit_code, Some(0));
    }

    #[test]
    fn test_save_run_if_status_not_found() {
        let dir = tempdir().unwrap();
        let storage = Storage::with_base_dir(dir.path().to_path_buf()).unwrap();

        let result = storage.save_run_if_status_with(
            "run_nonexistent",
            &[crate::RunStatus::Running],
            |_| {},
        );

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn test_result_storage() {
        use chrono::Utc;

        let dir = tempdir().unwrap();
        let storage = Storage::with_base_dir(dir.path().to_path_buf()).unwrap();

        let started = Utc::now();
        let finished = started + chrono::Duration::seconds(5);

        let result = crate::RunResult::new(
            "run_550e8400-e29b-41d4-a716-446655440000".to_string(),
            started,
            finished,
            0,
        );

        storage.save_result(&result).unwrap();

        let loaded = storage.load_result(&result.result_id).unwrap();
        assert_eq!(loaded.result_id, result.result_id);
        assert_eq!(loaded.run_id, result.run_id);
        assert_eq!(loaded.execution.exit_code, 0);

        let results = storage.list_results(10).unwrap();
        assert_eq!(results.len(), 1);

        storage.delete_result(&result.result_id).unwrap();
        let results = storage.list_results(10).unwrap();
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn test_list_results_for_run() {
        use chrono::Utc;

        let dir = tempdir().unwrap();
        let storage = Storage::with_base_dir(dir.path().to_path_buf()).unwrap();

        let run_id_1 = "run_aaaa0000-0000-0000-0000-000000000000".to_string();
        let run_id_2 = "run_bbbb0000-0000-0000-0000-000000000000".to_string();

        let started = Utc::now();
        let finished = started + chrono::Duration::seconds(1);

        let result1 = crate::RunResult::new(run_id_1.clone(), started, finished, 0);
        let result2 = crate::RunResult::new(run_id_1.clone(), started, finished, 1);
        let result3 = crate::RunResult::new(run_id_2.clone(), started, finished, 0);

        storage.save_result(&result1).unwrap();
        storage.save_result(&result2).unwrap();
        storage.save_result(&result3).unwrap();

        let results_for_run1 = storage.list_results_for_run(&run_id_1).unwrap();
        assert_eq!(results_for_run1.len(), 2);

        let results_for_run2 = storage.list_results_for_run(&run_id_2).unwrap();
        assert_eq!(results_for_run2.len(), 1);
    }

    #[test]
    fn test_blob_storage() {
        let dir = tempdir().unwrap();
        let storage = Storage::with_base_dir(dir.path().to_path_buf()).unwrap();

        let content = b"Hello, World! This is stdout content.";
        let blob_ref = storage.save_blob(content).unwrap();

        assert!(blob_ref.starts_with("blobs/"));
        assert!(storage.blob_exists(&blob_ref));

        let loaded = storage.load_blob(&blob_ref).unwrap();
        assert_eq!(loaded, content);

        let blob_ref_2 = storage.save_blob(content).unwrap();
        assert_eq!(blob_ref, blob_ref_2);
    }

    #[test]
    fn test_resolve_result_id() {
        use chrono::Utc;

        let dir = tempdir().unwrap();
        let storage = Storage::with_base_dir(dir.path().to_path_buf()).unwrap();

        let started = Utc::now();
        let finished = started + chrono::Duration::seconds(1);

        let mut result = crate::RunResult::new("run_test".to_string(), started, finished, 0);
        result.result_id = "result_550e8400-e29b-41d4-a716-446655440000".to_string();

        storage.save_result(&result).unwrap();

        let resolved = storage.resolve_result_id("550e").unwrap();
        assert_eq!(resolved, result.result_id);

        let resolved = storage.resolve_result_id(&result.result_id).unwrap();
        assert_eq!(resolved, result.result_id);
    }

    #[test]
    fn test_resolve_runnable_template() {
        let dir = tempdir().unwrap();
        let storage = Storage::with_base_dir(dir.path().to_path_buf()).unwrap();

        // Create a template
        let template = crate::RunTemplate {
            template_version: 0,
            template_id: "tpl_echo".to_string(),
            name: "Echo Command".to_string(),
            exec: crate::TemplateExec {
                argv: vec!["echo".to_string(), "hello".to_string()],
                cwd: ".".to_string(),
                env: std::collections::HashMap::new(),
                timeout_sec: 0,
            },
            bindings: None,
            code_state: crate::TemplateCodeState {
                repo_url: "git@github.com:org/repo.git".to_string(),
            },
        };
        storage.save_template(&template).unwrap();

        // Get the template's short ID
        let runnable = crate::Runnable::Template("tpl_echo".to_string());
        let short_id = runnable.short_id();

        // Resolve by short ID prefix
        let resolved = storage.resolve_runnable(&short_id[..4], 100).unwrap();
        match resolved {
            crate::Runnable::Template(id) => assert_eq!(id, "tpl_echo"),
            _ => panic!("Expected Template runnable"),
        }
    }

    #[test]
    fn test_resolve_runnable_replay() {
        let dir = tempdir().unwrap();
        let storage = Storage::with_base_dir(dir.path().to_path_buf()).unwrap();

        // Create a run
        let mut run = Run::new(
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
        run.run_id = "run_550e8400-e29b-41d4-a716-446655440000".to_string();
        storage.save_run(&run).unwrap();

        // Resolve by short ID prefix
        let resolved = storage.resolve_runnable("550e", 100).unwrap();
        match resolved {
            crate::Runnable::Replay(id) => {
                assert_eq!(id, "run_550e8400-e29b-41d4-a716-446655440000")
            }
            _ => panic!("Expected Replay runnable"),
        }
    }

    #[test]
    fn test_resolve_runnable_record_replay() {
        let dir = tempdir().unwrap();
        let storage = Storage::with_base_dir(dir.path().to_path_buf()).unwrap();

        let mut record = crate::Record::new(
            crate::RecordGitState {
                repo_url: "git@github.com:org/repo.git".to_string(),
                commit: "a1b2c3d4e5f6789012345678901234567890abcd".to_string(),
                patch_ref: None,
            },
            crate::RecordCommand {
                argv: vec!["echo".to_string(), "hello".to_string()],
                cwd: ".".to_string(),
                env: HashMap::new(),
            },
        );
        record.record_id = "rec_550e8400-e29b-41d4-a716-446655440000".to_string();
        storage.save_record(&record).unwrap();

        let resolved = storage.resolve_runnable("550e", 100).unwrap();
        match resolved {
            crate::Runnable::Replay(id) => {
                assert_eq!(id, "rec_550e8400-e29b-41d4-a716-446655440000")
            }
            _ => panic!("Expected Replay runnable"),
        }
    }

    #[test]
    fn test_resolve_runnable_playlist_item() {
        let dir = tempdir().unwrap();
        let storage = Storage::with_base_dir(dir.path().to_path_buf()).unwrap();

        // Create a template first
        let template = crate::RunTemplate {
            template_version: 0,
            template_id: "tpl_echo".to_string(),
            name: "Echo Command".to_string(),
            exec: crate::TemplateExec {
                argv: vec!["echo".to_string(), "hello".to_string()],
                cwd: ".".to_string(),
                env: std::collections::HashMap::new(),
                timeout_sec: 0,
            },
            bindings: None,
            code_state: crate::TemplateCodeState {
                repo_url: "git@github.com:org/repo.git".to_string(),
            },
        };
        storage.save_template(&template).unwrap();

        // Create a playlist with items
        let mut playlist = Playlist::new("pl_daily", "Daily Tasks");
        playlist.add("tpl_echo", Some("Echo Hello"));
        storage.save_playlist(&playlist).unwrap();

        // Get the playlist item's short ID
        let runnable = crate::Runnable::PlaylistItem {
            playlist_id: "pl_daily".to_string(),
            index: 0,
            template_id: "tpl_echo".to_string(),
            label: Some("Echo Hello".to_string()),
        };
        let short_id = runnable.short_id();

        // Resolve by short ID prefix
        let resolved = storage.resolve_runnable(&short_id[..4], 100).unwrap();
        match resolved {
            crate::Runnable::PlaylistItem {
                playlist_id,
                index,
                template_id,
                ..
            } => {
                assert_eq!(playlist_id, "pl_daily");
                assert_eq!(index, 0);
                assert_eq!(template_id, "tpl_echo");
            }
            _ => panic!("Expected PlaylistItem runnable"),
        }
    }

    #[test]
    fn test_resolve_runnable_not_found() {
        let dir = tempdir().unwrap();
        let storage = Storage::with_base_dir(dir.path().to_path_buf()).unwrap();

        // Use a valid hex string that doesn't match anything
        let result = storage.resolve_runnable("deadbeef", 100);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("No runnable found"));
    }

    #[test]
    fn test_resolve_runnable_empty_input() {
        let dir = tempdir().unwrap();
        let storage = Storage::with_base_dir(dir.path().to_path_buf()).unwrap();

        let result = storage.resolve_runnable("", 100);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Empty input"));
    }

    #[test]
    fn test_resolve_runnable_invalid_input() {
        let dir = tempdir().unwrap();
        let storage = Storage::with_base_dir(dir.path().to_path_buf()).unwrap();

        // Non-hex, non-full-ID input should be rejected
        let result = storage.resolve_runnable("xyz", 100);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid short ID"));
    }

    #[test]
    fn test_resolve_runnable_full_template_id() {
        let dir = tempdir().unwrap();
        let storage = Storage::with_base_dir(dir.path().to_path_buf()).unwrap();

        // Create a template
        let template = crate::RunTemplate {
            template_version: 0,
            template_id: "tpl_echo".to_string(),
            name: "Echo Command".to_string(),
            exec: crate::TemplateExec {
                argv: vec!["echo".to_string(), "hello".to_string()],
                cwd: ".".to_string(),
                env: std::collections::HashMap::new(),
                timeout_sec: 0,
            },
            bindings: None,
            code_state: crate::TemplateCodeState {
                repo_url: "git@github.com:org/repo.git".to_string(),
            },
        };
        storage.save_template(&template).unwrap();

        // Resolve by full template ID
        let resolved = storage.resolve_runnable("tpl_echo", 100).unwrap();
        match resolved {
            crate::Runnable::Template(id) => assert_eq!(id, "tpl_echo"),
            _ => panic!("Expected Template runnable"),
        }
    }

    #[test]
    fn test_resolve_runnable_full_run_id() {
        let dir = tempdir().unwrap();
        let storage = Storage::with_base_dir(dir.path().to_path_buf()).unwrap();

        // Create a run
        let mut run = Run::new(
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
        run.run_id = "run_550e8400-e29b-41d4-a716-446655440000".to_string();
        storage.save_run(&run).unwrap();

        // Resolve by full run ID
        let resolved = storage
            .resolve_runnable("run_550e8400-e29b-41d4-a716-446655440000", 100)
            .unwrap();
        match resolved {
            crate::Runnable::Replay(id) => {
                assert_eq!(id, "run_550e8400-e29b-41d4-a716-446655440000")
            }
            _ => panic!("Expected Replay runnable"),
        }
    }

    #[test]
    fn test_resolve_runnable_full_record_id() {
        let dir = tempdir().unwrap();
        let storage = Storage::with_base_dir(dir.path().to_path_buf()).unwrap();

        let mut record = crate::Record::new(
            crate::RecordGitState {
                repo_url: "git@github.com:org/repo.git".to_string(),
                commit: "a1b2c3d4e5f6789012345678901234567890abcd".to_string(),
                patch_ref: None,
            },
            crate::RecordCommand {
                argv: vec!["echo".to_string(), "hello".to_string()],
                cwd: ".".to_string(),
                env: HashMap::new(),
            },
        );
        record.record_id = "rec_550e8400-e29b-41d4-a716-446655440000".to_string();
        storage.save_record(&record).unwrap();

        let resolved = storage
            .resolve_runnable("rec_550e8400-e29b-41d4-a716-446655440000", 100)
            .unwrap();
        match resolved {
            crate::Runnable::Replay(id) => {
                assert_eq!(id, "rec_550e8400-e29b-41d4-a716-446655440000")
            }
            _ => panic!("Expected Replay runnable"),
        }
    }

    #[test]
    fn test_resolve_runnable_ambiguous() {
        let dir = tempdir().unwrap();
        let storage = Storage::with_base_dir(dir.path().to_path_buf()).unwrap();

        // Create two templates that might have similar short ID prefixes
        // We'll use a more direct approach by creating multiple items
        let template1 = crate::RunTemplate {
            template_version: 0,
            template_id: "tpl_aaa".to_string(),
            name: "AAA".to_string(),
            exec: crate::TemplateExec {
                argv: vec!["echo".to_string()],
                cwd: ".".to_string(),
                env: std::collections::HashMap::new(),
                timeout_sec: 0,
            },
            bindings: None,
            code_state: crate::TemplateCodeState {
                repo_url: "git@github.com:org/repo.git".to_string(),
            },
        };
        let template2 = crate::RunTemplate {
            template_version: 0,
            template_id: "tpl_bbb".to_string(),
            name: "BBB".to_string(),
            exec: crate::TemplateExec {
                argv: vec!["echo".to_string()],
                cwd: ".".to_string(),
                env: std::collections::HashMap::new(),
                timeout_sec: 0,
            },
            bindings: None,
            code_state: crate::TemplateCodeState {
                repo_url: "git@github.com:org/repo.git".to_string(),
            },
        };
        storage.save_template(&template1).unwrap();
        storage.save_template(&template2).unwrap();

        // Get both short IDs and check if they share a prefix
        let runnable1 = crate::Runnable::Template("tpl_aaa".to_string());
        let runnable2 = crate::Runnable::Template("tpl_bbb".to_string());
        let short1 = runnable1.short_id();
        let short2 = runnable2.short_id();

        // If they happen to share a prefix (unlikely but possible), test ambiguity
        // Otherwise, just verify that resolution works correctly
        if short1.chars().next() == short2.chars().next() {
            let result = storage.resolve_runnable(&short1[..1], 100);
            assert!(result.is_err());
            let err_msg = result.unwrap_err().to_string();
            assert!(err_msg.contains("Ambiguous"));
        }

        // Regardless, full short IDs should resolve uniquely
        let resolved1 = storage.resolve_runnable(&short1, 100).unwrap();
        match resolved1 {
            crate::Runnable::Template(id) => assert_eq!(id, "tpl_aaa"),
            _ => panic!("Expected Template runnable"),
        }
    }

    #[test]
    fn test_record_storage() {
        let dir = tempdir().unwrap();
        let storage = Storage::with_base_dir(dir.path().to_path_buf()).unwrap();

        let record = crate::Record::new(
            crate::RecordGitState {
                repo_url: "git@github.com:org/repo.git".to_string(),
                commit: "a1b2c3d4e5f6789012345678901234567890abcd".to_string(),
                patch_ref: None,
            },
            crate::RecordCommand {
                argv: vec!["echo".to_string(), "hello".to_string()],
                cwd: ".".to_string(),
                env: HashMap::new(),
            },
        );

        // Save
        storage.save_record(&record).unwrap();

        // Load
        let loaded = storage.load_record(&record.record_id).unwrap();
        assert_eq!(loaded.record_id, record.record_id);
        assert_eq!(loaded.git_state.repo_url, "git@github.com:org/repo.git");
        assert_eq!(loaded.command.argv, vec!["echo", "hello"]);

        // List
        let records = storage.list_records(10).unwrap();
        assert_eq!(records.len(), 1);

        // Resolve by short ID
        let short = &record.record_id[4..8];
        let resolved = storage.resolve_record_id(short).unwrap();
        assert_eq!(resolved, record.record_id);
    }
}
