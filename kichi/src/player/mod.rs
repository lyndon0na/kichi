//! 外部播放器：拼命令、启动、回收进程。
//!
//! 设置页可选 mpv / VLC / 自定义。默认 mpv 指系统 `PATH` 里的 `mpv`；
//! 预设在播放前会检查是否可用，缺一个就换另一个已安装的，两个都没有才报错。
//! 不探测 Flatpak：装 Flatpak 版 mpv 的用户在设置里用自定义命令
//! （`flatpak run --die-with-parent io.mpv.Mpv`）。
//!
//! 可用环境变量覆盖：
//! - `KICHI_PLAYER`：整条命令（按空格切分，如 `mpv`、`vlc` 或 `flatpak run io.mpv.Mpv`）；
//!   显式给出的命令不会被自动替换
//! - `KICHI_PLAYER_ARGS`：附加参数，同样按空格切分（如 `--fs --vo=gpu`）；
//!   自定义播放器默认 `{playlist}`，也可用 `{title}` / `{url}`
//!
//! 安全约定：
//! - 不经 shell；需要进 argv 的文件名 / 标题作为独立参数传递，并避免被当成选项；
//! - mpv 的直链只经 IPC（`loadfile`）下发；VLC 与其它播放器默认使用 0600
//!   临时清单；自定义参数显式使用 `{url}` 时直链会进入 argv；
//! - 直链是带签名的临时 URL，日志里只打主机名。
//!
//! 模块分工（见 `docs/adr/0008-mpv-idle-ipc.md`）：
//! - 本文件：配置解析、命令切分、标题 / 脱敏工具；
//! - [`input`]：自定义播放器输入占位符：`{title}` / `{playlist}` / `{url}`；
//! - [`launch`]：其它播放器：清单 + spawn + 收尾线程；
//! - [`ipc`]：mpv 专用：socket、`loadfile`、窗口观测、`quit`；
//! - [`vlc`]：VLC 专用：oldrc socket、状态 / 进度查询、seek、quit；
//! - [`session`]：播放会话登记表 + 监控线程 + 退出收尾。

mod connection;
pub(crate) mod embed;
mod input;
pub mod ipc;
mod launch;
mod progress;
pub(crate) mod screensaver;
pub mod session;
pub mod subtitles;
pub mod vlc;

use std::ffi::OsStr;
use std::path::Path;

use kichi_core::PlayOption;

use crate::settings::PlayerBackend;
pub(crate) use input::expand_play_arg;
pub use session::{shutdown_all, start, EndReason, PlaybackExtras, StartEvent};

/// 默认播放器命令：系统 `PATH` 里的 `mpv`。
///
/// 不写死 Flatpak 包名：默认值要对没装 Flatpak 的机器也可用；
/// Flatpak 版 mpv 用设置里的自定义命令。
pub const DEFAULT_PLAYER: &str = "mpv";
/// 默认参数：窗口标题用文件名，并关掉 ytdl 探测。
///
/// `--ytdl=no` 很重要：mpv 默认会把 HTTP 链接先交给 youtube-dl 抓一遍，
/// 对带签名的 PikPak 直链既多一次外发请求，也可能撞上 403。
pub const DEFAULT_ARGS: &str = "--force-media-title={title} --ytdl=no";
/// 系统 VLC 命令；设置页选 VLC 时用它，不强行安装 Flatpak 版本。
pub const DEFAULT_VLC_PLAYER: &str = "vlc";
/// VLC 的附加参数默认留空；RC 控制参数由播放器模块在启动时追加。
pub const DEFAULT_VLC_ARGS: &str = "";
/// 通用播放器的默认附加参数：把临时清单路径作为位置参数传入。
///
/// 大多数播放器接受「打开这个文件」的位置参数；需要 `--playlist=` 这类
/// 特定选项时，用户自己写 `--playlist={playlist}`。
pub const DEFAULT_GENERIC_ARGS: &str = "{playlist}";

