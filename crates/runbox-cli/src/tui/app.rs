//! TUI Application state and main loop

use super::event::{Event, EventHandler};
use super::ui;
use super::views::{LogView, ProcessListView};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::prelude::*;
use runbox_core::{RuntimeRegistry, RunStatus, Storage};
use std::io;
use std::time::Duration;

/// Application mode/view
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMode {
    /// Process list view (main view)
    ProcessList,
    /// Log viewer for a specific run
    LogViewer,
    /// Help overlay
    Help,
}

/// Main TUI application state
pub struct App<'a> {
    /// Storage backend
    storage: &'a Storage,
    /// Runtime registry for process management
    runtime_registry: RuntimeRegistry,
    /// Current application mode
    mode: AppMode,
    /// Whether the app should quit
    should_quit: bool,
    /// Process list view state
    process_view: ProcessListView,
    /// Log viewer state (when viewing logs)
    log_view: Option<LogView>,
    /// Status message to display
    status_message: Option<String>,
    /// Tick counter for auto-refresh
    tick_count: u64,
}

impl<'a> App<'a> {
    /// Create a new App instance
    pub fn new(storage: &'a Storage) -> Self {
        Self {
            storage,
            runtime_registry: RuntimeRegistry::new(),
            mode: AppMode::ProcessList,
            should_quit: false,
            process_view: ProcessListView::new(),
            log_view: None,
            status_message: None,
            tick_count: 0,
        }
    }

    /// Run the main event loop
    pub fn run(&mut self, terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
        // Create event handler with 500ms tick rate for auto-refresh
        let event_handler = EventHandler::new(Duration::from_millis(500));

        // Initial data load
        self.refresh_data()?;

        loop {
            // Draw UI
            terminal.draw(|frame| self.draw(frame))?;

            // Handle events
            match event_handler.next()? {
                Event::Tick => {
                    self.tick_count += 1;
                    // Refresh data every 2 ticks (1 second)
                    if self.tick_count % 2 == 0 {
                        self.refresh_data()?;
                    }
                }
                Event::Key(key) => {
                    self.handle_key(key)?;
                }
                Event::Mouse(_) => {
                    // Mouse events not implemented yet
                }
                Event::Resize(_, _) => {
                    // Terminal will auto-redraw on resize
                }
            }

            if self.should_quit {
                break;
            }
        }

        Ok(())
    }

    /// Refresh data from storage
    fn refresh_data(&mut self) -> Result<()> {
        // Reconcile run statuses
        self.reconcile_runs()?;

        // Load runs
        let runs = self.storage.list_runs(100)?;
        self.process_view.update_runs(runs);

        // Update log view if active
        if let Some(ref mut log_view) = self.log_view {
            log_view.refresh(self.storage)?;
        }

        Ok(())
    }

    /// Reconcile run statuses by checking if processes are still alive
    fn reconcile_runs(&self) -> Result<()> {
        let runs = self.storage.list_runs(usize::MAX)?;

        for run in runs {
            if run.status != RunStatus::Running {
                continue;
            }

            if let Some(ref handle) = run.handle {
                if let Some(adapter) = self.runtime_registry.get(&run.runtime) {
                    if !adapter.is_alive(handle) {
                        // Process is no longer running, update status
                        let _ = self.storage.save_run_if_status_with(
                            &run.run_id,
                            &[RunStatus::Running],
                            |current| {
                                current.status = RunStatus::Unknown;
                                current.reconcile_reason =
                                    Some("Process not found (reconciled by TUI)".to_string());
                            },
                        );
                    }
                }
            }
        }

        Ok(())
    }

    /// Draw the current view
    fn draw(&self, frame: &mut Frame) {
        match self.mode {
            AppMode::ProcessList => {
                ui::draw_process_list(frame, &self.process_view, self.status_message.as_deref());
            }
            AppMode::LogViewer => {
                if let Some(ref log_view) = self.log_view {
                    ui::draw_log_view(frame, log_view);
                }
            }
            AppMode::Help => {
                ui::draw_process_list(frame, &self.process_view, self.status_message.as_deref());
                ui::draw_help_overlay(frame);
            }
        }
    }

    /// Handle keyboard input
    fn handle_key(&mut self, key: KeyEvent) -> Result<()> {
        // Global keybindings
        match key.code {
            KeyCode::Char('q') => {
                if self.mode == AppMode::Help {
                    self.mode = AppMode::ProcessList;
                } else if self.mode == AppMode::LogViewer {
                    self.mode = AppMode::ProcessList;
                    self.log_view = None;
                } else {
                    self.should_quit = true;
                }
                return Ok(());
            }
            KeyCode::Esc => {
                if self.mode != AppMode::ProcessList {
                    self.mode = AppMode::ProcessList;
                    self.log_view = None;
                } else {
                    self.should_quit = true;
                }
                return Ok(());
            }
            KeyCode::Char('?') => {
                self.mode = if self.mode == AppMode::Help {
                    AppMode::ProcessList
                } else {
                    AppMode::Help
                };
                return Ok(());
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.should_quit = true;
                return Ok(());
            }
            _ => {}
        }

        // Mode-specific keybindings
        match self.mode {
            AppMode::ProcessList => self.handle_process_list_key(key),
            AppMode::LogViewer => self.handle_log_viewer_key(key),
            AppMode::Help => Ok(()), // Help mode only responds to q/Esc/?
        }
    }

