use anyhow::{Context, Result};
use std::path::Path;
use tokio::fs;

const IB_SYSFS_PATH: &str = "/sys/class/infiniband";

/// 在硬核模式下，IBPort 仅作为发现阶段的元数据容器
/// 实际的统计数据（速度、历史曲线）全部移交给了 monitor.rs 中的 PortHistory 管理
#[derive(Debug, Clone)]
pub struct IBPort {
    /// 格式: "mlx5_0-1" (设备名-端口号)
    pub name: String,
}

impl IBPort {
    fn new(name: String) -> Self {
        Self { name }
    }
}

/// 扫描系统中的 InfiniBand/RDMA 端口
pub async fn discover_ports(_monitor_queues: bool) -> Result<Vec<IBPort>> {
    let mut ports = Vec::new();
    
    // 1. 检查 sysfs 路径是否存在
    if !Path::new(IB_SYSFS_PATH).is_dir() {
        return Ok(ports); // 没有 RDMA 设备
    }

    // 2. 遍历 /sys/class/infiniband 下的所有设备
    let mut devices = fs::read_dir(IB_SYSFS_PATH).await
        .context("Failed to read IB sysfs directory")?;
        
    while let Some(device_entry) = devices.next_entry().await? {
        let device_name = device_entry.file_name().to_string_lossy().to_string();
        let ports_path = device_entry.path().join("ports");

        if ports_path.is_dir() {
            let mut port_entries = fs::read_dir(ports_path).await?;
            while let Some(port_entry) = port_entries.next_entry().await? {
                let port_num = port_entry.file_name().to_string_lossy().to_string();
                
                // 构造唯一标识符: "mlx5_0-1"
                // 这个名字会被 monitor.rs 用来拼接路径：
                // /sys/class/infiniband/mlx5_0/ports/1/counters/...
                let port_name = format!("{}-{}", device_name, port_num);
                
                ports.push(IBPort::new(port_name));
            }
        }
    }

    // 3. 按名称排序，保证 UI 显示顺序固定
    ports.sort_by(|a, b| a.name.cmp(&b.name));
    
    Ok(ports)
}