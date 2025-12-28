//! Tmux runtime adapter
//!
//! Runs processes in tmux windows for easy monitoring and attachment.

use super::RuntimeAdapter;
use crate::run::{Exec, RuntimeHandle};
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

    /// Ensure the tmux session exists, creating it if necessary
    fn ensure_session(&self) -> Result<()> {
        let has_session = Command::new("tmux")
            .args(["has-session", "-t", &self.session_name])
            .output()
            .context("Failed to check tmux session")?
            .status
            .success();

        if !has_session {
            Command::new("tmux")
                .args(["new-session", "-d", "-s", &self.session_name])
                .output()
                .context("Failed to create tmux session")?;
        }

        Ok(())
    }

    /// Get a short window name from run_id
    fn window_name(run_id: &str) -> String {
        // run_id format: "run_{uuid}"
        if run_id.len() >= 12 {
            run_id[4..12].to_string()
        } else {
            run_id.to_string()
        }
    }

    /// Escape a string for shell execution
    fn shell_escape(s: &str) -> String {
        // Simple escaping: wrap in single quotes, escape single quotes
        format!("'{}'", s.replace('\'', "'\"'\"'"))
    }
}

impl RuntimeAdapter for TmuxAdapter {
    fn name(&self) -> &str {
        "tmux"
    }

    fn spawn(&self, exec: &Exec, run_id: &str, log_path: &Path) -> Result<RuntimeHandle> {
        // Ensure session exists
        self.ensure_session()?;

        let window_name = Self::window_name(run_id);

        // Build environment prefix (VAR=value VAR2=value2 ...)
        let env_prefix = exec
            .env
            .iter()
            .map(|(k, v)| format!("{}={}", Self::shell_escape(k), Self::shell_escape(v)))
            .collect::<Vec<_>>()
            .join(" ");

        // Build the command string with proper escaping
        let argv_escaped: Vec<String> = exec.argv.iter().map(|s| Self::shell_escape(s)).collect();
        let cmd_str = argv_escaped.join(" ");

        // Build full command with env prefix and log redirection
        let full_cmd = if env_prefix.is_empty() {
            format!(
                "exec {} > '{}' 2>&1",
                cmd_str,
                log_path.display()
            )
        } else {
            format!(
                "{} exec {} > '{}' 2>&1",
                env_prefix,
                cmd_str,
                log_path.display()
            )
        };

        // Create new window in the session with -c for cwd and bash -lc for execution
        let output = Command::new("tmux")
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
                &full_cmd,
            ])
            .output()
            .context("Failed to create tmux window")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("Failed to create tmux window: {}", stderr);
        }

        Ok(RuntimeHandle::Tmux {
            session: self.session_name.clone(),
            window: window_name,
        })
    }

    fn stop(&self, handle: &RuntimeHandle, _force: bool) -> Result<()> {
        if let RuntimeHandle::Tmux { session, window } = handle {
            let target = format!("{}:{}", session, window);

            // Kill the window (this also kills the process)
            // Note: tmux kill-window sends SIGHUP to the process
            // The force flag is not used for tmux as it handles termination internally
            let output = Command::new("tmux")
                .args(["kill-window", "-t", &target])
                .output()
                .context("Failed to kill tmux window")?;

            // Ignore errors if window doesn't exist
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                // "can't find window" is expected if already closed
                if !stderr.contains("can't find") {
                    bail!("Failed to kill tmux window: {}", stderr);
                }
            }

            Ok(())
        } else {
            bail!("Invalid handle type for TmuxAdapter")
        }
    }

    fn attach(&self, handle: &RuntimeHandle) -> Result<()> {
        if let RuntimeHandle::Tmux { session, window } = handle {
            let target = format!("{}:{}", session, window);

            // Select the window first
            Command::new("tmux")
                .args(["select-window", "-t", &target])
                .output()
                .context("Failed to select tmux window")?;

            // Check if we're already in tmux
            if std::env::var("TMUX").is_ok() {
                // Switch client to the target session
                let err = Command::new("tmux")
                    .args(["switch-client", "-t", session])
                    .exec();
                // exec() replaces the current process, so we only get here on error
                bail!("Failed to switch tmux client: {}", err);
            } else {
                // Attach to the session
                let err = Command::new("tmux")
                    .args(["attach-session", "-t", session])
                    .exec();
                bail!("Failed to attach to tmux session: {}", err);
            }
        } else {
            bail!("Invalid handle type for TmuxAdapter")
        }
    }

    fn is_alive(&self, handle: &RuntimeHandle) -> bool {
        if let RuntimeHandle::Tmux { session, window } = handle {
            // Note: has-session only checks session, not windows
            // Use list-windows to check if specific window exists
            Command::new("tmux")
                .args(["list-windows", "-t", session, "-F", "#{window_name}"])
                .output()
                .map(|o| {
                    String::from_utf8_lossy(&o.stdout)
                        .lines()
                        .any(|line| line == window)
                })
                .unwrap_or(false)
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_window_name() {
        assert_eq!(
            TmuxAdapter::window_name("run_550e8400-e29b-41d4-a716-446655440000"),
            "550e8400"
        );
        assert_eq!(TmuxAdapter::window_name("short"), "short");
    }

    #[test]
    fn test_shell_escape() {
        assert_eq!(TmuxAdapter::shell_escape("hello"), "'hello'");
        assert_eq!(TmuxAdapter::shell_escape("hello world"), "'hello world'");
        assert_eq!(
            TmuxAdapter::shell_escape("it's"),
            "'it'\"'\"'s'"
        );
    }
}
