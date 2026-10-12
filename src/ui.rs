//! Terminal rendering. Pure with respect to application state: it only reads
//! [`App`] and writes to the frame.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Cell, Paragraph, Row, Table, TableState, Wrap};

use crate::app::{App, Focus, ScanState};
use crate::ports::{DEFAULT_PORTS, service_name};
use crate::scan::PortStatus;

const HINT: &str = "Enter scan  d defaults  r rescan  c clear  Tab focus  up/down move  q quit";

/// Draw the whole interface into `frame`.
pub fn draw(frame: &mut Frame, app: &App) {
    let [input_area, results_area, footer_area] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    draw_input(frame, app, input_area);
    draw_results(frame, app, results_area);
    draw_footer(frame, app, footer_area);
}

fn draw_input(frame: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let focused = app.focus() == Focus::Input;
    let border_style = if focused {
        Style::new().fg(Color::Yellow)
    } else {
        Style::new().fg(Color::DarkGray)
    };

    let title = Line::from(vec![
        Span::raw(" Ports "),
        Span::styled(format!("{}  ", app.host()), Style::new().fg(Color::Cyan)),
    ]);

    let block = Block::bordered().title(title).border_style(border_style);

    let body = if app.input().is_empty() {
        let defaults = DEFAULT_PORTS
            .iter()
            .map(u16::to_string)
            .collect::<Vec<_>>()
            .join(",");
        Line::from(Span::styled(
            format!("default: {defaults}"),
            Style::new().fg(Color::DarkGray),
        ))
    } else {
        Line::from(Span::raw(app.input()))
    };

    frame.render_widget(Paragraph::new(body).block(block), area);

    if focused {
        let inner_width = area.width.saturating_sub(2);
        let max_offset = inner_width.saturating_sub(1);
        let offset = u16::try_from(app.cursor())
            .unwrap_or(u16::MAX)
            .min(max_offset);
        frame.set_cursor_position((area.x + 1 + offset, area.y + 1));
    }
}

fn draw_results(frame: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let header = Row::new(["PORT", "SERVICE", "STATUS"]).style(Style::new().bold());

    let rows = app.results().iter().map(|result| {
        let status_style = match result.status {
            PortStatus::Open => Style::new().fg(Color::Green).bold(),
            PortStatus::Closed => Style::new().fg(Color::Red),
            PortStatus::Filtered => Style::new().fg(Color::Yellow),
        };
        Row::new(vec![
            Cell::from(result.port.to_string()),
            Cell::from(service_name(result.port).unwrap_or("-")),
            Cell::from(Span::styled(result.status.label(), status_style)),
        ])
    });

    let title = format!(" Results ({}) ", app.results().len());
    let table = Table::new(
        rows,
        [
            Constraint::Length(8),
            Constraint::Length(20),
            Constraint::Length(12),
        ],
    )
    .header(header)
    .block(Block::bordered().title(title))
    .highlight_symbol("> ")
    .row_highlight_style(Style::new().add_modifier(Modifier::REVERSED));

    let mut state =
        TableState::default().with_selected((!app.results().is_empty()).then_some(app.selected()));

    frame.render_stateful_widget(table, area, &mut state);
}

fn draw_footer(frame: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let [left, right] =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(hint_width())]).areas(area);

    let mut spans = vec![Span::styled(
        app.status_text(),
        Style::new().fg(state_color(app.state())),
    )];
    if let Some(message) = app.message() {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(message, Style::new().fg(Color::Red)));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), left);
    frame.render_widget(
        Paragraph::new(Span::styled(HINT, Style::new().fg(Color::DarkGray)))
            .alignment(ratatui::layout::Alignment::Right)
            .wrap(Wrap { trim: true }),
        right,
    );
}

fn state_color(state: ScanState) -> Color {
    match state {
        ScanState::Idle => Color::Gray,
        ScanState::Scanning { .. } => Color::Yellow,
        ScanState::Done => Color::Green,
        ScanState::Failed => Color::Red,
    }
}

fn hint_width() -> u16 {
    u16::try_from(HINT.chars().count()).unwrap_or(u16::MAX)
}