/// 播放器控制能力。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerProtocol {
    Mpv,
    Vlc,
    Generic,
}

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
    fn resolve_with(
        backend: PlayerBackend,
        settings_command: Option<&str>,
        settings_args: Option<&str>,
        env_command: Option<&str>,
        env_args: Option<&str>,
    ) -> Self {
        let env_command_set = env_command.is_some();
        let mut config = match env_command {
            Some(command) => parse_command(command).unwrap_or_default(),
            None => match backend {
                PlayerBackend::Embedded | PlayerBackend::Mpv => Self::preset(PlayerBackend::Mpv),
                PlayerBackend::Vlc => Self::preset(PlayerBackend::Vlc),
                PlayerBackend::Custom => settings_command
                    .and_then(parse_command)
                    .unwrap_or_else(|| Self::preset(PlayerBackend::Mpv)),
            },
        };
        let args = env_args.or_else(|| {
            (!env_command_set && backend == PlayerBackend::Custom)
                .then_some(settings_args)
                .flatten()
        });
        let default_args = if config.is_mpv() {
            DEFAULT_ARGS
        } else if config.is_vlc() {
            DEFAULT_VLC_ARGS
        } else {
            DEFAULT_GENERIC_ARGS
        };
        config.access_args = match args {
            Some(args) => split_words(args),
            None if config.uses_input_placeholder() => Vec::new(),
            None => split_words(default_args),
        };
        config
    }

    /// 播放时解析配置：预设播放器不在 PATH 里时换成另一个已安装的预设，
    /// 并返回给提示条显示的说明；两个都缺时保持原预设，由
    /// [`check_available`](Self::check_available) 报错。
    ///
    /// 只探测系统 `mpv` / `vlc`，不探测 Flatpak；显式填写的自定义命令
    /// 与 `KICHI_PLAYER` 永远不会被替换。
    pub fn resolve_playable(
        backend: PlayerBackend,
        settings_command: Option<&str>,
        settings_args: Option<&str>,
    ) -> (Self, Option<String>) {
        Self::resolve_playable_with(
            backend,
            settings_command,
            settings_args,
            read_env("KICHI_PLAYER").as_deref(),
            read_env("KICHI_PLAYER_ARGS").as_deref(),
            program_exists,
        )
    }

    fn resolve_playable_with<F: Fn(&str) -> bool>(
        backend: PlayerBackend,
        settings_command: Option<&str>,
        settings_args: Option<&str>,
        env_command: Option<&str>,
        env_args: Option<&str>,
        available: F,
    ) -> (Self, Option<String>) {
        let primary = Self::resolve_with(
            backend,
            settings_command,
            settings_args,
            env_command,
            env_args,
        );
        let explicit = env_command.is_some()
            || (backend == PlayerBackend::Custom && settings_command.is_some());
        if explicit || available(&primary.program) {
            return (primary, None);
        }
        let alternative = match backend {
            // 空的自定义命令等价于内置 mpv；内嵌不可用时也退回外部 mpv
            PlayerBackend::Embedded | PlayerBackend::Mpv | PlayerBackend::Custom => {
                PlayerBackend::Vlc
            }
            PlayerBackend::Vlc => PlayerBackend::Mpv,
        };
        let fallback = Self::preset(alternative);
        if !available(&fallback.program) {
            return (primary, None);
        }
        let notice = format!("未找到 {}，已改用 {}", primary.program, fallback.program);
        (fallback, Some(notice))
    }

    /// 预设对应的内置命令（预设忽略设置里的自定义命令）。
    fn preset(backend: PlayerBackend) -> Self {
        let (command, args) = match backend {
            PlayerBackend::Vlc => (DEFAULT_VLC_PLAYER, DEFAULT_VLC_ARGS),
            PlayerBackend::Embedded | PlayerBackend::Mpv | PlayerBackend::Custom => {
                (DEFAULT_PLAYER, DEFAULT_ARGS)
            }
        };
        let mut config = parse_command(command).unwrap_or_default();
        config.access_args = split_words(args);
        config
    }

    /// 播放前确认命令里的程序能在本机执行。
    ///
    /// 只看 `PATH` / 绝对路径，不启动播放器；`flatpak run …` 这类壳命令
    /// 只检查 `flatpak` 本身。
    pub fn check_available(&self) -> Result<(), String> {
        if program_exists(&self.program) {
            return Ok(());
        }
        Err(format!(
            "未找到播放器「{}」；请先安装它，或在设置 → 播放里选择已安装的播放器，或填写自定义播放器命令",
            self.program
        ))
    }

    pub fn program(&self) -> &str {
        &self.program
    }

    /// 这是不是 mpv（或包着 mpv 的壳命令）。
    ///
    /// 只有 mpv 系才有 IPC 观测能力：命令或前置参数里出现 `mpv` 才算，
    /// 覆盖 `mpv`、`/usr/bin/mpv`、`flatpak run io.mpv.Mpv`。
    /// VLC 另走 [`vlc`]；其余播放器退回旧清单方式（spawn 成功即视为开播，不能取消）。
    pub fn is_mpv(&self) -> bool {
        std::iter::once(&self.program)
            .chain(self.pre_args.iter())
            .any(|word| word.to_lowercase().contains("mpv"))
    }

    /// 这是不是 VLC（或包着 VLC 的壳命令）。
    pub fn is_vlc(&self) -> bool {
        std::iter::once(&self.program)
            .chain(self.pre_args.iter())
            .any(|word| word.to_lowercase().contains("vlc"))
    }

    pub fn protocol(&self) -> PlayerProtocol {
        if self.is_mpv() {
            PlayerProtocol::Mpv
        } else if self.is_vlc() {
            PlayerProtocol::Vlc
        } else {
            PlayerProtocol::Generic
        }
    }

    /// 能主动读取播放位置，因此可接官方历史 / 续播 / 进度写回。
    pub fn supports_history(&self) -> bool {
        matches!(self.protocol(), PlayerProtocol::Mpv | PlayerProtocol::Vlc)
    }
}

