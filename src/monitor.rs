use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{Duration, Instant};
use crate::fast_io::FastSysfsReader;

// 图表历史数据结构
pub struct PortHistory {
    pub name: String,
    pub rx_data: std::collections::VecDeque<(f64, f64)>, 
    pub tx_data: std::collections::VecDeque<(f64, f64)>,
}

impl PortHistory {
    pub fn new(name: String) -> Self {
        Self {
            name,
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
    device_name: String, 
    port_num: String, 
    history: Arc<RwLock<PortHistory>>
) {
    thread::spawn(move || {
        let base_path = format!("/sys/class/infiniband/{}/ports/{}/counters", device_name, port_num);
        
        // 容错打开文件
        let mut rx_reader = match FastSysfsReader::new(&format!("{}/port_rcv_data", base_path)) {
            Ok(f) => f,
            Err(_) => return, // 无法监控该端口，直接退出线程
        };
        let mut tx_reader = match FastSysfsReader::new(&format!("{}/port_xmit_data", base_path)) {
            Ok(f) => f,
            Err(_) => return,
        };

        // --- 核心状态 ---
        let mut prev_rx: u64 = 0;
        let mut prev_tx: u64 = 0;
        let mut initialized = false; // 避免第一次计算出现无限大

        // --- 时间控制 ---
        let loop_interval = Duration::from_micros(1000); // 1ms 采样
        let commit_interval = Duration::from_millis(50); // 50ms 提交
        let mut next_tick = Instant::now();
        let mut last_commit_time = Instant::now();
        let mut logical_time_axis = 0.0; // 逻辑时间轴

        // --- 局部峰值保持器 (Local Peak Holder) ---
        // 采纳审查建议：在 50ms 窗口期内，完全不碰锁，只更新局部变量
        let mut window_max_rx: f64 = 0.0;
        let mut window_max_tx: f64 = 0.0;
        let mut prev_sample_time = Instant::now();

        // 预读取第一次，建立基准
        if let (Ok(rx), Ok(tx)) = (rx_reader.read_u64(), tx_reader.read_u64()) {
            prev_rx = rx;
            prev_tx = tx;
            initialized = true;
        }

        loop {
            // 1. 绝对时间锚点计算 (Drift Correction)
            next_tick += loop_interval;
            let now = Instant::now();

            // 2. 极速采集
            let curr_rx_res = rx_reader.read_u64();
            let curr_tx_res = tx_reader.read_u64();
            
            // 3. 数据处理 (带防刺逻辑)
            if initialized {
                if let (Ok(curr_rx), Ok(curr_tx)) = (curr_rx_res, curr_tx_res) {
                    let delta_time = (now - prev_sample_time).as_secs_f64();
                    
                    // 防御性计算：只有当 delta_time 有意义且数值正常时才计算
                    if delta_time > 0.000_001 {
                        // 逻辑修正：如果当前值 < 上次值，说明网卡计数器溢出或重置
                        // 这种情况下这一帧数据作废，只更新 prev 指针
                        if curr_rx >= prev_rx && curr_tx >= prev_tx {
                            let rx_speed = (curr_rx - prev_rx) as f64 * 4.0 / delta_time;
                            let tx_speed = (curr_tx - prev_tx) as f64 * 4.0 / delta_time;

                            // 更新局部峰值
                            if rx_speed > window_max_rx { window_max_rx = rx_speed; }
                            if tx_speed > window_max_tx { window_max_tx = tx_speed; }
                        }
                    }
                    
                    prev_rx = curr_rx;
                    prev_tx = curr_tx;
                } else {
                    // 读取失败 (硬件拔出?)，这里可以选择重置 initialized 或记录错误
                    // 暂时保持静默，等待下一次恢复
                }
            } else {
                // 尝试重新初始化
                 if let (Ok(rx), Ok(tx)) = (curr_rx_res, curr_tx_res) {
                    prev_rx = rx;
                    prev_tx = tx;
                    initialized = true;
                }
            }
            prev_sample_time = now;

            // 4. 窗口提交 (每 50ms 拿一次锁)
            if now.duration_since(last_commit_time) >= commit_interval {
                if let Ok(mut h) = history.write() {
                    // 提交峰值到 UI 队列
                    h.push_point(logical_time_axis, window_max_rx, window_max_tx);
                }
                
                // 重置局部状态
                window_max_rx = 0.0;
                window_max_tx = 0.0;
                last_commit_time = now;
                logical_time_axis += 0.05; // 坚持使用逻辑时间轴，保证图表平滑
            }

            // 5. 精确休眠 (Compensating Sleep)
            let time_until_next = next_tick.saturating_duration_since(Instant::now());
            if !time_until_next.is_zero() {
                // 如果系统支持，spin_sleep 精度更高，这里用标准 sleep 
                thread::sleep(time_until_next);
            } else {
                // 已经超时了 (Overrun)，立即重置锚点，防止雪崩
                next_tick = Instant::now();
            }
        }
    });
}