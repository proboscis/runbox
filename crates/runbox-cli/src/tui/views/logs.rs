#![allow(dead_code)]
//!
//! Interactive log viewer with scrollback, search, and follow mode.

use anyhow::Result;
use crossterm::event::KeyEvent;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
    Frame,
};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

use crate::tui::event::KeyBindings;
use crate::tui::ui::{render_help_bar, titled_block};

/// Log viewer state
pub struct LogView {
    /// Run ID being viewed
    run_id: String,
    /// Run short ID for display
    short_id: String,
    /// Command being run
    command: String,
    /// Path to log file
    log_path: PathBuf,
    /// Loaded log lines
    lines: Vec<String>,
    /// Current scroll position (line offset)
    scroll_offset: usize,
    /// Viewport height (number of visible lines)
    viewport_height: usize,
    /// Follow mode (auto-scroll to bottom)
    follow_mode: bool,
    /// Search query
    search_query: String,
    /// Is in search input mode
    search_mode: bool,
    /// Current search match index
    search_match_idx: Option<usize>,
    /// All matching line indices
    search_matches: Vec<usize>,
    /// File size at last read (for detecting new content)
    last_file_size: u64,
}

impl LogView {
    pub fn new(run_id: String, short_id: String, command: String, log_path: PathBuf) -> Self {
        Self {
            run_id,
            short_id,
            command,
            log_path,
            lines: Vec::new(),
            scroll_offset: 0,
            viewport_height: 20,
            follow_mode: true,  // Start in follow mode
            search_query: String::new(),
            search_mode: false,
            search_match_idx: None,
            search_matches: Vec::new(),
            last_file_size: 0,
        }
    }

    /// Load or refresh log content
    pub fn refresh(&mut self) -> Result<()> {
        if !self.log_path.exists() {
            self.lines = vec!["Log file not found (run may still be starting...)".to_string()];
            return Ok(());
        }

        let file = File::open(&self.log_path)?;
        let metadata = file.metadata()?;
        let new_size = metadata.len();

        // Only reload if file has changed
        if new_size != self.last_file_size {
            let reader = BufReader::new(file);
            self.lines = reader.lines().filter_map(|l| l.ok()).collect();
            self.last_file_size = new_size;

            // Update search matches if we have a query
            if !self.search_query.is_empty() {
                self.update_search_matches();
            }

            // Auto-scroll to bottom in follow mode
            if self.follow_mode {
                self.scroll_to_bottom();
            }
        }

        Ok(())
    }

    /// Scroll to the bottom of the log
    pub fn scroll_to_bottom(&mut self) {
        if self.lines.len() > self.viewport_height {
            self.scroll_offset = self.lines.len() - self.viewport_height;
        } else {
            self.scroll_offset = 0;
        }
    }

    /// Scroll to the top of the log
    pub fn scroll_to_top(&mut self) {
        self.scroll_offset = 0;
        self.follow_mode = false;
    }

    /// Handle keyboard input
    /// Returns: (should_go_back, action)
    pub fn handle_key(&mut self, key: KeyEvent) -> (bool, Option<LogAction>) {
        // Handle search mode input
        if self.search_mode {
            return self.handle_search_input(key);
        }

        if KeyBindings::is_back(key) || KeyBindings::is_quit(key) {
            return (true, None);
        }

        if KeyBindings::is_up(key) {
            self.scroll_up(1);
            self.follow_mode = false;
            return (false, None);
        }

        if KeyBindings::is_down(key) {
            self.scroll_down(1);
            return (false, None);
        }

        if KeyBindings::is_page_up(key) {
            self.scroll_up(self.viewport_height.saturating_sub(1));
            self.follow_mode = false;
            return (false, None);
        }

        if KeyBindings::is_page_down(key) {
            self.scroll_down(self.viewport_height.saturating_sub(1));
            return (false, None);
        }

        if KeyBindings::is_goto_top(key) {
            self.scroll_to_top();
            return (false, None);
        }

        if KeyBindings::is_goto_bottom(key) {
            self.scroll_to_bottom();
            self.follow_mode = true;
            return (false, None);
        }

        if KeyBindings::is_follow(key) {
            self.follow_mode = !self.follow_mode;
            if self.follow_mode {
                self.scroll_to_bottom();
            }
            return (false, None);
        }

        if KeyBindings::is_search(key) {
            self.search_mode = true;
            self.search_query.clear();
            return (false, None);
        }

        // n = next match, N = previous match
        if let crossterm::event::KeyCode::Char('n') = key.code {
            if key.modifiers == crossterm::event::KeyModifiers::NONE {
                self.next_match();
            } else if key.modifiers == crossterm::event::KeyModifiers::SHIFT {
                self.prev_match();
            }
        }

        (false, None)
    }

    fn handle_search_input(&mut self, key: KeyEvent) -> (bool, Option<LogAction>) {
        match key.code {
            crossterm::event::KeyCode::Enter => {
                self.search_mode = false;
                self.update_search_matches();
                if !self.search_matches.is_empty() {
                    self.search_match_idx = Some(0);
                    self.scroll_to_match(0);
                }
            }
            crossterm::event::KeyCode::Esc => {
                self.search_mode = false;
                self.search_query.clear();
                self.search_matches.clear();
                self.search_match_idx = None;
            }
            crossterm::event::KeyCode::Backspace => {
                self.search_query.pop();
            }
            crossterm::event::KeyCode::Char(c) => {
                self.search_query.push(c);
            }
            _ => {}
        }
        (false, None)
    }

