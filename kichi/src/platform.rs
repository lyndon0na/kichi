//! 运行环境探测。
//!
//! 只放「当前进程跑在什么环境里」这类判断，不涉及业务配置。

/// 是否运行在 Flatpak 沙箱里。
///
/// `/.flatpak-info` 是 Flatpak 一定会挂进沙箱的标记文件；`FLATPAK_ID`
/// 作为兜底（有些手工 bwrap / 开发环境不一定有标记文件，但会设变量）。
/// 两个条件都只看只读信息，不启动外部进程。
pub fn flatpak_sandbox() -> bool {
    std::path::Path::new("/.flatpak-info").exists() || std::env::var_os("FLATPAK_ID").is_some()
}

/// 是否运行在 KDE 会话里。
///
/// KDE 的 `org.kde.desktop` 控件风格由 Kirigami / `kdeglobals` 驱动，配色和
/// 强调色已经能跟随 Plasma；其他桌面没有这套配置，改用完全由 `QPalette` 驱动
/// 的 Fusion，再由 `appearance` 模块注入系统深色偏好与强调色。
///
/// 只读环境变量，不启动外部进程；`XDG_CURRENT_DESKTOP` 可能是
/// `KDE:GNOME` 这类冒号分隔的列表。
pub fn kde_session() -> bool {
    let matches = |value: Option<std::ffi::OsString>| {
        value.is_some_and(|value| {
            value
                .to_string_lossy()
                .split(':')
                .any(|part| part.eq_ignore_ascii_case("kde"))
        })
    };

    std::env::var_os("KDE_FULL_SESSION").is_some_and(|value| value.to_string_lossy() == "true")
        || matches(std::env::var_os("XDG_CURRENT_DESKTOP"))
        || matches(std::env::var_os("XDG_SESSION_DESKTOP"))
        || std::env::var_os("DESKTOP_SESSION").is_some_and(|value| {
            let value = value.to_string_lossy().to_ascii_lowercase();
            value.contains("plasma") || value.contains("kde")
        })
}
