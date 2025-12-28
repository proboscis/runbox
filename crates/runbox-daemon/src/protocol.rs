//! Protocol messages for CLI <-> Daemon communication
//!
//! Uses JSON messages over Unix socket with length-prefix framing:
//! [4-byte length (big endian)][JSON payload]

use runbox_core::Exec;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::PathBuf;

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

/// Read a framed message from a stream
pub fn read_message<R: Read, T: for<'de> Deserialize<'de>>(reader: &mut R) -> std::io::Result<T> {
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
pub fn write_message<W: Write, T: Serialize>(writer: &mut W, msg: &T) -> std::io::Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::io::Cursor;

    #[test]
    fn test_request_serialization() {
        let request = Request::Spawn {
            run_id: "run_test123".to_string(),
            exec: Exec {
                argv: vec!["echo".to_string(), "hello".to_string()],
                cwd: ".".to_string(),
                env: HashMap::new(),
                timeout_sec: 0,
            },
            log_path: PathBuf::from("/tmp/test.log"),
        };

        let mut buf = Vec::new();
        write_message(&mut buf, &request).unwrap();

        let mut cursor = Cursor::new(buf);
        let parsed: Request = read_message(&mut cursor).unwrap();

        match parsed {
            Request::Spawn { run_id, .. } => {
                assert_eq!(run_id, "run_test123");
            }
            _ => panic!("Wrong request type"),
        }
    }

    #[test]
    fn test_response_serialization() {
        let response = Response::Spawned { pid: 1234, pgid: 1234 };

        let mut buf = Vec::new();
        write_message(&mut buf, &response).unwrap();

        let mut cursor = Cursor::new(buf);
        let parsed: Response = read_message(&mut cursor).unwrap();

        match parsed {
            Response::Spawned { pid, pgid } => {
                assert_eq!(pid, 1234);
                assert_eq!(pgid, 1234);
            }
            _ => panic!("Wrong response type"),
        }
    }

    #[test]
    fn test_ping_pong() {
        let request = Request::Ping;

        let mut buf = Vec::new();
        write_message(&mut buf, &request).unwrap();

        let mut cursor = Cursor::new(buf);
        let parsed: Request = read_message(&mut cursor).unwrap();

        assert!(matches!(parsed, Request::Ping));
    }
}
