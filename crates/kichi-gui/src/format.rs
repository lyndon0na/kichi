use kichi_core::consts::OFFLINE_PHASES;

pub fn fmt_bytes(bytes: i64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    const TB: f64 = GB * 1024.0;
    let b = bytes as f64;
    if bytes < 0 {
        "-".into()
    } else if b < KB {
        format!("{bytes} B")
    } else if b < MB {
        format!("{:.1} KB", b / KB)
    } else if b < GB {
        format!("{:.1} MB", b / MB)
    } else if b < TB {
        format!("{:.2} GB", b / GB)
    } else {
        format!("{:.2} TB", b / TB)
    }
}

/// 把 ISO 时间串裁剪为 "YYYY-MM-DD HH:MM"。
pub fn fmt_time(s: &str) -> String {
    let mut out = String::with_capacity(16);
    let mut t_seen = false;
    for ch in s.chars() {
        if ch == 'T' {
            out.push(' ');
            t_seen = true;
            continue;
        }
        if t_seen && (ch == '.' || ch == '+' || ch == 'Z' || ch == ' ') {
            break;
        }
        out.push(ch);
    }
    out
}

/// 人类可读的任务状态标签。phase 由服务端 filter 决定, 为 None 时返回 "未知"。
pub fn phase_label(phase: &str) -> &'static str {
    match phase {
        "PHASE_TYPE_PENDING" => "等待中",
        "PHASE_TYPE_RUNNING" => "下载中",
        "PHASE_TYPE_COMPLETE" => "已完成",
        "PHASE_TYPE_ERROR" => "失败",
        _ => "未知",
    }
}

/// 固定的展示顺序(与请求顺序一致)。
pub const PHASE_ORDER: [&str; 4] = OFFLINE_PHASES;

/// 把云端文件名收敛为安全的本地文件名(仅取 basename, 拒绝路径穿越/空名)。
/// 返回 None 表示没有可用的名字, 调用方可回退到 "download"。
pub fn safe_file_name(name: &str) -> Option<String> {
    let base = std::path::Path::new(name)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .trim();
    if base.is_empty() || base == "." || base == ".." {
        return None;
    }
    Some(base.to_string())
}

/// 当前 unix 秒。
pub fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 依据剩余字节与速率估算剩余时间, 形如 "12s" / "3m20s" / "1h05m"; 速率未知时为空。
pub fn fmt_eta(remaining: u64, speed: u64) -> String {
    if speed == 0 {
        return String::new();
    }
    fmt_duration(remaining / speed)
}

/// 相对时间: "刚刚" / "3 分钟前" / "2 小时前" / "3 天前"; at 为 0 或未来时为空。
pub fn fmt_rel(now: u64, at: u64) -> String {
    if at == 0 || now < at {
        return String::new();
    }
    let d = now - at;
    if d < 60 {
        "刚刚".into()
    } else if d < 3600 {
        format!("{} 分钟前", d / 60)
    } else if d < 86400 {
        format!("{} 小时前", d / 3600)
    } else {
        format!("{} 天前", d / 86400)
    }
}

fn fmt_duration(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m{:02}s", secs / 60, secs % 60)
    } else {
        format!("{}h{:02}m", secs / 3600, (secs % 3600) / 60)
    }
}

#[cfg(test)]
mod tests {
    use super::safe_file_name;

    #[test]
    fn keeps_plain_names() {
        assert_eq!(safe_file_name("movie.mkv").as_deref(), Some("movie.mkv"));
        assert_eq!(safe_file_name("  a b.mp4 ").as_deref(), Some("a b.mp4"));
    }

    #[test]
    fn strips_directory_components() {
        assert_eq!(safe_file_name("a/b/c.txt").as_deref(), Some("c.txt"));
        assert_eq!(
            safe_file_name("../../etc/passwd").as_deref(),
            Some("passwd")
        );
    }

    #[test]
    fn rejects_empty_or_dot_names() {
        assert_eq!(safe_file_name(""), None);
        assert_eq!(safe_file_name("  "), None);
        assert_eq!(safe_file_name(".."), None);
        assert_eq!(safe_file_name("."), None);
        assert_eq!(safe_file_name("/"), None);
    }

    #[test]
    fn etas_and_relative_times() {
        assert_eq!(super::fmt_eta(0, 0), "");
        assert_eq!(super::fmt_eta(120, 10), "12s");
        assert_eq!(super::fmt_eta(200, 1), "3m20s");
        assert_eq!(super::fmt_eta(3600 * 2, 1), "2h00m");

        assert_eq!(super::fmt_rel(100, 0), "");
        assert_eq!(super::fmt_rel(100, 100), "刚刚");
        assert_eq!(super::fmt_rel(1000, 100), "15 分钟前");
        assert_eq!(super::fmt_rel(100 + 7200, 100), "2 小时前");
        assert_eq!(super::fmt_rel(100 + 86400 * 3, 100), "3 天前");
    }
}
