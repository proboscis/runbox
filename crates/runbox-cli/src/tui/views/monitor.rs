#![allow(dead_code)]
//!
//! Displays a list of running and recent processes with real-time updates.

use anyhow::Result;
use chrono::Utc;
use crossterm::event::KeyEvent;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::Span,
    widgets::{Row, Table, TableState},
    Frame,
};
use runbox_core::{Run, RunStatus, Storage};

use crate::tui::event::KeyBindings;
use crate::tui::ui::{format_duration, format_time, render_help_bar, titled_block, truncate_str, Styles};

/// Process info for display
#[derive(Clone)]
pub struct ProcessInfo {
    pub run: Run,
    pub runtime_display: String,
}

impl ProcessInfo {
    pub fn from_run(run: Run) -> Self {
        let runtime_display = if run.runtime.is_empty() {
            "background".to_string()
        } else {
            run.runtime.clone()
        };
        Self { run, runtime_display }
    }

    /// Calculate runtime duration in seconds
    pub fn runtime_seconds(&self) -> i64 {
        let start = self.run.timeline.started_at;
        let end = self.run.timeline.ended_at;
        
        match (start, end) {
            (Some(s), Some(e)) => (e - s).num_seconds(),
            (Some(s), None) => (Utc::now() - s).num_seconds(),
            _ => -1,
        }
    }

    /// Get command display string
    pub fn command_display(&self) -> String {
        self.run.exec.argv.join(" ")
    }

    /// Get started time display
    pub fn started_display(&self) -> String {
        self.run.timeline.started_at
            .map(|t| format_time(&t))
            .unwrap_or_else(|| "-".to_string())
    }
}

/// Monitor view state
pub struct MonitorView {
    /// All processes
    processes: Vec<ProcessInfo>,
    /// Table state for selection
    table_state: TableState,
    /// Current filter (None = show all)
    status_filter: Option<RunStatus>,
    /// Last refresh timestamp
    last_refresh: chrono::DateTime<Utc>,
}

impl MonitorView {
    pub fn new() -> Self {
        Self {
            processes: Vec::new(),
            table_state: TableState::default(),
            status_filter: None,
            last_refresh: Utc::now(),
        }
    }

    /// Refresh process list from storage
    pub fn refresh(&mut self, storage: &Storage) -> Result<()> {
        let runs = storage.list_runs(100)?;
        self.processes = runs.into_iter().map(ProcessInfo::from_run).collect();
        
        // Apply filter
        if let Some(ref status) = self.status_filter {
            self.processes.retain(|p| &p.run.status == status);
        }

        // Maintain selection within bounds
        if !self.processes.is_empty() {
            let selected = self.table_state.selected().unwrap_or(0);
            if selected >= self.processes.len() {
                self.table_state.select(Some(self.processes.len() - 1));
            } else if self.table_state.selected().is_none() {
                self.table_state.select(Some(0));
            }
        } else {
            self.table_state.select(None);
        }

        self.last_refresh = Utc::now();
        Ok(())
    }

    /// Get the currently selected process
    pub fn selected_process(&self) -> Option<&ProcessInfo> {
        self.table_state.selected().and_then(|i| self.processes.get(i))
    }

    /// Get the run ID of the selected process
    pub fn selected_run_id(&self) -> Option<&str> {
        self.selected_process().map(|p| p.run.run_id.as_str())
    }

    /// Count running processes
    pub fn running_count(&self) -> usize {
        self.processes.iter().filter(|p| p.run.status == RunStatus::Running).count()
    }

    /// Handle keyboard input
    /// Returns: (should_quit, action_to_perform)
    pub fn handle_key(&mut self, key: KeyEvent) -> (bool, Option<MonitorAction>) {
        if KeyBindings::is_quit(key) {
            return (true, None);
        }

        if KeyBindings::is_up(key) {
            self.select_previous();
            return (false, None);
        }

        if KeyBindings::is_down(key) {
            self.select_next();
            return (false, None);
        }

        if KeyBindings::is_select(key) {
            if let Some(process) = self.selected_process() {
                return (false, Some(MonitorAction::ViewLogs(process.run.run_id.clone())));
            }
        }

        if KeyBindings::is_stop(key) {
            if let Some(process) = self.selected_process() {
                if process.run.status == RunStatus::Running {
                    return (false, Some(MonitorAction::StopProcess(process.run.run_id.clone())));
                }
            }
        }

        if KeyBindings::is_attach(key) {
            if let Some(process) = self.selected_process() {
                return (false, Some(MonitorAction::AttachProcess(process.run.run_id.clone())));
            }
        }

        if KeyBindings::is_refresh(key) {
            return (false, Some(MonitorAction::Refresh));
        }

        (false, None)
    }

