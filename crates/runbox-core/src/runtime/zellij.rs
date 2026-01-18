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
        // Use --no-formatting to avoid ANSI escape codes in output
        let output = Command::new("zellij")
            .args(["list-sessions", "--no-formatting"])
            .output()
            .context("Failed to list zellij sessions. Is zellij installed?")?;

        let sessions = String::from_utf8_lossy(&output.stdout);
        let session_exists = sessions
            .lines()
            .any(|line| {
                // Session lines may have format "session_name" or "session_name (EXITED)"
                let trimmed = line.trim();
                trimmed == self.session_name 
                    || trimmed.starts_with(&format!("{} ", self.session_name))
            });

        if !session_exists {
            // Create a new session in the background using zellij's built-in mechanism
            // Note: We start a session that will run detached
            let create_output = Command::new("zellij")
                .args(["--session", &self.session_name, "options", "--detached"])
                .output();
            
            // If that fails, try the attach with create approach
            if create_output.is_err() || !create_output.as_ref().unwrap().status.success() {
                // Fallback: use attach --create which creates if not exists
                let _ = Command::new("zellij")
                    .args(["attach", "--create", &self.session_name])
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn();
                
                // Wait and verify session exists with bounded retry
                for _ in 0..10 {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    let check = Command::new("zellij")
                        .args(["list-sessions", "--no-formatting"])
                        .output();
                    if let Ok(out) = check {
                        let sessions = String::from_utf8_lossy(&out.stdout);
                        if sessions.lines().any(|l| l.trim().starts_with(&self.session_name)) {
                            return Ok(());
                        }
                    }
                }
            }
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

    /// Escape a string for shell execution (for values only)
    fn shell_escape(s: &str) -> String {
        // Simple escaping: wrap in single quotes, escape single quotes
        format!("'{}'", s.replace('\'', "'\"'\"'"))
    }

    /// Check if we're currently inside a zellij session
    fn is_inside_zellij() -> bool {
        std::env::var("ZELLIJ").is_ok()
    }

    /// Check if the named tab exists in the session
    fn tab_exists(session: &str, tab_name: &str) -> bool {
        // Use zellij action query-tab-names to list tabs
        let output = Command::new("zellij")
            .args(["--session", session, "action", "query-tab-names"])
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

        // Build environment prefix using `env` command for proper syntax
        // Format: env KEY='value' KEY2='value2' ...
        let env_args: Vec<String> = exec
            .env
            .iter()
            .map(|(k, v)| format!("{}={}", k, Self::shell_escape(v)))
            .collect();
        
        let env_prefix = if env_args.is_empty() {
            String::new()
        } else {
            format!("env {} ", env_args.join(" "))
        };

        // Build the command string with proper escaping
        let argv_escaped: Vec<String> = exec.argv.iter().map(|s| Self::shell_escape(s)).collect();
        let cmd_str = argv_escaped.join(" ");

        // Build full command with env prefix and log redirection
        // Using exec to replace the shell process with the command
        let full_cmd = format!(
            "{}exec {} > {} 2>&1",
            env_prefix,
            cmd_str,
            Self::shell_escape(&log_path.display().to_string())
        );

        // Use `zellij run` to execute the command in a new pane/tab
        // The --name flag sets the pane name, --cwd sets the working directory
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
            .context("Failed to create zellij pane")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("Failed to create zellij pane: {}", stderr);
        }

        Ok(RuntimeHandle::Zellij {
            session: self.session_name.clone(),
            tab: tab_name,
        })
    }

    fn stop(&self, handle: &RuntimeHandle, _force: bool) -> Result<()> {
        if let RuntimeHandle::Zellij { session, tab } = handle {
            // Try to focus the pane/tab by name, then close it
            // Note: This approach has limitations - it may not work reliably
            // if no client is attached to the session

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
                // Now close the tab/pane
                let close_output = Command::new("zellij")
                    .args(["--session", session, "action", "close-tab"])
                    .output()
                    .context("Failed to close zellij tab")?;

                if !close_output.status.success() {
                    let stderr = String::from_utf8_lossy(&close_output.stderr);
                    // Tab might already be closed or not found
                    if !stderr.contains("no tab") && !stderr.contains("not found") && !stderr.is_empty() {
                        bail!("Failed to close zellij tab: {}", stderr);
                    }
                }
            }
            // If go-to-tab-name fails, the tab/pane might not exist anymore, which is fine

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
                // Try to focus the tab first (best effort)
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
        if let RuntimeHandle::Zellij { session, tab } = handle {
            // Use the session from the handle, not self.session_name
            Self::tab_exists(session, tab)
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

    #[test]
    fn test_env_prefix_format() {
        // Verify the env prefix format is valid bash
        let adapter = ZellijAdapter::new("test".to_string());
        let escaped = ZellijAdapter::shell_escape("value with spaces");
        // Should produce: KEY='value with spaces'
        let env_arg = format!("KEY={}", escaped);
        assert_eq!(env_arg, "KEY='value with spaces'");
    }
}
