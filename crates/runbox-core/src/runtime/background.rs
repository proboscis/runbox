//! Background process adapter
//!
//! Runs processes in the background using process groups for proper cleanup.

use super::RuntimeAdapter;
use crate::run::{Exec, RuntimeHandle};
use crate::storage::update_run_on_exit;
use anyhow::{bail, Result};
use std::fs::File;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};

/// Adapter for running processes in the background
pub struct BackgroundAdapter;

impl BackgroundAdapter {
    pub fn new() -> Self {
        Self
    }

    /// Wait for a child process to exit and update the run status
    ///
    /// This method runs in a spawned thread and blocks until the child process exits.
    /// When the process exits, it captures the exit code and updates the Run status
    /// in storage using CAS-style update.
    ///
    /// # Exit Code Handling
    ///
    /// - Normal exit: Uses the exit code from the process
    /// - Signal termination (e.g., SIGKILL): Uses 128 + signal_number convention
    /// - Unknown termination: Uses -1 as a sentinel value
    ///
    /// # Limitations
    ///
    /// This thread is tied to the CLI process lifetime. If the CLI exits before
    /// the background process completes:
    /// - The exit status will not be captured by this thread
    /// - The run status will remain Running until reconcile detects the process is gone
    /// - Reconcile will then set status to Unknown
    ///
    /// For short-lived commands (like `echo`, `true`, `false`), this approach works well
    /// because the process typically exits before the CLI does. For long-running
    /// background processes, reconcile provides the fallback behavior.
    fn wait_for_exit(mut child: std::process::Child, run_id: String) {
        match child.wait() {
            Ok(status) => {
                let exit_code = Self::extract_exit_code(&status);
                if let Err(e) = update_run_on_exit(&run_id, exit_code) {
                    eprintln!("Failed to update run status for {}: {}", run_id, e);
                }
            }
            Err(e) => {
                eprintln!("Failed to wait for process {}: {}", run_id, e);
                // Try to update with error status
                if let Err(e) = update_run_on_exit(&run_id, -1) {
                    eprintln!("Failed to update run status for {}: {}", run_id, e);
                }
            }
        }
    }

