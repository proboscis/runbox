#![allow(dead_code)]
//!
//! Manages the application state, event loop, and view switching.

use anyhow::{Context, Result};
use crossterm::{
    event::DisableMouseCapture,
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use runbox_core::Storage;
use std::io::{self, Stdout};

use std::process::Command;
use std::time::Duration;

use super::event::{Event, EventHandler};
use super::views::{LogView, MonitorAction, MonitorView};

/// Current view/mode of the application
#[derive(Debug, Clone, PartialEq)]
pub enum AppMode {
    /// Process monitor (main view)
    Monitor,
    /// Log viewer for a specific run
    Logs { run_id: String },
}

/// Main TUI application state
pub struct App {
    /// Storage for data access
    storage: Storage,
    /// Current mode/view
    mode: AppMode,
    /// Process monitor view
    monitor: MonitorView,
    /// Log viewer (lazily created when needed)
    log_view: Option<LogView>,
    /// Should quit the application
    should_quit: bool,
    /// Message to display after exiting TUI (for attach)
    exit_message: Option<String>,
    /// Action to perform after exiting (e.g., attach to tmux)
    post_exit_action: Option<PostExitAction>,
}

/// Actions to perform after exiting the TUI
#[derive(Debug, Clone)]
pub enum PostExitAction {
    AttachTmux { session: String },
    AttachZellij { session: String },
}

impl App {
    /// Create a new application with the given storage
    pub fn new(storage: Storage) -> Self {
        Self {
            storage,
            mode: AppMode::Monitor,
            monitor: MonitorView::new(),
            log_view: None,
            should_quit: false,
            exit_message: None,
            post_exit_action: None,
        }
    }

    /// Initialize the application (load initial data)
    pub fn init(&mut self) -> Result<()> {
        self.monitor.refresh(&self.storage)?;
        Ok(())
    }

    /// Handle a tick event (periodic refresh)
    pub fn on_tick(&mut self) -> Result<()> {
        match self.mode {
            AppMode::Monitor => {
                self.monitor.refresh(&self.storage)?;
            }
            AppMode::Logs { .. } => {
                if let Some(ref mut log_view) = self.log_view {
                    log_view.refresh()?;
                }
            }
        }
        Ok(())
    }

    /// Handle a key event
    pub fn on_key(&mut self, key: crossterm::event::KeyEvent) -> Result<()> {
        match self.mode {
            AppMode::Monitor => {
                let (quit, action) = self.monitor.handle_key(key);
                self.should_quit = quit;

                if let Some(action) = action {
                    self.handle_monitor_action(action)?;
                }
            }
            AppMode::Logs { .. } => {
                if let Some(ref mut log_view) = self.log_view {
                    let (go_back, _action) = log_view.handle_key(key);
                    if go_back {
                        self.mode = AppMode::Monitor;
                        self.log_view = None;
                    }
                }
            }
        }
        Ok(())
    }

    /// Handle an action from the monitor view
    fn handle_monitor_action(&mut self, action: MonitorAction) -> Result<()> {
        match action {
            MonitorAction::ViewLogs(run_id) => {
                self.switch_to_logs(&run_id)?;
            }
            MonitorAction::StopProcess(run_id) => {
                self.stop_process(&run_id)?;
            }
            MonitorAction::AttachProcess(run_id) => {
                self.attach_process(&run_id)?;
            }
            MonitorAction::Refresh => {
                self.monitor.refresh(&self.storage)?;
            }
        }
        Ok(())
    }

    /// Switch to log view for a specific run
    fn switch_to_logs(&mut self, run_id: &str) -> Result<()> {
        let run = self.storage.load_run(run_id)?;
        let log_path = run.log_ref.clone()
            .map(|lr| lr.path)
            .unwrap_or_else(|| self.storage.log_path(run_id));
        
        let command = run.exec.argv.join(" ");
        let short_id = run.short_id().to_string();
        
        let mut log_view = LogView::new(
            run_id.to_string(),
            short_id,
            command,
            log_path,
        );
        log_view.refresh()?;
        
        self.log_view = Some(log_view);
        self.mode = AppMode::Logs { run_id: run_id.to_string() };
        Ok(())
    }

    /// Stop a running process
    fn stop_process(&mut self, run_id: &str) -> Result<()> {
        use runbox_core::DaemonClient;
        
        let client = DaemonClient::new();
        if client.is_running() {
            client.stop(run_id, false)?;
        } else {
            // Fallback: try to kill directly if we have PID
            let run = self.storage.load_run(run_id)?;
            if let Some(runbox_core::RuntimeHandle::Background { pid, .. }) = run.handle {
                unsafe {
                    libc::kill(pid as i32, libc::SIGTERM);
                }
            }
        }
        
        // Refresh to show updated status
        self.monitor.refresh(&self.storage)?;
        Ok(())
    }

    /// Attach to a running tmux/zellij session
    fn attach_process(&mut self, run_id: &str) -> Result<()> {
        let run = self.storage.load_run(run_id)?;
        
        match run.handle {
            Some(runbox_core::RuntimeHandle::Tmux { session, .. }) => {
                self.post_exit_action = Some(PostExitAction::AttachTmux { session });
                self.should_quit = true;
            }
            Some(runbox_core::RuntimeHandle::Zellij { session, .. }) => {
                self.post_exit_action = Some(PostExitAction::AttachZellij { session });
                self.should_quit = true;
            }
            _ => {
                // Not a tmux/zellij run, nothing to attach to
            }
        }
        
        Ok(())
    }

    /// Render the current view
    pub fn render(&mut self, frame: &mut ratatui::Frame) {
        let area = frame.area();
        
        match self.mode {
            AppMode::Monitor => {
                self.monitor.render(frame, area);
            }
            AppMode::Logs { .. } => {
                if let Some(ref mut log_view) = self.log_view {
                    log_view.render(frame, area);
                }
            }
        }
    }

    /// Check if the application should quit
    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    /// Get the post-exit action
    pub fn post_exit_action(&self) -> Option<&PostExitAction> {
        self.post_exit_action.as_ref()
    }
}

/// Terminal wrapper for cleanup
struct TerminalGuard {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl TerminalGuard {
    fn new() -> Result<Self> {
        enable_raw_mode().context("Failed to enable raw mode")?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen).context("Failed to enter alternate screen")?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend).context("Failed to create terminal")?;
        Ok(Self { terminal })
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture);
        let _ = self.terminal.show_cursor();
    }
}

