//! Zellij runtime adapter
//!
//! Runs processes in zellij tabs for easy monitoring and attachment.

use super::RuntimeAdapter;
use crate::run::{Exec, RuntimeHandle};
use anyhow::{bail, Context, Result};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Command;

/// Adapter for running processes in zellij tabs
pub struct ZellijAdapter {
    session_name: String,
}

impl ZellijAdapter {
    pub fn new(session_name: String) -> Self {
        Self { session_name }
    }

    /// Ensure the zellij session exists, creating it if necessary
    fn ensure_session(&self) -> Result<()> {
        let output = Command::new("zellij")
            .args(["list-sessions"])
            .output()
            .context("Failed to list zellij sessions")?;

        let sessions = String::from_utf8_lossy(&output.stdout);
        let session_exists = sessions
            .lines()
            .any(|line| line.trim() == self.session_name || line.starts_with(&format!("{} ", self.session_name)));

        if !session_exists {
            // Create a new detached session using a background shell process
            let _ = Command::new("sh")
                .args([
                    "-c",
                    &format!(
                        "zellij --session {} &>/dev/null &",
                        Self::shell_escape(&self.session_name)
                    ),
                ])
                .output();
            // Give it a moment to start
            std::thread::sleep(std::time::Duration::from_millis(500));
        }

        Ok(())
    }

    /// Get a short tab name from run_id
    fn tab_name(run_id: &str) -> String {
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

    /// Check if we're currently inside a zellij session
    fn is_inside_zellij() -> bool {
        std::env::var("ZELLIJ").is_ok()
    }

    /// Check if the named tab exists in the session
    fn tab_exists(&self, tab_name: &str) -> bool {
        // Use zellij action query-tab-names to list tabs
        let output = Command::new("zellij")
            .args(["--session", &self.session_name, "action", "query-tab-names"])
            .output();

        match output {
            Ok(out) if out.status.success() => {
                let tabs = String::from_utf8_lossy(&out.stdout);
                tabs.lines().any(|line| line.trim() == tab_name)
            }
            _ => false,
        }
    }
}

impl RuntimeAdapter for ZellijAdapter {
    fn name(&self) -> &str {
        "zellij"
    }

    fn spawn(&self, exec: &Exec, run_id: &str, log_path: &Path) -> Result<RuntimeHandle> {
        // Ensure session exists
        self.ensure_session()?;

        let tab_name = Self::tab_name(run_id);

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
            format!("{} > {} 2>&1", cmd_str, Self::shell_escape(&log_path.display().to_string()))
        } else {
            format!(
                "{} {} > {} 2>&1",
                env_prefix,
                cmd_str,
                Self::shell_escape(&log_path.display().to_string())
            )
        };

        // Use `zellij run` to execute the command in a new tab
        // The --name flag sets the tab name, --cwd sets the working directory
        // We wrap in bash -lc to ensure proper shell environment
        let output = Command::new("zellij")
            .args([
                "--session",
                &self.session_name,
                "run",
                "--name",
                &tab_name,
                "--cwd",
                &exec.cwd,
                "--",
                "bash",
                "-lc",
                &full_cmd,
            ])
            .output()
            .context("Failed to create zellij tab")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("Failed to create zellij tab: {}", stderr);
        }

        Ok(RuntimeHandle::Zellij {
            session: self.session_name.clone(),
            tab: tab_name,
        })
    }

    fn stop(&self, handle: &RuntimeHandle, _force: bool) -> Result<()> {
        if let RuntimeHandle::Zellij { session, tab } = handle {
            // First, try to focus the tab, then close it
            // Note: zellij doesn't have a direct "kill tab by name" command
            // We need to use action go-to-tab-name and then close-tab

            // Try to close the tab using zellij action
            // We run this in the context of the session
            let output = Command::new("zellij")
                .args([
                    "--session",
                    session,
                    "action",
                    "go-to-tab-name",
                    tab,
                ])
                .output()
                .context("Failed to go to zellij tab")?;

            if output.status.success() {
                // Now close the tab
                let close_output = Command::new("zellij")
                    .args(["--session", session, "action", "close-tab"])
                    .output()
                    .context("Failed to close zellij tab")?;

                if !close_output.status.success() {
                    let stderr = String::from_utf8_lossy(&close_output.stderr);
                    // Tab might already be closed
                    if !stderr.contains("no tab") && !stderr.is_empty() {
                        bail!("Failed to close zellij tab: {}", stderr);
                    }
                }
            }
            // If go-to-tab-name fails, the tab might not exist anymore, which is fine

            Ok(())
        } else {
            bail!("Invalid handle type for ZellijAdapter")
        }
    }

    fn attach(&self, handle: &RuntimeHandle) -> Result<()> {
        if let RuntimeHandle::Zellij { session, tab } = handle {
            if Self::is_inside_zellij() {
                // If we're already inside zellij, use action to switch to the tab
                let err = Command::new("zellij")
                    .args(["action", "go-to-tab-name", tab])
                    .exec();
                // exec() replaces the current process, so we only get here on error
                bail!("Failed to switch to zellij tab: {}", err);
            } else {
                // First, attach to the session
                // Then we need to go to the specific tab
                // We use a compound approach: attach and immediately go to tab
                
                // Go to the tab first (this will queue the action)
                let _ = Command::new("zellij")
                    .args(["--session", session, "action", "go-to-tab-name", tab])
                    .output();

                // Then attach to the session
                let err = Command::new("zellij")
                    .args(["attach", session])
                    .exec();
                bail!("Failed to attach to zellij session: {}", err);
            }
        } else {
            bail!("Invalid handle type for ZellijAdapter")
        }
    }

    fn is_alive(&self, handle: &RuntimeHandle) -> bool {
        if let RuntimeHandle::Zellij { session: _, tab } = handle {
            self.tab_exists(tab)
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tab_name() {
        assert_eq!(
            ZellijAdapter::tab_name("run_550e8400-e29b-41d4-a716-446655440000"),
            "550e8400"
        );
        assert_eq!(ZellijAdapter::tab_name("short"), "short");
    }

    #[test]
    fn test_shell_escape() {
        assert_eq!(ZellijAdapter::shell_escape("hello"), "'hello'");
        assert_eq!(ZellijAdapter::shell_escape("hello world"), "'hello world'");
        assert_eq!(ZellijAdapter::shell_escape("it's"), "'it'\"'\"'s'");
    }

    #[test]
    fn test_is_inside_zellij() {
        // This test depends on whether we're running inside zellij
        // Just verify it doesn't panic
        let _ = ZellijAdapter::is_inside_zellij();
    }
}
