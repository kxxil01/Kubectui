//! Scale dialog component for deployment replica scaling.
use ratatui::{layout::{Constraint, Direction, Layout, Rect}, prelude::{Color, Frame, Line, Span, Style}, widgets::{Block, Borders, Clear, Paragraph}};
use crate::app::ScaleDialogState;

pub fn render_scale_dialog(frame: &mut Frame, area: Rect, state: &ScaleDialogState) {
    let popup = centered_rect(60, 40, area);
    frame.render_widget(Clear, popup);
    let chunks = Layout::default().direction(Direction::Vertical).constraints([Constraint::Length(3), Constraint::Length(4), Constraint::Length(4), Constraint::Length(4), Constraint::Length(3)]).split(popup);

    let title = format!("Scale: {}", state.deployment_name);
    let header = Paragraph::new(Line::from(vec![Span::styled(title, Style::default().fg(Color::Cyan))])).block(Block::default().borders(Borders::ALL).title("Scale Deployment"));
    frame.render_widget(header, chunks[0]);

    let current_info = vec![Line::from(format!("Namespace: {}", state.namespace)), Line::from(format!("Current Replicas: {}", state.current_replicas))];
    let info_widget = Paragraph::new(current_info).block(Block::default().borders(Borders::ALL).title("Current"));
    frame.render_widget(info_widget, chunks[1]);

    let input_display = if state.input_buffer.is_empty() { "0".to_string() } else { state.input_buffer.clone() };
    let input_lines = vec![Line::from("Target Replicas (0-100):"), Line::from(Span::styled(format!("┌─ {} ─┐", input_display), Style::default().fg(Color::Yellow)))];
    let input_widget = Paragraph::new(input_lines).block(Block::default().borders(Borders::ALL).title("Input"));
    frame.render_widget(input_widget, chunks[2]);

    let error_text = if let Some(err) = &state.error { Line::from(Span::styled(format!("ERROR: {}", err), Style::default().fg(Color::Red))) } else { Line::from(Span::styled("Use +/- to adjust, type to enter, Enter to confirm", Style::default().fg(Color::Gray))) };
    let help_widget = Paragraph::new(error_text).block(Block::default().borders(Borders::ALL).title("Help"));
    frame.render_widget(help_widget, chunks[3]);

    let footer = Paragraph::new("[Enter] Confirm    [Esc] Cancel    [+/-] Adjust").style(Style::default().fg(Color::Gray)).block(Block::default().borders(Borders::ALL).title("Actions"));
    frame.render_widget(footer, chunks[4]);
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default().direction(Direction::Vertical).constraints([Constraint::Percentage((100 - percent_y) / 2), Constraint::Percentage(percent_y), Constraint::Percentage((100 - percent_y) / 2)]).split(area);
    Layout::default().direction(Direction::Horizontal).constraints([Constraint::Percentage((100 - percent_x) / 2), Constraint::Percentage(percent_x), Constraint::Percentage((100 - percent_x) / 2)]).split(vertical[1])[1]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_scale_dialog_state() {
        let state = ScaleDialogState::new("nginx", "default", 3);
        assert_eq!(state.deployment_name, "nginx");
    }
}
