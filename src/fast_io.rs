use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};

/// 专用于 sysfs 计数器文件的高性能读取器。
/// 
/// 针对 Linux /sys/class/... 下的单数值文件进行了极度优化：
/// 1. 避免重复 open/close 系统调用。
/// 2. 避免 String 内存分配。
/// 3. 使用字节级手动解析代替标准库 parse。
pub struct FastSysfsReader {
    file: File,
    // 64字节的栈缓冲区足以容纳 u64::MAX (20位) + 换行符 + 冗余空间。
    // 使用栈内存比堆内存（Vec/String）快且对 CPU 缓存更友好。
    buffer: [u8; 64],
}

impl FastSysfsReader {
    /// 打开指定路径的文件并准备读取。
    /// 仅在初始化时调用一次 open syscall。
    pub fn new(path: &str) -> io::Result<Self> {
        let file = File::open(path)?;
        Ok(Self {
            file,
            buffer: [0u8; 64],
        })
    }

    /// 执行一次极速读取并解析为 u64。
    /// 
    /// # 性能分析
    /// 在 1ms 循环中，此函数的耗时通常在微秒(us)级别。
    #[inline(always)]
    pub fn read_u64(&mut self) -> io::Result<u64> {
        // 1. 重置文件指针到开头 (lseek)
        // 这是读取 sysfs 动态文件的必要操作。
        self.file.seek(SeekFrom::Start(0))?;

        // 2. 读取内容到栈缓冲区 (read)
        let n = self.file.read(&mut self.buffer)?;
        
        if n == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "Empty sysfs file"));
        }

        // 3. 手动字节解析 (Manual Byte Parsing)
        // 比 String::parse::<u64>() 快，因为：
        // - 无需 UTF-8 有效性检查
        // - 无需处理复杂的 Result/Option 包装链
        // - 遇到非数字字符立即停止
        let mut num: u64 = 0;
        
        // 使用迭代器切片，编译器通常能优化为非常高效的汇编指令
        for &b in &self.buffer[..n] {
            if b >= b'0' && b <= b'9' {
                // 累加数值: num = num * 10 + digit
                num = num.wrapping_mul(10).wrapping_add((b - b'0') as u64);
            } else if b == b'\n' || b == 0 || b == b' ' {
                // 遇到换行、空字符或空格，视为结束
                break;
            } else {
                // 遇到其他非法字符（如字母、符号），视为数据损坏
                // 在极速模式下，与其猜测不如报错，防止脏数据污染图表
                return Err(io::Error::new(io::ErrorKind::InvalidData, "Non-digit encountered"));
            }
        }
        
        Ok(num)
    }
}