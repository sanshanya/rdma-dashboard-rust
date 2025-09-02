use crate::app::App;
use crate::data::IBPort;
use itertools::Itertools;
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Paragraph},
};
use std::collections::BTreeMap;

const PORT_WIDTH: u16 = 44;
const BASE_PORT_HEIGHT: u16 = 4;
const QUEUE_HEADER_HEIGHT: u16 = 1;
const QUEUE_LINE_HEIGHT: u16 = 1;

fn group_queues_by_prio(
    queues: &std::collections::HashMap<String, f64>,
) -> BTreeMap<String, (Option<f64>, Option<f64>)> {
    let mut prio_map: BTreeMap<String, (Option<f64>, Option<f64>)> = BTreeMap::new();
    for (name, &speed) in queues {
        if speed <= 1.0 {
            continue;
        }
        let parts: Vec<&str> = name.split_whitespace().collect();
        if parts.len() == 2 {
            let direction = parts[0];
            let prio_part = parts[1];
            if let Some(prio_num_str) = prio_part.strip_prefix("Prio") {
                let entry = prio_map.entry(prio_num_str.to_string()).or_default();
                if direction == "RX" {
                    entry.0 = Some(speed);
                } else if direction == "TX" {
                    entry.1 = Some(speed);
                }
            }
        }
    }
    prio_map
}

pub fn render(app: &App, f: &mut Frame) {
    let main_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(f.area());
    render_ports_grid(app, f, main_layout[0]);
    render_footer(app, f, main_layout[1]);
}

fn render_footer(app: &App, f: &mut Frame, area: Rect) {
    let sort_mode = match app.sort_key {
        crate::app::SortKey::Name => "Name",
        crate::app::SortKey::Rx => "RX Speed",
        crate::app::SortKey::Tx => "TX Speed",
    };
    let title = format!(" RDMA Dashboard v{} ", app.version);
    let footer_text = Line::from(vec![
        Span::styled(title, Style::default().bold()),
        Span::raw(format!("─ Sorting by: {} ─ ", sort_mode)),
        Span::styled("(n)", Style::default().bold()),
        Span::raw("ame | "),
        Span::styled("(r)", Style::default().bold()),
        Span::raw("x | "),
        Span::styled("(t)", Style::default().bold()),
        Span::raw("x | "),
        Span::styled("(q)", Style::default().bold()),
        Span::raw("uit"),
    ]);
    f.render_widget(
        Paragraph::new(footer_text).alignment(Alignment::Center),
        area,
    );
}

fn render_ports_grid(app: &App, f: &mut Frame, area: Rect) {
    if area.width < PORT_WIDTH {
        let message = Paragraph::new("Terminal too narrow").alignment(Alignment::Center);
        f.render_widget(message, area);
        return;
    }
    let num_columns = (area.width / PORT_WIDTH) as usize;
    let port_chunks = app.ports.iter().chunks(num_columns);
    let mut vertical_constraints = Vec::new();
    for chunk in port_chunks.into_iter() {
        let max_height = chunk
            .map(calculate_port_height)
            .max()
            .unwrap_or(BASE_PORT_HEIGHT);
        vertical_constraints.push(Constraint::Length(max_height));
    }
    let rows_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vertical_constraints)
        .vertical_margin(1)
        .split(area);
    let port_chunks = app.ports.iter().chunks(num_columns);
    for (row_index, chunk) in port_chunks.into_iter().enumerate() {
        let ports_in_row: Vec<_> = chunk.collect();
        let num_ports_in_row = ports_in_row.len();
        let horizontal_constraints = std::iter::repeat(Constraint::Length(PORT_WIDTH))
            .take(num_ports_in_row)
            .collect::<Vec<_>>();
        if let Some(row_area) = rows_layout.get(row_index) {
            let columns_layout = Layout::default()
                .direction(Direction::Horizontal)
                .constraints(horizontal_constraints)
                .horizontal_margin(1)
                .split(*row_area);
            for (col_index, port) in ports_in_row.into_iter().enumerate() {
                if let Some(cell_area) = columns_layout.get(col_index) {
                    render_port_widget(f, *cell_area, port);
                }
            }
        }
    }
}