    /// Handle keys in process list view
    fn handle_process_list_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            // Navigation
            KeyCode::Down | KeyCode::Char('j') => {
                self.process_view.next();
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.process_view.previous();
            }
            KeyCode::Home | KeyCode::Char('g') => {
                self.process_view.first();
            }
            KeyCode::End | KeyCode::Char('G') => {
                self.process_view.last();
            }
            KeyCode::PageDown => {
                self.process_view.page_down(10);
            }
            KeyCode::PageUp => {
                self.process_view.page_up(10);
            }

            // Actions
            KeyCode::Enter | KeyCode::Char('l') => {
                // View logs for selected run
                if let Some(run) = self.process_view.selected_run() {
                    self.log_view = Some(LogView::new(run.run_id.clone()));
                    if let Some(ref mut lv) = self.log_view {
                        lv.refresh(self.storage)?;
                    }
                    self.mode = AppMode::LogViewer;
                }
            }
            KeyCode::Char('s') => {
                // Stop selected run
                if let Some(run) = self.process_view.selected_run() {
                    let run_id = run.run_id.clone();
                    let short_id = run.short_id().to_string();
                    let status = run.status.clone();
                    
                    if status == RunStatus::Running {
                        self.stop_run(&run_id, false)?;
                        self.status_message = Some(format!("Stopped: {}", short_id));
                        self.refresh_data()?;
                    } else {
                        self.status_message =
                            Some(format!("Cannot stop: {} ({})", short_id, status));
                    }
                }
            }
            KeyCode::Char('S') => {
                // Force stop selected run
                if let Some(run) = self.process_view.selected_run() {
                    let run_id = run.run_id.clone();
                    let short_id = run.short_id().to_string();
                    let status = run.status.clone();
                    
                    if status == RunStatus::Running {
                        self.stop_run(&run_id, true)?;
                        self.status_message = Some(format!("Force stopped: {}", short_id));
                        self.refresh_data()?;
                    }
                }
            }
            KeyCode::Char('a') => {
                // Attach to tmux/zellij session
                if let Some(run) = self.process_view.selected_run() {
                    let runtime = run.runtime.clone();
                    let short_id = run.short_id().to_string();
                    
                    if runtime == "tmux" || runtime == "zellij" {
                        // For attach, we need to exit TUI mode first
                        self.status_message = Some(format!(
                            "Use 'runbox attach {}' to attach to the session",
                            short_id
                        ));
                    } else {
                        self.status_message = Some(format!(
                            "Attach only for tmux/zellij (current: {})",
                            if runtime.is_empty() { "none".to_string() } else { runtime }
                        ));
                    }
                }
            }
            KeyCode::Char('r') => {
                // Manual refresh
                self.refresh_data()?;
                self.status_message = Some("Refreshed".to_string());
            }

            _ => {}
        }
        Ok(())
    }

    /// Handle keys in log viewer
    fn handle_log_viewer_key(&mut self, key: KeyEvent) -> Result<()> {
        if let Some(ref mut log_view) = self.log_view {
            match key.code {
                // Scrolling
                KeyCode::Down | KeyCode::Char('j') => log_view.scroll_down(1),
                KeyCode::Up | KeyCode::Char('k') => log_view.scroll_up(1),
                KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    log_view.scroll_down(20)
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    log_view.scroll_up(20)
                }
                KeyCode::PageDown => log_view.scroll_down(20),
                KeyCode::PageUp => log_view.scroll_up(20),
                KeyCode::Home | KeyCode::Char('g') => log_view.scroll_to_top(),
                KeyCode::End | KeyCode::Char('G') => log_view.scroll_to_bottom(),

                // Follow mode
                KeyCode::Char('f') => log_view.toggle_follow(),

                _ => {}
            }
        }
        Ok(())
    }

    /// Stop a running process
    fn stop_run(&self, run_id: &str, force: bool) -> Result<()> {
        let run = self.storage.load_run(run_id)?;

        if let Some(ref handle) = run.handle {
            if let Some(adapter) = self.runtime_registry.get(&run.runtime) {
                adapter.stop(handle, force)?;

                // Update status
                let _ = self.storage.save_run_if_status_with(
                    run_id,
                    &[RunStatus::Running, RunStatus::Pending],
                    |current| {
                        current.status = RunStatus::Killed;
                        if current.timeline.ended_at.is_none() {
                            current.timeline.ended_at = Some(chrono::Utc::now());
                        }
                    },
                );
            }
        }

        Ok(())
    }
}
