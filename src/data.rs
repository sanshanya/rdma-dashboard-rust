use anyhow::{Context, Result};
use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;
use tokio::fs;
use tokio::process::Command;

const IB_SYSFS_PATH: &str = "/sys/class/infiniband";
const COUNTER_DATA_UNIT_BYTES: u64 = 4;

static QUEUE_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\s*(tx|rx)_prio(\d+)_bytes:\s*(\d+)").unwrap());

#[derive(Debug, Clone)]
pub struct IBPort {
    // Identity
    pub name: String,
    pub eth_name: Option<String>,
    // Statistics
    pub state: String,
    pub rx_byteps: f64, // Bytes per second
    pub tx_byteps: f64, // Bytes per second
    pub errors: u64,
    pub queue_speeds: HashMap<String, f64>,
    // Status Flags
    pub read_error: bool,
    pub queue_read_error: bool,
    // Internal state for rate calculation
    prev_rx_raw: u64,
    prev_tx_raw: u64,
    prev_queue_raw: HashMap<String, u64>,
    last_update_time: Instant,
}

impl Default for IBPort {
    fn default() -> Self {
        Self {
            name: String::new(),
            eth_name: None,
            state: "N/A".to_string(),
            last_update_time: Instant::now(),
            rx_byteps: 0.0,
            tx_byteps: 0.0,
            errors: 0,
            queue_speeds: HashMap::new(),
            read_error: false,
            queue_read_error: false,
            prev_rx_raw: 0,
            prev_tx_raw: 0,
            prev_queue_raw: HashMap::new(),
        }
    }
}

impl IBPort {
    fn new(name: String, eth_name: Option<String>) -> Self {
        Self {
            name,
            eth_name,
            ..Default::default()
        }
    }
}

pub async fn discover_ports(monitor_queues: bool) -> Result<Vec<IBPort>> {
    let mut ports = Vec::new();
    if !Path::new(IB_SYSFS_PATH).is_dir() {
        return Ok(ports);
    }
    let mut devices = fs::read_dir(IB_SYSFS_PATH).await?;
    while let Some(device_entry) = devices.next_entry().await? {
        let device_name = device_entry.file_name().to_string_lossy().to_string();
        let ports_path = device_entry.path().join("ports");
        if ports_path.is_dir() {
            let mut port_entries = fs::read_dir(ports_path).await?;
            while let Some(port_entry) = port_entries.next_entry().await? {
                let port_num = port_entry.file_name().to_string_lossy().to_string();
                let port_name = format!("{}-{}", device_name, port_num);
                let eth_name = if monitor_queues {
                    find_eth_device(&device_name).await
                } else {
                    None
                };
                ports.push(IBPort::new(port_name, eth_name));
            }
        }
    }
    ports.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(ports)
}

async fn find_eth_device(ib_device_name: &str) -> Option<String> {
    let net_path = Path::new(IB_SYSFS_PATH)
        .join(ib_device_name)
        .join("device/net");
    if let Ok(mut entries) = fs::read_dir(net_path).await {
        if let Ok(Some(entry)) = entries.next_entry().await {
            return Some(entry.file_name().to_string_lossy().to_string());
        }
    }
    None
}

async fn read_sysfs_val(path: &Path) -> Result<String> {
    fs::read_to_string(path)
        .await
        .context(format!("Failed to read {:?}", path))
}

pub async fn update_port_stats(port: &mut IBPort) {
    port.read_error = false;
    let parts: Vec<&str> = port.name.rsplitn(2, '-').collect();
    if parts.len() != 2 {
        port.read_error = true;
        return;
    }
    let [port_num, device] = [parts[0], parts[1]];
    let base_path = Path::new(IB_SYSFS_PATH)
        .join(device)
        .join("ports")
        .join(port_num);

    let now = Instant::now();
    let time_delta = (now - port.last_update_time).as_secs_f64();

    let counters_path = base_path.join("counters");
    let state_res = read_sysfs_val(&base_path.join("state")).await;
    let rx_res = read_sysfs_val(&counters_path.join("port_rcv_data")).await;
    let tx_res = read_sysfs_val(&counters_path.join("port_xmit_data")).await;
    let err_res = read_sysfs_val(&counters_path.join("port_rcv_errors")).await;

    match state_res {
        Ok(s) => port.state = s.split(':').last().unwrap_or("N/A").trim().to_string(),
        Err(_) => {
            port.read_error = true;
            return;
        }
    }

    let (curr_rx_raw, curr_tx_raw, errors) = match (rx_res, tx_res, err_res) {
        (Ok(rx), Ok(tx), Ok(err)) => (
            rx.trim().parse::<u64>().unwrap_or(0),
            tx.trim().parse::<u64>().unwrap_or(0),
            err.trim().parse::<u64>().unwrap_or(0),
        ),
        _ => {
            port.read_error = true;
            return;
        }
    };

    port.errors = errors;

    if time_delta > 0.0 && port.prev_rx_raw > 0 {
        let rx_delta = curr_rx_raw.saturating_sub(port.prev_rx_raw);
        let tx_delta = curr_tx_raw.saturating_sub(port.prev_tx_raw);
        port.rx_byteps = (rx_delta * COUNTER_DATA_UNIT_BYTES) as f64 / time_delta;
        port.tx_byteps = (tx_delta * COUNTER_DATA_UNIT_BYTES) as f64 / time_delta;
    } else {
        port.rx_byteps = 0.0;
        port.tx_byteps = 0.0;
    }

    port.prev_rx_raw = curr_rx_raw;
    port.prev_tx_raw = curr_tx_raw;

    if let Some(eth_name) = port.eth_name.clone() {
        update_queue_stats(port, &eth_name, time_delta).await;
    }

    port.last_update_time = now;
}

async fn update_queue_stats(port: &mut IBPort, eth_name: &str, time_delta: f64) {
    port.queue_read_error = false;
    let output = Command::new("ethtool").arg("-S").arg(eth_name).output().await;

    match output {
        Ok(output) => {
            if !output.status.success() {
                port.queue_read_error = true;
                return;
            }
            let stdout = String::from_utf8_lossy(&output.stdout);
            let mut current_queue_raw = HashMap::new();

            for line in stdout.lines() {
                if let Some(caps) = QUEUE_REGEX.captures(line) {
                    let direction = &caps[1];
                    let prio = &caps[2];
                    let value: u64 = caps[3].parse().unwrap_or(0);
                    let key = format!("{} Prio{}", direction.to_uppercase(), prio);
                    current_queue_raw.insert(key, value);
                }
            }

            port.queue_speeds.clear();
            if time_delta > 0.0 {
                for (name, curr_val) in &current_queue_raw {
                    if let Some(prev_val) = port.prev_queue_raw.get(name) {
                        let delta = curr_val.saturating_sub(*prev_val);
                        port.queue_speeds
                            .insert(name.clone(), delta as f64 / time_delta);
                    }
                }
            }
            port.prev_queue_raw = current_queue_raw;
        }
        Err(_) => {
            port.queue_read_error = true;
        }
    }
}