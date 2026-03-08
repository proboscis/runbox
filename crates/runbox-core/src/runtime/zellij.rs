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
    session_prefix: String,
}

impl ZellijAdapter {
    pub fn new(session_prefix: String) -> Self {
        Self { session_prefix }
    }

    fn session_name(&self, run_id: &str) -> String {
        format!("{}-{}", self.session_prefix, Self::tab_name(run_id))
    }

    fn tab_name(run_id: &str) -> String {
        run_id.get(4..12).unwrap_or(run_id).to_string()
    }

    fn session_is_running(session_name: &str) -> Result<bool> {
        let output = Command::new("zellij")
            .args(["list-sessions", "--no-formatting"])
            .output()
            .context("Failed to list zellij sessions")?;

        if !output.status.success() {
            return Ok(false);
        }

        let sessions = String::from_utf8_lossy(&output.stdout);
        Ok(sessions.lines().any(|line| {
            let trimmed = line.trim();
            trimmed == session_name
                || (trimmed.starts_with(&format!("{} ", session_name))
                    && !trimmed.contains("(EXITED)"))
        }))
    }

    fn session_exists(session_name: &str) -> Result<bool> {
        let output = Command::new("zellij")
            .args(["list-sessions", "--no-formatting"])
            .output()
            .context("Failed to list zellij sessions")?;

        if !output.status.success() {
            return Ok(false);
        }

        let sessions = String::from_utf8_lossy(&output.stdout);
        Ok(sessions.lines().any(|line| {
            let trimmed = line.trim();
            trimmed == session_name || trimmed.starts_with(&format!("{} ", session_name))
        }))
    }

    fn tab_exists(session_name: &str, tab_name: &str) -> Result<bool> {
        let output = Command::new("zellij")
            .args(["--session", session_name, "action", "query-tab-names"])
            .output()
            .context("Failed to query zellij tab names")?;

        if !output.status.success() {
            return Ok(false);
        }

        let tabs = String::from_utf8_lossy(&output.stdout);
        Ok(tabs.lines().any(|line| line.trim() == tab_name))
    }

    fn shell_escape(s: &str) -> String {
        format!("'{}'", s.replace('\'', "'\"'\"'"))
    }

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
        let tab_name = Self::tab_name(run_id);

        if Self::session_exists(&session_name)? {
            bail!("Zellij session '{}' already exists", session_name);
        }

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

        let argv_escaped: Vec<String> = exec.argv.iter().map(|s| Self::shell_escape(s)).collect();
        let cmd_str = argv_escaped.join(" ");
        let full_cmd = format!(
            "{}exec {} > {} 2>&1",
            env_prefix,
            cmd_str,
            Self::shell_escape(&log_path.display().to_string())
        );

        let session_output = Command::new("zellij")
            .args(["attach", &session_name, "--create-background"])
            .output()
            .context("Failed to create zellij session")?;

        if !session_output.status.success() {
            let stderr = String::from_utf8_lossy(&session_output.stderr);
            bail!(
                "Failed to create zellij session '{}': {}",
                session_name,
                stderr
            );
        }

        let tab_output = Command::new("zellij")
            .args([
                "--session",
                &session_name,
                "action",
                "new-tab",
                "--name",
                &tab_name,
                "--cwd",
                &exec.cwd,
            ])
            .output()
            .context("Failed to create zellij tab")?;

        if !tab_output.status.success() {
            let stderr = String::from_utf8_lossy(&tab_output.stderr);
            bail!("Failed to create zellij tab: {}", stderr);
        }

        let run_output = Command::new("zellij")
            .args([
                "--session",
                &session_name,
                "run",
                "--cwd",
                &exec.cwd,
                "--close-on-exit",
                "--",
                "bash",
                "-lc",
                &full_cmd,
            ])
            .output()
            .context("Failed to run command in zellij")?;

        if !run_output.status.success() {
            let stderr = String::from_utf8_lossy(&run_output.stderr);
            bail!("Failed to run command in zellij: {}", stderr);
        }

        if !Self::tab_exists(&session_name, &tab_name)? {
            bail!("Zellij tab '{}:{}' was not created", session_name, tab_name);
        }

        Ok(RuntimeHandle::Zellij {
            session: session_name,
            tab: tab_name,
        })
    }

    fn stop(&self, handle: &RuntimeHandle, _force: bool) -> Result<()> {
        if let RuntimeHandle::Zellij { session, .. } = handle {
            let output = Command::new("zellij")
                .args(["kill-session", session])
                .output()
                .context("Failed to kill zellij session")?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let lower = stderr.to_lowercase();
                if !lower.contains("not found")
                    && !lower.contains("no session")
                    && !lower.contains("doesn't exist")
                    && !stderr.trim().is_empty()
                {
                    bail!("Failed to kill zellij session '{}': {}", session, stderr);
                }
            }

            Ok(())
        } else {
            bail!("Invalid handle type for ZellijAdapter")
        }
    }

    fn attach(&self, handle: &RuntimeHandle) -> Result<()> {
        if let RuntimeHandle::Zellij { session, tab } = handle {
            if !Self::tab_exists(session, tab)? {
                bail!("Zellij tab '{}:{}' not found", session, tab);
            }

            if Self::is_inside_zellij() {
                bail!(
                    "Cannot attach from inside zellij. Detach first (Ctrl+O, D) then run: zellij attach {}",
                    session
                );
            }

            let err = Command::new("zellij").args(["attach", session]).exec();
            bail!("Failed to attach to zellij session: {}", err);
        } else {
            bail!("Invalid handle type for ZellijAdapter")
        }
    }

    fn is_alive(&self, handle: &RuntimeHandle) -> bool {
        if let RuntimeHandle::Zellij { session, tab } = handle {
            Self::session_is_running(session).unwrap_or(false)
                && Self::tab_exists(session, tab).unwrap_or(false)
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
            "550e8400".to_string()
        );
        assert_eq!(ZellijAdapter::tab_name("short"), "short".to_string());
        assert_eq!(ZellijAdapter::tab_name("run"), "run".to_string());
    }

    #[test]
    fn test_session_name_uses_short_id() {
        let adapter = ZellijAdapter::new("runbox".to_string());
        let session = adapter.session_name("run_550e8400-e29b-41d4-a716-446655440000");
        assert_eq!(session, "runbox-550e8400");
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
