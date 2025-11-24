use crate::app::{App, ViewMode};
use crate::data::PortType;
use ratatui::{
    prelude::*,
    symbols,
    widgets::{Axis, Block, Borders, Chart, Dataset, GraphType, Paragraph},
};

/// UI 渲染主入口
pub fn render(app: &App, f: &mut Frame) {
    // 垂直布局：主区域 (Min(0)) + 底部状态栏 (Length(1))
    let main_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(f.area());

    // 根据视图模式分发渲染逻辑
    match app.view_mode {
        ViewMode::Table => render_table_view(app, f, main_layout[0]),
        ViewMode::Chart => render_chart_view(app, f, main_layout[0]),
    }

    render_footer(app, f, main_layout[1]);
}

/// 底部状态栏渲染
fn render_footer(app: &App, f: &mut Frame, area: Rect) {
    let mode_str = match app.view_mode {
        ViewMode::Table => "Table Mode (Instant Speed)",
        ViewMode::Chart => "Oscilloscope Mode (1ms Precision)",
    };
    
    let footer_text = Line::from(vec![
        Span::styled(format!(" RDMA Monitor v{} ", app.version), Style::default().bold()),
        Span::raw(" | "),
        Span::styled(mode_str, Style::default().fg(Color::Cyan)),
        Span::raw(" | Press "),
        Span::styled("<Tab>", Style::default().bold().fg(Color::Yellow)),
        Span::raw(" to switch view | "),
        Span::styled("<q>", Style::default().bold().fg(Color::Red)),
        Span::raw(" to quit"),
    ]);
    
    f.render_widget(
        Paragraph::new(footer_text).alignment(Alignment::Center),
        area,
    );
}

/// 表格模式：显示当前最新的瞬时速度数值
fn render_table_view(app: &App, f: &mut Frame, area: Rect) {
    let chunks = layout_grid(area, app.histories.len());

    for (i, history_lock) in app.histories.iter().enumerate() {
        if i >= chunks.len() { break; }
        
        // 获取读锁 (Read Lock)
        // 这里的读锁与 monitor.rs 中的 try_write() 配合：
        // 如果 UI 正在读，monitor 会跳过一次提交，避免阻塞，保证采集线程不掉帧。
        if let Ok(history) = history_lock.read() {
            // 获取队列中最晚的一个数据点 (Instant)
            let (last_rx, last_tx) = match (history.rx_data.back(), history.tx_data.back()) {
                (Some((_, rx)), Some((_, tx))) => (*rx, *tx),
                _ => (0.0, 0.0),
            };

            // 根据端口类型决定颜色和标题
            let (type_str, title_color) = match history.port_type {
                PortType::Rdma => ("[RDMA]", Color::Magenta),
                PortType::Ethernet => ("[ETH] ", Color::Green),
            };

            let text = vec![
                Line::from(vec![
                    Span::styled("RX: ", Style::default().fg(Color::Green)),
                    Span::raw(format_speed(last_rx)),
                ]),
                Line::from(""), // 空行分隔
                Line::from(vec![
                    Span::styled("TX: ", Style::default().fg(Color::Magenta)),
                    Span::raw(format_speed(last_tx)),
                ]),
            ];

            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(title_color))
                .title(Span::styled(
                    format!("{} {}", type_str, history.name), 
                    Style::default().bold()
                ));

            f.render_widget(
                Paragraph::new(text)
                    .block(block)
                    .alignment(Alignment::Center)
                    .wrap(ratatui::widgets::Wrap { trim: true }), 
                chunks[i]
            );
        }
    }
}

