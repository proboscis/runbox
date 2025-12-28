//! Daemon client for communicating with runbox-daemon
//!
//! Provides auto-start capability and request/response handling.

use crate::Exec;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::io::{BufReader, BufWriter, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

/// Request from CLI to Daemon
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Request {
    /// Spawn a new process
    Spawn {
        run_id: String,
        exec: Exec,
        log_path: PathBuf,
    },
    /// Stop a running process
    Stop { run_id: String, force: bool },
    /// Get status of a process
    Status { run_id: String },
    /// Ping to check if daemon is alive
    Ping,
    /// Request graceful shutdown
    Shutdown,
}

/// Response from Daemon to CLI
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Response {
    /// Process spawned successfully
    Spawned { pid: u32, pgid: u32 },
    /// Process stopped
    Stopped,
    /// Process status
    Status {
        alive: bool,
        exit_code: Option<i32>,
        signal: Option<i32>,
    },
    /// Pong response
    Pong,
    /// Shutdown acknowledged
    ShutdownAck,
    /// Error occurred
    Error { message: String },
}

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
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"));

    runtime_dir.join("runbox").join("daemon.pid")
}

/// Read a framed message from a stream
fn read_message<R: Read, T: for<'de> Deserialize<'de>>(reader: &mut R) -> std::io::Result<T> {
    // Read length prefix (4 bytes, big endian)
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf)?;
    let len = u32::from_be_bytes(len_buf) as usize;

    // Sanity check on length
    if len > 1024 * 1024 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("Message too large: {} bytes", len),
        ));
    }

    // Read payload
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf)?;

    // Parse JSON
    serde_json::from_slice(&buf).map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("Invalid JSON: {}", e),
        )
    })
}

/// Write a framed message to a stream
fn write_message<W: Write, T: Serialize>(writer: &mut W, msg: &T) -> std::io::Result<()> {
    let json = serde_json::to_vec(msg).map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("Failed to serialize: {}", e),
        )
    })?;

    // Write length prefix
    let len = json.len() as u32;
    writer.write_all(&len.to_be_bytes())?;

    // Write payload
    writer.write_all(&json)?;
    writer.flush()?;

    Ok(())
}

/// Daemon client for communicating with runbox-daemon
pub struct DaemonClient {
    socket_path: PathBuf,
}

impl DaemonClient {
    /// Create a new client with the default socket path
    pub fn new() -> Self {
        Self {
            socket_path: default_socket_path(),
        }
    }

    /// Create a new client with a custom socket path
    pub fn with_socket_path(socket_path: PathBuf) -> Self {
        Self { socket_path }
    }

    /// Check if the daemon is running
    pub fn is_running(&self) -> bool {
        UnixStream::connect(&self.socket_path).is_ok()
    }

    /// Connect to the daemon, starting it if necessary
    pub fn connect(&self) -> Result<UnixStream> {
        // Try to connect directly first
        if let Ok(stream) = UnixStream::connect(&self.socket_path) {
            return Ok(stream);
        }

        // Daemon not running, try to start it
        self.start_daemon()?;

        // Retry connection with backoff
        let max_retries = 10;
        let mut retry_delay = Duration::from_millis(50);

        for attempt in 1..=max_retries {
            std::thread::sleep(retry_delay);

            match UnixStream::connect(&self.socket_path) {
                Ok(stream) => {
                    log::debug!("Connected to daemon on attempt {}", attempt);
                    return Ok(stream);
                }
                Err(e) => {
                    log::debug!(
                        "Connection attempt {} failed: {}, retrying...",
                        attempt,
                        e
                    );
                    retry_delay = std::cmp::min(retry_delay * 2, Duration::from_secs(1));
                }
            }
        }

        bail!(
            "Failed to connect to daemon after {} attempts. Socket: {}",
            max_retries,
            self.socket_path.display()
        )
    }

    /// Start the daemon if not already running
    fn start_daemon(&self) -> Result<()> {
        log::debug!("Starting daemon...");

        // Find the daemon binary
        // First check if runbox-daemon is in PATH
        let daemon_path = which_daemon()?;

        // Create socket directory if needed
        if let Some(parent) = self.socket_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Start the daemon
        let mut cmd = Command::new(&daemon_path);
        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        // Pass custom socket path if different from default
        if self.socket_path != default_socket_path() {
            cmd.arg("--socket").arg(&self.socket_path);
        }

        let child = cmd.spawn().with_context(|| {
            format!(
                "Failed to start daemon from {}",
                daemon_path.display()
            )
        })?;

        log::debug!("Started daemon process {}", child.id());

        // The daemon will daemonize itself, so we don't need to track this child
        // It will double-fork and exit immediately
        Ok(())
    }

