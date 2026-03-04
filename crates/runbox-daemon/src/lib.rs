//! runbox-daemon library
//!
//! This module exports types for CLI to communicate with the daemon.

pub mod process_manager;
pub mod protocol;
pub mod server;

pub use protocol::{read_message, write_message, Request, Response};
pub use server::{default_pid_path, default_socket_path, is_daemon_running};
