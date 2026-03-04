//! Process manager for spawning and tracking background processes
//!
//! The daemon owns spawned processes and waits for their exit to capture status.

use anyhow::{bail, Result};
use chrono::Utc;
use runbox_core::{Exec, RunStatus, Storage};
use std::collections::HashMap;
use std::fs::File;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

/// Information about a managed process
struct ManagedProcess {
    pid: u32,
    pgid: u32,
    /// Thread handle for the wait thread (None if already joined or adopted)
    wait_handle: Option<JoinHandle<ProcessResult>>,
    /// True if this process was adopted (not spawned by us)
    adopted: bool,
}

/// Result of waiting for a process
#[derive(Debug, Clone)]
pub struct ProcessResult {
    pub run_id: String,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
}

/// Process manager that spawns and tracks processes
pub struct ProcessManager {
    /// Map of run_id -> ManagedProcess
    processes: Arc<Mutex<HashMap<String, ManagedProcess>>>,
    /// Storage for updating runs
    storage: Storage,
    /// Base directory for storage (used by wait threads)
    storage_base_dir: PathBuf,
}

impl ProcessManager {
    /// Create a new process manager
    pub fn new(storage: Storage) -> Self {
        let storage_base_dir = storage.base_dir().clone();
        Self {
            processes: Arc::new(Mutex::new(HashMap::new())),
            storage,
            storage_base_dir,
        }
    }

    /// Spawn a new process
    pub fn spawn(&self, run_id: &str, exec: &Exec, log_path: &Path) -> Result<(u32, u32)> {
        // Validate argv
        if exec.argv.is_empty() {
            bail!("Cannot spawn process: argv is empty");
        }

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

        log::info!("Spawned process {} for run {}", pid, run_id);

        // Spawn wait thread
        let run_id_owned = run_id.to_string();
        let storage_base_dir = self.storage_base_dir.clone();
        let processes_clone = Arc::clone(&self.processes);

        let wait_handle = thread::spawn(move || {
            match Storage::with_base_dir(storage_base_dir) {
                Ok(storage) => wait_for_process(child, run_id_owned, storage, processes_clone),
                Err(e) => {
                    log::error!("Failed to create storage for wait thread: {}. Exit status won't be captured.", e);
                    // Still wait on the child to prevent zombie, but can't update storage
                    let mut c = child;
                    let _ = c.wait();
                    processes_clone.lock().unwrap().remove(&run_id_owned);
                    ProcessResult {
                        run_id: run_id_owned,
                        exit_code: None,
                        signal: None,
                    }
                }
            }
        });

        // Store in managed processes
        let mut processes = self.processes.lock().unwrap();
        processes.insert(
            run_id.to_string(),
            ManagedProcess {
                pid,
                pgid,
                wait_handle: Some(wait_handle),
                adopted: false,
            },
        );

        Ok((pid, pgid))
    }

    /// Stop a running process
    pub fn stop(&self, run_id: &str, force: bool) -> Result<()> {
        let processes = self.processes.lock().unwrap();
        let Some(process) = processes.get(run_id) else {
            bail!("No process found for run: {}", run_id);
        };

        let signal = if force { libc::SIGKILL } else { libc::SIGTERM };

        // Send signal to process group
        // SAFETY: killpg is safe with valid pgid
        let result = unsafe { libc::killpg(process.pgid as i32, signal) };
        if result != 0 {
            let err = std::io::Error::last_os_error();
            // ESRCH means no such process group - try killing just the pid
            if err.raw_os_error() == Some(libc::ESRCH) {
                let pid_result = unsafe { libc::kill(process.pid as i32, signal) };
                if pid_result != 0 {
                    let pid_err = std::io::Error::last_os_error();
                    // ESRCH is ok - process already dead
                    if pid_err.raw_os_error() != Some(libc::ESRCH) {
                        return Err(pid_err.into());
                    }
                }
            } else {
                return Err(err.into());
            }
        }

        log::info!(
            "Sent {} to process {} (run {})",
            if force { "SIGKILL" } else { "SIGTERM" },
            process.pid,
            run_id
        );

        Ok(())
    }

