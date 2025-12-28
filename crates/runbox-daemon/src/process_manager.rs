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
    /// Thread handle for the wait thread (None if already joined)
    wait_handle: Option<JoinHandle<ProcessResult>>,
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
            let storage = Storage::with_base_dir(storage_base_dir).expect("Failed to create storage");
            wait_for_process(child, run_id_owned, storage, processes_clone)
        });

        // Store in managed processes
        let mut processes = self.processes.lock().unwrap();
        processes.insert(
            run_id.to_string(),
            ManagedProcess {
                pid,
                pgid,
                wait_handle: Some(wait_handle),
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
            // ESRCH means no such process - try killing just the pid
            if err.raw_os_error() == Some(libc::ESRCH) {
                unsafe { libc::kill(process.pid as i32, signal) };
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

        // Join any completed wait threads
        let mut completed = Vec::new();
        for (run_id, process) in processes.iter_mut() {
            if let Some(handle) = &process.wait_handle {
                if handle.is_finished() {
                    completed.push(run_id.clone());
                }
            }
        }

        // Remove completed processes
        for run_id in completed {
            if let Some(mut process) = processes.remove(&run_id) {
                if let Some(handle) = process.wait_handle.take() {
                    let _ = handle.join();
                }
            }
        }
    }

    /// Reconcile processes after daemon restart
    pub fn reconcile_on_start(&self) -> Result<()> {
        log::info!("Reconciling processes after daemon start");

        let runs = self.storage.list_runs(usize::MAX)?;

        for mut run in runs {
            if run.status != RunStatus::Running {
                continue;
            }

            // Check if the process is still alive
            if let Some(ref handle) = run.handle {
                if let runbox_core::RuntimeHandle::Background { pid, .. } = handle {
                    // Check if process exists
                    let alive = unsafe { libc::kill(*pid as i32, 0) == 0 };

                    if !alive {
                        // Process is dead but was Running - mark as Unknown
                        log::warn!(
                            "Run {} has dead process {} - marking as Unknown",
                            run.run_id,
                            pid
                        );
                        run.status = RunStatus::Unknown;
                        run.reconcile_reason = Some(format!(
                            "daemon restarted, process {} not found",
                            pid
                        ));
                        if run.timeline.ended_at.is_none() {
                            run.timeline.ended_at = Some(Utc::now());
                        }
                        self.storage.save_run(&run)?;
                    }
                }
            } else {
                // Running status but no handle
                log::warn!("Run {} is Running but has no handle - marking as Unknown", run.run_id);
                run.status = RunStatus::Unknown;
                run.reconcile_reason = Some("daemon restarted, no runtime handle".to_string());
                if run.timeline.ended_at.is_none() {
                    run.timeline.ended_at = Some(Utc::now());
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

    // CAS: Only update if still Running
    if run.status != RunStatus::Running {
        log::warn!(
            "Run {} is not Running (status: {}), skipping update",
            run_id,
            run.status
        );
        return Ok(());
    }

    // Determine new status
    run.status = match (exit_code, signal) {
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

    // Set exit code
    if let Some(code) = exit_code {
        run.exit_code = Some(code);
    } else if let Some(sig) = signal {
        // Convention: exit code = 128 + signal number
        run.exit_code = Some(128 + sig);
    }

    // Set ended_at (CAS: don't overwrite if already set)
    if run.timeline.ended_at.is_none() {
        run.timeline.ended_at = Some(Utc::now());
    }

    storage.save_run(&run)?;

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
        assert!(matches!(updated_run.status, RunStatus::Killed | RunStatus::Failed));
    }
}
