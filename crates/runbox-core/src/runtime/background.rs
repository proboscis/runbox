//! Background runtime adapter for running processes in the background.

use super::RuntimeAdapter;
use crate::{Exec, RuntimeHandle};
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
        let log_file = File::create(log_path)?;

        let mut cmd = Command::new(&exec.argv[0]);
        cmd.args(&exec.argv[1..])
            .current_dir(&exec.cwd)
            .envs(&exec.env)
            .stdout(Stdio::from(log_file.try_clone()?))
            .stderr(Stdio::from(log_file));

        // Create a new process group so we can kill all children
        cmd.process_group(0);

        let child = cmd.spawn()?;

        let pid = child.id();
        let pgid = pid; // process_group(0) means pid == pgid

        // Spawn a thread to wait for the process and clean up
        // Note: We're not updating the run status here - that's handled by the caller
        std::thread::spawn(move || {
            let _ = child.wait_with_output();
        });

        Ok(RuntimeHandle::Background { pid, pgid })
    }

    fn stop(&self, handle: &RuntimeHandle, force: bool) -> Result<()> {
        if let RuntimeHandle::Background { pgid, .. } = handle {
            // Send SIGTERM (default) or SIGKILL (force)
            let signal = if force { libc::SIGKILL } else { libc::SIGTERM };
            unsafe {
                let ret = libc::killpg(*pgid as i32, signal);
                if ret != 0 {
                    // Process may already be dead, which is fine
                    let errno = std::io::Error::last_os_error();
                    if errno.raw_os_error() != Some(libc::ESRCH) {
                        bail!("Failed to kill process group: {}", errno);
                    }
                }
            }
        }
        Ok(())
    }

    fn attach(&self, _handle: &RuntimeHandle) -> Result<()> {
        bail!("Background runtime does not support attach")
    }

    fn is_alive(&self, handle: &RuntimeHandle) -> bool {
        if let RuntimeHandle::Background { pid, .. } = handle {
            // Use kill -0 to check if process exists
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
    use tempfile::tempdir;

    #[test]
    fn test_background_spawn_and_stop() {
        let adapter = BackgroundAdapter::new();
        let dir = tempdir().unwrap();
        let log_path = dir.path().join("test.log");

        let exec = Exec {
            argv: vec!["sleep".to_string(), "10".to_string()],
            cwd: ".".to_string(),
            env: HashMap::new(),
            timeout_sec: 0,
        };

        let handle = adapter.spawn(&exec, "test_run", &log_path).unwrap();

        // Should be alive
        assert!(adapter.is_alive(&handle));

        // Stop it (graceful SIGTERM)
        adapter.stop(&handle, false).unwrap();

        // Give it a moment to die
        std::thread::sleep(std::time::Duration::from_millis(100));

        // Should be dead now
        assert!(!adapter.is_alive(&handle));
    }

    #[test]
    fn test_background_quick_exit() {
        let adapter = BackgroundAdapter::new();
        let dir = tempdir().unwrap();
        let log_path = dir.path().join("test.log");

        let exec = Exec {
            argv: vec!["echo".to_string(), "hello".to_string()],
            cwd: ".".to_string(),
            env: HashMap::new(),
            timeout_sec: 0,
        };

        let handle = adapter.spawn(&exec, "test_run", &log_path).unwrap();

        // Give it time to finish
        std::thread::sleep(std::time::Duration::from_millis(100));

        // Should be dead
        assert!(!adapter.is_alive(&handle));

        // Check log content
        let content = std::fs::read_to_string(&log_path).unwrap();
        assert!(content.contains("hello"));
    }
}
