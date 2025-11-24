use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{Duration, Instant};
use crate::fast_io::FastSysfsReader;
use crate::data::PortType; 

/// 端口历史数据容器
/// 存储用于 UI 绘图的最近 N 个时间点的数据
pub struct PortHistory {
    pub name: String,
    pub port_type: PortType, // 用于 UI 决定颜色 (紫色 vs 绿色)
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
    
    /// 推入一个新的聚合数据点 (时间, RX速度, TX速度)
    pub fn push_point(&mut self, time: f64, rx: f64, tx: f64) {
        if self.rx_data.len() >= 200 {
            self.rx_data.pop_front();
            self.tx_data.pop_front();
        }
        self.rx_data.push_back((time, rx));
        self.tx_data.push_back((time, tx));
    }
}

/// 启动一个独立的、高优先级的监控线程
/// 该线程以 1ms 的频率运行，但在 50ms 的窗口内只输出峰值
pub fn spawn_chart_monitor(
    dev_part: String,
    port_part: String,
    p_type: PortType,
    history: Arc<RwLock<PortHistory>>
) {
    thread::spawn(move || {
        // -----------------------------------------------------------------
        // 1. 根据设备类型构建 sysfs 路径
        // -----------------------------------------------------------------
        let (rx_path, tx_path) = match p_type {
            PortType::Rdma => {
                // RDMA: /sys/class/infiniband/mlx5_0/ports/1/counters/port_rcv_data
                let base = format!("/sys/class/infiniband/{}/ports/{}/counters", dev_part, port_part);
                (format!("{}/port_rcv_data", base), format!("{}/port_xmit_data", base))
            },
            PortType::Ethernet => {
                // Ethernet: /sys/class/net/eth0/statistics/rx_bytes
                // Ethernet 通常没有 port_num 子目录结构，直接在 statistics 下
                let base = format!("/sys/class/net/{}/statistics", dev_part);
                (format!("{}/rx_bytes", base), format!("{}/tx_bytes", base))
            }
        };

        // -----------------------------------------------------------------
        // 2. 初始化极速读取器 (FastSysfsReader)
        // -----------------------------------------------------------------
        // 如果文件打不开（例如网卡突然消失），线程静默退出
        let mut rx_reader = match FastSysfsReader::new(&rx_path) {
            Ok(f) => f, Err(_) => return,
        };
        let mut tx_reader = match FastSysfsReader::new(&tx_path) {
            Ok(f) => f, Err(_) => return,
        };

        // -----------------------------------------------------------------
        // 3. 定义核心状态变量
        // -----------------------------------------------------------------
        let mut prev_rx: u64 = 0;
        let mut prev_tx: u64 = 0;
        let mut initialized = false; 

        // 时间控制
        let loop_interval = Duration::from_micros(1000); // 1ms 采样周期
        let commit_interval = Duration::from_millis(50); // 50ms 聚合提交
        let mut next_tick = Instant::now();
        let mut last_commit_time = Instant::now();
        let mut logical_time_axis = 0.0; // 逻辑时间轴，保证图表平滑

        // 局部峰值保持器 (避免频繁锁竞争)
        let mut window_max_rx: f64 = 0.0;
        let mut window_max_tx: f64 = 0.0;
        let mut prev_sample_time = Instant::now();

        // 预读取第一次，建立基准
        if let (Ok(rx), Ok(tx)) = (rx_reader.read_u64(), tx_reader.read_u64()) {
            prev_rx = rx;
            prev_tx = tx;
            initialized = true;
        }

        // -----------------------------------------------------------------
        // 4. 硬核循环 (The Hardcore Loop)
        // -----------------------------------------------------------------
        loop {
            // A. 绝对时间锚点计算 (防止 drift)
            next_tick += loop_interval;
            let now = Instant::now();

            // B. 极速采集
            let curr_rx_res = rx_reader.read_u64();
            let curr_tx_res = tx_reader.read_u64();
            
            // C. 数据计算 (带防刺逻辑)
            if initialized {
                if let (Ok(curr_rx), Ok(curr_tx)) = (curr_rx_res, curr_tx_res) {
                    let delta_time = (now - prev_sample_time).as_secs_f64();
                    
                    // 防御性计算：只有当 delta_time 有意义时才计算
                    if delta_time > 0.000_001 {
                        // 逻辑修正：如果当前值 < 上次值，说明网卡计数器溢出或重置
                        // 这种情况下这一帧数据作废，只更新 prev 指针
                        if curr_rx >= prev_rx && curr_tx >= prev_tx {
                            // 计算瞬时速率 (Bytes/s)
                            // 注意：Sysfs 中的计数器单位通常就是 Bytes，不需要 * 4.0
                            let rx_speed = (curr_rx - prev_rx) as f64 / delta_time;
                            let tx_speed = (curr_tx - prev_tx) as f64 / delta_time;

                            // 更新局部峰值 (Peak Hold)
                            if rx_speed > window_max_rx { window_max_rx = rx_speed; }
                            if tx_speed > window_max_tx { window_max_tx = tx_speed; }
                        }
                    }
                    
                    prev_rx = curr_rx;
                    prev_tx = curr_tx;
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

            // D. 窗口提交 (每 50ms 拿一次锁)
            if now.duration_since(last_commit_time) >= commit_interval {
                if let Ok(mut h) = history.write() {
                    // 提交峰值到 UI 队列
                    h.push_point(logical_time_axis, window_max_rx, window_max_tx);
                }
                
                // 重置局部状态
                window_max_rx = 0.0;
                window_max_tx = 0.0;
                last_commit_time = now;
                logical_time_axis += 0.05; // 逻辑步长 0.05s
            }

            // E. 精确休眠 (Compensating Sleep)
            let time_until_next = next_tick.saturating_duration_since(Instant::now());
            if !time_until_next.is_zero() {
                thread::sleep(time_until_next);
            } else {
                // 已经超时了 (Overrun)，立即重置锚点，防止雪崩
                next_tick = Instant::now();
            }
        }
    });
}