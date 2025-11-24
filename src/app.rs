use crate::data::{discover_ports, PortInfo};
use crate::monitor::{spawn_chart_monitor, PortHistory};
use crate::handler::handle_key_event;
use crate::tui::Tui;
use crate::ui;
use crate::Args;
use anyhow::{Context, Result};
use crossterm::event::{Event, EventStream};
use futures::StreamExt;
use ratatui::widgets::ScrollbarState; // 新增引用
use std::sync::{Arc, RwLock};
use std::time::Duration;

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum ViewMode {
    Table,
    Chart,
}

pub struct App {
    pub should_quit: bool,
    pub view_mode: ViewMode,
    pub version: String,
    
    // 核心数据源
    pub histories: Vec<Arc<RwLock<PortHistory>>>,

    // --- 新增：滚动状态 ---
    pub vertical_scroll: usize, // 当前第一行显示的是第几个网卡
    pub scroll_state: ScrollbarState, // Ratatui 的滚动条状态
}

impl App {
    pub async fn try_new(args: Args) -> Result<Self> {
        let version = env!("CARGO_PKG_VERSION").to_string();

        let initial_ports = discover_ports(false)
            .await
            .context("Failed to discover network ports.")?;

        if initial_ports.is_empty() {
            anyhow::bail!("No physical RDMA or Ethernet interfaces found.");
        }

        let selected_ports: Vec<PortInfo> = if args.mode.all {
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
            anyhow::bail!("No valid interfaces selected to monitor.");
        }

        let mut histories = Vec::new();
        for port in selected_ports {
            let history = Arc::new(RwLock::new(
                PortHistory::new(port.name.clone(), port.port_type)
            ));
            spawn_chart_monitor(
                port.device_path_part, 
                port.port_num_part, 
                port.port_type, 
                history.clone()
            );
            histories.push(history);
        }

        let items_count = histories.len();

        Ok(Self {
            should_quit: false,
            view_mode: ViewMode::Chart,
            version,
            histories,
            // 初始化滚动状态
            vertical_scroll: 0,
            scroll_state: ScrollbarState::new(items_count), 
        })
    }

    pub async fn run(&mut self, tui: &mut Tui) -> Result<()> {
        let mut event_stream = EventStream::new();
        let mut ui_interval = tokio::time::interval(Duration::from_millis(100));

        while !self.should_quit {
            tui.draw(|f| ui::render(self, f))?;

            tokio::select! {
                _ = ui_interval.tick() => {},
                Some(Ok(event)) = event_stream.next() => {
                    if let Event::Key(key) = event {
                        handle_key_event(key, self)?;
                    }
                },
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

    // --- 新增：滚动控制逻辑 ---
    pub fn on_up(&mut self) {
        if self.vertical_scroll > 0 {
            self.vertical_scroll = self.vertical_scroll.saturating_sub(1);
            self.scroll_state = self.scroll_state.position(self.vertical_scroll);
        }
    }

    pub fn on_down(&mut self) {
        if self.vertical_scroll < self.histories.len().saturating_sub(1) {
            self.vertical_scroll = self.vertical_scroll.saturating_add(1);
            self.scroll_state = self.scroll_state.position(self.vertical_scroll);
        }
    }
}