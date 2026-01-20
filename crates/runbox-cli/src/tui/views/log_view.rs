//! Log viewer with scrollback and search

use anyhow::Result;
use runbox_core::Storage;
use std::fs::File;
use std::io::{BufRead, BufReader};

/// State for the log viewer
pub struct LogView {
    /// Run ID being viewed
    run_id: String,
    /// Run short ID for display
    short_id: String,
    /// Command being run
    command: String,
    /// Log lines
    lines: Vec<String>,
    /// Current scroll position (line index at top of view)
    scroll_position: usize,
    /// Whether to follow new output (auto-scroll to bottom)
    follow_mode: bool,
    /// Search query (if searching) - for future use
    #[allow(dead_code)]
    search_query: Option<String>,
    /// Search result positions - for future use
    #[allow(dead_code)]
    search_results: Vec<usize>,
    /// Current search result index - for future use
    #[allow(dead_code)]
    current_search_result: usize,
}

impl LogView {
    /// Create a new log view for a run
    pub fn new(run_id: String) -> Self {
        let short_id = if run_id.len() >= 12 {
            run_id[4..12].to_string()
        } else {
            run_id.clone()
        };

        Self {
            run_id,
            short_id,
            command: String::new(),
            lines: Vec::new(),
            scroll_position: 0,
            follow_mode: true,
            search_query: None,
            search_results: Vec::new(),
            current_search_result: 0,
        }
    }

    /// Refresh log content from storage
    pub fn refresh(&mut self, storage: &Storage) -> Result<()> {
        // Load run info
        let run = storage.load_run(&self.run_id)?;
        self.command = run.exec.argv.join(" ");

        // Get log path
        let log_path = if let Some(ref log_ref) = run.log_ref {
            log_ref.path.clone()
        } else {
            storage.log_path(&self.run_id)
        };

        // Read log file
        if log_path.exists() {
            let file = File::open(&log_path)?;
            let reader = BufReader::new(file);
            self.lines = reader.lines().filter_map(|l| l.ok()).collect();

            // Auto-scroll to bottom if in follow mode
            if self.follow_mode && !self.lines.is_empty() {
                // Will be adjusted in render based on visible height
                self.scroll_position = self.lines.len().saturating_sub(1);
            }
        } else {
            self.lines = vec!["[No log file found]".to_string()];
        }

        Ok(())
    }

    /// Get the short ID
    pub fn short_id(&self) -> &str {
        &self.short_id
    }

    /// Get the command
    pub fn command(&self) -> &str {
        &self.command
    }

    /// Get scroll position
    pub fn scroll_position(&self) -> usize {
        self.scroll_position
    }

    /// Check if follow mode is enabled
    pub fn is_follow_mode(&self) -> bool {
        self.follow_mode
    }

    /// Toggle follow mode
    pub fn toggle_follow(&mut self) {
        self.follow_mode = !self.follow_mode;
        if self.follow_mode && !self.lines.is_empty() {
            self.scroll_position = self.lines.len().saturating_sub(1);
        }
    }

    /// Scroll down by n lines
    pub fn scroll_down(&mut self, n: usize) {
        self.follow_mode = false;
        self.scroll_position = (self.scroll_position + n).min(self.lines.len().saturating_sub(1));
    }

    /// Scroll up by n lines
    pub fn scroll_up(&mut self, n: usize) {
        self.follow_mode = false;
        self.scroll_position = self.scroll_position.saturating_sub(n);
    }

    /// Scroll to top
    pub fn scroll_to_top(&mut self) {
        self.follow_mode = false;
        self.scroll_position = 0;
    }

    /// Scroll to bottom
    pub fn scroll_to_bottom(&mut self) {
        self.follow_mode = true;
        if !self.lines.is_empty() {
            self.scroll_position = self.lines.len().saturating_sub(1);
        }
    }

    /// Get visible lines for rendering
    pub fn visible_lines(&self, height: usize) -> &[String] {
        let start = self.scroll_position;
        let end = (start + height).min(self.lines.len());
        &self.lines[start..end]
    }

    /// Get total line count
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }
}
