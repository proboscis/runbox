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
    /// Prefix for session names (e.g., "runbox" -> "runbox-550e8400-e29b-...")
    session_prefix: String,
}

impl ZellijAdapter {
    pub fn new(session_prefix: String) -> Self {
        Self { session_prefix }
    }

    /// Generate session name for a run (uses full UUID to avoid collisions)
    fn session_name(&self, run_id: &str) -> String {
        // run_id format: "run_{uuid}" - use full UUID portion for uniqueness
        let uuid_part = run_id.get(4..).unwrap_or(run_id);
        format!("{}-{}", self.session_prefix, uuid_part)
    }

    /// Get short ID from run_id for display purposes only
    fn short_id(run_id: &str) -> &str {
        // run_id format: "run_{uuid}" - first 8 chars of UUID for display
        run_id.get(4..12).unwrap_or(run_id)
    }

    /// Check if a session exists and is running (not EXITED)
    fn session_is_running(session_name: &str) -> bool {
        let output = Command::new("zellij")
            .args(["list-sessions", "--no-formatting"])
            .output();

        match output {
            Ok(out) if out.status.success() => {
                let sessions = String::from_utf8_lossy(&out.stdout);
                sessions.lines().any(|line| {
                    let trimmed = line.trim();
                    // Session lines: "name" (running) or "name (EXITED)" (dead)
                    // Only consider running if exact match without EXITED
                    if trimmed == session_name {
                        return true;
                    }
                    // Check for "name " prefix but NOT "(EXITED)"
                    if trimmed.starts_with(&format!("{} ", session_name)) {
                        return !trimmed.contains("(EXITED)");
                    }
                    false
                })
            }
            _ => false,
        }
    }

    /// Check if a session exists (running or exited)
    fn session_exists(session_name: &str) -> bool {
        let output = Command::new("zellij")
            .args(["list-sessions", "--no-formatting"])
            .output();

        match output {
            Ok(out) if out.status.success() => {
                let sessions = String::from_utf8_lossy(&out.stdout);
                sessions.lines().any(|line| {
                    let trimmed = line.trim();
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

        // Check if session already exists (collision or leftover)
        if Self::session_exists(&session_name) {
            bail!("Zellij session '{}' already exists. This may indicate a collision or leftover session.", session_name);
        }

        // Build environment using `env` command
        // Note: env keys should be valid identifiers, values are escaped
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

        // Verify session was created with bounded retry
        for _ in 0..5 {
            if Self::session_exists(&session_name) {
                return Ok(RuntimeHandle::Zellij {
                    session: session_name,
                    tab: Self::short_id(run_id).to_string(),
                });
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        bail!("Zellij session '{}' was not created", session_name);
    }

    fn stop(&self, handle: &RuntimeHandle, _force: bool) -> Result<()> {
        if let RuntimeHandle::Zellij { session, .. } = handle {
            let output = Command::new("zellij")
                .args(["kill-session", session])
                .output()
                .context("Failed to kill zellij session")?;

            // Accept success or "session not found" errors
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr).to_lowercase();
                // Ignore various "not found" error messages
                if !stderr.contains("not found")
                    && !stderr.contains("no session")
                    && !stderr.contains("doesn't exist")
                    && !stderr.is_empty()
                {
                    bail!(
                        "Failed to kill zellij session: {}",
                        String::from_utf8_lossy(&output.stderr)
                    );
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
                bail!("Cannot attach from inside zellij. Detach first (Ctrl+O, D) then run: zellij attach {}", session);
            }

            let err = Command::new("zellij").args(["attach", session]).exec();
            bail!("Failed to attach to zellij session: {}", err);
        } else {
            bail!("Invalid handle type for ZellijAdapter")
        }
    }

    fn is_alive(&self, handle: &RuntimeHandle) -> bool {
        if let RuntimeHandle::Zellij { session, .. } = handle {
            // Only consider running sessions as alive, not EXITED ones
            Self::session_is_running(session)
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
        assert_eq!(ZellijAdapter::short_id("run"), "run"); // Edge case
    }

    #[test]
    fn test_session_name_uses_full_uuid() {
        let adapter = ZellijAdapter::new("runbox".to_string());
        let session = adapter.session_name("run_550e8400-e29b-41d4-a716-446655440000");
        // Should use full UUID, not just 8 chars
        assert_eq!(session, "runbox-550e8400-e29b-41d4-a716-446655440000");
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
