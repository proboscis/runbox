//! Project-local .runbox/ storage support
//!
//! Provides detection and resolution of project-local templates and playlists,
//! allowing git-managed project-specific configurations that override global ones.

use anyhow::{Context, Result};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

/// Scope of a runnable item (local vs global)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Project-local (.runbox/ directory)
    Local,
    /// Global (XDG data directory)
    Global,
}

impl std::fmt::Display for Scope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Scope::Local => write!(f, "local"),
            Scope::Global => write!(f, "global"),
        }
    }
}

/// Search for the project-local .runbox/ directory by traversing upward from cwd.
///
/// Returns the path to the .runbox/ directory if located, None otherwise.
/// Handles git worktrees by also checking the main worktree's location.
pub fn locate_local_runbox_dir() -> Option<PathBuf> {
    locate_local_runbox_dir_from(env::current_dir().ok()?)
}

/// Search for .runbox/ starting from a specific directory.
pub fn locate_local_runbox_dir_from(start: PathBuf) -> Option<PathBuf> {
    let mut current = start;

    loop {
        let runbox_dir = current.join(".runbox");
        if runbox_dir.is_dir() {
            return Some(runbox_dir);
        }

        // Check if we're in a git worktree and should also check the main repo
        if let Some(main_worktree) = get_main_worktree_path(&current) {
            let main_runbox = main_worktree.join(".runbox");
            if main_runbox.is_dir() && main_runbox != runbox_dir {
                return Some(main_runbox);
            }
        }

        // Move to parent directory
        if let Some(parent) = current.parent() {
            current = parent.to_path_buf();
        } else {
            break;
        }
    }

    None
}

