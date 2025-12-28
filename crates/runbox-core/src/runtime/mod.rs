//! Runtime adapter module for managing different execution environments.

mod background;
mod tmux;

pub use background::BackgroundAdapter;
pub use tmux::TmuxAdapter;

use crate::{Exec, RuntimeHandle};
use anyhow::Result;
use std::path::Path;

/// Trait for runtime adapters that spawn and manage processes
pub trait RuntimeAdapter: Send + Sync {
    /// Runtime name ("background", "tmux", "zellij")
    fn name(&self) -> &str;

    /// Spawn a process
    ///
    /// # Arguments
    /// * `exec` - The execution specification
    /// * `run_id` - The run identifier (used for naming windows/tabs)
    /// * `log_path` - Path for stdout/stderr output
    ///
    /// # Returns
    /// A RuntimeHandle containing runtime-specific data
    fn spawn(&self, exec: &Exec, run_id: &str, log_path: &Path) -> Result<RuntimeHandle>;

    /// Stop a running process
    fn stop(&self, handle: &RuntimeHandle) -> Result<()>;

    /// Attach to a running process (terminal takeover)
    fn attach(&self, handle: &RuntimeHandle) -> Result<()>;

    /// Check if the process is still alive (for reconcile)
    fn is_alive(&self, handle: &RuntimeHandle) -> bool;
}

/// Get a runtime adapter by name
pub fn get_adapter(name: &str) -> Option<Box<dyn RuntimeAdapter>> {
    match name {
        "background" | "bg" => Some(Box::new(BackgroundAdapter::new())),
        "tmux" => Some(Box::new(TmuxAdapter::new("runbox".to_string()))),
        _ => None,
    }
}

/// List available runtime names
pub fn available_runtimes() -> Vec<&'static str> {
    vec!["background", "tmux"]
}
