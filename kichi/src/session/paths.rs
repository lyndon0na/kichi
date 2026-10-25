//! 会话存储路径：由应用决定，供 QML 侧构造 `Kichi` 时使用。

use std::path::PathBuf;

/// 降级文件位置（钥匙串不可用时才写入）。可用 `KICHI_SESSION_FILE` 覆盖。
pub fn store_path() -> PathBuf {
    if let Some(path) = std::env::var_os("KICHI_SESSION_FILE") {
        return PathBuf::from(path);
    }
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("kichi").join("session.token")
}

/// 给 QML 用的字符串形式。
pub fn store_path_string() -> String {
    store_path().to_string_lossy().into_owned()
}
