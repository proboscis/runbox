//! Background process adapter
//!
//! Runs processes in the background using process groups for proper cleanup.

use super::RuntimeAdapter;
use crate::run::{Exec, RuntimeHandle};
use anyhow::{bail, Result};
use std::fs::File;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};

/// Adapter for running processes in the background
pub struct BackgroundAdapter;

impl BackgroundAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for BackgroundAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeAdapter for BackgroundAdapter {
    fn name(&self) -> &str {
        "background"
    }

    fn spawn(&self, exec: &Exec, _run_id: &str, log_path: &Path) -> Result<RuntimeHandle> {
        // Create log file for stdout/stderr
        let log_file = File::create(log_path)?;
        let log_file_err = log_file.try_clone()?;

        // Build the command
        let mut cmd = Command::new(&exec.argv[0]);
        cmd.args(&exec.argv[1..])
            .current_dir(&exec.cwd)
            .stdout(Stdio::from(log_file))
            .stderr(Stdio::from(log_file_err));

        // Add environment variables
        for (key, value) in &exec.env {
            cmd.env(key, value);
        }

        // Create new process group (pid == pgid)
        // SAFETY: pre_exec is called after fork but before exec
        unsafe {
            cmd.pre_exec(|| {
                // Create new process group
                libc::setpgid(0, 0);
                Ok(())
            });
        }

        // Spawn the process
        let child = cmd.spawn()?;
        let pid = child.id();
        let pgid = pid; // Due to setpgid(0, 0), pid == pgid

        // Note: We intentionally don't call child.wait() here.
        // The process runs in the background.
        // Status updates are handled by reconcile or a separate mechanism.
        std::mem::forget(child);

        Ok(RuntimeHandle::Background { pid, pgid })
    }

    fn stop(&self, handle: &RuntimeHandle) -> Result<()> {
        if let RuntimeHandle::Background { pid, pgid } = handle {
            // First try SIGTERM to the process group
            // SAFETY: killpg is safe with valid pgid
            let result = unsafe { libc::killpg(*pgid as i32, libc::SIGTERM) };
            if result != 0 {
                let err = std::io::Error::last_os_error();
                // ESRCH means no such process - try killing just the pid
                if err.raw_os_error() == Some(libc::ESRCH) {
                    // Process group doesn't exist, try direct kill
                    unsafe { libc::kill(*pid as i32, libc::SIGTERM) };
                } else {
                    return Err(err.into());
                }
            }

            // Give it a moment to terminate gracefully
            std::thread::sleep(std::time::Duration::from_millis(100));

            // If still alive, send SIGKILL
            if self.is_alive(handle) {
                unsafe {
                    libc::killpg(*pgid as i32, libc::SIGKILL);
                    libc::kill(*pid as i32, libc::SIGKILL);
                };
            }

            Ok(())
        } else {
            bail!("Invalid handle type for BackgroundAdapter")
        }
    }

    fn attach(&self, _handle: &RuntimeHandle) -> Result<()> {
        bail!("Background runtime does not support attach")
    }

    fn is_alive(&self, handle: &RuntimeHandle) -> bool {
        if let RuntimeHandle::Background { pid, .. } = handle {
            // Try to reap any zombie first (non-blocking)
            // SAFETY: waitpid is safe with WNOHANG
            unsafe {
                let mut status = 0i32;
                let result = libc::waitpid(*pid as i32, &mut status, libc::WNOHANG);
                // If waitpid returns the pid, the process exited
                if result == *pid as i32 {
                    return false;
                }
            }

            // Check if process exists using kill with signal 0
            // SAFETY: kill with signal 0 just checks existence
            unsafe { libc::kill(*pid as i32, 0) == 0 }
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::thread;
    use std::time::Duration;
    use tempfile::tempdir;

    #[test]
    fn test_spawn_and_stop() {
        let adapter = BackgroundAdapter::new();
        let dir = tempdir().unwrap();
        let log_path = dir.path().join("test.log");

        let exec = Exec {
            argv: vec!["sleep".to_string(), "60".to_string()],
            cwd: ".".to_string(),
            env: HashMap::new(),
            timeout_sec: 0,
        };

        let handle = adapter.spawn(&exec, "test_run", &log_path).unwrap();

        // Process should be alive
        assert!(adapter.is_alive(&handle));

        // Stop the process
        adapter.stop(&handle).unwrap();

        // Wait for process to terminate with retries
        // is_alive() will reap any zombie processes
        for _ in 0..20 {
            if !adapter.is_alive(&handle) {
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }

        // Process should be dead
        assert!(!adapter.is_alive(&handle));
    }

    #[test]
    fn test_spawn_with_output() {
        let adapter = BackgroundAdapter::new();
        let dir = tempdir().unwrap();
        let log_path = dir.path().join("test.log");

        let exec = Exec {
            argv: vec!["echo".to_string(), "hello world".to_string()],
            cwd: ".".to_string(),
            env: HashMap::new(),
            timeout_sec: 0,
        };

        let handle = adapter.spawn(&exec, "test_run", &log_path).unwrap();

        // Wait for command to complete with retries
        for _ in 0..40 {
            if !adapter.is_alive(&handle) {
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }

        // Check log output (the process may or may not be alive, just check output)
        let output = std::fs::read_to_string(&log_path).unwrap_or_default();
        assert!(output.contains("hello world"), "Output was: {}", output);
    }
}
