use crate::{Playlist, Run, RunResult, RunStatus, RunTemplate};
use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;

fn normalize_for_match(id: &str) -> String {
    id.trim_start_matches("run_")
        .trim_start_matches("tpl_")
        .trim_start_matches("pl_")
        .trim_start_matches("result_")
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
    /// Create a new Storage
    ///
    /// Uses RUNBOX_HOME environment variable if set, otherwise uses XDG data directory
    pub fn new() -> Result<Self> {
        let base_dir = if let Ok(home) = std::env::var("RUNBOX_HOME") {
            PathBuf::from(home)
        } else {
            dirs::data_dir()
                .context("Could not find data directory")?
                .join("runbox")
        };
        Self::with_base_dir(base_dir)
    }

    pub fn with_base_dir(base_dir: PathBuf) -> Result<Self> {
        fs::create_dir_all(base_dir.join("runs"))?;
        fs::create_dir_all(base_dir.join("templates"))?;
        fs::create_dir_all(base_dir.join("playlists"))?;
        fs::create_dir_all(base_dir.join("results"))?;
        fs::create_dir_all(base_dir.join("blobs"))?;
        fs::create_dir_all(base_dir.join("logs"))?;

        Ok(Self { base_dir })
    }

    /// Get the base directory
    pub fn base_dir(&self) -> &PathBuf {
        &self.base_dir
    }

    // === Run operations ===

    /// Save a run (with atomic write via rename)
    pub fn save_run(&self, run: &Run) -> Result<PathBuf> {
        let path = self.base_dir.join("runs").join(format!("{}.json", run.run_id));
        let temp_path = self.base_dir.join("runs").join(format!("{}.json.tmp", run.run_id));

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
        let path = self.base_dir.join("runs").join(format!("{}.json", run_id));
        let lock_path = self.base_dir.join("runs").join(format!("{}.json.lock", run_id));

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
        let temp_path = self.base_dir.join("runs").join(format!("{}.json.tmp", run_id));
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

    pub fn delete_playlist(&self, playlist_id: &str) -> Result<()> {
        let path = self
            .base_dir
            .join("playlists")
            .join(format!("{}.json", playlist_id));
        fs::remove_file(&path).with_context(|| format!("Playlist not found: {}", playlist_id))?;
        Ok(())
    }

    // === Result operations ===

    pub fn save_result(&self, result: &RunResult) -> Result<PathBuf> {
        let path = self
            .base_dir
            .join("results")
            .join(format!("{}.json", result.result_id));
        let temp_path = self
            .base_dir
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
            .base_dir
            .join("results")
            .join(format!("{}.json", result_id));
        let json = fs::read_to_string(&path)
            .with_context(|| format!("Result not found: {}", result_id))?;
        let result: RunResult = serde_json::from_str(&json)?;
        Ok(result)
    }

    pub fn list_results(&self, limit: usize) -> Result<Vec<RunResult>> {
        let results_dir = self.base_dir.join("results");
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
            .base_dir
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

        let blob_path = self.base_dir.join("blobs").join(&hash);

        if !blob_path.exists() {
            let temp_path = self.base_dir.join("blobs").join(format!("{}.tmp", hash));
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
        let blob_path = self.base_dir.join("blobs").join(hash);
        let content = fs::read(&blob_path)
            .with_context(|| format!("Blob not found: {}", blob_ref))?;
        Ok(content)
    }

    pub fn blob_exists(&self, blob_ref: &str) -> bool {
        let hash = blob_ref.trim_start_matches("blobs/");
        self.base_dir.join("blobs").join(hash).exists()
    }

    pub fn blobs_dir(&self) -> PathBuf {
        self.base_dir.join("blobs")
    }

    pub fn results_dir(&self) -> PathBuf {
        self.base_dir.join("results")
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

/// Represents a resolved target for smart run resolution
#[derive(Debug, Clone)]
pub enum ResolvedTarget {
    /// A template was resolved
    Template {
        template_id: String,
        template_name: String,
    },
    /// A playlist item was resolved
    PlaylistItem {
        playlist_id: String,
        playlist_name: String,
        index: usize,
        template_id: String,
        label: Option<String>,
        short_id: String,
    },
}

impl ResolvedTarget {
    /// Get the template ID from the resolved target
    pub fn template_id(&self) -> &str {
        match self {
            ResolvedTarget::Template { template_id, .. } => template_id,
            ResolvedTarget::PlaylistItem { template_id, .. } => template_id,
        }
    }

    /// Get the display short ID for this target
    pub fn display_short_id(&self) -> String {
        match self {
            ResolvedTarget::Template { template_id, .. } => short_id(template_id),
            ResolvedTarget::PlaylistItem { short_id, .. } => short_id.clone(),
        }
    }

    /// Get a human-readable description of what was resolved
    pub fn description(&self) -> String {
        match self {
            ResolvedTarget::Template { template_id, template_name } => {
                format!("template \"{}\" ({})", template_name, template_id)
            }
            ResolvedTarget::PlaylistItem {
                playlist_id,
                playlist_name,
                index,
                label,
                template_id,
                ..
            } => {
                let item_label = label.as_deref().unwrap_or(template_id);
                let playlist_short = short_id(playlist_id);
                format!(
                    "playlist \"{}\" ({}) item {} \"{}\" ({})",
                    playlist_name, playlist_short, index, item_label, template_id
                )
            }
        }
    }

    /// Get a formatted candidate line for ambiguity display
    pub fn candidate_line(&self) -> String {
        match self {
            ResolvedTarget::Template { template_id, template_name } => {
                format!(
                    "  [template]       {}  \"{}\" ({})",
                    short_id(template_id),
                    template_name,
                    template_id
                )
            }
            ResolvedTarget::PlaylistItem {
                playlist_id,
                index: _,
                template_id,
                label,
                short_id,
                ..
            } => {
                let playlist_short = crate::storage::short_id(playlist_id);
                let item_label = label.as_deref().unwrap_or("(no label)");
                format!(
                    "  [playlist:{}] {}  \"{}\" ({})",
                    playlist_short, short_id, item_label, template_id
                )
            }
        }
    }
}

/// Error type for resolution failures
#[derive(Debug, thiserror::Error)]
pub enum ResolveTargetError {
    #[error("No template or playlist item found matching \"{0}\"")]
    NotFound(String),
    #[error("Ambiguous short ID \"{input}\" matches {count} items:\n{candidates}\n\nUse more characters or specify explicitly:\n  runbox run --template <id>\n  runbox playlist run <playlist> <item>")]
    Ambiguous {
        input: String,
        count: usize,
        candidates: String,
    },
    #[error(transparent)]
    Storage(#[from] anyhow::Error),
}

impl Storage {
    /// Resolve a target string to either a template or playlist item.
    /// 
    /// Resolution order:
    /// 1. Check templates for exact match on template_id
    /// 2. Check templates for prefix match on normalized ID
    /// 3. Check playlist items for prefix match on generated hex short ID
    /// 
    /// Returns an error if no match is found or if the match is ambiguous.
    pub fn resolve_target(&self, target: &str) -> Result<ResolvedTarget, ResolveTargetError> {
        let mut matches: Vec<ResolvedTarget> = vec![];

        // 1. Check templates
        let templates = self.list_templates().map_err(ResolveTargetError::Storage)?;
        
        // Exact match on template_id first
        for template in &templates {
            if template.template_id == target {
                return Ok(ResolvedTarget::Template {
                    template_id: template.template_id.clone(),
                    template_name: template.name.clone(),
                });
            }
        }

        // Prefix match on normalized ID
        let target_normalized = normalize_for_match(target);
        for template in &templates {
            let id_normalized = normalize_for_match(&template.template_id);
            if id_normalized.starts_with(&target_normalized) {
                matches.push(ResolvedTarget::Template {
                    template_id: template.template_id.clone(),
                    template_name: template.name.clone(),
                });
            }
        }

        // 2. Check playlist items
        let playlists = self.list_playlists().map_err(ResolveTargetError::Storage)?;
        let target_lower = target.to_lowercase();
        
        for playlist in &playlists {
            for (idx, item) in playlist.items.iter().enumerate() {
                let item_short = item.short_id(&playlist.playlist_id, idx);
                if item_short.starts_with(&target_lower) {
                    matches.push(ResolvedTarget::PlaylistItem {
                        playlist_id: playlist.playlist_id.clone(),
                        playlist_name: playlist.name.clone(),
                        index: idx,
                        template_id: item.template_id.clone(),
                        label: item.label.clone(),
                        short_id: item_short,
                    });
                }
            }
        }

        // Return based on match count
        match matches.len() {
            0 => Err(ResolveTargetError::NotFound(target.to_string())),
            1 => Ok(matches.remove(0)),
            n => {
                let candidates = matches
                    .iter()
                    .map(|m| m.candidate_line())
                    .collect::<Vec<_>>()
                    .join("\n");
                Err(ResolveTargetError::Ambiguous {
                    input: target.to_string(),
                    count: n,
                    candidates,
                })
            }
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
        let result = storage.save_run_if_status_with(
            &run.run_id,
            &[crate::RunStatus::Running],
            |current| {
                current.status = crate::RunStatus::Exited;
                current.exit_code = Some(0);
            },
        ).unwrap();

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
        let result = storage.save_run_if_status_with(
            &run.run_id,
            &[crate::RunStatus::Running], // Expecting Running but it's Exited
            |current| {
                current.status = crate::RunStatus::Unknown;
                current.exit_code = Some(99);
            },
        ).unwrap();

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

        let mut result = crate::RunResult::new(
            "run_test".to_string(),
            started,
            finished,
            0,
        );
        result.result_id = "result_550e8400-e29b-41d4-a716-446655440000".to_string();

        storage.save_result(&result).unwrap();

        let resolved = storage.resolve_result_id("550e").unwrap();
        assert_eq!(resolved, result.result_id);

        let resolved = storage.resolve_result_id(&result.result_id).unwrap();
        assert_eq!(resolved, result.result_id);
    }

    #[test]
    fn test_resolve_target_template() {
        let dir = tempdir().unwrap();
        let storage = Storage::with_base_dir(dir.path().to_path_buf()).unwrap();

        // Create a template
        let template = crate::RunTemplate {
            template_version: 0,
            template_id: "tpl_hello_world".to_string(),
            name: "Hello World".to_string(),
            exec: crate::TemplateExec {
                argv: vec!["echo".to_string(), "hello".to_string()],
                cwd: ".".to_string(),
                env: HashMap::new(),
                timeout_sec: 0,
            },
            bindings: None,
            code_state: crate::TemplateCodeState {
                repo_url: "git@github.com:org/repo.git".to_string(),
            },
        };
        storage.save_template(&template).unwrap();

        // Exact match
        let resolved = storage.resolve_target("tpl_hello_world").unwrap();
        assert!(matches!(resolved, ResolvedTarget::Template { .. }));
        assert_eq!(resolved.template_id(), "tpl_hello_world");

        // Prefix match
        let resolved = storage.resolve_target("hello").unwrap();
        assert!(matches!(resolved, ResolvedTarget::Template { .. }));
        assert_eq!(resolved.template_id(), "tpl_hello_world");
    }

    #[test]
    fn test_resolve_target_playlist_item() {
        let dir = tempdir().unwrap();
        let storage = Storage::with_base_dir(dir.path().to_path_buf()).unwrap();

        // Create a template first
        let template = crate::RunTemplate {
            template_version: 0,
            template_id: "tpl_echo".to_string(),
            name: "Echo".to_string(),
            exec: crate::TemplateExec {
                argv: vec!["echo".to_string(), "test".to_string()],
                cwd: ".".to_string(),
                env: HashMap::new(),
                timeout_sec: 0,
            },
            bindings: None,
            code_state: crate::TemplateCodeState {
                repo_url: "git@github.com:org/repo.git".to_string(),
            },
        };
        storage.save_template(&template).unwrap();

        // Create a playlist with an item
        let mut playlist = crate::Playlist::new("pl_daily", "Daily Tasks");
        playlist.add("tpl_echo", Some("Echo Task"));
        storage.save_playlist(&playlist).unwrap();

        // Get the short ID of the playlist item
        let item_short_id = playlist.items[0].short_id(&playlist.playlist_id, 0);

        // Resolve by short ID
        let resolved = storage.resolve_target(&item_short_id).unwrap();
        assert!(matches!(resolved, ResolvedTarget::PlaylistItem { .. }));
        assert_eq!(resolved.template_id(), "tpl_echo");

        // Resolve by prefix
        let prefix = &item_short_id[..4];
        let resolved = storage.resolve_target(prefix).unwrap();
        assert!(matches!(resolved, ResolvedTarget::PlaylistItem { .. }));
    }

    #[test]
    fn test_resolve_target_not_found() {
        let dir = tempdir().unwrap();
        let storage = Storage::with_base_dir(dir.path().to_path_buf()).unwrap();

        let result = storage.resolve_target("nonexistent");
        assert!(matches!(result, Err(ResolveTargetError::NotFound(_))));
    }

    #[test]
    fn test_resolve_target_ambiguous() {
        let dir = tempdir().unwrap();
        let storage = Storage::with_base_dir(dir.path().to_path_buf()).unwrap();

        // Create two templates with similar prefixes
        let template1 = crate::RunTemplate {
            template_version: 0,
            template_id: "tpl_abc_one".to_string(),
            name: "ABC One".to_string(),
            exec: crate::TemplateExec {
                argv: vec!["echo".to_string(), "one".to_string()],
                cwd: ".".to_string(),
                env: HashMap::new(),
                timeout_sec: 0,
            },
            bindings: None,
            code_state: crate::TemplateCodeState {
                repo_url: "git@github.com:org/repo.git".to_string(),
            },
        };
        storage.save_template(&template1).unwrap();

        let template2 = crate::RunTemplate {
            template_version: 0,
            template_id: "tpl_abc_two".to_string(),
            name: "ABC Two".to_string(),
            exec: crate::TemplateExec {
                argv: vec!["echo".to_string(), "two".to_string()],
                cwd: ".".to_string(),
                env: HashMap::new(),
                timeout_sec: 0,
            },
            bindings: None,
            code_state: crate::TemplateCodeState {
                repo_url: "git@github.com:org/repo.git".to_string(),
            },
        };
        storage.save_template(&template2).unwrap();

        // Short prefix should be ambiguous
        let result = storage.resolve_target("abc");
        assert!(matches!(result, Err(ResolveTargetError::Ambiguous { .. })));

        // More specific prefix should work
        let resolved = storage.resolve_target("abc_one").unwrap();
        assert_eq!(resolved.template_id(), "tpl_abc_one");
    }

}
