use crate::data::discover_ports;
use crate::monitor::{spawn_chart_monitor, PortHistory};
use crate::handler::handle_key_event;
use crate::tui::Tui;
use crate::ui;
use crate::Args;
use anyhow::{Context, Result};
use crossterm::event::{Event, EventStream};
use futures::StreamExt;
use std::sync::{Arc, RwLock};

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum ViewMode {
    Table, // 数字列表模式
    Chart, // 示波器/折线图模式
}

pub struct App {
    pub should_quit: bool,
    pub view_mode: ViewMode,
    pub version: String,
    // 核心数据源：多线程共享的历史数据
    pub histories: Vec<Arc<RwLock<PortHistory>>>,
}

impl App {
    pub async fn try_new(args: Args) -> Result<Self> {
        let monitor_queues = args.monitor_queues; // 注意：毫秒级模式下通常不建议开启队列监控，IO压力过大
        let version = env!("CARGO_PKG_VERSION").to_string();

        // 1. 使用原有的逻辑发现端口 (复用 data.rs)
        let initial_ports = discover_ports(monitor_queues)
            .await
            .context("Failed to discover InfiniBand ports.")?;

        if initial_ports.is_empty() {
            anyhow::bail!("No InfiniBand interfaces found.");
        }

        // 2. 根据参数过滤端口
        let selected_ports = if args.mode.all {
            initial_ports
        } else if let Some(iface_names) = args.mode.interfaces {
            initial_ports
                .into_iter()
                .filter(|p| iface_names.contains(&p.name))
                .collect()
        } else {
            unreachable!();
        };

        if selected_ports.is_empty() {
            anyhow::bail!("No valid RDMA interfaces selected.");
        }

        // 3. 初始化硬核监控线程
        let mut histories = Vec::new();

        for port in selected_ports {
            // 解析名称，例如 "mlx5_0-1" -> device: "mlx5_0", port: "1"
            let parts: Vec<&str> = port.name.split('-').collect();
            if parts.len() != 2 {
                eprintln!("Skipping invalid port name format: {}", port.name);
                continue;
            }
            let device = parts[0].to_string();
            let port_num = parts[1].to_string();

            // 创建共享内存
            let history = Arc::new(RwLock::new(PortHistory::new(port.name.clone())));
            
            // !!! 启动 1ms 监控线程 !!!
            spawn_chart_monitor(device, port_num, history.clone());
            
            histories.push(history);
        }

        Ok(Self {
            should_quit: false,
            view_mode: ViewMode::Chart, // 默认进入图表模式，因为这才是精华
            version,
            histories,
        })
    }

    pub async fn run(&mut self, tui: &mut Tui) -> Result<()> {
        let mut event_stream = EventStream::new();

        // UI 刷新率：这里设为 100ms (10FPS) 足够了，因为数据是后台 50ms 提交一次的
        // 太快没意义，太慢会卡顿
        let mut ui_interval = tokio::time::interval(std::time::Duration::from_millis(100));

        while !self.should_quit {
            tui.draw(|f| ui::render(self, f))?;

            tokio::select! {
                // 定时刷新 UI
                _ = ui_interval.tick() => {
                    // 这里不需要做任何数据拉取逻辑，
                    // 因为 draw 函数会直接去读 RwLock 中的数据
                },
                // 处理按键
                Some(Ok(event)) = event_stream.next() => {
                    if let Event::Key(key) = event {
                        handle_key_event(key, self)?;
                    }
                },
                // 处理 Ctrl+C
                _ = tokio::signal::ctrl_c() => {
                    self.quit();
                },
            }
        }
        Ok(())
    }

    pub fn toggle_view_mode(&mut self) {
        self.view_mode = match self.view_mode {
            ViewMode::Table => ViewMode::Chart,
            ViewMode::Chart => ViewMode::Table,
        };
    }

    pub fn quit(&mut self) {
        self.should_quit = true;
    }
}