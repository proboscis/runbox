//! Tmux runtime adapter for running processes in tmux windows.

use super::RuntimeAdapter;
use crate::{Exec, RuntimeHandle};
use anyhow::{bail, Context, Result};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Command;

/// Adapter for running processes in tmux windows
pub struct TmuxAdapter {
    session_name: String,
}

impl TmuxAdapter {
    pub fn new(session_name: String) -> Self {
        Self { session_name }
    }

    /// Ensure the tmux session exists
    fn ensure_session(&self) -> Result<()> {
        let has_session = Command::new("tmux")
            .args(["has-session", "-t", &self.session_name])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        if !has_session {
            let status = Command::new("tmux")
                .args(["new-session", "-d", "-s", &self.session_name])
                .status()
                .context("Failed to create tmux session")?;

            if !status.success() {
                bail!("Failed to create tmux session: {}", self.session_name);
            }
        }

        Ok(())
    }

    /// Get short ID from run_id string
    fn short_id(run_id: &str) -> String {
        let uuid_part = run_id.strip_prefix("run_").unwrap_or(run_id);
        if uuid_part.len() >= 8 {
            uuid_part[..8].to_string()
        } else {
            uuid_part.to_string()
        }
    }
}

impl RuntimeAdapter for TmuxAdapter {
    fn name(&self) -> &str {
        "tmux"
    }

    fn spawn(&self, exec: &Exec, run_id: &str, log_path: &Path) -> Result<RuntimeHandle> {
        // Ensure session exists
        self.ensure_session()?;

        // Window name from short run_id
        let window_name = Self::short_id(run_id);

        // Build env prefix (VAR=value format)
        // Note: .envs() only affects tmux command, not the spawned shell
        let env_prefix = exec
            .env
            .iter()
            .map(|(k, v)| format!("{}={}", shell_escape(k), shell_escape(v)))
            .collect::<Vec<_>>()
            .join(" ");

        // Build the command string with output redirection
        // Use single quotes for log_path as per spec
        let cmd_str = format!(
            "{} exec {} > '{}' 2>&1",
            env_prefix,
            shell_escape_argv(&exec.argv),
            log_path.display()
        );

        // Trim leading space if no env vars
        let cmd_str = cmd_str.trim_start();

        // Create new window and run command with bash -lc
        // Note: -c option sets the working directory for the new window
        let status = Command::new("tmux")
            .args([
                "new-window",
                "-t",
                &self.session_name,
                "-n",
                &window_name,
                "-c",
                &exec.cwd,
                "bash",
                "-lc",
                cmd_str,
            ])
            .status()
            .context("Failed to create tmux window")?;

        if !status.success() {
            bail!("Failed to create tmux window for run: {}", run_id);
        }

        Ok(RuntimeHandle::Tmux {
            session: self.session_name.clone(),
            window: window_name,
        })
    }

    fn stop(&self, handle: &RuntimeHandle, _force: bool) -> Result<()> {
        // For tmux, kill-window sends SIGHUP to processes
        // The force parameter doesn't change behavior for tmux
        // (kill-window is the same regardless)
        if let RuntimeHandle::Tmux { session, window } = handle {
            let target = format!("{}:{}", session, window);
            let status = Command::new("tmux")
                .args(["kill-window", "-t", &target])
                .status()
                .context("Failed to kill tmux window")?;

            if !status.success() {
                // Window may already be dead
                eprintln!("Warning: tmux window may already be closed");
            }
        }
        Ok(())
    }

    fn attach(&self, handle: &RuntimeHandle) -> Result<()> {
        if let RuntimeHandle::Tmux { session, window } = handle {
            let target = format!("{}:{}", session, window);

            // First, select the window
            let _ = Command::new("tmux")
                .args(["select-window", "-t", &target])
                .status();

            // Check if we're already inside tmux
            if std::env::var("TMUX").is_ok() {
                // Switch client to the session
                let err = Command::new("tmux")
                    .args(["switch-client", "-t", session])
                    .exec();
                bail!("Failed to switch tmux client: {:?}", err);
            } else {
                // Attach to the session
                let err = Command::new("tmux")
                    .args(["attach", "-t", session])
                    .exec();
                bail!("Failed to attach to tmux: {:?}", err);
            }
        }
        Ok(())
    }

    fn is_alive(&self, handle: &RuntimeHandle) -> bool {
        if let RuntimeHandle::Tmux { session, window } = handle {
            // Check if window exists using list-windows
            Command::new("tmux")
                .args(["list-windows", "-t", session, "-F", "#{window_name}"])
                .output()
                .map(|o| {
                    let output = String::from_utf8_lossy(&o.stdout);
                    output.lines().any(|line| line == window)
                })
                .unwrap_or(false)
        } else {
            false
        }
    }
}

/// Escape a string for shell use
fn shell_escape(s: &str) -> String {
    if s.chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == '.' || c == '/')
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

/// Escape an argv array for shell use
fn shell_escape_argv(argv: &[String]) -> String {
    argv.iter().map(|s| shell_escape(s)).collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shell_escape() {
        assert_eq!(shell_escape("hello"), "hello");
        assert_eq!(shell_escape("hello world"), "'hello world'");
        assert_eq!(shell_escape("it's"), "'it'\\''s'");
        assert_eq!(shell_escape("/path/to/file"), "/path/to/file");
    }

    #[test]
    fn test_shell_escape_argv() {
        let argv = vec!["echo".to_string(), "hello world".to_string()];
        assert_eq!(shell_escape_argv(&argv), "echo 'hello world'");
    }

    #[test]
    fn test_short_id() {
        assert_eq!(
            TmuxAdapter::short_id("run_550e8400-e29b-41d4-a716-446655440000"),
            "550e8400"
        );
        assert_eq!(TmuxAdapter::short_id("run_short"), "short");
    }
}