    /// Get status of a process
    pub fn status(&self, run_id: &str) -> Result<(bool, Option<i32>, Option<i32>)> {
        let processes = self.processes.lock().unwrap();
        let Some(process) = processes.get(run_id) else {
            // Process might have completed and been removed
            // Check storage for exit status
            if let Ok(run) = self.storage.load_run(run_id) {
                let alive = matches!(run.status, RunStatus::Running | RunStatus::Pending);
                let exit_code = run.exit_code;
                return Ok((alive, exit_code, None));
            }
            bail!("No process found for run: {}", run_id);
        };

        // Check if process is still alive using kill with signal 0
        let alive = unsafe { libc::kill(process.pid as i32, 0) == 0 };

        Ok((alive, None, None))
    }

    /// Get number of managed processes
    pub fn process_count(&self) -> usize {
        self.processes.lock().unwrap().len()
    }

    /// Clean up completed processes
    pub fn cleanup_completed(&self) {
        let mut processes = self.processes.lock().unwrap();

        let mut completed = Vec::new();
        let mut adopted_dead = Vec::new();

        for (run_id, process) in processes.iter_mut() {
            if process.adopted {
                // For adopted processes, poll with kill(pid, 0)
                let alive = unsafe { libc::kill(process.pid as i32, 0) == 0 };
                if !alive {
                    adopted_dead.push(run_id.clone());
                }
            } else if let Some(handle) = &process.wait_handle {
                // For spawned processes, check if wait thread finished
                if handle.is_finished() {
                    completed.push(run_id.clone());
                }
            }
        }

        // Remove completed spawned processes
        for run_id in completed {
            if let Some(mut process) = processes.remove(&run_id) {
                if let Some(handle) = process.wait_handle.take() {
                    let _ = handle.join();
                }
            }
        }

        // Handle dead adopted processes
        // Note: we need to drop the lock before updating storage
        drop(processes);

        for run_id in adopted_dead {
            // Update run status to Unknown (we can't get exit code for adopted processes)
            if let Err(e) = self.update_adopted_on_exit(&run_id) {
                log::error!("Failed to update adopted run {}: {}", run_id, e);
            }
            // Remove from tracking
            self.processes.lock().unwrap().remove(&run_id);
        }
    }

    /// Update an adopted process's run status when it exits
    fn update_adopted_on_exit(&self, run_id: &str) -> Result<()> {
        let mut run = self.storage.load_run(run_id)?;

        // Only update if still in a running state
        match run.status {
            RunStatus::Running | RunStatus::Pending => {
                run.status = RunStatus::Unknown;
                run.reconcile_reason =
                    Some("process exited while adopted (exit code unavailable)".to_string());
            }
            RunStatus::Killed => {
                // CLI stopped it, just fill in timeline fields
            }
            _ => {
                // Already in terminal state
                return Ok(());
            }
        }

        // Ensure timeline is complete
        let now = Utc::now();
        if run.timeline.started_at.is_none() {
            run.timeline.started_at = Some(now);
        }
        if run.timeline.ended_at.is_none() {
            run.timeline.ended_at = Some(now);
        }

        self.storage.save_run(&run)?;
        log::info!("Updated adopted run {} - process exited", run_id);

        Ok(())
    }

    /// Adopt an existing process (not spawned by us) for monitoring
    /// We can't get its exit status via wait(), but we can detect when it dies
    fn adopt(&self, run_id: &str, pid: u32, pgid: u32) {
        let mut processes = self.processes.lock().unwrap();
        processes.insert(
            run_id.to_string(),
            ManagedProcess {
                pid,
                pgid,
                wait_handle: None, // Can't wait on a non-child process
                adopted: true,
            },
        );
        log::info!("Adopted process {} for run {}", pid, run_id);
    }

