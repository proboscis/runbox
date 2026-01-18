//! Zellij runtime adapter
//!
//! Runs processes in zellij sessions for easy monitoring and attachment.
//! Uses one session per run for reliable stop/attach semantics.

use super::RuntimeAdapter;
use crate::run::{Exec, RuntimeHandle};
use anyhow::{bail, Context, Result};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Command;

/// Adapter for running processes in zellij sessions (one session per run)
pub struct ZellijAdapter {
    /// Prefix for session names (e.g., "runbox" -> "runbox-550e8400")
    session_prefix: String,
}

impl ZellijAdapter {
    pub fn new(session_prefix: String) -> Self {
        Self { session_prefix }
    }

    /// Generate session name for a run
    fn session_name(&self, run_id: &str) -> String {
        format!("{}-{}", self.session_prefix, Self::short_id(run_id))
    }

    /// Get short ID from run_id (first 8 chars of UUID)
    fn short_id(run_id: &str) -> &str {
        // run_id format: "run_{uuid}"
        if run_id.len() >= 12 {
            &run_id[4..12]
        } else {
            run_id
        }
    }

    /// Check if a session exists
    fn session_exists(session_name: &str) -> bool {
        let output = Command::new("zellij")
            .args(["list-sessions", "--no-formatting"])
            .output();

        match output {
            Ok(out) if out.status.success() => {
                let sessions = String::from_utf8_lossy(&out.stdout);
                sessions.lines().any(|line| {
                    let trimmed = line.trim();
                    // Session lines: "name" or "name (EXITED)"
                    trimmed == session_name || trimmed.starts_with(&format!("{} ", session_name))
                })
            }
            _ => false,
        }
    }

    /// Escape a string for shell execution
    fn shell_escape(s: &str) -> String {
        format!("'{}'", s.replace('\'', "'\"'\"'"))
    }

    /// Check if we're currently inside a zellij session
    fn is_inside_zellij() -> bool {
        std::env::var("ZELLIJ").is_ok()
    }
}

impl RuntimeAdapter for ZellijAdapter {
    fn name(&self) -> &str {
        "zellij"
    }

    fn spawn(&self, exec: &Exec, run_id: &str, log_path: &Path) -> Result<RuntimeHandle> {
        let session_name = self.session_name(run_id);

        // Build environment using `env` command
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

        // Build command with escaping
        let argv_escaped: Vec<String> = exec.argv.iter().map(|s| Self::shell_escape(s)).collect();
        let cmd_str = argv_escaped.join(" ");

        // Full command with log redirection
        let full_cmd = format!(
            "{}exec {} > {} 2>&1",
            env_prefix,
            cmd_str,
            Self::shell_escape(&log_path.display().to_string())
        );

        // Create a new zellij session running the command
        // Using `zellij --session <name> options --default-cwd <cwd>` then `zellij run`
        // Or simpler: use `zellij -s <session> run` which creates session if needed
        let output = Command::new("zellij")
            .args([
                "--session",
                &session_name,
                "run",
                "--cwd",
                &exec.cwd,
                "--",
                "bash",
                "-lc",
                &full_cmd,
            ])
            .output()
            .context("Failed to create zellij session. Is zellij installed?")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("Failed to create zellij session: {}", stderr);
        }

        // Verify session was created
        if !Self::session_exists(&session_name) {
            // Give it a moment and retry check
            std::thread::sleep(std::time::Duration::from_millis(200));
            if !Self::session_exists(&session_name) {
                bail!("Zellij session '{}' was not created", session_name);
            }
        }

        Ok(RuntimeHandle::Zellij {
            session: session_name,
            tab: Self::short_id(run_id).to_string(), // Keep for compatibility
        })
    }

    fn stop(&self, handle: &RuntimeHandle, _force: bool) -> Result<()> {
        if let RuntimeHandle::Zellij { session, .. } = handle {
            // Kill the entire session - this is reliable
            let output = Command::new("zellij")
                .args(["kill-session", session])
                .output()
                .context("Failed to kill zellij session")?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                // Session might already be gone
                if !stderr.contains("not found") && !stderr.contains("No session") && !stderr.is_empty() {
                    bail!("Failed to kill zellij session: {}", stderr);
                }
            }

            Ok(())
        } else {
            bail!("Invalid handle type for ZellijAdapter")
        }
    }

    fn attach(&self, handle: &RuntimeHandle) -> Result<()> {
        if let RuntimeHandle::Zellij { session, .. } = handle {
            if Self::is_inside_zellij() {
                // Can't attach to another session from inside zellij easily
                // User needs to detach first or use zellij's session switcher
                bail!("Cannot attach from inside zellij. Detach first (Ctrl+O, D) then run: zellij attach {}", session);
            }

            // Attach to the session
            let err = Command::new("zellij")
                .args(["attach", session])
                .exec();
            // exec() replaces current process, only get here on error
            bail!("Failed to attach to zellij session: {}", err);
        } else {
            bail!("Invalid handle type for ZellijAdapter")
        }
    }

    fn is_alive(&self, handle: &RuntimeHandle) -> bool {
        if let RuntimeHandle::Zellij { session, .. } = handle {
            Self::session_exists(session)
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_short_id() {
        assert_eq!(
            ZellijAdapter::short_id("run_550e8400-e29b-41d4-a716-446655440000"),
            "550e8400"
        );
        assert_eq!(ZellijAdapter::short_id("short"), "short");
    }

    #[test]
    fn test_session_name() {
        let adapter = ZellijAdapter::new("runbox".to_string());
        assert_eq!(
            adapter.session_name("run_550e8400-e29b-41d4-a716-446655440000"),
            "runbox-550e8400"
        );
    }

    #[test]
    fn test_shell_escape() {
        assert_eq!(ZellijAdapter::shell_escape("hello"), "'hello'");
        assert_eq!(ZellijAdapter::shell_escape("hello world"), "'hello world'");
        assert_eq!(ZellijAdapter::shell_escape("it's"), "'it'\"'\"'s'");
    }

    #[test]
    fn test_is_inside_zellij() {
        let _ = ZellijAdapter::is_inside_zellij();
    }
}