/// Run the TUI application
pub fn run_app(storage: Storage, tick_rate: Duration) -> Result<Option<PostExitAction>> {
    // Set up terminal
    let mut guard = TerminalGuard::new()?;
    
    // Create app
    let mut app = App::new(storage);
    app.init()?;
    
    // Create event handler
    let events = EventHandler::new(tick_rate);
    
    // Main loop
    loop {
        // Draw
        guard.terminal.draw(|f| app.render(f))?;
        
        // Handle events
        match events.next()? {
            Event::Key(key) => {
                app.on_key(key)?;
            }
            Event::Tick => {
                app.on_tick()?;
            }
            Event::Resize(_, _) => {
                // Terminal will handle resize automatically
            }
        }
        
        if app.should_quit() {
            break;
        }
    }
    
    // Return post-exit action
    Ok(app.post_exit_action().cloned())
}

/// Execute post-exit action (after terminal is restored)
pub fn execute_post_exit_action(action: PostExitAction) -> Result<()> {
    match action {
        PostExitAction::AttachTmux { session } => {
            let status = Command::new("tmux")
                .args(["attach-session", "-t", &session])
                .status()
                .context("Failed to attach to tmux session")?;
            
            if !status.success() {
                anyhow::bail!("tmux attach failed with status: {:?}", status.code());
            }
        }
        PostExitAction::AttachZellij { session } => {
            let status = Command::new("zellij")
                .args(["attach", &session])
                .status()
                .context("Failed to attach to zellij session")?;
            
            if !status.success() {
                anyhow::bail!("zellij attach failed with status: {:?}", status.code());
            }
        }
    }
    Ok(())
}
