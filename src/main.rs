mod app;
mod data;
mod handler;
mod tui;
mod ui;

use crate::app::App;
use anyhow::Result;
use clap::Parser;

#[derive(Parser, Debug)]
#[group(required = true, multiple = false)]
struct Mode {
    /// Monitor all available RDMA ports.
    #[arg(short, long)]
    all: bool,

    /// Specify one or more RDMA interfaces (e.g., mlx5_0-1).
    #[arg(short, long, name = "IFACE")]
    interfaces: Option<Vec<String>>,
}

#[derive(Parser, Debug)]
#[command(author, version, about = "A TUI dashboard for monitoring RDMA/InfiniBand ports.", long_about = None)]
struct Args {
    #[command(flatten)]
    mode: Mode,

    /// Enable per-priority queue monitoring via ethtool (may need sudo).
    #[arg(short = 'q', long, default_value_t = false)]
    monitor_queues: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    // 1. 解析参数
    let args = Args::parse();

    // 2. 初始化终端 (RAII)
    let mut tui = tui::Tui::new()?;

    // 3. 创建并初始化 App
    let mut app = App::try_new(args).await?;

    // 4. 运行 App
    app.run(&mut tui).await?;

    Ok(())
}