//! Unix socket server for the daemon
//!
//! Listens on a Unix socket and handles requests from CLI clients.

use crate::process_manager::ProcessManager;
use crate::protocol::{read_message, write_message, Request, Response};
use anyhow::{bail, Context, Result};
use std::fs::{self, File};
use std::io::{BufReader, BufWriter};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Get the default socket path
pub fn default_socket_path() -> PathBuf {
    // Use XDG_RUNTIME_DIR if available, otherwise fall back to /tmp
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"));

    runtime_dir.join("runbox").join("daemon.sock")
}

/// Get the default PID file path
pub fn default_pid_path() -> PathBuf {
    pid_path_for_socket(&default_socket_path())
}

/// Get the PID file path for a given socket path
/// This ensures each socket gets its own PID file, allowing multiple daemons
pub fn pid_path_for_socket(socket_path: &Path) -> PathBuf {
    // Use the socket path with .pid extension
    socket_path.with_extension("pid")
}

/// Get the default lock file path
pub fn default_lock_path() -> PathBuf {
    lock_path_for_socket(&default_socket_path())
}

/// Get the lock file path for a given socket path
/// This ensures each socket gets its own lock, allowing multiple daemons
pub fn lock_path_for_socket(socket_path: &Path) -> PathBuf {
    // Use the socket path with .lock extension
    socket_path.with_extension("lock")
}

/// Acquire an exclusive lock on the lock file
/// Returns the lock file handle (must be kept open to maintain lock)
pub fn acquire_daemon_lock(lock_path: &Path) -> Result<File> {
    // Ensure parent directory exists
    if let Some(parent) = lock_path.parent() {
        fs::create_dir_all(parent)?;
    }

    // Open/create the lock file
    let lock_file = File::create(lock_path)
        .with_context(|| format!("Failed to create lock file: {}", lock_path.display()))?;

    // Try to acquire exclusive lock (non-blocking)
    // SAFETY: flock is safe with valid fd
    let fd = std::os::unix::io::AsRawFd::as_raw_fd(&lock_file);
    let result = unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) };

    if result != 0 {
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::EWOULDBLOCK) {
            bail!("Another daemon instance is already running");
        }
        return Err(err.into());
    }

    log::debug!("Acquired daemon lock: {}", lock_path.display());
    Ok(lock_file)
}

/// Unix socket server
pub struct Server {
    socket_path: PathBuf,
    listener: UnixListener,
    process_manager: ProcessManager,
    shutdown: Arc<AtomicBool>,
    idle_timeout: Option<Duration>,
}

impl Server {
    /// Create a new server at the given socket path
    pub fn new(socket_path: PathBuf, process_manager: ProcessManager) -> Result<Self> {
        // Ensure parent directory exists
        if let Some(parent) = socket_path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Remove existing socket if it exists
        if socket_path.exists() {
            fs::remove_file(&socket_path)?;
        }

        // Create the listener
        let listener = UnixListener::bind(&socket_path)
            .with_context(|| format!("Failed to bind to {}", socket_path.display()))?;

        // Set non-blocking so we can check for shutdown
        listener.set_nonblocking(true)?;

        log::info!("Server listening on {}", socket_path.display());

        Ok(Self {
            socket_path,
            listener,
            process_manager,
            shutdown: Arc::new(AtomicBool::new(false)),
            idle_timeout: Some(Duration::from_secs(3600)), // 1 hour default
        })
    }

    /// Set the idle timeout (None = no timeout)
    pub fn set_idle_timeout(&mut self, timeout: Option<Duration>) {
        self.idle_timeout = timeout;
    }

    /// Get a shutdown handle
    pub fn shutdown_handle(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.shutdown)
    }

    /// Request shutdown
    pub fn request_shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
    }

    /// Run the server loop
    pub fn run(&self) -> Result<()> {
        let mut last_activity = std::time::Instant::now();

        loop {
            // Check for shutdown
            if self.shutdown.load(Ordering::SeqCst) {
                log::info!("Shutdown requested, exiting");
                break;
            }

            // Check for idle timeout
            if let Some(timeout) = self.idle_timeout {
                if self.process_manager.process_count() == 0 && last_activity.elapsed() > timeout {
                    log::info!("Idle timeout reached, exiting");
                    break;
                }
            }

            // Try to accept a connection (non-blocking)
            match self.listener.accept() {
                Ok((stream, _)) => {
                    last_activity = std::time::Instant::now();
                    if let Err(e) = self.handle_connection(stream) {
                        log::error!("Error handling connection: {}", e);
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    // No connection available, sleep briefly and continue
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(e) => {
                    log::error!("Accept error: {}", e);
                    std::thread::sleep(Duration::from_millis(100));
                }
            }

            // Periodically clean up completed processes
            self.process_manager.cleanup_completed();
        }

        // Cleanup
        self.cleanup();

        Ok(())
    }

    /// Handle a single client connection
    fn handle_connection(&self, stream: UnixStream) -> Result<()> {
        // Set blocking mode for this connection
        stream.set_nonblocking(false)?;
        stream.set_read_timeout(Some(Duration::from_secs(30)))?;
        stream.set_write_timeout(Some(Duration::from_secs(30)))?;

        let mut reader = BufReader::new(&stream);
        let mut writer = BufWriter::new(&stream);

        // Read request
        let request: Request = read_message(&mut reader)?;
        log::debug!("Received request: {:?}", request);

        // Process request
        let response = self.handle_request(request);
        log::debug!("Sending response: {:?}", response);

        // Write response
        write_message(&mut writer, &response)?;

        Ok(())
    }

    /// Handle a single request
    fn handle_request(&self, request: Request) -> Response {
        match request {
            Request::Spawn {
                run_id,
                exec,
                log_path,
            } => match self.process_manager.spawn(&run_id, &exec, &log_path) {
                Ok((pid, pgid)) => Response::Spawned { pid, pgid },
                Err(e) => Response::Error {
                    message: e.to_string(),
                },
            },
            Request::Stop { run_id, force } => match self.process_manager.stop(&run_id, force) {
                Ok(()) => Response::Stopped,
                Err(e) => Response::Error {
                    message: e.to_string(),
                },
            },
            Request::Status { run_id } => match self.process_manager.status(&run_id) {
                Ok((alive, exit_code, signal)) => Response::Status {
                    alive,
                    exit_code,
                    signal,
                },
                Err(e) => Response::Error {
                    message: e.to_string(),
                },
            },
            Request::Ping => Response::Pong,
            Request::Shutdown => {
                self.request_shutdown();
                Response::ShutdownAck
            }
        }
    }

    /// Clean up resources
    fn cleanup(&self) {
        // Remove socket file
        if self.socket_path.exists() {
            let _ = fs::remove_file(&self.socket_path);
        }
        log::info!("Server shutdown complete");
    }
}

/// Write PID to file
pub fn write_pid_file(path: &Path) -> Result<()> {
    let pid = std::process::id();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, pid.to_string())?;
    log::info!("Wrote PID {} to {}", pid, path.display());
    Ok(())
}

