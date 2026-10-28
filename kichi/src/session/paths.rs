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

/// 传输任务记录（下载历史）位置。可用 `KICHI_TRANSFERS_FILE` 覆盖。
pub fn transfers_path() -> PathBuf {
    if let Some(path) = std::env::var_os("KICHI_TRANSFERS_FILE") {
        return PathBuf::from(path);
    }
    config_dir().join("transfers.json")
}
