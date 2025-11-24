use crate::data::{discover_ports, PortInfo};
use crate::monitor::{spawn_chart_monitor, PortHistory};
use crate::handler::handle_key_event;
use crate::tui::Tui;
use crate::ui;
use crate::Args;
use anyhow::{Context, Result};
use crossterm::event::{Event, EventStream};
use futures::StreamExt;
use std::sync::{Arc, RwLock};
use std::time::Duration;

/// 视图模式：决定 UI 显示波形图还是数字列表
#[derive(PartialEq, Eq, Clone, Copy)]
pub enum ViewMode {
    Table, // 数字列表模式 (显示当前瞬时速度)
    Chart, // 示波器模式 (显示 1ms 精度的历史趋势)
}

pub struct App {
    pub should_quit: bool,
    pub view_mode: ViewMode,
    pub version: String,
    
    // 核心数据源
    // UI 线程只读 (Read Lock)，后台 1ms 线程写入 (Write Lock)
    pub histories: Vec<Arc<RwLock<PortHistory>>>,
}

impl App {
    pub async fn try_new(args: Args) -> Result<Self> {
        let version = env!("CARGO_PKG_VERSION").to_string();

        // 1. 发现系统中的物理端口 (RDMA + Ethernet)
        // 参数 false 表示不开启 ethtool 队列监控 (为了保证 1ms 精度)
        let initial_ports = discover_ports(false)
            .await
            .context("Failed to discover network ports.")?;

        if initial_ports.is_empty() {
            anyhow::bail!("No physical RDMA or Ethernet interfaces found.");
        }

        // 2. 根据命令行参数过滤端口
        let selected_ports: Vec<PortInfo> = if args.mode.all {
            initial_ports
        } else if let Some(iface_names) = args.mode.interfaces {
            // 用户指定了名称 (如 "mlx5_0-1" 或 "eth0")
            initial_ports
                .into_iter()
                .filter(|p| iface_names.contains(&p.name))
                .collect()
        } else {
            // 理论上 clap 保证了不会走到这里
            unreachable!();
        };

        if selected_ports.is_empty() {
            anyhow::bail!("No valid interfaces selected to monitor.");
        }

        // 3. 初始化监控架构
        let mut histories = Vec::new();

        for port in selected_ports {
            // 创建线程安全的共享历史记录容器
            let history = Arc::new(RwLock::new(
                PortHistory::new(port.name.clone(), port.port_type)
            ));
            
            // !!! 启动硬核 1ms 监控线程 !!!
            // 这是一个 "Fire and Forget" 的线程，它会一直运行直到程序结束。
            // 我们传入路径组成部分，让线程自己去拼接 /sys 路径。
            spawn_chart_monitor(
                port.device_path_part, 
                port.port_num_part, 
                port.port_type, 
                history.clone()
            );
            
            histories.push(history);
        }

        Ok(Self {
            should_quit: false,
            view_mode: ViewMode::Chart, // 默认进入最炫酷的图表模式
            version,
            histories,
        })
    }

    pub async fn run(&mut self, tui: &mut Tui) -> Result<()> {
        let mut event_stream = EventStream::new();

        // UI 刷新频率：100ms (10 FPS)
        // 注意：这只是 UI 绘图的频率，不影响后台数据的采集频率(1ms)或聚合频率(50ms)。
        // 10 FPS 对人眼来说已经足够流畅，且不会占用过多主线程 CPU。
        let mut ui_interval = tokio::time::interval(Duration::from_millis(100));

        while !self.should_quit {
            // 绘制 UI
            // draw 会调用 ui::render，进而获取 RwLock 读取最新数据
            tui.draw(|f| ui::render(self, f))?;

            tokio::select! {
                // 定时器触发 UI 刷新
                _ = ui_interval.tick() => {
                    // 这里的 tick 只是为了唤醒 select 循环进行 draw
                    // 实际的数据更新完全由后台线程负责
                },
                
                // 处理键盘输入
                Some(Ok(event)) = event_stream.next() => {
                    if let Event::Key(key) = event {
                        handle_key_event(key, self)?;
                    }
                },
                
                // 处理 Ctrl+C 信号
                _ = tokio::signal::ctrl_c() => {
                    self.quit();
                },
            }
        }
        Ok(())
    }

    /// 切换视图模式 (Table <-> Chart)
    pub fn toggle_view_mode(&mut self) {
        self.view_mode = match self.view_mode {
            ViewMode::Table => ViewMode::Chart,
            ViewMode::Chart => ViewMode::Table,
        };
    }

    /// 设置退出标志
    pub fn quit(&mut self) {
        self.should_quit = true;
    }
}