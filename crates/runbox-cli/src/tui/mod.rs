//! Terminal User Interface (TUI) for runbox
//!
//! Provides an interactive terminal interface with:
//! - Process monitor with real-time updates
//! - Interactive log viewer with scrollback and search
//! - Keyboard navigation and actions

pub mod app;
pub mod event;
pub mod ui;
pub mod views;

pub use app::run_app;
