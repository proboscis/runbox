//! runbox-daemon: Background process manager daemon
//!
//! This daemon manages background processes for runbox, capturing exit status
//! and updating storage when processes complete.
//!
//! Usage:
//!   runbox-daemon [--foreground] [--socket PATH]
//!
//! The daemon is typically started automatically by the CLI when needed.

mod process_manager;
mod protocol;
mod server;

use anyhow::{Context, Result};
use process_manager::ProcessManager;
use runbox_core::Storage;
use server::{
    acquire_daemon_lock, default_socket_path, lock_path_for_socket, pid_path_for_socket,
    remove_pid_file, write_pid_file, Server,
};
use std::path::PathBuf;

/// Daemonize the current process using double-fork
/// Returns Ok(true) if we're the daemon, Ok(false) if we're the parent
#[cfg(unix)]
fn daemonize() -> Result<bool> {
    use nix::sys::stat::Mode;
    use nix::unistd::{chdir, fork, setsid, ForkResult};

    // First fork
    match unsafe { fork() }? {
        ForkResult::Parent { .. } => {
            // Parent exits
            return Ok(false);
        }
        ForkResult::Child => {
            // Continue as first child
        }
    }

    // Create new session
    setsid()?;

    // Change to root directory to avoid blocking unmounts
    chdir("/")?;

    // Second fork to prevent acquiring a controlling terminal
    match unsafe { fork() }? {
        ForkResult::Parent { .. } => {
            // First child exits
            std::process::exit(0);
        }
        ForkResult::Child => {
            // Continue as daemon
        }
    }

    // Set file mode mask
    nix::sys::stat::umask(Mode::from_bits_truncate(0o022));

    // Close standard file descriptors
    // Redirect stdin/stdout/stderr to /dev/null
    let dev_null = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/null")?;

    use std::os::unix::io::AsRawFd;
    let null_fd = dev_null.as_raw_fd();

    unsafe {
        libc::dup2(null_fd, 0); // stdin
        libc::dup2(null_fd, 1); // stdout
        libc::dup2(null_fd, 2); // stderr
    }

    Ok(true)
}

fn setup_logging(foreground: bool) -> Result<()> {
    if foreground {
        // Log to stderr when running in foreground
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    } else {
        // Log to file when daemonized
        let log_dir = dirs::data_dir()
            .context("Could not find data directory")?
            .join("runbox");
        std::fs::create_dir_all(&log_dir)?;

        let log_path = log_dir.join("daemon.log");

        // Simple file-based logging
        let target = Box::new(
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_path)?,
        );

        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
            .target(env_logger::Target::Pipe(target))
            .init();
    }
    Ok(())
}

fn setup_signal_handlers(shutdown: std::sync::Arc<std::sync::atomic::AtomicBool>) -> Result<()> {
    // Handle SIGTERM and SIGINT for graceful shutdown
    signal_hook::flag::register(signal_hook::consts::SIGTERM, shutdown.clone())?;
    signal_hook::flag::register(signal_hook::consts::SIGINT, shutdown)?;
    Ok(())
}

fn run_daemon(socket_path: PathBuf, foreground: bool) -> Result<()> {
    // Acquire exclusive lock first to prevent duplicate daemons
    // Lock path is derived from socket path so different sockets can coexist
    let lock_path = lock_path_for_socket(&socket_path);
    let _lock_file = acquire_daemon_lock(&lock_path)?;
    // Note: lock_file must stay in scope to maintain the lock

    // Set up paths for cleanup (derived from socket path for multi-daemon support)
    let pid_path = pid_path_for_socket(&socket_path);
    let pid_path_cleanup = pid_path.clone();
    let socket_path_cleanup = socket_path.clone();
    let lock_path_cleanup = lock_path.clone();

    // Create storage and process manager
    let storage = Storage::new()?;
    let process_manager = ProcessManager::new(storage);

    // Reconcile any processes from before restart
    process_manager.reconcile_on_start()?;

    // Create server (binds socket)
    let server = Server::new(socket_path, process_manager)?;

    // Write PID file AFTER successful socket bind
    write_pid_file(&pid_path)?;

    // Set up signal handlers
    setup_signal_handlers(server.shutdown_handle())?;

    log::info!(
        "runbox-daemon started (pid: {}, foreground: {})",
        std::process::id(),
        foreground
    );

    // Run the server
    let result = server.run();

    // Cleanup
    remove_pid_file(&pid_path_cleanup);
    if socket_path_cleanup.exists() {
        let _ = std::fs::remove_file(&socket_path_cleanup);
    }
    if lock_path_cleanup.exists() {
        let _ = std::fs::remove_file(&lock_path_cleanup);
    }

    result
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    let foreground = args.contains(&"--foreground".to_string()) || args.contains(&"-f".to_string());

    let socket_path = args
        .iter()
        .position(|a| a == "--socket")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
        .unwrap_or_else(default_socket_path);

    if foreground {
        // Run in foreground
        setup_logging(true)?;
        run_daemon(socket_path, true)
    } else {
        // Daemonize
        if daemonize()? {
            // We're the daemon
            setup_logging(false)?;
            run_daemon(socket_path, false)
        } else {
            // We're the parent, exit successfully
            Ok(())
        }
    }
}