    /// Send a request and receive a response
    pub fn send(&self, request: Request) -> Result<Response> {
        let stream = self.connect()?;
        stream.set_read_timeout(Some(Duration::from_secs(30)))?;
        stream.set_write_timeout(Some(Duration::from_secs(30)))?;

        let mut reader = BufReader::new(&stream);
        let mut writer = BufWriter::new(&stream);

        // Send request
        write_message(&mut writer, &request).context("Failed to send request")?;

        // Read response
        let response: Response = read_message(&mut reader).context("Failed to read response")?;

        Ok(response)
    }

    /// Ping the daemon
    pub fn ping(&self) -> Result<bool> {
        match self.send(Request::Ping)? {
            Response::Pong => Ok(true),
            Response::Error { message } => bail!("Ping failed: {}", message),
            _ => bail!("Unexpected response to ping"),
        }
    }

    /// Spawn a process through the daemon
    pub fn spawn(&self, run_id: &str, exec: &Exec, log_path: &Path) -> Result<(u32, u32)> {
        let request = Request::Spawn {
            run_id: run_id.to_string(),
            exec: exec.clone(),
            log_path: log_path.to_path_buf(),
        };

        match self.send(request)? {
            Response::Spawned { pid, pgid } => Ok((pid, pgid)),
            Response::Error { message } => bail!("Spawn failed: {}", message),
            _ => bail!("Unexpected response to spawn"),
        }
    }

    /// Stop a process through the daemon
    pub fn stop(&self, run_id: &str, force: bool) -> Result<()> {
        let request = Request::Stop {
            run_id: run_id.to_string(),
            force,
        };

        match self.send(request)? {
            Response::Stopped => Ok(()),
            Response::Error { message } => bail!("Stop failed: {}", message),
            _ => bail!("Unexpected response to stop"),
        }
    }

    /// Get status of a process through the daemon
    pub fn status(&self, run_id: &str) -> Result<(bool, Option<i32>, Option<i32>)> {
        let request = Request::Status {
            run_id: run_id.to_string(),
        };

        match self.send(request)? {
            Response::Status {
                alive,
                exit_code,
                signal,
            } => Ok((alive, exit_code, signal)),
            Response::Error { message } => bail!("Status failed: {}", message),
            _ => bail!("Unexpected response to status"),
        }
    }

    /// Request daemon shutdown
    pub fn shutdown(&self) -> Result<()> {
        match self.send(Request::Shutdown)? {
            Response::ShutdownAck => Ok(()),
            Response::Error { message } => bail!("Shutdown failed: {}", message),
            _ => bail!("Unexpected response to shutdown"),
        }
    }
}

impl Default for DaemonClient {
    fn default() -> Self {
        Self::new()
    }
}

/// Find the daemon binary path
fn which_daemon() -> Result<PathBuf> {
    // First, check if RUNBOX_DAEMON_PATH env var is set
    if let Ok(path) = std::env::var("RUNBOX_DAEMON_PATH") {
        let path = PathBuf::from(path);
        if path.exists() {
            return Ok(path);
        }
    }

    // Check if runbox-daemon is in the same directory as the current executable
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let daemon_path = dir.join("runbox-daemon");
            if daemon_path.exists() {
                return Ok(daemon_path);
            }
        }
    }

    // Check PATH
    if let Ok(path_env) = std::env::var("PATH") {
        for dir in path_env.split(':') {
            let daemon_path = PathBuf::from(dir).join("runbox-daemon");
            if daemon_path.exists() {
                return Ok(daemon_path);
            }
        }
    }

    // Fallback to hoping it's in PATH
    Ok(PathBuf::from("runbox-daemon"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_socket_path() {
        let path = default_socket_path();
        assert!(path.ends_with("runbox/daemon.sock"));
    }

    #[test]
    fn test_default_pid_path() {
        let path = default_pid_path();
        assert!(path.ends_with("runbox/daemon.pid"));
    }

    #[test]
    fn test_request_serialization() {
        let request = Request::Ping;
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("Ping"));
    }

    #[test]
    fn test_response_serialization() {
        let response = Response::Spawned { pid: 1234, pgid: 1234 };
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("Spawned"));
        assert!(json.contains("1234"));
    }
}
