use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{Duration, Instant};
use crate::fast_io::FastSysfsReader;
use crate::data::PortType;

pub struct PortHistory {
    pub name: String,
    pub port_type: PortType,
    pub rx_data: std::collections::VecDeque<(f64, f64)>, 
    pub tx_data: std::collections::VecDeque<(f64, f64)>,
}

impl PortHistory {
    pub fn new(name: String, port_type: PortType) -> Self {
        Self {
            name,
            port_type,
            rx_data: std::collections::VecDeque::with_capacity(200),
            tx_data: std::collections::VecDeque::with_capacity(200),
        }
    }
    
    pub fn push_point(&mut self, time: f64, rx: f64, tx: f64) {
        if self.rx_data.len() >= 200 {
            self.rx_data.pop_front();
            self.tx_data.pop_front();
        }
        self.rx_data.push_back((time, rx));
        self.tx_data.push_back((time, tx));
    }
}

pub fn spawn_chart_monitor(
    dev_part: String,
    port_part: String,
    p_type: PortType,
    history: Arc<RwLock<PortHistory>>
) {
    thread::spawn(move || {
        // --- 1. 路径与倍率配置 ---
        // 关键修复：RDMA 计数器 port_rcv_data 是 4 字节单位，Ethernet 是 1 字节
        let (rx_path, tx_path, unit_multiplier) = match p_type {
            PortType::Rdma => {
                let base = format!("/sys/class/infiniband/{}/ports/{}/counters", dev_part, port_part);
                (
                    format!("{}/port_rcv_data", base), 
                    format!("{}/port_xmit_data", base),
                    4.0 // RDMA word size
                )
            },
            PortType::Ethernet => {
                let base = format!("/sys/class/net/{}/statistics", dev_part);
                (
                    format!("{}/rx_bytes", base), 
                    format!("{}/tx_bytes", base),
                    1.0 // Ethernet byte size
                )
            }
        };

        // --- 2. 初始化 ---
        let mut rx_reader = match FastSysfsReader::new(&rx_path) {
            Ok(f) => f, Err(_) => return,
        };
        let mut tx_reader = match FastSysfsReader::new(&tx_path) {
            Ok(f) => f, Err(_) => return,
        };

        let mut prev_rx: u64 = 0;
        let mut prev_tx: u64 = 0;
        let mut initialized = false; 

        let loop_interval = Duration::from_micros(1000); 
        let commit_interval = Duration::from_millis(50);
        
        let mut next_tick = Instant::now();
        let mut last_commit_time = Instant::now();
        
        // 使用真实启动时间作为 X 轴零点
        let start_time = Instant::now();

        // 局部峰值保持器
        let mut window_max_rx: f64 = 0.0;
        let mut window_max_tx: f64 = 0.0;
        let mut prev_sample_time = Instant::now();

        // 预读取
        if let (Ok(rx), Ok(tx)) = (rx_reader.read_u64(), tx_reader.read_u64()) {
            prev_rx = rx;
            prev_tx = tx;
            initialized = true;
        }

        // --- 3. 1ms 硬核循环 ---
        loop {
            // A. 时间锚点 (Drift Correction)
            next_tick += loop_interval;
            let now = Instant::now();

            // B. 极速采集
            let curr_rx_res = rx_reader.read_u64();
            let curr_tx_res = tx_reader.read_u64();
            
            if initialized {
                if let (Ok(curr_rx), Ok(curr_tx)) = (curr_rx_res, curr_tx_res) {
                    let delta_time = (now - prev_sample_time).as_secs_f64();
                    
                    if delta_time > 0.000_001 {
                        if curr_rx >= prev_rx && curr_tx >= prev_tx {
                            // 关键修复：应用单位倍率
                            let rx_speed = ((curr_rx - prev_rx) as f64 * unit_multiplier) / delta_time;
                            let tx_speed = ((curr_tx - prev_tx) as f64 * unit_multiplier) / delta_time;

                            if rx_speed > window_max_rx { window_max_rx = rx_speed; }
                            if tx_speed > window_max_tx { window_max_tx = tx_speed; }
                        }
                    }
                    prev_rx = curr_rx;
                    prev_tx = curr_tx;
                }
            } else {
                 if let (Ok(rx), Ok(tx)) = (curr_rx_res, curr_tx_res) {
                    prev_rx = rx;
                    prev_tx = tx;
                    initialized = true;
                }
            }
            prev_sample_time = now;

            // C. 非阻塞提交 (Non-blocking Commit)
            // 只有当距离上次提交超过 50ms 时才尝试提交
            if now.duration_since(last_commit_time) >= commit_interval {
                // 关键修复：使用 try_write()
                // 如果 UI 正在读数据，这里会失败，不会阻塞。
                // 失败后果：本次不提交，window_max 继续保留并累加，下一次循环再试。
                // 结果：峰值绝对不会丢，只会延迟几毫秒显示。
                if let Ok(mut h) = history.try_write() {
                    let actual_time = now.duration_since(start_time).as_secs_f64();
                    h.push_point(actual_time, window_max_rx, window_max_tx);
                    
                    // 只有成功提交了，才重置峰值
                    window_max_rx = 0.0;
                    window_max_tx = 0.0;
                    last_commit_time = now;
                }
            }

            // D. 精确休眠
            let time_until_next = next_tick.saturating_duration_since(Instant::now());
            if !time_until_next.is_zero() {
                thread::sleep(time_until_next);
            } else {
                next_tick = Instant::now();
            }
        }
    });
}