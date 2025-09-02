use crate::data::{discover_ports, update_port_stats, IBPort};
use crate::handler::handle_key_event;
use crate::tui::Tui;
use crate::ui;
use crate::Args;
use anyhow::{Context, Result};
use crossterm::event::{Event, EventStream};
use futures::StreamExt;
use std::time::Duration;
use tokio::sync::mpsc;

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum SortKey {
    Name,
    Rx,
    Tx,
}

pub struct App {
    pub should_quit: bool,
    pub ports: Vec<IBPort>,
    pub sort_key: SortKey,
    pub version: String,
}

impl App {
    pub async fn try_new(args: Args) -> Result<Self> {
        let monitor_queues = args.monitor_queues;
        let version = env!("CARGO_PKG_VERSION").to_string();

        let initial_ports = discover_ports(monitor_queues)
            .await
            .context("Failed to discover InfiniBand ports. Is the kernel module loaded?")?;

        if initial_ports.is_empty() {
            anyhow::bail!("No InfiniBand interfaces found in /sys/class/infiniband.");
        }

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
            anyhow::bail!("No valid RDMA interfaces selected to monitor.");
        }

        Ok(Self {
            should_quit: false,
            ports: selected_ports,
            sort_key: SortKey::Name,
            version,
        })
    }

    pub async fn run(&mut self, tui: &mut Tui) -> Result<()> {
        let (tx, mut rx) = mpsc::channel(1);

        let mut data_ports = self.ports.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            loop {
                interval.tick().await;
                for port in &mut data_ports {
                    update_port_stats(port).await;
                }
                if tx.send(data_ports.clone()).await.is_err() {
                    break;
                }
            }
        });

        let mut event_stream = EventStream::new();
        while !self.should_quit {
            tui.draw(|f| ui::render(self, f))?;
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {
                    self.quit();
                },
                Some(updated_ports) = rx.recv() => {
                    self.on_tick(updated_ports);
                },
                Some(Ok(event)) = event_stream.next() => {
                    if let Event::Key(key) = event {
                        handle_key_event(key, self)?;
                    }
                },
            }
        }
        Ok(())
    }

    pub fn on_tick(&mut self, new_ports: Vec<IBPort>) {
        self.ports = new_ports;
        self.sort_ports();
    }

    pub fn set_sort_key(&mut self, key: SortKey) {
        self.sort_key = key;
        self.sort_ports();
    }

    fn sort_ports(&mut self) {
        match self.sort_key {
            SortKey::Name => self.ports.sort_by(|a, b| a.name.cmp(&b.name)),
            SortKey::Rx => self
                .ports
                .sort_by(|a, b| b.rx_byteps.partial_cmp(&a.rx_byteps).unwrap()),
            SortKey::Tx => self
                .ports
                .sort_by(|a, b| b.tx_byteps.partial_cmp(&a.tx_byteps).unwrap()),
        }
    }

    pub fn quit(&mut self) {
        self.should_quit = true;
    }
}