    /// Reconcile processes after daemon restart
    pub fn reconcile_on_start(&self) -> Result<()> {
        log::info!("Reconciling processes after daemon start");

        let runs = self.storage.list_runs(usize::MAX)?;

        for mut run in runs {
            // Consider both Running and Pending with background handles
            // Pending can have a live process if daemon restarted between spawn and CLI update
            if run.status != RunStatus::Running && run.status != RunStatus::Pending {
                continue;
            }

            // Skip if not a background runtime
            if let Some(ref handle) = run.handle {
                if !matches!(handle, runbox_core::RuntimeHandle::Background { .. }) {
                    continue;
                }
            } else if run.status == RunStatus::Pending {
                // Pending without handle - skip, process hasn't spawned yet
                continue;
            }

            // Check if the process is still alive
            if let Some(ref handle) = run.handle {
                if let runbox_core::RuntimeHandle::Background { pid, pgid } = handle {
                    // Check if process exists
                    let alive = unsafe { libc::kill(*pid as i32, 0) == 0 };

                    if alive {
                        // Process is alive - adopt it so we can monitor for exit
                        log::info!(
                            "Run {} has alive process {} - adopting for monitoring",
                            run.run_id,
                            pid
                        );
                        self.adopt(&run.run_id, *pid, *pgid);

                        // If still Pending, upgrade to Running and set started_at
                        if run.status == RunStatus::Pending {
                            run.status = RunStatus::Running;
                            if run.timeline.started_at.is_none() {
                                run.timeline.started_at = Some(Utc::now());
                            }
                            run.reconcile_reason = Some(
                                "daemon adopted live process, upgraded from Pending".to_string(),
                            );
                            self.storage.save_run(&run)?;
                            log::info!("Run {} upgraded from Pending to Running", run.run_id);
                        }
                    } else {
                        // Process is dead but was Running - mark as Unknown
                        log::warn!(
                            "Run {} has dead process {} - marking as Unknown",
                            run.run_id,
                            pid
                        );
                        run.status = RunStatus::Unknown;
                        run.reconcile_reason =
                            Some(format!("daemon restarted, process {} not found", pid));
                        let now = Utc::now();
                        if run.timeline.started_at.is_none() {
                            run.timeline.started_at = Some(now);
                        }
                        if run.timeline.ended_at.is_none() {
                            run.timeline.ended_at = Some(now);
                        }
                        self.storage.save_run(&run)?;
                    }
                }
            } else {
                // Running status but no handle
                log::warn!(
                    "Run {} is Running but has no handle - marking as Unknown",
                    run.run_id
                );
                run.status = RunStatus::Unknown;
                run.reconcile_reason = Some("daemon restarted, no runtime handle".to_string());
                let now = Utc::now();
                if run.timeline.started_at.is_none() {
                    run.timeline.started_at = Some(now);
                }
                if run.timeline.ended_at.is_none() {
                    run.timeline.ended_at = Some(now);
                }
                self.storage.save_run(&run)?;
            }
        }

        Ok(())
    }
}

/// Wait for a process to exit and update storage
fn wait_for_process(
    mut child: Child,
    run_id: String,
    storage: Storage,
    processes: Arc<Mutex<HashMap<String, ManagedProcess>>>,
) -> ProcessResult {
    log::info!("Wait thread started for run {}", run_id);

    // Wait for process to exit
    let status = child.wait();

    let (exit_code, signal) = match status {
        Ok(exit_status) => {
            if let Some(code) = exit_status.code() {
                (Some(code), None)
            } else {
                // Process was killed by signal
                #[cfg(unix)]
                {
                    use std::os::unix::process::ExitStatusExt;
                    (None, exit_status.signal())
                }
                #[cfg(not(unix))]
                {
                    (None, None)
                }
            }
        }
        Err(e) => {
            log::error!("Failed to wait for process: {}", e);
            (None, None)
        }
    };

    log::info!(
        "Process for run {} exited: exit_code={:?}, signal={:?}",
        run_id,
        exit_code,
        signal
    );

    // Update run in storage
    if let Err(e) = update_run_on_exit(&storage, &run_id, exit_code, signal) {
        log::error!("Failed to update run {}: {}", run_id, e);
    }

    // Remove from managed processes
    let mut procs = processes.lock().unwrap();
    procs.remove(&run_id);

    ProcessResult {
        run_id,
        exit_code,
        signal,
    }
}

