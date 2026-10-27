//! 外部播放器：拼命令、启动、回收进程。
//!
//! 默认调用 flatpak 版 mpv（`io.mpv.Mpv`），可用环境变量覆盖：
//! - `KICHI_PLAYER`：整条命令（按空格切分，如 `mpv` 或 `flatpak run io.mpv.Mpv`）
//! - `KICHI_PLAYER_ARGS`：追加参数，同样按空格切分（如 `--fs --vo=gpu`）
//!
//! 安全约定：
//! - URL 与文件名永远走命令行参数数组（不经 shell），URL 前加 `--` 分隔，
//!   避免文件名被当成选项；
//! - mpv 的直链**只经 IPC**（`loadfile`）下发，不进 argv、不落盘；其它播放器
//!   退回 0600 的临时清单（见 [`launch`]）；
//! - 直链是带签名的临时 URL，日志里只打主机名。
//!
//! 模块分工（见 `docs/adr/0008-mpv-idle-ipc.md`）：
//! - 本文件：配置解析、命令切分、标题 / 脱敏工具；
//! - [`launch`]：非 mpv 播放器：清单 + spawn + 收尾线程；
//! - [`ipc`]：mpv 专用：socket、`loadfile`、窗口观测、`quit`；
//! - [`session`]：播放会话登记表 + 监控线程 + 退出收尾。

pub mod ipc;
mod launch;
pub mod session;

use kichi_core::PlayOption;

pub use session::{shutdown_all, start, StartEvent};

/// 默认播放器命令（flatpak 包名，2026-10-26 实测可用）。
pub const DEFAULT_PLAYER: &str = "flatpak run io.mpv.Mpv";
/// 默认参数：窗口标题用文件名，并关掉 ytdl 探测。
///
/// `--ytdl=no` 很重要：mpv 默认会把 HTTP 链接先交给 youtube-dl 抓一遍，
/// 对带签名的 PikPak 直链既多一次外发请求，也可能撞上 403。
pub const DEFAULT_ARGS: &str = "--force-media-title={title} --ytdl=no";

/// 一条播放命令的模板（解析自环境变量或默认值）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerConfig {
    /// 程序名 + 固定前置参数
    pub program: String,
    pub pre_args: Vec<String>,
    pub(crate) access_args: Vec<String>,
}

impl PlayerConfig {
    /// 按优先级取配置：环境变量 > 界面设置 > 内置默认。
    ///
    /// 环境变量排在设置之前：它本来就是排障与临时覆盖用的，
    /// 优先级更高才不会被界面里存着的命令挡住。
    pub fn resolve(settings_command: Option<&str>, settings_args: Option<&str>) -> Self {
        let command = read_env("KICHI_PLAYER").or_else(|| settings_command.map(str::to_owned));
        let mut config = command
            .as_deref()
            .and_then(parse_command)
            .unwrap_or_default();
        let args = read_env("KICHI_PLAYER_ARGS").or_else(|| settings_args.map(str::to_owned));
        config.access_args = split_words(args.as_deref().unwrap_or(DEFAULT_ARGS));
        config
    }

    pub fn program(&self) -> &str {
        &self.program
    }

    /// 这是不是 mpv（或包着 mpv 的壳命令）。
    ///
    /// 只有 mpv 系才有 IPC 观测能力：命令或前置参数里出现 `mpv` 才算，
    /// 覆盖 `mpv`、`/usr/bin/mpv`、`flatpak run io.mpv.Mpv`。
    /// 其它播放器退回旧清单方式（spawn 成功即视为开播，不能取消）。
    pub fn is_mpv(&self) -> bool {
        std::iter::once(&self.program)
            .chain(self.pre_args.iter())
            .any(|word| word.to_lowercase().contains("mpv"))
    }
}

impl Default for PlayerConfig {
    fn default() -> Self {
        let mut config = parse_command(DEFAULT_PLAYER).unwrap_or_default();
        config.access_args = split_words(DEFAULT_ARGS);
        config
    }
}

/// 播放临时目录：旧清单（非 mpv）与 IPC socket 共用。
pub(crate) fn play_dir() -> std::path::PathBuf {
    std::env::temp_dir().join("kichi-play")
}

