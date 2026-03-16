use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Frame,
};

use crate::agent::AgentStatus;
use crate::app::{App, AppMode};

pub fn draw(frame: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Min(10),  // Main content
            Constraint::Length(3), // Status bar
        ])
        .split(frame.area());

    draw_header(frame, app, chunks[0]);

    match app.mode {
        AppMode::Input => draw_input(frame, app, chunks[1]),
        AppMode::Running | AppMode::Done => draw_agents(frame, app, chunks[1]),
    }

    draw_status_bar(frame, app, chunks[2]);
}

fn draw_header(frame: &mut Frame, app: &App, area: Rect) {
    let elapsed = app.elapsed_ms as f64 / 1000.0;
    let kernel_msgs = app.kernel.len();
    let title = format!(
        " concurrently  -  {} agents  |  kernel: {} msgs  |  {elapsed:.1}s ",
        app.agents.len(),
        kernel_msgs,
    );

    let header = Paragraph::new(Line::from(vec![
        Span::styled(
            title,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(" concurrently ")
            .title_style(Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)),
    );

    frame.render_widget(header, area);
}

fn draw_input(frame: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),  // Help text
            Constraint::Length(3),  // Input
            Constraint::Min(0),    // Spacer
        ])
        .split(area);

    let help_lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  Enter a complex task. It will be decomposed into parallel subtasks.",
            Style::default().fg(Color::Gray),
        )),
        Line::from(Span::styled(
            "  Each subtask runs as a concurrent agent with streaming output.",
            Style::default().fg(Color::Gray),
        )),
    ];
    let help = Paragraph::new(help_lines);
    frame.render_widget(help, chunks[0]);

    let input = Paragraph::new(app.input.as_str())
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan))
                .title(" Task ")
                .title_style(Style::default().fg(Color::Cyan)),
        )
        .style(Style::default().fg(Color::White));
    frame.render_widget(input, chunks[1]);

    // Show cursor
    frame.set_cursor_position((chunks[1].x + app.input.len() as u16 + 1, chunks[1].y + 1));
}

fn draw_agents(frame: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(30), // Agent list
            Constraint::Percentage(70), // Agent detail
        ])
        .split(area);

    // Agent list
    let items: Vec<ListItem> = app
        .agents
        .iter()
        .enumerate()
        .map(|(i, agent)| {
            let (status_icon, status_color) = match &agent.status {
                AgentStatus::Queued => ("○", Color::DarkGray),
                AgentStatus::Running => ("◉", Color::Yellow),
                AgentStatus::Done => ("●", Color::Green),
                AgentStatus::Error(_) => ("✗", Color::Red),
            };

            let style = if i == app.selected_agent {
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
                    .bg(Color::DarkGray)
            } else {
                Style::default().fg(Color::Gray)
            };

            // Show current tool or token count
            let info = if let Some(tool) = &agent.current_tool {
                format!(" [{tool}]")
            } else if agent.cost_usd > 0.0 {
                format!(" ${:.3}", agent.cost_usd)
            } else if agent.tokens_received > 0 {
                format!(" ({}tk)", agent.tokens_received)
            } else {
                String::new()
            };

            ListItem::new(Line::from(vec![
                Span::styled(format!(" {status_icon} "), Style::default().fg(status_color)),
                Span::styled(agent.name.clone(), style),
                Span::styled(
                    info,
                    Style::default().fg(if agent.current_tool.is_some() {
                        Color::Magenta
                    } else {
                        Color::DarkGray
                    }),
                ),
            ]))
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(" Agents ")
            .title_style(Style::default().fg(Color::Yellow)),
    );
    frame.render_widget(list, chunks[0]);

    // Agent detail view
    if let Some(agent) = app.agents.get(app.selected_agent) {
        let cost_str = if agent.cost_usd > 0.0 {
            format!(" ${:.4}", agent.cost_usd)
        } else {
            String::new()
        };
        let title = format!(" {} [{}]{} ", agent.name, agent.status, cost_str);
        let title_color = match &agent.status {
            AgentStatus::Running => Color::Yellow,
            AgentStatus::Done => Color::Green,
            AgentStatus::Error(_) => Color::Red,
            AgentStatus::Queued => Color::DarkGray,
        };

        let mut lines = vec![
            Line::from(vec![
                Span::styled("Task: ", Style::default().fg(Color::Cyan)),
                Span::styled(&agent.task, Style::default().fg(Color::White)),
            ]),
            Line::from(""),
        ];

        // Add output lines with tool call highlighting
        let output = &agent.output;
        for line in output.lines() {
            let color = if line.starts_with('[') && line.ends_with(']') {
                Color::Magenta
            } else {
                Color::White
            };
            lines.push(Line::from(Span::styled(
                line.to_string(),
                Style::default().fg(color),
            )));
        }

        // Show cursor if still running
        if agent.status == AgentStatus::Running {
            lines.push(Line::from(Span::styled(
                "▌",
                Style::default().fg(Color::Yellow),
            )));
        }

        let detail = Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(title_color))
                    .title(title)
                    .title_style(Style::default().fg(title_color)),
            )
            .wrap(Wrap { trim: false })
            .scroll((app.scroll_offset, 0));

        frame.render_widget(detail, chunks[1]);
    }
}

fn draw_status_bar(frame: &mut Frame, app: &App, area: Rect) {
    let keys = match app.mode {
        AppMode::Input => "Enter: submit | Ctrl+C: quit",
        AppMode::Running => "↑/↓: select agent | j/k: scroll | Ctrl+C: quit",
        AppMode::Done => "↑/↓: select | j/k: scroll | s: synthesize | n: new task | q: quit",
    };

    let status = Paragraph::new(Line::from(vec![
        Span::styled(
            format!(" {} ", app.status_message),
            Style::default().fg(Color::White),
        ),
        Span::styled(" | ", Style::default().fg(Color::DarkGray)),
        Span::styled(keys, Style::default().fg(Color::DarkGray)),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray)),
    );

    frame.render_widget(status, area);
}