impl Default for PlayerConfig {
    fn default() -> Self {
        Self::preset(PlayerBackend::Mpv)
    }
}

/// 命令里的程序能否在本机执行（无 `/` 时查 `PATH`）。
fn program_exists(program: &str) -> bool {
    if program.is_empty() {
        return false;
    }
    if program.contains('/') {
        return is_executable(Path::new(program));
    }
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    program_in_paths(program, &paths)
}

fn program_in_paths(program: &str, paths: &OsStr) -> bool {
    std::env::split_paths(paths).any(|dir| is_executable(&dir.join(program)))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

/// 播放临时目录：播放清单与控制 socket 共用。
pub(crate) fn play_dir() -> std::path::PathBuf {
    std::env::temp_dir().join("kichi-play")
}

/// 建好播放临时目录，并保证权限是 0700。
///
/// `create_dir_all` 按 umask 建（本机 022 ⇒ `drwxr-xr-x`），同机其他用户
/// 能列出目录、看到播放清单与 socket 的文件名。清单本身是 0600，socket
/// 也只能由本用户连接，但文件名一样会漏；这里每次启动都把目录收紧，
/// 旧版本留下的 0755 目录也一并纠正。
pub(crate) fn ensure_play_dir() -> Result<std::path::PathBuf, String> {
    let dir = play_dir();
    std::fs::create_dir_all(&dir).map_err(|err| format!("创建播放临时目录失败：{err}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|err| format!("收紧播放临时目录权限失败：{err}"))?;
    }
    Ok(dir)
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

    fn resolve(backend: PlayerBackend, command: Option<&str>, args: Option<&str>) -> PlayerConfig {
        PlayerConfig::resolve_with(backend, command, args, None, None)
    }

    fn test_config() -> PlayerConfig {
        let mut config = parse_command(DEFAULT_PLAYER).expect("默认命令可解析");
        config.access_args = split_words(DEFAULT_ARGS);
        config
    }

    #[test]
    fn settings_override_defaults_but_not_environment() {
        // 设置提供命令与参数
        let config = resolve(PlayerBackend::Custom, Some("mpv"), Some("--fs"));
        assert_eq!(config.program, "mpv");
        assert_eq!(config.access_args, vec!["--fs"]);

        // 空设置回落到内置默认（系统 mpv）
        let config = resolve(PlayerBackend::Mpv, None, None);
        assert_eq!(config.program, "mpv");
        assert!(config.access_args.contains(&"--ytdl=no".to_owned()));
    }

    #[test]
    fn arbitrary_custom_players_default_to_the_playlist_path() {
        let config = resolve(PlayerBackend::Custom, Some("haruna"), None);
        assert_eq!(config.program, "haruna");
        assert_eq!(config.access_args, vec!["{playlist}"]);
        assert!(config.uses_playlist_placeholder());
    }

    #[test]
    fn command_field_input_placeholders_suppress_default_generic_args() {
        let config = resolve(PlayerBackend::Custom, Some("haruna {url}"), None);
        assert_eq!(config.program, "haruna");
        assert!(config.access_args.is_empty());
        assert!(config.uses_url_placeholder());

        let config = resolve(PlayerBackend::Custom, Some("haruna {playlist}"), None);
        assert!(config.access_args.is_empty());
        assert!(config.uses_playlist_placeholder());

        let config = resolve(PlayerBackend::Custom, Some("haruna --title={title}"), None);
        assert_eq!(config.access_args, vec!["{playlist}"]);
    }

    #[test]
    fn vlc_preset_ignores_hidden_custom_command() {
        let config = resolve(PlayerBackend::Vlc, Some("mpv"), Some("--fs"));
        assert_eq!(config.program, DEFAULT_VLC_PLAYER);
        assert!(config.is_vlc());
        assert_eq!(config.protocol(), PlayerProtocol::Vlc);
        assert!(config.access_args.is_empty());
    }

    #[test]
    fn default_command_uses_system_mpv() {
        let config = test_config();
        assert_eq!(config.program, "mpv");
        assert!(config.pre_args.is_empty());
    }

    #[test]
    fn missing_preset_falls_back_to_the_other_installed_player() {
        let (config, notice) = PlayerConfig::resolve_playable_with(
            PlayerBackend::Mpv,
            None,
            None,
            None,
            None,
            |program| program == "vlc",
        );
        assert_eq!(config.program, "vlc");
        assert!(config.is_vlc());
        assert!(config.access_args.is_empty());
        assert_eq!(notice.as_deref(), Some("未找到 mpv，已改用 vlc"));

        let (config, notice) = PlayerConfig::resolve_playable_with(
            PlayerBackend::Vlc,
            None,
            None,
            None,
            None,
            |program| program == "mpv",
        );
        assert_eq!(config.program, "mpv");
        assert!(config.is_mpv());
        assert_eq!(notice.as_deref(), Some("未找到 vlc，已改用 mpv"));
    }

    #[test]
    fn both_missing_keeps_the_selected_preset_for_the_error_message() {
        let (config, notice) =
            PlayerConfig::resolve_playable_with(PlayerBackend::Mpv, None, None, None, None, |_| {
                false
            });
        assert_eq!(config.program, "mpv");
        assert!(notice.is_none());
    }

    #[test]
    fn blank_custom_command_behaves_like_the_mpv_preset() {
        let (config, notice) = PlayerConfig::resolve_playable_with(
            PlayerBackend::Custom,
            None,
            None,
            None,
            None,
            |program| program == "vlc",
        );
        assert_eq!(config.program, "vlc");
        assert!(notice.is_some());
    }

    #[test]
    fn explicit_commands_are_never_replaced() {
        let (config, notice) = PlayerConfig::resolve_playable_with(
            PlayerBackend::Custom,
            Some("haruna"),
            None,
            None,
            None,
            |program| program == "mpv",
        );
        assert_eq!(config.program, "haruna");
        assert!(notice.is_none());

        let (config, notice) = PlayerConfig::resolve_playable_with(
            PlayerBackend::Mpv,
            None,
            None,
            Some("flatpak run io.mpv.Mpv"),
            None,
            |_| false,
        );
        assert_eq!(config.program, "flatpak");
        assert!(notice.is_none());
    }

    #[test]
    fn program_lookup_follows_path_and_execute_bits() {
        let dir = std::env::temp_dir().join(format!(
            "kichi-player-path-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let program = dir.join("mpv");
        std::fs::write(&program, b"#!/bin/sh\n").expect("写假播放器");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755))
                .expect("设为可执行");
        }
        let paths = std::env::join_paths([&dir]).expect("拼 PATH");
        assert!(program_in_paths("mpv", &paths));
        assert!(!program_in_paths("vlc", &paths));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o644))
                .expect("去掉可执行位");
            assert!(!program_in_paths("mpv", &paths));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn check_available_names_the_missing_program() {
        let config = PlayerConfig {
            program: "kichi-definitely-missing-player".to_owned(),
            pre_args: Vec::new(),
            access_args: Vec::new(),
        };
        let error = config.check_available().expect_err("不存在的播放器应报错");
        assert!(error.contains("kichi-definitely-missing-player"));
        assert!(error.contains("设置 → 播放"));
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
        for command in [
            "vlc",
            "cvlc",
            "/usr/bin/vlc",
            "flatpak run org.videolan.VLC",
        ] {
            let config = parse_command(command).expect("命令可解析");
            assert!(config.is_vlc(), "应当认出 VLC：{command}");
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

    #[test]
    fn play_dir_is_private() {
        let dir = ensure_play_dir().unwrap_or_else(|e| panic!("{e}"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&dir)
                .map(|m| m.permissions().mode() & 0o777)
                .unwrap_or(0);
            assert_eq!(mode, 0o700, "播放临时目录不能让同机其他用户列出");
        }
        #[cfg(not(unix))]
        assert!(dir.exists());
    }
}
