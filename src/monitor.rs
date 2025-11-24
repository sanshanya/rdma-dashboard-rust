use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{Duration, Instant};
use crate::fast_io::FastSysfsReader;
use crate::data::PortType;

// --- 配置常量 ---
const HISTORY_CAPACITY: usize = 600; // 600点 * 10ms = 6秒历史
const COMMIT_MS: u64 = 10;           // 10ms 聚合一次 (视觉精度)
const TIME_STEP: f64 = 0.01;         // X轴每步 0.01s

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
            rx_data: std::collections::VecDeque::with_capacity(HISTORY_CAPACITY),
            tx_data: std::collections::VecDeque::with_capacity(HISTORY_CAPACITY),
        }
    }
    
    pub fn push_point(&mut self, time: f64, rx: f64, tx: f64) {
        if self.rx_data.len() >= HISTORY_CAPACITY {
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
        // 1. 路径与单位配置
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

        // 2. 初始化读取器
        let mut rx_reader = match FastSysfsReader::new(&rx_path) {
            Ok(f) => f, Err(_) => return,
        };
        let mut tx_reader = match FastSysfsReader::new(&tx_path) {
            Ok(f) => f, Err(_) => return,
        };

        // 3. 状态变量
        let mut prev_rx: u64 = 0;
        let mut prev_tx: u64 = 0;
        let mut initialized = false; 

        let loop_interval = Duration::from_micros(1000); // 采样依然是 1ms 硬核物理极限
        let commit_interval = Duration::from_millis(COMMIT_MS); // 提交改为 10ms
        
        let mut next_tick = Instant::now();
        let mut last_commit_time = Instant::now();
        
        // 逻辑时间轴 (0.00, 0.01, 0.02 ...)
        let mut logical_time_axis = 0.0;

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

        // 4. 循环
        loop {
            next_tick += loop_interval;
            let now = Instant::now();

            let curr_rx_res = rx_reader.read_u64();
            let curr_tx_res = tx_reader.read_u64();
            
            if initialized {
                if let (Ok(curr_rx), Ok(curr_tx)) = (curr_rx_res, curr_tx_res) {
                    let delta_time = (now - prev_sample_time).as_secs_f64();
                    
                    if delta_time > 0.000_001 {
                        if curr_rx >= prev_rx && curr_tx >= prev_tx {
                            // 计算瞬时速度 (1ms slice)
                            let rx_speed = ((curr_rx - prev_rx) as f64 * unit_multiplier) / delta_time;
                            let tx_speed = ((curr_tx - prev_tx) as f64 * unit_multiplier) / delta_time;

                            // 峰值保持 (Peak Hold)
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

            // 5. 提交逻辑 (10ms 一次)
            if now.duration_since(last_commit_time) >= commit_interval {
                // 非阻塞提交：如果 UI 在读，这帧就先攒着，不丢峰值
                if let Ok(mut h) = history.try_write() {
                    h.push_point(logical_time_axis, window_max_rx, window_max_tx);
                    
                    // 只有成功提交才重置
                    window_max_rx = 0.0;
                    window_max_tx = 0.0;
                    last_commit_time = now;
                    logical_time_axis += TIME_STEP;
                }
            }

            // 6. 休眠
            let time_until_next = next_tick.saturating_duration_since(Instant::now());
            if !time_until_next.is_zero() {
                thread::sleep(time_until_next);
            } else {
                next_tick = Instant::now();
            }
        }
    });
}