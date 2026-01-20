//! Process list view for monitoring running and recent processes

use runbox_core::Run;

/// State for the process list view
pub struct ProcessListView {
    /// List of runs
    runs: Vec<Run>,
    /// Currently selected index
    selected: usize,
    /// Scroll offset for rendering - for future use with large lists
    #[allow(dead_code)]
    scroll_offset: usize,
}

impl ProcessListView {
    /// Create a new process list view
    pub fn new() -> Self {
        Self {
            runs: Vec::new(),
            selected: 0,
            scroll_offset: 0,
        }
    }

    /// Update the runs list
    pub fn update_runs(&mut self, runs: Vec<Run>) {
        // Try to keep selection on the same run if possible
        let prev_selected_id = self.selected_run().map(|r| r.run_id.clone());

        self.runs = runs;

        // Restore selection or clamp to valid range
        if let Some(prev_id) = prev_selected_id {
            if let Some(idx) = self.runs.iter().position(|r| r.run_id == prev_id) {
                self.selected = idx;
            } else {
                self.selected = self.selected.min(self.runs.len().saturating_sub(1));
            }
        } else {
            self.selected = 0;
        }
    }

    /// Get the currently selected run
    pub fn selected_run(&self) -> Option<&Run> {
        self.runs.get(self.selected)
    }

    /// Get all runs
    pub fn runs(&self) -> &[Run] {
        &self.runs
    }

    /// Get the selected index
    pub fn selected_index(&self) -> usize {
        self.selected
    }

    /// Move selection to next item
    pub fn next(&mut self) {
        if !self.runs.is_empty() {
            self.selected = (self.selected + 1).min(self.runs.len() - 1);
        }
    }

    /// Move selection to previous item
    pub fn previous(&mut self) {
        if !self.runs.is_empty() {
            self.selected = self.selected.saturating_sub(1);
        }
    }

    /// Move to first item
    pub fn first(&mut self) {
        self.selected = 0;
    }

    /// Move to last item
    pub fn last(&mut self) {
        if !self.runs.is_empty() {
            self.selected = self.runs.len() - 1;
        }
    }

    /// Page down
    pub fn page_down(&mut self, page_size: usize) {
        if !self.runs.is_empty() {
            self.selected = (self.selected + page_size).min(self.runs.len() - 1);
        }
    }

    /// Page up
    pub fn page_up(&mut self, page_size: usize) {
        self.selected = self.selected.saturating_sub(page_size);
    }

    /// Count of running processes
    pub fn running_count(&self) -> usize {
        self.runs
            .iter()
            .filter(|r| r.status == runbox_core::RunStatus::Running)
            .count()
    }

    /// Total count
    pub fn total_count(&self) -> usize {
        self.runs.len()
    }
}

impl Default for ProcessListView {
    fn default() -> Self {
        Self::new()
    }
}
