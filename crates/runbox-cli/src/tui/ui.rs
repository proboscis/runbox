//! UI rendering functions

use super::views::{LogView, ProcessListView};
use ratatui::{
    prelude::*,
    widgets::{
        Block, Borders, Cell, Clear, Paragraph, Row, Scrollbar, ScrollbarOrientation,
        ScrollbarState, Table, Wrap,
    },
};
use runbox_core::RunStatus;

/// Color scheme
mod colors {
    use ratatui::style::Color;

    pub const RUNNING: Color = Color::Green;
    pub const EXITED: Color = Color::Blue;
    pub const FAILED: Color = Color::Red;
    pub const KILLED: Color = Color::Yellow;
    pub const PENDING: Color = Color::Gray;
    pub const UNKNOWN: Color = Color::DarkGray;

    pub const SELECTED_BG: Color = Color::DarkGray;
    pub const HEADER: Color = Color::Cyan;
    pub const BORDER: Color = Color::Gray;
    pub const TITLE: Color = Color::White;
    pub const HELP_KEY: Color = Color::Yellow;
    pub const STATUS_BAR: Color = Color::DarkGray;
}

/// Get color for run status
fn status_color(status: &RunStatus) -> Color {
    match status {
        RunStatus::Running => colors::RUNNING,
        RunStatus::Exited => colors::EXITED,
        RunStatus::Failed => colors::FAILED,
        RunStatus::Killed => colors::KILLED,
        RunStatus::Pending => colors::PENDING,
        RunStatus::Unknown => colors::UNKNOWN,
    }
}

/// Draw the process list view
pub fn draw_process_list(frame: &mut Frame, view: &ProcessListView, status_msg: Option<&str>) {
    let area = frame.size();

    // Layout: main content + status bar at bottom
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(5),    // Main content
            Constraint::Length(1), // Status bar
        ])
        .split(area);

    let main_area = chunks[0];
    let status_area = chunks[1];

    // Title with process count
    let running_count = view.running_count();
    let title = format!(
        " runbox monitor ({} running / {} total) ",
        running_count,
        view.total_count()
    );

    // Create table
    let header = Row::new(vec![
        Cell::from("SHORT").style(Style::default().fg(colors::HEADER).bold()),
        Cell::from("STATUS").style(Style::default().fg(colors::HEADER).bold()),
        Cell::from("RUNTIME").style(Style::default().fg(colors::HEADER).bold()),
        Cell::from("STARTED").style(Style::default().fg(colors::HEADER).bold()),
        Cell::from("COMMAND").style(Style::default().fg(colors::HEADER).bold()),
    ])
    .height(1);

    let rows: Vec<Row> = view
        .runs()
        .iter()
        .enumerate()
        .map(|(i, run)| {
            let is_selected = i == view.selected_index();

            // Format runtime
            let runtime = if run.runtime.is_empty() {
                "-".to_string()
            } else {
                run.runtime.clone()
            };

            // Format started time
            let started = run
                .timeline
                .started_at
                .map(|t| t.format("%H:%M:%S").to_string())
                .unwrap_or_else(|| "-".to_string());

            // Format command
            let cmd = run.exec.argv.join(" ");
            let cmd_truncated = if cmd.len() > 40 {
                format!("{}...", &cmd[..37])
            } else {
                cmd
            };

            let cells = vec![
                Cell::from(format!("{}", if is_selected { "► " } else { "  " }) + run.short_id()),
                Cell::from(run.status.to_string()).style(Style::default().fg(status_color(&run.status))),
                Cell::from(runtime),
                Cell::from(started),
                Cell::from(cmd_truncated),
            ];

            let row = Row::new(cells);
            if is_selected {
                row.style(Style::default().bg(colors::SELECTED_BG))
            } else {
                row
            }
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(12), // SHORT
            Constraint::Length(10), // STATUS
            Constraint::Length(12), // RUNTIME
            Constraint::Length(10), // STARTED
            Constraint::Min(20),    // COMMAND
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(colors::BORDER))
            .title(title)
            .title_style(Style::default().fg(colors::TITLE).bold()),
    );

    frame.render_widget(table, main_area);

    // Status bar with keybindings
    let status_text = if let Some(msg) = status_msg {
        format!(
            " {} │ [Enter] Logs  [s] Stop  [a] Attach  [r] Refresh  [?] Help  [q] Quit",
            msg
        )
    } else {
        " [Enter] View logs  [s] Stop  [a] Attach  [r] Refresh  [?] Help  [q] Quit".to_string()
    };

    let status_bar = Paragraph::new(status_text)
        .style(Style::default().bg(colors::STATUS_BAR).fg(Color::White));

    frame.render_widget(status_bar, status_area);
}

