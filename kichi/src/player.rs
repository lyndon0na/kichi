//! 外部播放器：拼命令、启动、回收进程。
//!
//! 默认调用 flatpak 版 mpv（`io.mpv.Mpv`），可用环境变量覆盖：
//! - `KICHI_PLAYER`：整条命令（按空格切分，如 `mpv` 或 `flatpak run io.mpv.Mpv`）
//! - `KICHI_PLAYER_ARGS`：追加参数，同样按空格切分（如 `--fs --vo=gpu`）
//! - `KICHI_PLAYER_SUB`：字幕参数模板，`{sub}` 会被替换成字幕地址
//!
//! 安全约定：
//! - URL 与文件名永远走命令行参数数组（不经 shell），且 URL 前加 `--`
//!   分隔，避免文件名被当成选项；
//! - 直链是带签名的临时 URL，日志里只打主机名。

use std::process::{Command, Stdio};

use kichi_core::PlayOption;

/// 默认播放器命令（flatpak 包名，2026-10-26 实测可用）。
pub const DEFAULT_PLAYER: &str = "flatpak run io.mpv.Mpv";
/// 默认参数：窗口标题用文件名。
pub const DEFAULT_ARGS: &str = "--force-media-title={title}";

/// 一条播放命令的模板（解析自环境变量或默认值）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerConfig {
    /// 程序名 + 固定前置参数
    pub program: String,
    pub pre_args: Vec<String>,
    access_args: Vec<String>,
}

impl PlayerConfig {
    /// 从环境变量构造（`KICHI_PLAYER` 覆盖程序与前置参数）。
    pub fn from_env() -> Self {
        let overridden = read_env("KICHI_PLAYER").and_then(|command| parse_command(&command));
        let mut config = overridden.unwrap_or_default();
        config.access_args = split_words(
            read_env("KICHI_PLAYER_ARGS")
                .as_deref()
                .unwrap_or(DEFAULT_ARGS),
        );
        config
    }

    pub fn program(&self) -> &str {
        &self.program
    }

    /// 追加参数（供界面展示）。
    pub fn args(&self) -> &[String] {
        &self.access_args
    }
}

impl Default for PlayerConfig {
    fn default() -> Self {
        let mut config = parse_command(DEFAULT_PLAYER).unwrap_or_default();
        config.access_args = split_words(DEFAULT_ARGS);
        config
    }
}

/// 拼出要执行的命令（不启动）。
pub fn build_command(config: &PlayerConfig, url: &str, title: &str) -> Command {
    let mut command = Command::new(&config.program);
    command.args(&config.pre_args);
    for arg in &config.access_args {
        command.arg(expand(arg, title));
    }
    // `--` 之后一律当位置参数，杜绝 URL / 文件名注入成选项
    command.arg("--").arg(url);
    command.stdin(Stdio::null());
    command.stdout(Stdio::null());
    command.stderr(Stdio::null());
    command
}

/// 启动播放器；成功返回一个后台回收句柄（不等待退出）。
pub fn launch(config: &PlayerConfig, url: &str, title: &str) -> Result<(), String> {
    if url.trim().is_empty() {
        return Err("播放链接为空".to_owned());
    }
    let mut command = build_command(config, url, title);
    let program = config.program.clone();
    let child = command
        .spawn()
        .map_err(|err| format!("启动播放器 {program} 失败：{err}"))?;
    // 交给后台线程 wait：不 wait 会留僵尸进程；等完只记日志，不打扰界面
    std::thread::Builder::new()
        .name("kichi-player".into())
        .spawn(move || {
            if let Err(err) = child_into_status(child) {
                eprintln!("[kichi] 等待播放器退出失败：{err}");
            }
        })
        .map_err(|err| format!("启动播放器守护线程失败：{err}"))?;
    Ok(())
}

fn child_into_status(mut child: std::process::Child) -> std::io::Result<()> {
    let status = child.wait()?;
    if !status.success() {
        eprintln!("[kichi] 播放器退出码：{status}");
    }
    Ok(())
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
fn split_words(text: &str) -> Vec<String> {
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
fn expand(template: &str, title: &str) -> String {
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
        config.access_args = vec!["--force-media-title={title}".into()];
        config
    }

    #[test]
    fn default_command_uses_flatpak_mpv() {
        let config = test_config();
        assert_eq!(config.program, "flatpak");
        assert_eq!(config.pre_args, vec!["run", "io.mpv.Mpv"]);
    }

    #[test]
    fn malicious_file_name_stays_a_plain_argument() {
        let config = test_config();
        let name = "--fs \"quoted\" $(touch pwned)";
        let command = build_command(&config, "https://cdn.invalid/v?expire=1", name);
        let args: Vec<_> = command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            vec![
                "run".to_owned(),
                "io.mpv.Mpv".to_owned(),
                format!("--force-media-title={name}"),
                "--".to_owned(),
                "https://cdn.invalid/v?expire=1".to_owned(),
            ]
        );
        assert_eq!(
            command.get_program().to_string_lossy(),
            "flatpak",
            "程序名不受文件名影响"
        );
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
    fn masking_never_leaks_signature() {
        let masked = mask_url("https://dl-a1.pikpakcdn.com/file/x?sign=SECRET&expire=1");
        assert_eq!(masked, "dl-a1.pikpakcdn.com/…");
        assert!(!masked.contains("SECRET"));
        assert_eq!(mask_url("not a url"), "（无效链接）");
    }

    #[test]
    fn empty_url_is_rejected_before_spawn() {
        let config = test_config();
        assert!(launch(&config, "  ", "x").is_err());
    }
}
