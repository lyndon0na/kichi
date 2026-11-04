//! 展示用的小格式化工具。
//!
//! 单独成文件：会话层（下载失败原因里的「已下载 1.2 GB」）与
//! 桥接层（文件大小、传输记录）都要用同一套数字格式，避免两份实现漂移。

/// 账号容量转人类可读：≥1 TB 用 TB，其余沿用文件大小的单位。
///
/// 文件大小仍走 [`size`]：列表里的 GB 级数字已经够用，不必改观感。
pub(crate) fn capacity(bytes: u64) -> String {
    const TB: f64 = 1024.0 * 1024.0 * 1024.0 * 1024.0;
    let size = bytes as f64;
    if size >= TB {
        format!("{:.1} TB", size / TB)
    } else {
        self::size(bytes)
    }
}

/// 字节数转人类可读（GB/MB 保留一位小数，KB 取整）。
pub(crate) fn size(bytes: u64) -> String {
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    const KB: f64 = 1024.0;
    let size = bytes as f64;
    if size >= GB {
        format!("{:.1} GB", size / GB)
    } else if size >= MB {
        format!("{:.1} MB", size / MB)
    } else if size >= KB {
        format!("{:.0} KB", size / KB)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn units_switch_at_each_threshold() {
        assert_eq!(size(0), "0 B");
        assert_eq!(size(1023), "1023 B");
        assert_eq!(size(1024), "1 KB");
        assert_eq!(size(1536), "2 KB", "KB 取整");
        assert_eq!(size(1024 * 1024), "1.0 MB");
        assert_eq!(size(3 * 1024 * 1024 * 1024 / 2), "1.5 GB");
    }

    #[test]
    fn capacity_switches_to_tb_at_one_terabyte() {
        const TB: u64 = 1024 * 1024 * 1024 * 1024;
        assert_eq!(capacity(TB - 1024 * 1024 * 1024), "1023.0 GB");
        assert_eq!(capacity(TB), "1.0 TB");
        assert_eq!(capacity(10 * TB), "10.0 TB");
        assert_eq!(capacity(TB + TB / 2), "1.5 TB");
    }
}
