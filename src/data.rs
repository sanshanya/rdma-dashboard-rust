use anyhow::{Context, Result};
use std::path::Path;
use tokio::fs;

const IB_SYSFS_PATH: &str = "/sys/class/infiniband";
const NET_SYSFS_PATH: &str = "/sys/class/net";

/// 端口类型枚举：用于区分是 RDMA 设备还是普通以太网设备
/// 这将决定后续 monitor 读取哪个 sysfs 文件，以及 UI 显示什么颜色
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PortType {
    Rdma,     // InfiniBand 或 RoCE
    Ethernet, // 标准物理以太网
}

/// 端口元数据结构体
/// 仅用于发现阶段，不包含统计数据
#[derive(Debug, Clone)]
pub struct PortInfo {
    /// 显示名称，例如 "mlx5_0-1" 或 "eth0"
    pub name: String,
    
    /// 端口类型
    pub port_type: PortType,
    
    /// 路径拼接部分：设备名
    /// RDMA: "mlx5_0"
    /// Eth: "eth0"
    pub device_path_part: String,
    
    /// 路径拼接部分：端口号
    /// RDMA: "1"
    /// Eth: "" (空字符串)
    pub port_num_part: String,
}

impl PortInfo {
    fn new(name: String, port_type: PortType, dev: String, port: String) -> Self {
        Self {
            name,
            port_type,
            device_path_part: dev,
            port_num_part: port,
        }
    }
}

/// 扫描系统中的所有物理网络端口 (RDMA + Ethernet)
///
/// 参数 `_monitor_queues` 被忽略，因为硬核模式下不调用 ethtool 以保证 1ms 精度。
pub async fn discover_ports(_monitor_queues: bool) -> Result<Vec<PortInfo>> {
    let mut ports = Vec::new();

    // ---------------------------------------------------------
    // 1. 扫描 RDMA 设备 (/sys/class/infiniband)
    // ---------------------------------------------------------
    if Path::new(IB_SYSFS_PATH).is_dir() {
        let mut devices = fs::read_dir(IB_SYSFS_PATH).await
            .context("Failed to read IB sysfs")?;
            
        while let Some(entry) = devices.next_entry().await? {
            let dev_name = entry.file_name().to_string_lossy().to_string();
            let ports_path = entry.path().join("ports");
            
            if ports_path.is_dir() {
                let mut p_entries = fs::read_dir(ports_path).await?;
                while let Some(p_entry) = p_entries.next_entry().await? {
                    let port_num = p_entry.file_name().to_string_lossy().to_string();
                    let full_name = format!("{}-{}", dev_name, port_num);
                    
                    ports.push(PortInfo::new(
                        full_name,
                        PortType::Rdma,
                        dev_name.clone(),
                        port_num,
                    ));
                }
            }
        }
    }

    // ---------------------------------------------------------
    // 2. 扫描物理以太网设备 (/sys/class/net)
    // ---------------------------------------------------------
    if Path::new(NET_SYSFS_PATH).is_dir() {
        let mut devices = fs::read_dir(NET_SYSFS_PATH).await
            .context("Failed to read Net sysfs")?;
            
        while let Some(entry) = devices.next_entry().await? {
            let dev_name = entry.file_name().to_string_lossy().to_string();
            
            // 过滤回环接口
            if dev_name == "lo" { continue; }
            
            // 关键过滤：只显示物理网卡
            // 检查 /sys/class/net/<dev>/device 是否存在。
            // 虚拟网卡（如 docker0, veth, tun）通常没有 device 软链接。
            // 注意：bonding 接口也没有 device，如果你想看 bond，可以去掉这个检查。
            let device_link = entry.path().join("device");
            if fs::metadata(&device_link).await.is_ok() {
                ports.push(PortInfo::new(
                    dev_name.clone(),
                    PortType::Ethernet,
                    dev_name,
                    String::new(), // Ethernet 没有 ports/X 子目录结构
                ));
            }
        }
    }

    // 按名称排序，保证 UI 顺序稳定
    ports.sort_by(|a, b| a.name.cmp(&b.name));
    
    Ok(ports)
}