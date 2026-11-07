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