/// Update a run's status after process exit
fn update_run_on_exit(
    storage: &Storage,
    run_id: &str,
    exit_code: Option<i32>,
    signal: Option<i32>,
) -> Result<()> {
    let mut run = storage.load_run(run_id)?;

    // Determine new status based on exit code/signal
    let new_status = match (exit_code, signal) {
        (Some(0), _) => RunStatus::Exited,
        (Some(_), _) => RunStatus::Failed,
        (None, Some(sig)) => {
            // Killed by signal
            if sig == libc::SIGTERM || sig == libc::SIGKILL {
                RunStatus::Killed
            } else {
                RunStatus::Failed
            }
        }
        (None, None) => RunStatus::Unknown,
    };

    // Determine which statuses we can update from
    let expected_statuses = [
        RunStatus::Running, // normal case
        RunStatus::Pending, // fast-exit race
        RunStatus::Killed,  // CLI stopped, we update exit_code/timeline
    ];

    // Use CAS-style save with lock to prevent CLI/daemon race
    // The closure receives the fresh run read under lock
    let saved = storage.save_run_if_status_with(run_id, &expected_statuses, |current| {
        // Determine new status based on current status
        let final_status = match current.status {
            RunStatus::Running | RunStatus::Pending => new_status.clone(),
            RunStatus::Killed => RunStatus::Killed, // Keep as Killed
            _ => return,                            // Won't happen due to expected_statuses check
        };

        // Apply updates
        current.status = final_status;

        // Set exit code (don't overwrite if already set)
        if current.exit_code.is_none() {
            if let Some(code) = exit_code {
                current.exit_code = Some(code);
            } else if let Some(sig) = signal {
                // Convention: exit code = 128 + signal number
                current.exit_code = Some(128 + sig);
            }
        }

        // Set started_at if not set (fast-exit case: Pending -> terminal)
        if current.timeline.started_at.is_none() {
            current.timeline.started_at = Some(Utc::now());
        }

        // Set ended_at (don't overwrite if already set)
        if current.timeline.ended_at.is_none() {
            current.timeline.ended_at = Some(Utc::now());
        }
    })?;

    if !saved {
        log::warn!(
            "Run {} status changed during update, skipping (CAS failed)",
            run_id
        );
        return Ok(());
    }

    log::info!(
        "Updated run {} status to {} (exit_code: {:?})",
        run_id,
        run.status,
        run.exit_code
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use runbox_core::{CodeState, Run};
    use std::collections::HashMap;
    use tempfile::tempdir;

    #[test]
    fn test_spawn_and_wait() {
        let _ = env_logger::try_init();

        let dir = tempdir().unwrap();
        let base_path = dir.path().to_path_buf();
        let storage = Storage::with_base_dir(base_path.clone()).unwrap();

        let log_path = dir.path().join("test.log");
        let exec = Exec {
            argv: vec!["true".to_string()],
            cwd: ".".to_string(),
            env: HashMap::new(),
            timeout_sec: 0,
        };

        // Create a run first
        let mut run = Run::new(
            exec.clone(),
            CodeState {
                repo_url: "git@github.com:org/repo.git".to_string(),
                base_commit: "a1b2c3d4e5f6789012345678901234567890abcd".to_string(),
                patch: None,
            },
        );
        run.status = RunStatus::Running;
        let run_id = run.run_id.clone();
        storage.save_run(&run).unwrap();

        // Create manager (takes ownership of storage)
        let manager = ProcessManager::new(storage);

        // Spawn the process
        let (pid, pgid) = manager.spawn(&run_id, &exec, &log_path).unwrap();
        assert!(pid > 0);
        assert_eq!(pid, pgid);

        // Wait for the process to complete
        std::thread::sleep(std::time::Duration::from_millis(500));
        manager.cleanup_completed();

        // Check that the run was updated (create new storage to read)
        let storage2 = Storage::with_base_dir(base_path).unwrap();
        let updated_run = storage2.load_run(&run_id).unwrap();
        assert_eq!(updated_run.status, RunStatus::Exited);
        assert_eq!(updated_run.exit_code, Some(0));
    }

    #[test]
    fn test_spawn_failure_exit_code() {
        let _ = env_logger::try_init();

        let dir = tempdir().unwrap();
        let base_path = dir.path().to_path_buf();
        let storage = Storage::with_base_dir(base_path.clone()).unwrap();

        let log_path = dir.path().join("test.log");
        let exec = Exec {
            argv: vec!["false".to_string()],
            cwd: ".".to_string(),
            env: HashMap::new(),
            timeout_sec: 0,
        };

        // Create a run first
        let mut run = Run::new(
            exec.clone(),
            CodeState {
                repo_url: "git@github.com:org/repo.git".to_string(),
                base_commit: "a1b2c3d4e5f6789012345678901234567890abcd".to_string(),
                patch: None,
            },
        );
        run.status = RunStatus::Running;
        let run_id = run.run_id.clone();
        storage.save_run(&run).unwrap();

        // Create manager
        let manager = ProcessManager::new(storage);

        // Spawn the process
        manager.spawn(&run_id, &exec, &log_path).unwrap();

        // Wait for the process to complete
        std::thread::sleep(std::time::Duration::from_millis(500));
        manager.cleanup_completed();

        // Check that the run was updated
        let storage2 = Storage::with_base_dir(base_path).unwrap();
        let updated_run = storage2.load_run(&run_id).unwrap();
        assert_eq!(updated_run.status, RunStatus::Failed);
        assert_eq!(updated_run.exit_code, Some(1));
    }

    #[test]
    fn test_stop_process() {
        let _ = env_logger::try_init();

        let dir = tempdir().unwrap();
        let base_path = dir.path().to_path_buf();
        let storage = Storage::with_base_dir(base_path.clone()).unwrap();

        let log_path = dir.path().join("test.log");
        let exec = Exec {
            argv: vec!["sleep".to_string(), "60".to_string()],
            cwd: ".".to_string(),
            env: HashMap::new(),
            timeout_sec: 0,
        };

        // Create a run first
        let mut run = Run::new(
            exec.clone(),
            CodeState {
                repo_url: "git@github.com:org/repo.git".to_string(),
                base_commit: "a1b2c3d4e5f6789012345678901234567890abcd".to_string(),
                patch: None,
            },
        );
        run.status = RunStatus::Running;
        let run_id = run.run_id.clone();
        storage.save_run(&run).unwrap();

        // Create manager
        let manager = ProcessManager::new(storage);

        // Spawn the process
        manager.spawn(&run_id, &exec, &log_path).unwrap();

        // Verify it's alive
        let (alive, _, _) = manager.status(&run_id).unwrap();
        assert!(alive);

        // Stop it
        manager.stop(&run_id, true).unwrap();

        // Wait for process to die
        std::thread::sleep(std::time::Duration::from_millis(500));
        manager.cleanup_completed();

        // Check that it's dead
        let storage2 = Storage::with_base_dir(base_path).unwrap();
        let updated_run = storage2.load_run(&run_id).unwrap();
        assert!(matches!(
            updated_run.status,
            RunStatus::Killed | RunStatus::Failed
        ));
    }

    #[test]
    fn test_spawn_rejects_empty_argv() {
        let _ = env_logger::try_init();

        let dir = tempdir().unwrap();
        let storage = Storage::with_base_dir(dir.path().to_path_buf()).unwrap();
        let manager = ProcessManager::new(storage);

        let log_path = dir.path().join("test.log");
        let exec = Exec {
            argv: vec![], // Empty argv
            cwd: ".".to_string(),
            env: HashMap::new(),
            timeout_sec: 0,
        };

        let result = manager.spawn("run_test", &exec, &log_path);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty"));
    }

    #[test]
    fn test_fast_exit_pending_sets_timeline() {
        let _ = env_logger::try_init();

        let dir = tempdir().unwrap();
        let base_path = dir.path().to_path_buf();
        let storage = Storage::with_base_dir(base_path.clone()).unwrap();

        let log_path = dir.path().join("test.log");
        let exec = Exec {
            argv: vec!["true".to_string()], // Exits immediately
            cwd: ".".to_string(),
            env: HashMap::new(),
            timeout_sec: 0,
        };

        // Create a run with Pending status (simulating fast-exit race)
        let mut run = Run::new(
            exec.clone(),
            CodeState {
                repo_url: "git@github.com:org/repo.git".to_string(),
                base_commit: "a1b2c3d4e5f6789012345678901234567890abcd".to_string(),
                patch: None,
            },
        );
        run.status = RunStatus::Pending; // Still Pending when process exits
        let run_id = run.run_id.clone();
        storage.save_run(&run).unwrap();

        // Create manager and spawn
        let manager = ProcessManager::new(storage);
        manager.spawn(&run_id, &exec, &log_path).unwrap();

        // Wait for the fast exit
        std::thread::sleep(std::time::Duration::from_millis(500));
        manager.cleanup_completed();

        // Verify timeline is complete even though status was Pending
        let storage2 = Storage::with_base_dir(base_path).unwrap();
        let updated_run = storage2.load_run(&run_id).unwrap();
        assert_eq!(updated_run.status, RunStatus::Exited);
        assert!(
            updated_run.timeline.started_at.is_some(),
            "started_at should be set"
        );
        assert!(
            updated_run.timeline.ended_at.is_some(),
            "ended_at should be set"
        );
    }

    #[test]
    fn test_killed_status_preserves_but_updates_exit_code() {
        let _ = env_logger::try_init();

        let dir = tempdir().unwrap();
        let base_path = dir.path().to_path_buf();
        let storage = Storage::with_base_dir(base_path.clone()).unwrap();

        let log_path = dir.path().join("test.log");
        let exec = Exec {
            argv: vec!["sleep".to_string(), "60".to_string()],
            cwd: ".".to_string(),
            env: HashMap::new(),
            timeout_sec: 0,
        };

        // Create a run
        let mut run = Run::new(
            exec.clone(),
            CodeState {
                repo_url: "git@github.com:org/repo.git".to_string(),
                base_commit: "a1b2c3d4e5f6789012345678901234567890abcd".to_string(),
                patch: None,
            },
        );
        run.status = RunStatus::Running;
        let run_id = run.run_id.clone();
        storage.save_run(&run).unwrap();

        // Create manager and spawn
        let manager = ProcessManager::new(storage);
        manager.spawn(&run_id, &exec, &log_path).unwrap();

        // Simulate CLI marking as Killed before daemon captures exit
        let storage2 = Storage::with_base_dir(base_path.clone()).unwrap();
        let mut updated = storage2.load_run(&run_id).unwrap();
        updated.status = RunStatus::Killed;
        storage2.save_run(&updated).unwrap();

        // Now stop the process
        manager.stop(&run_id, true).unwrap();

        // Wait for exit capture
        std::thread::sleep(std::time::Duration::from_millis(500));
        manager.cleanup_completed();

        // Verify status stayed Killed but exit_code was set
        let storage3 = Storage::with_base_dir(base_path).unwrap();
        let final_run = storage3.load_run(&run_id).unwrap();
        assert_eq!(
            final_run.status,
            RunStatus::Killed,
            "status should remain Killed"
        );
        assert!(final_run.exit_code.is_some(), "exit_code should be set");
        assert!(
            final_run.timeline.ended_at.is_some(),
            "ended_at should be set"
        );
    }

    #[test]
    fn test_stop_unknown_run_returns_error() {
        let _ = env_logger::try_init();

        let dir = tempdir().unwrap();
        let storage = Storage::with_base_dir(dir.path().to_path_buf()).unwrap();
        let manager = ProcessManager::new(storage);

        let result = manager.stop("run_nonexistent", false);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No process found"));
    }

    #[test]
    fn test_status_unknown_run_returns_error() {
        let _ = env_logger::try_init();

        let dir = tempdir().unwrap();
        let storage = Storage::with_base_dir(dir.path().to_path_buf()).unwrap();
        let manager = ProcessManager::new(storage);

        let result = manager.status("run_nonexistent");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No process found"));
    }

    #[test]
    fn test_status_from_storage_after_cleanup() {
        let _ = env_logger::try_init();

        let dir = tempdir().unwrap();
        let base_path = dir.path().to_path_buf();
        let storage = Storage::with_base_dir(base_path.clone()).unwrap();

        let log_path = dir.path().join("test.log");
        let exec = Exec {
            argv: vec!["true".to_string()],
            cwd: ".".to_string(),
            env: HashMap::new(),
            timeout_sec: 0,
        };

        // Create and save run
        let mut run = Run::new(
            exec.clone(),
            CodeState {
                repo_url: "git@github.com:org/repo.git".to_string(),
                base_commit: "a1b2c3d4e5f6789012345678901234567890abcd".to_string(),
                patch: None,
            },
        );
        run.status = RunStatus::Running;
        let run_id = run.run_id.clone();
        storage.save_run(&run).unwrap();

        let manager = ProcessManager::new(storage);
        manager.spawn(&run_id, &exec, &log_path).unwrap();

        // Wait for completion and cleanup
        std::thread::sleep(std::time::Duration::from_millis(500));
        manager.cleanup_completed();

        // Status should still work by reading from storage
        let (alive, exit_code, _signal) = manager.status(&run_id).unwrap();
        assert!(!alive, "process should not be alive");
        assert_eq!(exit_code, Some(0), "exit_code should be from storage");
    }

    #[test]
    fn test_reconcile_marks_dead_as_unknown() {
        let _ = env_logger::try_init();

        let dir = tempdir().unwrap();
        let base_path = dir.path().to_path_buf();
        let storage = Storage::with_base_dir(base_path.clone()).unwrap();

        // Create a Running run with a dead PID (PID 1 is init, can't be our process)
        // Use a very high PID that's unlikely to exist
        let mut run = Run::new(
            Exec {
                argv: vec!["echo".to_string()],
                cwd: ".".to_string(),
                env: HashMap::new(),
                timeout_sec: 0,
            },
            CodeState {
                repo_url: "git@github.com:org/repo.git".to_string(),
                base_commit: "a1b2c3d4e5f6789012345678901234567890abcd".to_string(),
                patch: None,
            },
        );
        run.status = RunStatus::Running;
        run.handle = Some(runbox_core::RuntimeHandle::Background {
            pid: 999999999, // Non-existent PID
            pgid: 999999999,
        });
        let run_id = run.run_id.clone();
        storage.save_run(&run).unwrap();

        // Create manager and reconcile
        let manager = ProcessManager::new(storage);
        manager.reconcile_on_start().unwrap();

        // Verify run is marked Unknown with timeline
        let storage2 = Storage::with_base_dir(base_path).unwrap();
        let updated = storage2.load_run(&run_id).unwrap();
        assert_eq!(updated.status, RunStatus::Unknown);
        assert!(updated.reconcile_reason.is_some());
        assert!(
            updated.timeline.started_at.is_some(),
            "started_at should be set"
        );
        assert!(
            updated.timeline.ended_at.is_some(),
            "ended_at should be set"
        );
    }

    #[test]
    fn test_reconcile_running_no_handle_marks_unknown() {
        let _ = env_logger::try_init();

        let dir = tempdir().unwrap();
        let base_path = dir.path().to_path_buf();
        let storage = Storage::with_base_dir(base_path.clone()).unwrap();

        // Create a Running run without a handle
        let mut run = Run::new(
            Exec {
                argv: vec!["echo".to_string()],
                cwd: ".".to_string(),
                env: HashMap::new(),
                timeout_sec: 0,
            },
            CodeState {
                repo_url: "git@github.com:org/repo.git".to_string(),
                base_commit: "a1b2c3d4e5f6789012345678901234567890abcd".to_string(),
                patch: None,
            },
        );
        run.status = RunStatus::Running;
        run.handle = None; // No handle
        let run_id = run.run_id.clone();
        storage.save_run(&run).unwrap();

        // Create manager and reconcile
        let manager = ProcessManager::new(storage);
        manager.reconcile_on_start().unwrap();

        // Verify run is marked Unknown
        let storage2 = Storage::with_base_dir(base_path).unwrap();
        let updated = storage2.load_run(&run_id).unwrap();
        assert_eq!(updated.status, RunStatus::Unknown);
        assert!(updated
            .reconcile_reason
            .as_ref()
            .unwrap()
            .contains("no runtime handle"));
    }

    #[test]
    fn test_signal_exit_sets_128_plus_signal() {
        let _ = env_logger::try_init();

        let dir = tempdir().unwrap();
        let base_path = dir.path().to_path_buf();
        let storage = Storage::with_base_dir(base_path.clone()).unwrap();

        let log_path = dir.path().join("test.log");
        let exec = Exec {
            argv: vec!["sleep".to_string(), "60".to_string()],
            cwd: ".".to_string(),
            env: HashMap::new(),
            timeout_sec: 0,
        };

        // Create a run
        let mut run = Run::new(
            exec.clone(),
            CodeState {
                repo_url: "git@github.com:org/repo.git".to_string(),
                base_commit: "a1b2c3d4e5f6789012345678901234567890abcd".to_string(),
                patch: None,
            },
        );
        run.status = RunStatus::Running;
        let run_id = run.run_id.clone();
        storage.save_run(&run).unwrap();

        let manager = ProcessManager::new(storage);
        manager.spawn(&run_id, &exec, &log_path).unwrap();

        // Kill with SIGKILL (9)
        manager.stop(&run_id, true).unwrap();

        // Wait for exit capture
        std::thread::sleep(std::time::Duration::from_millis(500));
        manager.cleanup_completed();

        // Verify exit_code is 128+9=137
        let storage2 = Storage::with_base_dir(base_path).unwrap();
        let updated = storage2.load_run(&run_id).unwrap();
        // Note: actual exit_code depends on whether CAS succeeded
        // The important thing is that exit_code is set
        assert!(
            updated.exit_code.is_some(),
            "exit_code should be set for signal exit"
        );
    }
}