/// 图表模式：示波器风格，显示 1ms 精度的流量趋势
fn render_chart_view(app: &App, f: &mut Frame, area: Rect) {
    let chunks = layout_grid(area, app.histories.len());

    for (i, history_lock) in app.histories.iter().enumerate() {
        if i >= chunks.len() { break; }

        if let Ok(history) = history_lock.read() {
            // 1. 准备数据
            // 必须使用 clone()。
            // 虽然发生了内存拷贝，但避免了持有锁去进行 make_contiguous (需要写锁)，
            // 从而彻底避免了阻塞后台的高频采集线程。
            // 200 个点的 f64 拷贝开销在纳秒级，完全可以接受。
            let rx_data: Vec<(f64, f64)> = history.rx_data.iter().cloned().collect();
            let tx_data: Vec<(f64, f64)> = history.tx_data.iter().cloned().collect();

            // 2. 颜色决策
            let (rx_color, tx_color, title_prefix, border_color) = match history.port_type {
                PortType::Rdma => (Color::Magenta, Color::Cyan, "[RDMA]", Color::Magenta),
                PortType::Ethernet => (Color::Green, Color::Yellow, "[ETH] ", Color::Green),
            };

            // 3. 计算 Y 轴范围 (Auto-Scale)
            // 找出当前窗口内的最大值，作为 Y 轴上限
            let max_val = rx_data.iter().chain(tx_data.iter())
                .map(|(_, v)| *v)
                .fold(0.0, f64::max);
            
            // 留 10% 头部空间，且设定最小显示范围为 1KB/s (防止由 0 除导致的绘图错误)
            let y_upper = if max_val <= 1024.0 { 1024.0 } else { max_val * 1.1 };

            // 4. 计算 X 轴范围 (Window)
            let min_x = rx_data.first().map(|(t, _)| *t).unwrap_or(0.0);
            let max_x = rx_data.last().map(|(t, _)| *t).unwrap_or(10.0);

            // 5. 构造数据集 (Datasets)
            let datasets = vec![
                Dataset::default()
                    .name("RX")
                    .marker(symbols::Marker::Braille) // 使用盲文点阵获得最高分辨率
                    .graph_type(GraphType::Line)
                    .style(Style::default().fg(rx_color))
                    .data(&rx_data),
                Dataset::default()
                    .name("TX")
                    .marker(symbols::Marker::Braille)
                    .graph_type(GraphType::Line)
                    .style(Style::default().fg(tx_color))
                    .data(&tx_data),
            ];

            // 6. 创建并渲染图表
            let chart = Chart::new(datasets)
                .block(Block::default()
                    .title(Span::styled(
                        format!("{} {}", title_prefix, history.name), 
                        Style::default().bold()
                    ))
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(border_color)))
                .x_axis(Axis::default()
                    // 省略 X 轴标题以节省空间
                    .style(Style::default().fg(Color::DarkGray))
                    .bounds([min_x, max_x])
                    .labels(vec![
                        // 只显示时间窗的起止点
                        Span::raw(format!("{:.1}", min_x)),
                        Span::raw(format!("{:.1}", max_x)),
                    ]))
                .y_axis(Axis::default()
                    // 省略 Y 轴标题
                    .style(Style::default().fg(Color::DarkGray))
                    .bounds([0.0, y_upper])
                    .labels(vec![
                        Span::raw("0"),
                        Span::styled(format_speed(y_upper), Style::default().bold()),
                    ]));

            f.render_widget(chart, chunks[i]);
        }
    }
}

/// 辅助函数：自动计算网格布局 (N x M)
/// 根据要显示的图表数量，自动切分屏幕区域，尽量保持方正
fn layout_grid(area: Rect, count: usize) -> Vec<Rect> {
    if count == 0 { return vec![]; }
    
    // 简单的布局策略：
    // 1 -> 1列
    // 2-4 -> 2列
    // >4 -> 3列
    let cols = if count == 1 { 1 } else if count <= 4 { 2 } else { 3 };
    let rows = (count as f64 / cols as f64).ceil() as usize;

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            std::iter::repeat(Constraint::Ratio(1, rows as u32))
                .take(rows)
                .collect::<Vec<_>>()
        )
        .split(area);

    let mut cells = Vec::new();
    for chunk in chunks.iter() {
        let row_cells = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(
                std::iter::repeat(Constraint::Ratio(1, cols as u32))
                    .take(cols)
                    .collect::<Vec<_>>()
            )
            .split(*chunk);
        cells.extend_from_slice(&row_cells);
    }
    cells
}

/// 辅助函数：人类可读的速度格式化
fn format_speed(bytes_per_sec: f64) -> String {
    if bytes_per_sec < 1024.0 {
        return format!("{:.0} B/s", bytes_per_sec);
    }
    let kbytes = bytes_per_sec / 1024.0;
    if kbytes < 1024.0 {
        return format!("{:.1} KB/s", kbytes);
    }
    let mbytes = kbytes / 1024.0;
    if mbytes < 1024.0 {
        return format!("{:.1} MB/s", mbytes);
    }
    let gbytes = mbytes / 1024.0;
    format!("{:.1} GB/s", gbytes)
}