    /// Extract exit code from ExitStatus, handling both normal exits and signals
    ///
    /// On Unix, processes can exit normally (with a code) or be killed by a signal.
    /// This function returns:
    /// - The exit code if the process exited normally
    /// - 128 + signal_number if killed by a signal (common shell convention)
    /// - -1 if neither is available (should not happen in practice)
    fn extract_exit_code(status: &ExitStatus) -> i32 {
        // First try to get the exit code (normal exit)
        if let Some(code) = status.code() {
            return code;
        }

        // If no exit code, check for signal (Unix only)
        if let Some(signal) = status.signal() {
            // Use shell convention: 128 + signal_number
            // This allows distinguishing signal exits from normal exits
            return 128 + signal;
        }

        // Fallback (should not reach here on Unix)
        -1
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

    fn spawn(&self, exec: &Exec, run_id: &str, log_path: &Path) -> Result<RuntimeHandle> {
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

        // Spawn a thread to wait for process exit and capture status
        let run_id_owned = run_id.to_string();
        std::thread::spawn(move || {
            Self::wait_for_exit(child, run_id_owned);
        });

        Ok(RuntimeHandle::Background { pid, pgid })
    }

    fn stop(&self, handle: &RuntimeHandle, force: bool) -> Result<()> {
        if let RuntimeHandle::Background { pid, pgid } = handle {
            // Choose signal based on force flag
            let signal = if force { libc::SIGKILL } else { libc::SIGTERM };

            // Send signal to the process group
            // SAFETY: killpg is safe with valid pgid
            let result = unsafe { libc::killpg(*pgid as i32, signal) };
            if result != 0 {
                let err = std::io::Error::last_os_error();
                // ESRCH means no such process - try killing just the pid
                if err.raw_os_error() == Some(libc::ESRCH) {
                    // Process group doesn't exist, try direct kill
                    unsafe { libc::kill(*pid as i32, signal) };
                } else {
                    return Err(err.into());
                }
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
            // Check if process exists using kill with signal 0
            // SAFETY: kill with signal 0 just checks existence
            // Note: We don't use waitpid here because the exit-watching thread
            // is responsible for reaping the process. Using waitpid here would
            // race with the wait thread and potentially lose the exit status.
            unsafe { libc::kill(*pid as i32, 0) == 0 }
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CodeState, Run, RunStatus, Storage};
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

        // Stop the process (use force=true for reliable test termination)
        adapter.stop(&handle, true).unwrap();

        // Wait for process to terminate with retries
        // Note: The exit-watching thread handles reaping; is_alive() just checks existence
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

    // === Integration tests for exit status capture ===
    // These tests verify that BackgroundAdapter correctly captures exit status
    // and updates the Run in storage via the exit-watching thread.

    /// Helper to create a Run in storage for testing
    fn create_run_in_storage(storage: &Storage, run_id: &str, exec: &Exec) -> Run {
        let mut run = Run::new(
            exec.clone(),
            CodeState {
                repo_url: "git@github.com:test/repo.git".to_string(),
                base_commit: "a1b2c3d4e5f6789012345678901234567890abcd".to_string(),
                patch: None,
            },
        );
        run.run_id = run_id.to_string();
        run.status = RunStatus::Running;
        storage.save_run(&run).unwrap();
        run
    }

    #[test]
    fn test_background_exit_status_success() {
        // This test verifies that successful commands (exit 0) result in Exited status
        // Note: Uses real XDG storage because update_run_on_exit calls Storage::new()
        let storage = Storage::new().unwrap();
        let adapter = BackgroundAdapter::new();
        let log_path = storage.logs_dir().join("test_exit_success.log");

        let run_id = format!("run_{}", uuid::Uuid::new_v4());
        let exec = Exec {
            argv: vec!["true".to_string()], // `true` always exits with 0
            cwd: ".".to_string(),
            env: HashMap::new(),
            timeout_sec: 0,
        };

        // Create the run in storage before spawning
        create_run_in_storage(&storage, &run_id, &exec);

        // Spawn the process
        let handle = adapter.spawn(&exec, &run_id, &log_path).unwrap();

        // Wait for process to exit and exit-watcher thread to update status
        for _ in 0..100 {
            if !adapter.is_alive(&handle) {
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }

        // Give the exit-watcher thread time to update storage
        thread::sleep(Duration::from_millis(200));

        // Verify status was updated to Exited
        let updated_run = storage.load_run(&run_id).unwrap();
        assert_eq!(
            updated_run.status,
            RunStatus::Exited,
            "Run should have Exited status after successful exit"
        );
        assert_eq!(
            updated_run.exit_code,
            Some(0),
            "Exit code should be 0"
        );
        assert!(
            updated_run.timeline.ended_at.is_some(),
            "ended_at should be set"
        );

        // Clean up: delete the test run from storage
        let _ = storage.delete_run(&run_id);
        let _ = std::fs::remove_file(&log_path);
    }

    #[test]
    fn test_background_exit_status_failure() {
        // This test verifies that failed commands (exit != 0) result in Failed status
        let storage = Storage::new().unwrap();
        let adapter = BackgroundAdapter::new();
        let log_path = storage.logs_dir().join("test_exit_failure.log");

        let run_id = format!("run_{}", uuid::Uuid::new_v4());
        let exec = Exec {
            argv: vec!["false".to_string()], // `false` always exits with 1
            cwd: ".".to_string(),
            env: HashMap::new(),
            timeout_sec: 0,
        };

        // Create the run in storage before spawning
        create_run_in_storage(&storage, &run_id, &exec);

        // Spawn the process
        let handle = adapter.spawn(&exec, &run_id, &log_path).unwrap();

        // Wait for process to exit and exit-watcher thread to update status
        for _ in 0..100 {
            if !adapter.is_alive(&handle) {
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }

        // Give the exit-watcher thread time to update storage
        thread::sleep(Duration::from_millis(200));

        // Verify status was updated to Failed
        let updated_run = storage.load_run(&run_id).unwrap();
        assert_eq!(
            updated_run.status,
            RunStatus::Failed,
            "Run should have Failed status after non-zero exit"
        );
        assert_eq!(
            updated_run.exit_code,
            Some(1),
            "Exit code should be 1"
        );
        assert!(
            updated_run.timeline.ended_at.is_some(),
            "ended_at should be set"
        );

        // Clean up: delete the test run from storage
        let _ = storage.delete_run(&run_id);
        let _ = std::fs::remove_file(&log_path);
    }

    #[test]
    fn test_background_exit_status_custom_exit_code() {
        // This test verifies that custom exit codes are captured correctly
        let storage = Storage::new().unwrap();
        let adapter = BackgroundAdapter::new();
        let log_path = storage.logs_dir().join("test_exit_custom.log");

        let run_id = format!("run_{}", uuid::Uuid::new_v4());
        let exec = Exec {
            argv: vec!["sh".to_string(), "-c".to_string(), "exit 42".to_string()],
            cwd: ".".to_string(),
            env: HashMap::new(),
            timeout_sec: 0,
        };

        // Create the run in storage before spawning
        create_run_in_storage(&storage, &run_id, &exec);

        // Spawn the process
        let handle = adapter.spawn(&exec, &run_id, &log_path).unwrap();

        // Wait for process to exit
        for _ in 0..100 {
            if !adapter.is_alive(&handle) {
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }

        // Give the exit-watcher thread time to update storage
        thread::sleep(Duration::from_millis(200));

        // Verify exit code was captured
        let updated_run = storage.load_run(&run_id).unwrap();
        assert_eq!(
            updated_run.status,
            RunStatus::Failed,
            "Run should have Failed status for exit code 42"
        );
        assert_eq!(
            updated_run.exit_code,
            Some(42),
            "Exit code should be 42"
        );

        // Clean up
        let _ = storage.delete_run(&run_id);
        let _ = std::fs::remove_file(&log_path);
    }

    #[test]
    fn test_background_signal_termination() {
        // This test verifies that signal termination is captured correctly
        // using the 128 + signal convention
        let storage = Storage::new().unwrap();
        let adapter = BackgroundAdapter::new();
        let log_path = storage.logs_dir().join("test_signal.log");

        let run_id = format!("run_{}", uuid::Uuid::new_v4());
        let exec = Exec {
            // Use a long sleep that we'll kill
            argv: vec!["sleep".to_string(), "60".to_string()],
            cwd: ".".to_string(),
            env: HashMap::new(),
            timeout_sec: 0,
        };

        // Create the run in storage before spawning
        create_run_in_storage(&storage, &run_id, &exec);

        // Spawn the process
        let handle = adapter.spawn(&exec, &run_id, &log_path).unwrap();

        // Verify process is running
        assert!(adapter.is_alive(&handle), "Process should be alive");

        // Wait a bit for the process to start
        thread::sleep(Duration::from_millis(100));

        // Kill with SIGTERM (signal 15)
        adapter.stop(&handle, false).unwrap();

        // Wait for process to die
        for _ in 0..50 {
            if !adapter.is_alive(&handle) {
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }

        // Give the exit-watcher thread time to update storage
        thread::sleep(Duration::from_millis(300));

        // Verify signal exit was captured
        let updated_run = storage.load_run(&run_id).unwrap();
        assert_eq!(
            updated_run.status,
            RunStatus::Failed,
            "Run should have Failed status after signal termination"
        );
        // SIGTERM is signal 15, so exit code should be 128 + 15 = 143
        assert_eq!(
            updated_run.exit_code,
            Some(143),
            "Exit code should be 143 (128 + SIGTERM=15)"
        );

        // Clean up
        let _ = storage.delete_run(&run_id);
        let _ = std::fs::remove_file(&log_path);
    }
}
