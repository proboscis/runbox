#![allow(dead_code)]

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};
use runbox_core::RunStatus;

/// Color scheme for the TUI
pub struct Colors;

impl Colors {
    /// Status color based on run status
    pub fn status(status: &RunStatus) -> Color {
        match status {
            RunStatus::Running => Color::Green,
            RunStatus::Pending => Color::Yellow,
            RunStatus::Exited => Color::Blue,
            RunStatus::Failed => Color::Red,
            RunStatus::Killed => Color::Magenta,
            RunStatus::Unknown => Color::DarkGray,
        }
    }

    /// Highlighted/selected row
    pub fn selected() -> Color {
        Color::Cyan
    }

    /// Header text
    pub fn header() -> Color {
        Color::White
    }

    /// Muted/secondary text
    pub fn muted() -> Color {
        Color::DarkGray
    }

    /// Border color
    pub fn border() -> Color {
        Color::Gray
    }

    /// Active border
    pub fn active_border() -> Color {
        Color::Cyan
    }

    /// Help text
    pub fn help() -> Color {
        Color::DarkGray
    }
}

/// Style presets
pub struct Styles;

impl Styles {
    /// Selected row style
    pub fn selected() -> Style {
        Style::default()
            .fg(Colors::selected())
            .add_modifier(Modifier::BOLD)
    }

    /// Header style
    pub fn header() -> Style {
        Style::default()
            .fg(Colors::header())
            .add_modifier(Modifier::BOLD)
    }

    /// Muted text style
    pub fn muted() -> Style {
        Style::default().fg(Colors::muted())
    }

    /// Normal text
    pub fn normal() -> Style {
        Style::default()
    }

    /// Status style
    pub fn status(status: &RunStatus) -> Style {
        Style::default().fg(Colors::status(status))
    }
}

/// Create a centered rect of given percentage width and height
pub fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

/// Render help bar at the bottom
pub fn render_help_bar(frame: &mut Frame, area: Rect, items: &[(&str, &str)]) {
    let spans: Vec<Span> = items
        .iter()
        .flat_map(|(key, desc)| {
            vec![
                Span::styled(format!("[{}]", key), Style::default().fg(Color::Yellow)),
                Span::raw(" "),
                Span::styled(*desc, Style::default().fg(Colors::help())),
                Span::raw("  "),
            ]
        })
        .collect();

    let help_line = Line::from(spans);
    let help = Paragraph::new(help_line);
    frame.render_widget(help, area);
}

/// Render a status badge
pub fn status_span(status: &RunStatus) -> Span<'static> {
    let text = format!("{:8}", status.to_string());
    Span::styled(text, Styles::status(status))
}

/// Format duration for display
pub fn format_duration(seconds: i64) -> String {
    if seconds < 0 {
        return "N/A".to_string();
    }
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let secs = seconds % 60;
    
    if hours > 0 {
        format!("{:02}:{:02}:{:02}", hours, minutes, secs)
    } else {
        format!("{:02}:{:02}", minutes, secs)
    }
}

/// Format timestamp for display
pub fn format_time(dt: &chrono::DateTime<chrono::Utc>) -> String {
    dt.format("%H:%M:%S").to_string()
}

/// Truncate string to fit width with ellipsis
pub fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else if max_len <= 3 {
        s.chars().take(max_len).collect()
    } else {
        format!("{}...", &s[..max_len - 3])
    }
}

/// Create a block with title and borders
pub fn titled_block(title: &str, active: bool) -> Block<'_> {
    let border_style = if active {
        Style::default().fg(Colors::active_border())
    } else {
        Style::default().fg(Colors::border())
    };

    Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(border_style)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(0), "00:00");
        assert_eq!(format_duration(59), "00:59");
        assert_eq!(format_duration(60), "01:00");
        assert_eq!(format_duration(3599), "59:59");
        assert_eq!(format_duration(3600), "01:00:00");
        assert_eq!(format_duration(3661), "01:01:01");
        assert_eq!(format_duration(-1), "N/A");
    }

    #[test]
    fn test_truncate_str() {
        assert_eq!(truncate_str("hello", 10), "hello");
        assert_eq!(truncate_str("hello world", 8), "hello...");
        assert_eq!(truncate_str("hi", 2), "hi");
        assert_eq!(truncate_str("hello", 3), "hel");
    }
}
