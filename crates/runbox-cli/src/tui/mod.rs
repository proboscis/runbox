//! Terminal User Interface (TUI) for runbox
//!
//! Provides interactive terminal-based views for:
//! - Process monitoring (`runbox monitor`)
//! - Log viewing with scrollback and search
//! - Runnable browser with filtering
//! - Dashboard combining all views

mod app;
mod event;
mod ui;
mod views;

pub use app::App;

use anyhow::Result;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::*;
use std::io;

/// Initialize the terminal for TUI mode
pub fn init_terminal() -> Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend)?;
    Ok(terminal)
}

/// Restore the terminal to normal mode
pub fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    Ok(())
}

/// Run the TUI application
pub fn run(storage: &runbox_core::Storage) -> Result<()> {
    let mut terminal = init_terminal()?;
    let result = App::new(storage).run(&mut terminal);
    restore_terminal(&mut terminal)?;
    result
}
