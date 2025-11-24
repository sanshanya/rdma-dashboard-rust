use crate::app::{App, ViewMode};
use crate::data::PortType;
use ratatui::{
    prelude::*,
    symbols,
    widgets::{Axis, Block, Borders, Chart, Dataset, GraphType, Paragraph, Scrollbar, ScrollbarOrientation},
};

// 定义每个图表占用的固定高度 (行数)
// 12行比较合适，既能看清波形，一屏也能显示 3-4 个
const CHART_HEIGHT: u16 = 12;

pub fn render(app: &App, f: &mut Frame) {
    let main_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(f.area());

    // 渲染主视图区域
    render_scrollable_view(app, f, main_layout[0]);

    // 渲染底部状态栏
    render_footer(app, f, main_layout[1]);
}

fn render_footer(app: &App, f: &mut Frame, area: Rect) {
    let mode_str = match app.view_mode {
        ViewMode::Table => "Table Mode",
        ViewMode::Chart => "Oscilloscope Mode (1ms Precision)",
    };
    
    let footer_text = Line::from(vec![
        Span::styled(format!(" RDMA Monitor v{} ", app.version), Style::default().bold()),
        Span::raw(" | "),
        Span::styled(mode_str, Style::default().fg(Color::Cyan)),
        Span::raw(" | "),
        Span::styled("↑/↓/j/k", Style::default().bold().fg(Color::Yellow)),
        Span::raw(" Scroll | "),
        Span::styled("Tab", Style::default().bold().fg(Color::Yellow)),
        Span::raw(" Switch View | "),
        Span::styled("q", Style::default().bold().fg(Color::Red)),
        Span::raw(" Quit"),
    ]);
    
    f.render_widget(
        Paragraph::new(footer_text).alignment(Alignment::Center),
        area,
    );
}

/// 统一的可滚动视图渲染逻辑
fn render_scrollable_view(app: &App, f: &mut Frame, area: Rect) {
    let total_items = app.histories.len();
    if total_items == 0 { return; }

    // 1. 计算当前屏幕能放下多少个图表
    let items_per_screen = (area.height / CHART_HEIGHT) as usize;
    // 确保至少渲染 1 个
    let num_render = if items_per_screen == 0 { 1 } else { items_per_screen + 1 }; 

    // 2. 计算当前可见的 items 范围
    let start_idx = app.vertical_scroll;
    let end_idx = (start_idx + num_render).min(total_items);

    // 3. 为可见的 items 创建布局区域
    // 我们手动计算每个 item 的 Rect
    let mut current_y = area.y;
    
    for i in start_idx..end_idx {
        // 防止超出屏幕底部
        if current_y >= area.y + area.height {
            break;
        }

        // 计算当前 item 的高度（处理最后一个可能被截断的情况）
        let remaining_height = (area.y + area.height).saturating_sub(current_y);
        let height = remaining_height.min(CHART_HEIGHT);
        
        if height == 0 { break; }

        let item_area = Rect {
            x: area.x,
            y: current_y,
            width: area.width - 1, // 留 1 列给滚动条
            height,
        };

        // 渲染单个 item
        match app.view_mode {
            ViewMode::Table => render_single_table_item(app, f, item_area, i),
            ViewMode::Chart => render_single_chart_item(app, f, item_area, i),
        }

        current_y += height;
    }

    // 4. 渲染滚动条
    let scrollbar = Scrollbar::default()
        .orientation(ScrollbarOrientation::VerticalRight)
        .begin_symbol(Some("↑"))
        .end_symbol(Some("↓"));
    
    let mut scroll_state = app.scroll_state;
    // ScrollbarState 需要知道 content_length 和 position (viewport_content_length 可选)
    // 这里我们用 item 数量作为长度
    scroll_state = scroll_state.content_length(total_items).viewport_content_length(items_per_screen);

    f.render_stateful_widget(
        scrollbar,
        area,
        &mut scroll_state,
    );
}