    fn update_search_matches(&mut self) {
        self.search_matches.clear();
        if self.search_query.is_empty() {
            return;
        }
        let query_lower = self.search_query.to_lowercase();
        for (idx, line) in self.lines.iter().enumerate() {
            if line.to_lowercase().contains(&query_lower) {
                self.search_matches.push(idx);
            }
        }
    }

    fn next_match(&mut self) {
        if self.search_matches.is_empty() {
            return;
        }
        let next_idx = match self.search_match_idx {
            Some(idx) => (idx + 1) % self.search_matches.len(),
            None => 0,
        };
        self.search_match_idx = Some(next_idx);
        self.scroll_to_match(next_idx);
    }

    fn prev_match(&mut self) {
        if self.search_matches.is_empty() {
            return;
        }
        let prev_idx = match self.search_match_idx {
            Some(idx) => {
                if idx > 0 {
                    idx - 1
                } else {
                    self.search_matches.len() - 1
                }
            }
            None => 0,
        };
        self.search_match_idx = Some(prev_idx);
        self.scroll_to_match(prev_idx);
    }

    fn scroll_to_match(&mut self, match_idx: usize) {
        if let Some(&line_idx) = self.search_matches.get(match_idx) {
            // Center the match in the viewport
            if line_idx >= self.viewport_height / 2 {
                self.scroll_offset = line_idx - self.viewport_height / 2;
            } else {
                self.scroll_offset = 0;
            }
            self.follow_mode = false;
        }
    }

    fn scroll_up(&mut self, lines: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(lines);
    }

    fn scroll_down(&mut self, lines: usize) {
        let max_offset = self.lines.len().saturating_sub(self.viewport_height);
        self.scroll_offset = (self.scroll_offset + lines).min(max_offset);
        
        // Enable follow mode if at bottom
        if self.scroll_offset >= max_offset {
            self.follow_mode = true;
        }
    }

    /// Render the log view
    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        // Update viewport height
        let inner_height = area.height.saturating_sub(4); // Borders + help bar
        self.viewport_height = inner_height as usize;

        // Layout: main content, help bar
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(3),
                Constraint::Length(1),
            ])
            .split(area);

        self.render_content(frame, chunks[0]);
        self.render_help(frame, chunks[1]);
    }

    fn render_content(&self, frame: &mut Frame, area: Rect) {
        let title = format!(
            " Logs: {} ({}) {} ",
            self.short_id,
            crate::tui::ui::truncate_str(&self.command, 30),
            if self.follow_mode { "[FOLLOW]" } else { "" }
        );

        // Build styled lines
        let visible_lines: Vec<Line> = self.lines
            .iter()
            .enumerate()
            .skip(self.scroll_offset)
            .take(self.viewport_height)
            .map(|(idx, line)| {
                // Highlight search matches
                let is_match = self.search_matches.contains(&idx);
                let is_current_match = self.search_match_idx
                    .map(|mi| self.search_matches.get(mi) == Some(&idx))
                    .unwrap_or(false);

                let style = if is_current_match {
                    Style::default().bg(Color::Yellow).fg(Color::Black)
                } else if is_match {
                    Style::default().bg(Color::DarkGray)
                } else {
                    Style::default()
                };

                Line::from(Span::styled(line.clone(), style))
            })
            .collect();

        let mut block = titled_block(&title, true);

        // Show search input if in search mode
        if self.search_mode {
            let search_title = format!(" Search: {} ", self.search_query);
            block = block.title_bottom(Line::from(search_title));
        } else if !self.search_query.is_empty() {
            let match_info = if self.search_matches.is_empty() {
                "No matches".to_string()
            } else {
                let current = self.search_match_idx.map(|i| i + 1).unwrap_or(0);
                format!("{}/{} matches", current, self.search_matches.len())
            };
            let search_info = format!(" /{} ({}) ", self.search_query, match_info);
            block = block.title_bottom(Line::from(search_info));
        }

        let paragraph = Paragraph::new(visible_lines)
            .block(block);

        frame.render_widget(paragraph, area);

        // Render scrollbar
        if self.lines.len() > self.viewport_height {
            let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(Some("▲"))
                .end_symbol(Some("▼"));
            
            let mut scrollbar_state = ScrollbarState::new(self.lines.len())
                .position(self.scroll_offset);
            
            let scrollbar_area = Rect {
                x: area.x + area.width - 1,
                y: area.y + 1,
                width: 1,
                height: area.height.saturating_sub(2),
            };
            
            frame.render_stateful_widget(scrollbar, scrollbar_area, &mut scrollbar_state);
        }
    }

    fn render_help(&self, frame: &mut Frame, area: Rect) {
        let help_items = if self.search_mode {
            vec![
                ("Enter", "Search"),
                ("Esc", "Cancel"),
            ]
        } else {
            vec![
                ("↑/k", "Up"),
                ("↓/j", "Down"),
                ("/", "Search"),
                ("n/N", "Next/Prev match"),
                ("g/G", "Top/Bottom"),
                ("f", "Follow"),
                ("q/Esc", "Back"),
            ]
        };

        render_help_bar(frame, area, &help_items);
    }
}

/// Actions that can be triggered from the log view
#[derive(Debug, Clone)]
pub enum LogAction {
    // Future: copy to clipboard, save to file, etc.
}
