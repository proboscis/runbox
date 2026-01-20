//! View components for the TUI

mod log_view;
mod process_list;

pub use log_view::LogView;
pub use process_list::ProcessListView;

/// Trait for views that can be rendered
#[allow(dead_code)]
pub trait View {
    /// Update the view's data
    fn refresh(&mut self, storage: &runbox_core::Storage) -> anyhow::Result<()>;
}