/// Draw the log viewer
pub fn draw_log_view(frame: &mut Frame, view: &LogView) {
    let area = frame.size();

    // Layout: main content + status bar
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(5),    // Main content
            Constraint::Length(1), // Status bar
        ])
        .split(area);

    let main_area = chunks[0];
    let status_area = chunks[1];

    // Calculate visible height
    let visible_height = main_area.height.saturating_sub(2) as usize; // -2 for borders

    // Title
    let follow_indicator = if view.is_follow_mode() { "[FOLLOW]" } else { "" };
    let title = format!(
        " Logs: {} ({}) {} ",
        view.short_id(),
        view.command(),
        follow_indicator
    );

    // Create paragraph with log content
    let visible_lines = view.visible_lines(visible_height);
    let log_text: String = visible_lines.join("\n");

    let log_paragraph = Paragraph::new(log_text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(colors::BORDER))
                .title(title)
                .title_style(Style::default().fg(colors::TITLE).bold()),
        )
        .wrap(Wrap { trim: false });

    frame.render_widget(log_paragraph, main_area);

    // Scrollbar
    if view.line_count() > visible_height {
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("▲"))
            .end_symbol(Some("▼"));

        let mut scrollbar_state = ScrollbarState::new(view.line_count())
            .position(view.scroll_position());

        let margin = Margin {
            horizontal: 0,
            vertical: 1,
        };

        frame.render_stateful_widget(
            scrollbar,
            main_area.inner(&margin),
            &mut scrollbar_state,
        );
    }

    // Status bar
    let position_info = format!(
        " Line {}/{} ",
        view.scroll_position() + 1,
        view.line_count()
    );

    let status_text = format!(
        "{}│ [j/k] Scroll  [g/G] Top/Bottom  [f] Follow  [/] Search  [q] Back",
        position_info
    );

    let status_bar = Paragraph::new(status_text)
        .style(Style::default().bg(colors::STATUS_BAR).fg(Color::White));

    frame.render_widget(status_bar, status_area);
}

/// Draw help overlay
pub fn draw_help_overlay(frame: &mut Frame) {
    let area = frame.size();

    // Calculate center popup area
    let popup_width = 60;
    let popup_height = 20;
    let popup_area = centered_rect(popup_width, popup_height, area);

    // Clear the popup area
    frame.render_widget(Clear, popup_area);

    let help_text = vec![
        Line::from(vec![
            Span::styled("Navigation", Style::default().bold().fg(colors::HEADER)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  j/↓     ", Style::default().fg(colors::HELP_KEY)),
            Span::raw("Move down"),
        ]),
        Line::from(vec![
            Span::styled("  k/↑     ", Style::default().fg(colors::HELP_KEY)),
            Span::raw("Move up"),
        ]),
        Line::from(vec![
            Span::styled("  g       ", Style::default().fg(colors::HELP_KEY)),
            Span::raw("Go to top"),
        ]),
        Line::from(vec![
            Span::styled("  G       ", Style::default().fg(colors::HELP_KEY)),
            Span::raw("Go to bottom"),
        ]),
        Line::from(vec![
            Span::styled("  PgUp/Dn ", Style::default().fg(colors::HELP_KEY)),
            Span::raw("Page up/down"),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Actions", Style::default().bold().fg(colors::HEADER)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Enter/l ", Style::default().fg(colors::HELP_KEY)),
            Span::raw("View logs"),
        ]),
        Line::from(vec![
            Span::styled("  s       ", Style::default().fg(colors::HELP_KEY)),
            Span::raw("Stop process (SIGTERM)"),
        ]),
        Line::from(vec![
            Span::styled("  S       ", Style::default().fg(colors::HELP_KEY)),
            Span::raw("Force stop (SIGKILL)"),
        ]),
        Line::from(vec![
            Span::styled("  a       ", Style::default().fg(colors::HELP_KEY)),
            Span::raw("Attach (tmux/zellij only)"),
        ]),
        Line::from(vec![
            Span::styled("  r       ", Style::default().fg(colors::HELP_KEY)),
            Span::raw("Refresh"),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  ?       ", Style::default().fg(colors::HELP_KEY)),
            Span::raw("Toggle help"),
        ]),
        Line::from(vec![
            Span::styled("  q/Esc   ", Style::default().fg(colors::HELP_KEY)),
            Span::raw("Quit / Back"),
        ]),
    ];

    let help = Paragraph::new(help_text).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(colors::HEADER))
            .title(" Help ")
            .title_style(Style::default().fg(colors::TITLE).bold()),
    );

    frame.render_widget(help, popup_area);
}

/// Helper to create a centered rectangle
fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let horizontal_margin = (area.width.saturating_sub(width)) / 2;
    let vertical_margin = (area.height.saturating_sub(height)) / 2;

    Rect {
        x: area.x + horizontal_margin,
        y: area.y + vertical_margin,
        width: width.min(area.width),
        height: height.min(area.height),
    }
}
