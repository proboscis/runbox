//! Runtime adapters for process execution management
//!
//! This module provides abstractions for running processes in different
//! execution environments (background, tmux, zellij).

mod background;
mod tmux;
mod zellij;

pub use background::BackgroundAdapter;
pub use tmux::TmuxAdapter;
pub use zellij::ZellijAdapter;

use crate::run::{Exec, RuntimeHandle};
use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;

/// Trait for runtime adapters that manage process execution
pub trait RuntimeAdapter: Send + Sync {
    /// Get the runtime name (e.g., "background", "tmux", "zellij")
    fn name(&self) -> &str;

    /// Spawn a process with the given exec configuration
    ///
    /// - exec: The command and environment to execute
    /// - run_id: Unique identifier for this run (used for naming windows, etc.)
    /// - log_path: Path where stdout/stderr should be written
    fn spawn(&self, exec: &Exec, run_id: &str, log_path: &Path) -> Result<RuntimeHandle>;

    /// Stop a running process
    /// - force=false: Send SIGTERM only (graceful)
    /// - force=true: Send SIGKILL (immediate)
    fn stop(&self, handle: &RuntimeHandle, force: bool) -> Result<()>;

    /// Attach to a running process (for interactive terminals)
    fn attach(&self, handle: &RuntimeHandle) -> Result<()>;

    /// Check if the process is still alive
    fn is_alive(&self, handle: &RuntimeHandle) -> bool;
}

/// Registry of available runtime adapters
pub struct RuntimeRegistry {
    adapters: HashMap<String, Box<dyn RuntimeAdapter>>,
}

impl RuntimeRegistry {
    /// Create a new registry with default adapters
    pub fn new() -> Self {
        let mut adapters: HashMap<String, Box<dyn RuntimeAdapter>> = HashMap::new();
        adapters.insert("background".to_string(), Box::new(BackgroundAdapter::new()));
        adapters.insert("bg".to_string(), Box::new(BackgroundAdapter::new()));
        adapters.insert(
            "tmux".to_string(),
            Box::new(TmuxAdapter::new("runbox".to_string())),
        );
        adapters.insert(
            "zellij".to_string(),
            Box::new(ZellijAdapter::new("runbox".to_string())),
        );
        Self { adapters }
    }

    /// Get an adapter by name
    pub fn get(&self, name: &str) -> Option<&dyn RuntimeAdapter> {
        self.adapters.get(name).map(|a| a.as_ref())
    }

    /// List available adapter names
    pub fn available(&self) -> Vec<&str> {
        self.adapters.keys().map(|s| s.as_str()).collect()
    }
}

impl Default for RuntimeRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_default_adapters() {
        let registry = RuntimeRegistry::new();
        assert!(registry.get("background").is_some());
        assert!(registry.get("bg").is_some());
        assert!(registry.get("tmux").is_some());
        assert!(registry.get("zellij").is_some());
        assert!(registry.get("nonexistent").is_none());
    }
}