/// 日志用的脱敏地址：只留主机名。
pub fn mask_url(url: &str) -> String {
    let host = url
        .split("://")
        .nth(1)
        .unwrap_or_default()
        .split(['/', '?'])
        .next()
        .unwrap_or_default();
    if host.is_empty() {
        "（无效链接）".to_owned()
    } else {
        format!("{host}/…")
    }
}

/// 播放选项 → 文件名（标题缺失时用档位名兜底）。
pub fn title_for(option: &PlayOption, name: &str) -> String {
    let name = name.trim();
    if name.is_empty() {
        option.label.clone()
    } else {
        name.to_owned()
    }
}

/// 按空格切分命令；支持双引号包裹带空格的参数（够用即可，不解析转义）。
pub(crate) fn split_words(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for ch in text.chars() {
        match ch {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

/// 解析整条播放器命令：第一个词是程序，其余是前置参数。
fn parse_command(text: &str) -> Option<PlayerConfig> {
    let mut words = split_words(text).into_iter();
    let program = words.next()?;
    Some(PlayerConfig {
        program,
        pre_args: words.collect(),
        access_args: Vec::new(),
    })
}

/// 替换 `{title}` 占位符；未知占位符原样保留。
pub(crate) fn expand(template: &str, title: &str) -> String {
    template.replace("{title}", title)
}

fn read_env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> PlayerConfig {
        let mut config = parse_command(DEFAULT_PLAYER).expect("默认命令可解析");
        config.access_args = split_words(DEFAULT_ARGS);
        config
    }

    #[test]
    fn settings_override_defaults_but_not_environment() {
        // 设置提供命令与参数
        let config = PlayerConfig::resolve(Some("mpv"), Some("--fs"));
        assert_eq!(config.program, "mpv");
        assert_eq!(config.access_args, vec!["--fs"]);

        // 空设置回落到内置默认
        let config = PlayerConfig::resolve(None, None);
        assert_eq!(config.program, "flatpak");
        assert!(config.access_args.contains(&"--ytdl=no".to_owned()));
    }

    #[test]
    fn default_command_uses_flatpak_mpv() {
        let config = test_config();
        assert_eq!(config.program, "flatpak");
        assert_eq!(config.pre_args, vec!["run", "io.mpv.Mpv"]);
    }

    #[test]
    fn mpv_is_recognised_through_wrappers() {
        for command in [
            "mpv",
            "/usr/bin/mpv",
            "flatpak run io.mpv.Mpv",
            "sh -c 'sleep 3; exec flatpak run io.mpv.Mpv \"$@\"' sh",
        ] {
            let config = parse_command(command).expect("命令可解析");
            assert!(config.is_mpv(), "应当认出 mpv：{command}");
        }
        for command in ["vlc", "/usr/bin/totem", "flatpak run org.videolan.VLC"] {
            let config = parse_command(command).expect("命令可解析");
            assert!(!config.is_mpv(), "不该误判成 mpv：{command}");
        }
    }

    #[test]
    fn unknown_placeholders_survive_expansion() {
        assert_eq!(
            expand("--title={title} {unknown}", "影·片"),
            "--title=影·片 {unknown}"
        );
        assert_eq!(split_words("--a=b  --c=d"), vec!["--a=b", "--c=d"]);
        assert_eq!(
            split_words("--title=\"two words\" x"),
            vec!["--title=two words", "x"]
        );
    }

    #[test]
    fn malicious_file_name_stays_a_plain_argument() {
        // 标题只经占位符展开进同一个参数，不会被拆成多个 argv
        let expanded = expand(
            "--force-media-title={title}",
            "--fs \"quoted\" $(touch pwned)",
        );
        assert_eq!(
            expanded,
            "--force-media-title=--fs \"quoted\" $(touch pwned)"
        );
    }

    #[test]
    fn masking_never_leaks_signature() {
        let masked = mask_url("https://dl-a1.pikpakcdn.com/file/x?sign=SECRET&expire=1");
        assert_eq!(masked, "dl-a1.pikpakcdn.com/…");
        assert!(!masked.contains("SECRET"));
        assert_eq!(mask_url("not a url"), "（无效链接）");
    }
}
