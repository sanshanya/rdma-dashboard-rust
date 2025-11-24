mod app;
mod data;
mod handler;
mod tui;
mod ui;

// !!! 新增: 注册硬核监控所需的模块 !!!
pub mod monitor;
pub mod fast_io;

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
pub struct Args {
    #[command(flatten)]
    mode: Mode,

    /// Enable per-priority queue monitoring.
    /// Note: In the current millisecond-precision mode, this flag might be ignored
    /// to ensure system performance, as calling ethtool is too slow.
    #[arg(short = 'q', long, default_value_t = false)]
    monitor_queues: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    // 1. 解析参数
    let args = Args::parse();

    // 2. 初始化终端 (RAII模式，自动处理进入/退出 raw mode)
    let mut tui = tui::Tui::new()?;

    // 3. 创建并初始化 App
    // 这里会启动后台的 1ms 硬核监控线程
    let mut app = App::try_new(args).await?;

    // 4. 运行 App 主循环
    app.run(&mut tui).await?;

    Ok(())
}