// 渲染单个图表项
fn render_single_chart_item(app: &App, f: &mut Frame, area: Rect, index: usize) {
    if let Some(history_lock) = app.histories.get(index) {
        if let Ok(history) = history_lock.read() {
            // 数据准备
            let rx_data: Vec<(f64, f64)> = history.rx_data.iter().cloned().collect();
            let tx_data: Vec<(f64, f64)> = history.tx_data.iter().cloned().collect();

            // 颜色
            let (rx_color, tx_color, title_prefix, border_color) = match history.port_type {
                PortType::Rdma => (Color::Magenta, Color::Cyan, "[RDMA]", Color::Magenta),
                PortType::Ethernet => (Color::Green, Color::Yellow, "[ETH] ", Color::Green),
            };

            // Y轴范围
            let max_val = rx_data.iter().chain(tx_data.iter())
                .map(|(_, v)| *v).fold(0.0, f64::max);
            let y_upper = if max_val <= 1024.0 { 1024.0 } else { max_val * 1.1 };
            
            // X轴范围
            let min_x = rx_data.first().map(|(t, _)| *t).unwrap_or(0.0);
            let max_x = rx_data.last().map(|(t, _)| *t).unwrap_or(10.0);

            // 绘图
            let datasets = vec![
                Dataset::default().name("RX").marker(symbols::Marker::Braille)
                    .graph_type(GraphType::Line).style(Style::default().fg(rx_color)).data(&rx_data),
                Dataset::default().name("TX").marker(symbols::Marker::Braille)
                    .graph_type(GraphType::Line).style(Style::default().fg(tx_color)).data(&tx_data),
            ];

            let chart = Chart::new(datasets)
                .block(Block::default()
                    .title(format!("{} {}", title_prefix, history.name))
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(border_color)))
                .x_axis(Axis::default().style(Style::default().fg(Color::DarkGray)).bounds([min_x, max_x])
                    .labels(vec![Span::raw(format!("{:.1}", min_x)), Span::raw(format!("{:.1}", max_x))]))
                .y_axis(Axis::default().style(Style::default().fg(Color::DarkGray)).bounds([0.0, y_upper])
                    .labels(vec![Span::raw("0"), Span::styled(format_speed(y_upper), Style::default().bold())]));

            f.render_widget(chart, area);
        }
    }
}

// 渲染单个表格项
fn render_single_table_item(app: &App, f: &mut Frame, area: Rect, index: usize) {
    if let Some(history_lock) = app.histories.get(index) {
        if let Ok(history) = history_lock.read() {
            let (last_rx, last_tx) = match (history.rx_data.back(), history.tx_data.back()) {
                (Some((_, rx)), Some((_, tx))) => (*rx, *tx),
                _ => (0.0, 0.0),
            };

            let (type_str, title_color) = match history.port_type {
                PortType::Rdma => ("[RDMA]", Color::Magenta),
                PortType::Ethernet => ("[ETH] ", Color::Green),
            };

            let text = vec![
                Line::from(vec![
                    Span::styled("RX Speed: ", Style::default().fg(Color::Green)),
                    Span::styled(format_speed(last_rx), Style::default().bold()),
                ]),
                Line::from(vec![
                    Span::styled("TX Speed: ", Style::default().fg(Color::Magenta)),
                    Span::styled(format_speed(last_tx), Style::default().bold()),
                ]),
            ];

            // 表格模式下，不需要那么高，可以在内部居中
            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(title_color))
                .title(Span::styled(format!("{} {}", type_str, history.name), Style::default().bold()));

            f.render_widget(
                Paragraph::new(text)
                    .block(block)
                    .alignment(Alignment::Left) // 表格模式下左对齐看起来更像列表
                    .wrap(ratatui::widgets::Wrap { trim: true }), 
                area
            );
        }
    }
}

fn format_speed(bytes_per_sec: f64) -> String {
    if bytes_per_sec < 1024.0 { return format!("{:.0} B/s", bytes_per_sec); }
    let kbytes = bytes_per_sec / 1024.0;
    if kbytes < 1024.0 { return format!("{:.1} KB/s", kbytes); }
    let mbytes = kbytes / 1024.0;
    if mbytes < 1024.0 { return format!("{:.1} MB/s", mbytes); }
    let gbytes = mbytes / 1024.0;
    format!("{:.1} GB/s", gbytes)
}