use crate::app::{App, ViewMode};
use ratatui::{
    prelude::*,
    symbols,
    widgets::{Axis, Block, Borders, Chart, Dataset, GraphType, Paragraph},
};

pub fn render(app: &App, f: &mut Frame) {
    let main_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(f.area());

    // 根据模式渲染不同内容
    match app.view_mode {
        ViewMode::Table => render_table_view(app, f, main_layout[0]),
        ViewMode::Chart => render_chart_view(app, f, main_layout[0]),
    }

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

// --- 表格模式 (简化的数字视图) ---
fn render_table_view(app: &App, f: &mut Frame, area: Rect) {
    // 这里简单地将屏幕分割成网格，显示最新的瞬时速度
    let chunks = layout_grid(area, app.histories.len());

    for (i, history_lock) in app.histories.iter().enumerate() {
        if i >= chunks.len() { break; }
        
        // 获取读锁
        if let Ok(history) = history_lock.read() {
            // 获取最新的一帧数据
            let (last_rx, last_tx) = match (history.rx_data.back(), history.tx_data.back()) {
                (Some((_, rx)), Some((_, tx))) => (*rx, *tx),
                _ => (0.0, 0.0),
            };

            let text = vec![
                Line::from(vec![
                    Span::styled("RX: ", Style::default().fg(Color::Green)),
                    Span::raw(format_speed(last_rx)),
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled("TX: ", Style::default().fg(Color::Magenta)),
                    Span::raw(format_speed(last_tx)),
                ]),
            ];

            let block = Block::default()
                .borders(Borders::ALL)
                .title(Span::styled(history.name.clone(), Style::default().bold()));

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

// --- 核心：图表模式 ---
fn render_chart_view(app: &App, f: &mut Frame, area: Rect) {
    let chunks = layout_grid(area, app.histories.len());

    for (i, history_lock) in app.histories.iter().enumerate() {
        if i >= chunks.len() { break; }

        if let Ok(history) = history_lock.read() {
            // 1. 准备数据
            // Ratatui 需要 Slice of (f64, f64)，所以我们必须把 VecDeque 转为 Vec
            // 虽然有内存分配，但对于 UI 渲染频率(10fps)和数据量(200点)来说微不足道
            let rx_data: Vec<(f64, f64)> = history.rx_data.iter().cloned().collect();
            let tx_data: Vec<(f64, f64)> = history.tx_data.iter().cloned().collect();

            // 2. 计算 Y 轴范围 (Auto-Scale)
            // 找出当前窗口内的最大值，作为 Y 轴上限
            let max_val = rx_data.iter().chain(tx_data.iter())
                .map(|(_, v)| *v)
                .fold(0.0, f64::max);
            
            // 留一点头部空间 (10%)，并防止除以零
            let y_upper = if max_val <= 1024.0 { 1024.0 } else { max_val * 1.1 };

            // 3. 计算 X 轴范围 (Time Window)
            let min_x = rx_data.first().map(|(t, _)| *t).unwrap_or(0.0);
            let max_x = rx_data.last().map(|(t, _)| *t).unwrap_or(10.0);

            // 4. 定义数据集
            let datasets = vec![
                Dataset::default()
                    .name("RX")
                    .marker(symbols::Marker::Braille) // 盲文模式分辨率最高
                    .graph_type(GraphType::Line)
                    .style(Style::default().fg(Color::Green))
                    .data(&rx_data),
                Dataset::default()
                    .name("TX")
                    .marker(symbols::Marker::Braille)
                    .graph_type(GraphType::Line)
                    .style(Style::default().fg(Color::Magenta))
                    .data(&tx_data),
            ];

            // 5. 创建图表组件
            let chart = Chart::new(datasets)
                .block(Block::default()
                    .title(Span::styled(format!(" {} ", history.name), Style::default().bold()))
                    .borders(Borders::ALL))
                .x_axis(Axis::default()
                    // .title("Time") // 空间有限，省略标题
                    .style(Style::default().fg(Color::DarkGray))
                    .bounds([min_x, max_x])
                    // 只显示首尾标签
                    .labels(vec![
                        Span::raw(format!("{:.1}", min_x)),
                        Span::raw(format!("{:.1}", max_x)),
                    ]))
                .y_axis(Axis::default()
                    // .title("Speed")
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

// 辅助函数：自动计算网格布局 (N x M)
fn layout_grid(area: Rect, count: usize) -> Vec<Rect> {
    if count == 0 { return vec![]; }
    
    // 简单的自动计算列数和行数
    // 1 -> 1x1
    // 2 -> 1x2
    // 3,4 -> 2x2
    // 5,6 -> 2x3
    // 7,8 -> 2x4 (或者 4x2 取决于宽高比，这里简化处理)
    
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

// 辅助函数：格式化速度
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