/// Read PID from file
pub fn read_pid_file(path: &Path) -> Result<u32> {
    let content = fs::read_to_string(path).context("Failed to read PID file")?;
    let pid: u32 = content.trim().parse().context("Invalid PID in file")?;
    Ok(pid)
}

/// Remove PID file
pub fn remove_pid_file(path: &Path) {
    if path.exists() {
        let _ = fs::remove_file(path);
    }
}

/// Check if daemon is running by checking PID file and process
pub fn is_daemon_running(pid_path: &Path, socket_path: &Path) -> bool {
    // Check if socket exists
    if !socket_path.exists() {
        return false;
    }

    // Check if PID file exists and process is alive
    if let Ok(pid) = read_pid_file(pid_path) {
        // Check if process exists
        unsafe { libc::kill(pid as i32, 0) == 0 }
    } else {
        // No PID file, but socket exists - try to connect
        UnixStream::connect(socket_path).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{read_message, write_message, Request, Response};
    use runbox_core::{Exec, Storage};
    use std::collections::HashMap;
    use std::os::unix::net::UnixStream;
    use std::thread;
    use tempfile::tempdir;

    #[test]
    fn test_server_ping_pong() {
        let _ = env_logger::try_init();

        let dir = tempdir().unwrap();
        let socket_path = dir.path().join("test.sock");
        let storage = Storage::with_base_dir(dir.path().to_path_buf()).unwrap();
        let manager = ProcessManager::new(storage);

        let mut server = Server::new(socket_path.clone(), manager).unwrap();
        server.set_idle_timeout(None); // Disable for test
        let shutdown = server.shutdown_handle();

        // Start server in background
        let server_thread = thread::spawn(move || {
            server.run().unwrap();
        });

        // Give server time to start
        thread::sleep(Duration::from_millis(100));

        // Connect and send ping
        let stream = UnixStream::connect(&socket_path).unwrap();
        stream.set_nonblocking(false).unwrap();
        let mut reader = BufReader::new(&stream);
        let mut writer = BufWriter::new(&stream);

        write_message(&mut writer, &Request::Ping).unwrap();
        let response: Response = read_message(&mut reader).unwrap();

        assert!(matches!(response, Response::Pong));

        // Shutdown server
        shutdown.store(true, Ordering::SeqCst);
        server_thread.join().unwrap();
    }

    #[test]
    fn test_server_spawn() {
        let _ = env_logger::try_init();

        let dir = tempdir().unwrap();
        let socket_path = dir.path().join("test.sock");
        let storage = Storage::with_base_dir(dir.path().to_path_buf()).unwrap();
        let manager = ProcessManager::new(storage);

        let mut server = Server::new(socket_path.clone(), manager).unwrap();
        server.set_idle_timeout(None);
        let shutdown = server.shutdown_handle();

        // Start server
        let server_thread = thread::spawn(move || {
            server.run().unwrap();
        });

        thread::sleep(Duration::from_millis(100));

        // Connect and send spawn request
        let stream = UnixStream::connect(&socket_path).unwrap();
        stream.set_nonblocking(false).unwrap();
        let mut reader = BufReader::new(&stream);
        let mut writer = BufWriter::new(&stream);

        let request = Request::Spawn {
            run_id: "run_test123".to_string(),
            exec: Exec {
                argv: vec!["true".to_string()],
                cwd: ".".to_string(),
                env: HashMap::new(),
                timeout_sec: 0,
            },
            log_path: dir.path().join("test.log"),
        };

        write_message(&mut writer, &request).unwrap();
        let response: Response = read_message(&mut reader).unwrap();

        match response {
            Response::Spawned { pid, pgid } => {
                assert!(pid > 0);
                assert_eq!(pid, pgid);
            }
            Response::Error { message } => panic!("Spawn failed: {}", message),
            _ => panic!("Unexpected response"),
        }

        // Shutdown
        shutdown.store(true, Ordering::SeqCst);
        server_thread.join().unwrap();
    }
}