    fn select_previous(&mut self) {
        if self.processes.is_empty() {
            return;
        }
        let i = match self.table_state.selected() {
            Some(i) => {
                if i > 0 {
                    i - 1
                } else {
                    self.processes.len() - 1
                }
            }
            None => 0,
        };
        self.table_state.select(Some(i));
    }

    fn select_next(&mut self) {
        if self.processes.is_empty() {
            return;
        }
        let i = match self.table_state.selected() {
            Some(i) => {
                if i < self.processes.len() - 1 {
                    i + 1
                } else {
                    0
                }
            }
            None => 0,
        };
        self.table_state.select(Some(i));
    }

    /// Render the monitor view
    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        // Layout: title area, table, help bar
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(3),  // Table (fills remaining space)
                Constraint::Length(1),  // Help bar
            ])
            .split(area);

        self.render_table(frame, chunks[0]);
        self.render_help(frame, chunks[1]);
    }

    fn render_table(&mut self, frame: &mut Frame, area: Rect) {
        let running_count = self.running_count();
        let title = format!(
            " runbox monitor ({} running, {} total) ",
            running_count,
            self.processes.len()
        );

        let header_cells = ["SHORT", "STATUS", "RUNTIME", "STARTED", "COMMAND"]
            .iter()
            .map(|h| Span::styled(*h, Styles::header()));
        let header = Row::new(header_cells).height(1);

        let rows: Vec<Row> = self.processes.iter().enumerate().map(|(idx, p)| {
            let selected = self.table_state.selected() == Some(idx);
            let pointer = if selected { "► " } else { "  " };
            
            let style = if selected {
                Styles::selected()
            } else {
                Style::default()
            };

            let status_style = if selected {
                Styles::selected()
            } else {
                Styles::status(&p.run.status)
            };

            let short_id = p.run.short_id();
            let runtime_str = format_duration(p.runtime_seconds());
            let cmd_display = truncate_str(&p.command_display(), 50);

            Row::new(vec![
                Span::styled(format!("{}{}", pointer, short_id), style),
                Span::styled(format!("{:8}", p.run.status), status_style),
                Span::styled(format!("{:10}", runtime_str), style),
                Span::styled(format!("{:10}", p.started_display()), style),
                Span::styled(cmd_display, style),
            ])
        }).collect();

        let widths = [
            Constraint::Length(12),  // SHORT (with pointer)
            Constraint::Length(10),  // STATUS
            Constraint::Length(12),  // RUNTIME
            Constraint::Length(12),  // STARTED
            Constraint::Min(20),     // COMMAND (flexible)
        ];

        let table = Table::new(rows, widths)
            .header(header)
            .block(titled_block(&title, true))
            .highlight_style(Style::default().add_modifier(Modifier::BOLD));

        frame.render_stateful_widget(table, area, &mut self.table_state);
    }

    fn render_help(&self, frame: &mut Frame, area: Rect) {
        let mut help_items = vec![
            ("↑/k", "Up"),
            ("↓/j", "Down"),
            ("Enter", "Logs"),
        ];

        // Add context-sensitive help
        if let Some(process) = self.selected_process() {
            if process.run.status == RunStatus::Running {
                help_items.push(("s", "Stop"));
            }
            // Show attach for tmux/zellij runs
            if matches!(process.run.runtime.as_str(), "tmux" | "zellij") {
                help_items.push(("a", "Attach"));
            }
        }

        help_items.push(("r", "Refresh"));
        help_items.push(("q", "Quit"));

        render_help_bar(frame, area, &help_items);
    }
}

impl Default for MonitorView {
    fn default() -> Self {
        Self::new()
    }
}

/// Actions that can be triggered from the monitor view
#[derive(Debug, Clone)]
pub enum MonitorAction {
    ViewLogs(String),
    StopProcess(String),
    AttachProcess(String),
    Refresh,
}
