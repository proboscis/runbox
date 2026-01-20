//! TUI Views
//!
//! Contains different view implementations for the TUI.

pub mod monitor;
pub mod logs;

pub use monitor::{MonitorView, MonitorAction};
pub use logs::LogView;