fn render_port_widget(f: &mut Frame, area: Rect, port: &IBPort) {
    let mut title_spans = vec![
        Span::styled(port.name.clone(), Style::default().bold()),
        Span::raw(" ["),
    ];
    let (state_style, state_str) = if port.read_error {
        (
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            "READ ERROR",
        )
    } else if port.state == "DOWN" {
        (
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            "DIE",
        )
    } else if port.state != "ACTIVE" {
        (Style::default().fg(Color::Yellow), port.state.as_str())
    } else {
        (Style::default().fg(Color::Green), port.state.as_str())
    };
    title_spans.push(Span::styled(state_str, state_style));
    title_spans.push(Span::raw("]"));
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .title(Line::from(title_spans));
    let mut text = vec![
        Line::from(format!("  RDMA RX:  {}", format_speed(port.rx_byteps))),
        Line::from(format!("  RDMA TX:  {}", format_speed(port.tx_byteps))),
    ];
    if port.errors > 0 {
        text.push(Line::from(Span::styled(
            format!("  Errors: {}", port.errors),
            Style::default().fg(Color::Red).bold(),
        )));
    }
    if port.eth_name.is_some() {
        if port.queue_read_error {
            text.push(Line::from(Span::styled(
                "  ethtool ERROR (sudo?)",
                Style::default().fg(Color::Red).bold(),
            )));
        } else {
            let grouped_queues = group_queues_by_prio(&port.queue_speeds);
            if !grouped_queues.is_empty() {
                text.push(Line::from("  ".to_string() + &"─".repeat(20)));
                for (prio_num, (rx_speed, tx_speed)) in grouped_queues {
                    let rx_part =
                        rx_speed.map_or("".to_string(), |s| format!("RX: {:>10}", format_speed(s)));
                    let tx_part =
                        tx_speed.map_or("".to_string(), |s| format!("TX: {:>10}", format_speed(s)));
                    let combined = match (rx_speed.is_some(), tx_speed.is_some()) {
                        (true, true) => format!("{} | {}", rx_part, tx_part),
                        (true, false) => rx_part,
                        (false, true) => tx_part,
                        (false, false) => "".to_string(),
                    };
                    text.push(Line::from(format!("  Prio{:<2} {}", prio_num, combined)));
                }
            }
        }
    }
    f.render_widget(Paragraph::new(text).block(block), area);
}

fn format_speed(bytes_per_sec: f64) -> String {
    if bytes_per_sec < 1024.0 {
        return format!("{:>6.2} B/s", bytes_per_sec);
    }
    let kbytes_per_sec = bytes_per_sec / 1024.0;
    if kbytes_per_sec < 1024.0 {
        return format!("{:>6.2} KB/s", kbytes_per_sec);
    }
    let mbytes_per_sec = kbytes_per_sec / 1024.0;
    if mbytes_per_sec < 1024.0 {
        return format!("{:>6.2} MB/s", mbytes_per_sec);
    }
    let gbytes_per_sec = mbytes_per_sec / 1024.0;
    format!("{:>6.2} GB/s", gbytes_per_sec)
}

fn calculate_port_height(port: &IBPort) -> u16 {
    let mut height = BASE_PORT_HEIGHT;
    if port.errors > 0 {
        height += 1;
    }
    if port.eth_name.is_some() {
        if port.queue_read_error {
            height += 1;
        } else {
            let grouped_queues_count = group_queues_by_prio(&port.queue_speeds).len();
            if grouped_queues_count > 0 {
                height += QUEUE_HEADER_HEIGHT + (grouped_queues_count as u16 * QUEUE_LINE_HEIGHT);
            }
        }
    }
    height
}