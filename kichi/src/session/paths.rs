//! 会话存储路径：由应用决定，供 QML 侧构造 `Kichi` 时使用。

use std::path::PathBuf;

/// kichi 自己的配置目录：`$XDG_CONFIG_HOME/kichi`，默认 `~/.config/kichi`。
///
/// 设置文件与令牌降级文件都放这里，用户要备份/清理只用看一个地方。
/// ⚠ 目录名 `kichi` 归本项目独占：旧版（Python + aria2 那版）的配置
/// 已迁到 `~/.config/kichi-py`，不要再往那里读写。
pub fn config_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("kichi")
}

/// 降级文件位置（钥匙串不可用时才写入）。可用 `KICHI_SESSION_FILE` 覆盖。
pub fn store_path() -> PathBuf {
    if let Some(path) = std::env::var_os("KICHI_SESSION_FILE") {
        return PathBuf::from(path);
    }
    config_dir().join("session.token")
}

/// 下载断点索引位置。可用 `KICHI_PARTIALS_FILE` 覆盖。
pub fn partials_path() -> PathBuf {
    if let Some(path) = std::env::var_os("KICHI_PARTIALS_FILE") {
        return PathBuf::from(path);
    }
    config_dir().join("partials.json")
}

/// 传输任务记录位置。可用 `KICHI_TRANSFERS_FILE` 覆盖。
pub fn transfers_path() -> PathBuf {
    if let Some(path) = std::env::var_os("KICHI_TRANSFERS_FILE") {
        return PathBuf::from(path);
    }
    config_dir().join("transfers.json")
}

/// 内嵌播放器的 mpv 配置：`<配置目录>/embed-mpv.conf`。
///
/// 用 kichi 自己的文件而不是用户 `~/.config/mpv/mpv.conf`：内嵌固定的
/// `vo` / `gpu-*` / 输入配置不能被外部配置改掉（见 ADR 0030）。
/// 可用 `KICHI_EMBED_MPV_CONF` 覆盖。
pub fn embed_mpv_conf_path() -> PathBuf {
    if let Some(path) = std::env::var_os("KICHI_EMBED_MPV_CONF") {
        return PathBuf::from(path);
    }
    config_dir().join("embed-mpv.conf")
}

/// 内嵌播放器的键位：`<配置目录>/embed-input.conf`。
///
/// 只加载 kichi 自己的文件，不读用户 `~/.config/mpv/input.conf`：桌面键位里
/// 依赖 uosc 等脚本的绑定在内嵌播放器里无法工作（见 ADR 0031）。
/// 可用 `KICHI_EMBED_INPUT_CONF` 覆盖。
pub fn embed_input_conf_path() -> PathBuf {
    if let Some(path) = std::env::var_os("KICHI_EMBED_INPUT_CONF") {
        return PathBuf::from(path);
    }
    config_dir().join("embed-input.conf")
}

/// 账号的本地索引库位置。可用 `KICHI_INDEX_FILE` 覆盖（测试 / 多实例）。
///
/// 文件名只用 `user_id`，**不用邮箱**：`docs/` 与日志都不该出现账号信息，
/// 文件名同样不应该。索引是可重建缓存，删掉不会丢数据。
pub fn index_path(user_id: &str) -> PathBuf {
    if let Some(path) = std::env::var_os("KICHI_INDEX_FILE") {
        return PathBuf::from(path);
    }
    config_dir().join(index_file_name(user_id))
}

/// 账号 id → 安全的库文件名：只留 ASCII 字母数字 / `-` / `_`，最长 64 字节。
/// 空 id 落成 `account`，保证同一台机器上不同账号不会共用一个文件。
pub(crate) fn index_file_name(user_id: &str) -> String {
    let mut safe = String::new();
    for ch in user_id.trim().chars() {
        if safe.len() >= 64 {
            break;
        }
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            safe.push(ch);
        }
    }
    if safe.is_empty() {
        safe.push_str("account");
    }
    format!("index-{safe}.sqlite3")
}

#[cfg(test)]
mod tests {
    use super::index_file_name;

    #[test]
    fn index_file_is_per_account_and_sanitized() {
        assert_ne!(index_file_name("1001"), index_file_name("1002"));
        assert_eq!(index_file_name("1001"), "index-1001.sqlite3");
        assert_eq!(
            index_file_name("a@b.com"),
            "index-abcom.sqlite3",
            "邮箱式的 id 不能把 @ / . 带进文件名"
        );
        assert_eq!(index_file_name("  "), "index-account.sqlite3");
        assert!(
            !index_file_name("../../etc/passwd").contains('/'),
            "路径分隔符必须被过滤"
        );
        assert!(index_file_name(&"x".repeat(200)).len() <= "index-".len() + 64 + ".sqlite3".len());
    }
}
