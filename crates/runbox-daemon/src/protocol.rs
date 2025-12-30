//! Protocol messages for CLI <-> Daemon communication
//!
//! Re-exports types from runbox-core to avoid duplication.

// Re-export protocol types from runbox-core
pub use runbox_core::daemon::{Request, Response};

use serde::{Deserialize, Serialize};
use std::io::{Read, Write};

/// Read a framed message from a stream
///
/// Uses length-prefix framing: [4-byte length (big endian)][JSON payload]
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
///
/// Uses length-prefix framing: [4-byte length (big endian)][JSON payload]
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
    use runbox_core::Exec;
    use std::collections::HashMap;
    use std::io::Cursor;
    use std::path::PathBuf;

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

    #[test]
    fn test_read_message_rejects_oversize() {
        // Create a message with length > 1MB
        let mut buf = Vec::new();
        let huge_len: u32 = 2 * 1024 * 1024; // 2MB
        buf.extend_from_slice(&huge_len.to_be_bytes());
        buf.extend_from_slice(b"{}"); // Some payload

        let mut cursor = Cursor::new(buf);
        let result: std::io::Result<Request> = read_message(&mut cursor);

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("too large"));
    }

    #[test]
    fn test_read_message_invalid_json() {
        // Create a message with invalid JSON
        let invalid_json = b"not valid json";
        let mut buf = Vec::new();
        let len = invalid_json.len() as u32;
        buf.extend_from_slice(&len.to_be_bytes());
        buf.extend_from_slice(invalid_json);

        let mut cursor = Cursor::new(buf);
        let result: std::io::Result<Request> = read_message(&mut cursor);

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("Invalid JSON"));
    }

    #[test]
    fn test_read_message_truncated() {
        // Create a message that claims more data than available
        let mut buf = Vec::new();
        let len: u32 = 100;
        buf.extend_from_slice(&len.to_be_bytes());
        buf.extend_from_slice(b"{}"); // Only 2 bytes, not 100

        let mut cursor = Cursor::new(buf);
        let result: std::io::Result<Request> = read_message(&mut cursor);

        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::UnexpectedEof);
    }
}