/// Get the main worktree path if we're in a git worktree.
fn get_main_worktree_path(dir: &Path) -> Option<PathBuf> {
    // Check for .git file (worktrees have a file, not a directory)
    let git_path = dir.join(".git");
    if git_path.is_file() {
        // Read the .git file to discover the actual git dir
        if let Ok(content) = fs::read_to_string(&git_path) {
            if let Some(gitdir_line) = content.lines().next() {
                if gitdir_line.starts_with("gitdir:") {
                    let gitdir = gitdir_line.trim_start_matches("gitdir:").trim();
                    let gitdir_path = PathBuf::from(gitdir);

                    // The gitdir is typically .git/worktrees/<name>
                    // The main repo is at ../../.. from there
                    if gitdir.contains("/worktrees/") {
                        // Navigate up to locate the main repo
                        if let Some(worktrees_dir) = gitdir_path.parent() {
                            if let Some(git_dir) = worktrees_dir.parent() {
                                if let Some(main_repo) = git_dir.parent() {
                                    return Some(main_repo.to_path_buf());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    None
}

/// Storage layer that combines local and global storage with resolution order.
///
/// Resolution order: Local (.runbox/) then Global ($XDG_DATA_HOME/runbox/)
/// Same-name items in local override global.
pub struct LayeredStorage {
    /// Project-local .runbox/ directory (if located)
    local_dir: Option<PathBuf>,
    /// Global storage
    global_storage: crate::Storage,
}

impl LayeredStorage {
    /// Create a new LayeredStorage, searching for local .runbox/ from cwd.
    pub fn new() -> Result<Self> {
        let global_storage = crate::Storage::new()?;
        let local_dir = locate_local_runbox_dir();

        // Create local directories if local_dir exists
        if let Some(ref dir) = local_dir {
            fs::create_dir_all(dir.join("templates"))?;
            fs::create_dir_all(dir.join("playlists"))?;
        }

        Ok(Self {
            local_dir,
            global_storage,
        })
    }

    /// Create with explicit paths (for testing).
    pub fn with_paths(local_dir: Option<PathBuf>, global_dir: PathBuf) -> Result<Self> {
        let global_storage = crate::Storage::with_base_dir(global_dir)?;

        Self::with_global_storage(local_dir, global_storage)
    }

    /// Create with explicit data/state directories.
    pub fn with_data_and_state_dirs(
        local_dir: Option<PathBuf>,
        global_data_dir: PathBuf,
        global_state_dir: PathBuf,
    ) -> Result<Self> {
        let global_storage =
            crate::Storage::with_data_and_state_dirs(global_data_dir, global_state_dir)?;

        Self::with_global_storage(local_dir, global_storage)
    }

    /// Create with a prepared global storage instance.
    pub fn with_global_storage(
        local_dir: Option<PathBuf>,
        global_storage: crate::Storage,
    ) -> Result<Self> {
        if let Some(ref dir) = local_dir {
            fs::create_dir_all(dir.join("templates"))?;
            fs::create_dir_all(dir.join("playlists"))?;
        }

        Ok(Self {
            local_dir,
            global_storage,
        })
    }

    /// Get the local directory path (if any).
    pub fn local_dir(&self) -> Option<&PathBuf> {
        self.local_dir.as_ref()
    }

    /// Get the global storage.
    pub fn global_storage(&self) -> &crate::Storage {
        &self.global_storage
    }

    /// Check if local storage is available.
    pub fn has_local(&self) -> bool {
        self.local_dir.is_some()
    }

    fn list_local_templates(&self) -> Result<Vec<crate::RunTemplate>> {
        let mut templates = Vec::new();

        if let Some(ref local_dir) = self.local_dir {
            let templates_dir = local_dir.join("templates");
            if templates_dir.is_dir() {
                for entry in fs::read_dir(&templates_dir)? {
                    let entry = entry?;
                    if entry
                        .path()
                        .extension()
                        .map(|extension| extension == "json")
                        .unwrap_or(false)
                    {
                        if let Ok(content) = fs::read_to_string(entry.path()) {
                            if let Ok(template) =
                                serde_json::from_str::<crate::RunTemplate>(&content)
                            {
                                templates.push(template);
                            }
                        }
                    }
                }
            }
        }

        Ok(templates)
    }

    fn list_local_playlists(&self) -> Result<Vec<crate::Playlist>> {
        let mut playlists = Vec::new();

        if let Some(ref local_dir) = self.local_dir {
            let playlists_dir = local_dir.join("playlists");
            if playlists_dir.is_dir() {
                for entry in fs::read_dir(&playlists_dir)? {
                    let entry = entry?;
                    if entry
                        .path()
                        .extension()
                        .map(|extension| extension == "json")
                        .unwrap_or(false)
                    {
                        if let Ok(content) = fs::read_to_string(entry.path()) {
                            if let Ok(playlist) = serde_json::from_str::<crate::Playlist>(&content)
                            {
                                playlists.push(playlist);
                            }
                        }
                    }
                }
            }
        }

        Ok(playlists)
    }

    // === Template operations with scope ===

    /// List all templates with their scope.
    ///
    /// Returns tuples of (template, scope). Local templates with the same ID
    /// as global ones override them (global duplicate is not returned).
    pub fn list_templates_with_scope(&self) -> Result<Vec<(crate::RunTemplate, Scope)>> {
        let mut result = Vec::new();
        let mut seen_ids = std::collections::HashSet::new();

        // First, add local templates
        for template in self.list_local_templates()? {
            seen_ids.insert(template.template_id.clone());
            result.push((template, Scope::Local));
        }

        // Then, add global templates (skip if same ID exists locally)
        for template in self.global_storage.list_templates()? {
            if !seen_ids.contains(&template.template_id) {
                result.push((template, Scope::Global));
            }
        }

        Ok(result)
    }

    /// List all templates with local overrides already applied.
    pub fn list_templates(&self) -> Result<Vec<crate::RunTemplate>> {
        Ok(self
            .list_templates_with_scope()?
            .into_iter()
            .map(|(template, _)| template)
            .collect())
    }

    /// Resolve a template ID from a full ID or short prefix.
    pub fn resolve_template_id(&self, input: &str) -> Result<String> {
        let templates = self.list_templates()?;
        crate::storage::resolve_id_from_items(&templates, input, |template| &template.template_id)
    }

    /// Resolve a template ID from a full ID or short prefix in a specific scope.
    pub fn resolve_template_id_in_scope(&self, input: &str, scope: Scope) -> Result<String> {
        let templates = match scope {
            Scope::Local => self.list_local_templates()?,
            Scope::Global => self.global_storage.list_templates()?,
        };
        crate::storage::resolve_id_from_items(&templates, input, |template| &template.template_id)
    }

    /// Load a template by ID (local overrides global).
    pub fn load_template(&self, template_id: &str) -> Result<(crate::RunTemplate, Scope)> {
        // Try local first
        if let Some(ref local_dir) = self.local_dir {
            let path = local_dir
                .join("templates")
                .join(format!("{}.json", template_id));
            if path.exists() {
                let content = fs::read_to_string(&path)
                    .with_context(|| format!("Failed to read template: {}", template_id))?;
                let template: crate::RunTemplate = serde_json::from_str(&content)?;
                return Ok((template, Scope::Local));
            }
        }

        // Fall back to global
        let template = self.global_storage.load_template(template_id)?;
        Ok((template, Scope::Global))
    }

    /// Load a template in a specific scope without fallback.
    pub fn load_template_in_scope(
        &self,
        template_id: &str,
        scope: Scope,
    ) -> Result<crate::RunTemplate> {
        match scope {
            Scope::Local => {
                let local_dir = self
                    .local_dir
                    .as_ref()
                    .context("No local .runbox/ directory available")?;
                let path = local_dir
                    .join("templates")
                    .join(format!("{}.json", template_id));
                let content = fs::read_to_string(&path)
                    .with_context(|| format!("Template not found: {}", template_id))?;
                Ok(serde_json::from_str(&content)?)
            }
            Scope::Global => self.global_storage.load_template(template_id),
        }
    }

    /// Save a template to the specified scope.
    pub fn save_template(&self, template: &crate::RunTemplate, scope: Scope) -> Result<PathBuf> {
        match scope {
            Scope::Local => {
                let local_dir = self
                    .local_dir
                    .as_ref()
                    .context("No local .runbox/ directory available")?;
                let path = local_dir
                    .join("templates")
                    .join(format!("{}.json", template.template_id));
                if path.exists() {
                    anyhow::bail!("Template already exists: {}", template.template_id);
                }
                let json = serde_json::to_string_pretty(template)?;
                fs::write(&path, json)?;
                Ok(path)
            }
            Scope::Global => self.global_storage.save_template(template),
        }
    }

    /// Delete a template in a specific scope.
    pub fn delete_template_in_scope(&self, template_id: &str, scope: Scope) -> Result<()> {
        match scope {
            Scope::Local => {
                let local_dir = self
                    .local_dir
                    .as_ref()
                    .context("No local .runbox/ directory available")?;
                let local_path = local_dir
                    .join("templates")
                    .join(format!("{}.json", template_id));
                fs::remove_file(&local_path)
                    .with_context(|| format!("Template not found: {}", template_id))?;
                Ok(())
            }
            Scope::Global => self.global_storage.delete_template(template_id),
        }
    }

    // === Playlist operations with scope ===

    /// List all playlists with their scope.
    pub fn list_playlists_with_scope(&self) -> Result<Vec<(crate::Playlist, Scope)>> {
        let mut result = Vec::new();
        let mut seen_ids = std::collections::HashSet::new();

        // First, add local playlists
        for playlist in self.list_local_playlists()? {
            seen_ids.insert(playlist.playlist_id.clone());
            result.push((playlist, Scope::Local));
        }

        // Then, add global playlists (skip if same ID exists locally)
        for playlist in self.global_storage.list_playlists()? {
            if !seen_ids.contains(&playlist.playlist_id) {
                result.push((playlist, Scope::Global));
            }
        }

        Ok(result)
    }

    /// List all playlists with local overrides already applied.
    pub fn list_playlists(&self) -> Result<Vec<crate::Playlist>> {
        Ok(self
            .list_playlists_with_scope()?
            .into_iter()
            .map(|(playlist, _)| playlist)
            .collect())
    }

    /// Resolve a playlist ID from a full ID or short prefix.
    pub fn resolve_playlist_id(&self, input: &str) -> Result<String> {
        let playlists = self.list_playlists()?;
        crate::storage::resolve_id_from_items(&playlists, input, |playlist| &playlist.playlist_id)
    }

    /// Resolve a playlist ID from a full ID or short prefix in a specific scope.
    pub fn resolve_playlist_id_in_scope(&self, input: &str, scope: Scope) -> Result<String> {
        let playlists = match scope {
            Scope::Local => self.list_local_playlists()?,
            Scope::Global => self.global_storage.list_playlists()?,
        };
        crate::storage::resolve_id_from_items(&playlists, input, |playlist| &playlist.playlist_id)
    }

    /// Load a playlist by ID (local overrides global).
    pub fn load_playlist(&self, playlist_id: &str) -> Result<(crate::Playlist, Scope)> {
        // Try local first
        if let Some(ref local_dir) = self.local_dir {
            let path = local_dir
                .join("playlists")
                .join(format!("{}.json", playlist_id));
            if path.exists() {
                let content = fs::read_to_string(&path)
                    .with_context(|| format!("Failed to read playlist: {}", playlist_id))?;
                let playlist: crate::Playlist = serde_json::from_str(&content)?;
                return Ok((playlist, Scope::Local));
            }
        }

        // Fall back to global
        let playlist = self.global_storage.load_playlist(playlist_id)?;
        Ok((playlist, Scope::Global))
    }

    /// Load a playlist in a specific scope without fallback.
    pub fn load_playlist_in_scope(
        &self,
        playlist_id: &str,
        scope: Scope,
    ) -> Result<crate::Playlist> {
        match scope {
            Scope::Local => {
                let local_dir = self
                    .local_dir
                    .as_ref()
                    .context("No local .runbox/ directory available")?;
                let path = local_dir
                    .join("playlists")
                    .join(format!("{}.json", playlist_id));
                let content = fs::read_to_string(&path)
                    .with_context(|| format!("Playlist not found: {}", playlist_id))?;
                Ok(serde_json::from_str(&content)?)
            }
            Scope::Global => self.global_storage.load_playlist(playlist_id),
        }
    }

    /// Save a playlist to the specified scope.
    pub fn save_playlist(&self, playlist: &crate::Playlist, scope: Scope) -> Result<PathBuf> {
        match scope {
            Scope::Local => {
                let local_dir = self
                    .local_dir
                    .as_ref()
                    .context("No local .runbox/ directory available")?;
                let path = local_dir
                    .join("playlists")
                    .join(format!("{}.json", playlist.playlist_id));
                let json = serde_json::to_string_pretty(playlist)?;
                fs::write(&path, json)?;
                Ok(path)
            }
            Scope::Global => self.global_storage.save_playlist(playlist),
        }
    }

    /// Delete a playlist in a specific scope.
    pub fn delete_playlist_in_scope(&self, playlist_id: &str, scope: Scope) -> Result<()> {
        match scope {
            Scope::Local => {
                let local_dir = self
                    .local_dir
                    .as_ref()
                    .context("No local .runbox/ directory available")?;
                let local_path = local_dir
                    .join("playlists")
                    .join(format!("{}.json", playlist_id));
                fs::remove_file(&local_path)
                    .with_context(|| format!("Playlist not found: {}", playlist_id))?;
                Ok(())
            }
            Scope::Global => self.global_storage.delete_playlist(playlist_id),
        }
    }

    // === Delegate run/result operations to global storage ===
    // (Runs are always stored globally, not per-project)

    pub fn save_run(&self, run: &crate::Run) -> Result<PathBuf> {
        self.global_storage.save_run(run)
    }

    pub fn load_run(&self, run_id: &str) -> Result<crate::Run> {
        self.global_storage.load_run(run_id)
    }

    pub fn list_runs(&self, limit: usize) -> Result<Vec<crate::Run>> {
        self.global_storage.list_runs(limit)
    }

    pub fn log_path(&self, run_id: &str) -> PathBuf {
        self.global_storage.log_path(run_id)
    }

    /// List all runnables with their scope.
    pub fn list_all_runnables_with_scope(
        &self,
        replay_limit: usize,
    ) -> Result<Vec<(crate::Runnable, Scope)>> {
        let mut runnables = Vec::new();

        for (template, scope) in self.list_templates_with_scope()? {
            runnables.push((crate::Runnable::Template(template.template_id), scope));
        }

        for run in self.list_runs(replay_limit)? {
            runnables.push((crate::Runnable::Replay(run.run_id), Scope::Global));
        }

        for (playlist, scope) in self.list_playlists_with_scope()? {
            for (index, item) in playlist.items.iter().enumerate() {
                runnables.push((
                    crate::Runnable::PlaylistItem {
                        playlist_id: playlist.playlist_id.clone(),
                        index,
                        template_id: item.template_id.clone(),
                        label: item.label.clone(),
                    },
                    scope,
                ));
            }
        }

        Ok(runnables)
    }

    /// Resolve any runnable by short ID prefix or full ID across local and global storage.
    pub fn resolve_runnable(&self, input: &str, limit: usize) -> Result<crate::Runnable> {
        let templates = self.list_templates()?;
        let runs = self.list_runs(limit)?;
        let records = self.global_storage.list_records(limit)?;
        let playlists = self.list_playlists()?;
        crate::storage::resolve_runnable_from_items(input, &templates, &runs, &records, &playlists)
    }

    /// Get the repo URL for a runnable using local overrides when applicable.
    pub fn get_runnable_repo_url(&self, runnable: &crate::Runnable) -> Option<String> {
        crate::storage::runnable_repo_url_with(
            runnable,
            |id| self.load_template(id).map(|(template, _)| template),
            |id| self.global_storage.load_run(id),
            |id| self.global_storage.load_record(id),
        )
    }

    /// Get a display-friendly name for a runnable using local overrides when applicable.
    pub fn get_runnable_display_name(&self, runnable: &crate::Runnable) -> String {
        crate::storage::runnable_display_name_with(
            runnable,
            |id| self.load_template(id).map(|(template, _)| template),
            |id| self.global_storage.load_run(id),
            |id| self.global_storage.load_record(id),
        )
    }

    /// Get tags for a runnable using local overrides when applicable.
    pub fn get_runnable_tags(&self, runnable: &crate::Runnable) -> Vec<String> {
        crate::storage::runnable_tags_with(
            runnable,
            |id| self.load_template(id).map(|(template, _)| template),
            |id| self.global_storage.load_run(id),
            |id| self.global_storage.load_record(id),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tempfile::tempdir;

    #[test]
    fn test_scope_display() {
        assert_eq!(Scope::Local.to_string(), "local");
        assert_eq!(Scope::Global.to_string(), "global");
    }

    #[test]
    fn test_locate_local_runbox_dir() {
        let temp = tempdir().unwrap();
        let runbox_dir = temp.path().join(".runbox");
        fs::create_dir(&runbox_dir).unwrap();

        // Should locate .runbox in current directory
        let located = locate_local_runbox_dir_from(temp.path().to_path_buf());
        assert_eq!(located, Some(runbox_dir.clone()));

        // Should locate .runbox from subdirectory
        let subdir = temp.path().join("subdir");
        fs::create_dir(&subdir).unwrap();
        let located = locate_local_runbox_dir_from(subdir);
        assert_eq!(located, Some(runbox_dir));
    }

    #[test]
    fn test_locate_local_runbox_dir_not_present() {
        let temp = tempdir().unwrap();
        // No .runbox directory
        let located = locate_local_runbox_dir_from(temp.path().to_path_buf());
        assert!(located.is_none());
    }

    #[test]
    fn test_layered_storage_local_overrides_global() {
        let temp = tempdir().unwrap();
        let local_dir = temp.path().join("project").join(".runbox");
        let global_dir = temp.path().join("global");

        fs::create_dir_all(&local_dir).unwrap();
        fs::create_dir_all(&global_dir).unwrap();

        let storage =
            LayeredStorage::with_paths(Some(local_dir.clone()), global_dir.clone()).unwrap();

        // Create a global template
        let global_template = crate::RunTemplate {
            template_version: 0,
            template_id: "tpl_test".to_string(),
            name: "Global Test".to_string(),
            exec: crate::TemplateExec {
                argv: vec!["echo".to_string(), "global".to_string()],
                cwd: ".".to_string(),
                env: HashMap::new(),
                timeout_sec: 0,
            },
            bindings: None,
            tags: Vec::new(),
            code_state: crate::TemplateCodeState {
                repo_url: "git@github.com:org/repo.git".to_string(),
            },
        };
        storage
            .global_storage()
            .save_template(&global_template)
            .unwrap();

        // Create a local template with same ID
        let local_template = crate::RunTemplate {
            template_version: 0,
            template_id: "tpl_test".to_string(),
            name: "Local Test".to_string(),
            exec: crate::TemplateExec {
                argv: vec!["echo".to_string(), "local".to_string()],
                cwd: ".".to_string(),
                env: HashMap::new(),
                timeout_sec: 0,
            },
            bindings: None,
            tags: Vec::new(),
            code_state: crate::TemplateCodeState {
                repo_url: "git@github.com:org/repo.git".to_string(),
            },
        };
        storage
            .save_template(&local_template, Scope::Local)
            .unwrap();

        // Load should return local version
        let (loaded, scope) = storage.load_template("tpl_test").unwrap();
        assert_eq!(scope, Scope::Local);
        assert_eq!(loaded.name, "Local Test");

        // List should only show local version (not both)
        let templates = storage.list_templates_with_scope().unwrap();
        let test_templates: Vec<_> = templates
            .iter()
            .filter(|(t, _)| t.template_id == "tpl_test")
            .collect();
        assert_eq!(test_templates.len(), 1);
        assert_eq!(test_templates[0].1, Scope::Local);
    }

    #[test]
    fn test_layered_storage_global_only() {
        let temp = tempdir().unwrap();
        let global_dir = temp.path().join("global");
        fs::create_dir_all(&global_dir).unwrap();

        // No local directory
        let storage = LayeredStorage::with_paths(None, global_dir.clone()).unwrap();

        assert!(!storage.has_local());

        // Create a global template
        let template = crate::RunTemplate {
            template_version: 0,
            template_id: "tpl_global".to_string(),
            name: "Global Only".to_string(),
            exec: crate::TemplateExec {
                argv: vec!["echo".to_string()],
                cwd: ".".to_string(),
                env: HashMap::new(),
                timeout_sec: 0,
            },
            bindings: None,
            tags: Vec::new(),
            code_state: crate::TemplateCodeState {
                repo_url: "git@github.com:org/repo.git".to_string(),
            },
        };
        storage.global_storage().save_template(&template).unwrap();

        // Load should work
        let (loaded, scope) = storage.load_template("tpl_global").unwrap();
        assert_eq!(scope, Scope::Global);
        assert_eq!(loaded.name, "Global Only");
    }

    #[test]
    fn test_save_to_local_requires_local_dir() {
        let temp = tempdir().unwrap();
        let global_dir = temp.path().join("global");
        fs::create_dir_all(&global_dir).unwrap();

        // No local directory
        let storage = LayeredStorage::with_paths(None, global_dir).unwrap();

        let template = crate::RunTemplate {
            template_version: 0,
            template_id: "tpl_test".to_string(),
            name: "Test".to_string(),
            exec: crate::TemplateExec {
                argv: vec!["echo".to_string()],
                cwd: ".".to_string(),
                env: HashMap::new(),
                timeout_sec: 0,
            },
            bindings: None,
            tags: Vec::new(),
            code_state: crate::TemplateCodeState {
                repo_url: "git@github.com:org/repo.git".to_string(),
            },
        };

        // Saving to local should fail
        let result = storage.save_template(&template, Scope::Local);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("No local .runbox/ directory"));
    }
}
