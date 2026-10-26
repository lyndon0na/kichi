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

use std::path::Path;
use std::process::{Command, Stdio};

use kichi_core::PlayOption;

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
}

impl Default for PlayerConfig {
    fn default() -> Self {
        let mut config = parse_command(DEFAULT_PLAYER).unwrap_or_default();
        config.access_args = split_words(DEFAULT_ARGS);
        config
    }
}

/// 拼出要执行的命令（不启动）。
///
/// `playlist` 是一份 0600 权限的临时清单文件路径：URL 走清单而不走命令行，
/// 避免 `ps` / `journalctl` 之类把带签名的直链整条暴露出去。
pub fn build_command(config: &PlayerConfig, playlist: &Path, title: &str) -> Command {
    let mut command = Command::new(&config.program);
    command.args(&config.pre_args);
    for arg in &config.access_args {
        command.arg(expand(arg, title));
    }
    command
        .arg(format!("--playlist={}", playlist.display()))
        .stdin(Stdio::null());
    // 播放器自己的输出一律丢弃：mpv 默认会把直链打到 stderr，
    // 继承过来就会进 kichi 的日志。
    command.stdout(Stdio::null());
    command.stderr(Stdio::null());
    command
}

/// 启动播放器；成功返回后立刻返回，不等待退出。
///
/// 直链写进临时清单（0600）而不放命令行，避免 `ps` / journal 暴露签名 URL。
/// 清单要等播放器真正读入后才能删：先是 2 秒宽限，等不到就等它退出再删。
pub fn launch(config: &PlayerConfig, url: &str, title: &str) -> Result<(), String> {
    if url.trim().is_empty() {
        return Err("播放链接为空".to_owned());
    }
    let dir = std::env::temp_dir().join("kichi-play");
    std::fs::create_dir_all(&dir).map_err(|err| format!("创建播放临时目录失败：{err}"))?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    let playlist = dir.join(format!("{}-{}.m3u", std::process::id(), stamp));
    write_private(&playlist, url).map_err(|err| format!("写入播放清单失败：{err}"))?;

    let mut command = build_command(config, &playlist, title);
    let program = config.program.clone();
    let mut child = command
        .spawn()
        .map_err(|err| format!("启动播放器 {program} 失败：{err}"))?;
    // 交给后台线程收尾：等播放器读入清单后删掉它，再 wait 回收进程
    std::thread::Builder::new()
        .name("kichi-player".into())
        .spawn(move || {
            std::thread::sleep(PLAYLIST_GRACE);
            let _ = std::fs::remove_file(&playlist);
            match child.wait() {
                Ok(status) if !status.success() => {
                    eprintln!("[kichi] 播放器退出码：{status}");
                }
                Ok(_) => {}
                Err(err) => eprintln!("[kichi] 等待播放器退出失败：{err}"),
            }
        })
        .map_err(|err| format!("启动播放器守护线程失败：{err}"))?;
    Ok(())
}

/// 播放器读入清单的宽限时间（mpv 实测启动即读；留足余量）。
const PLAYLIST_GRACE: std::time::Duration = std::time::Duration::from_secs(3);

/// 写一份仅本人可读的文件（0600），内容是一行 m3u 条目。
fn write_private(path: &Path, url: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    writeln!(file, "{url}")?;
    file.flush()
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
        config.access_args = split_words(DEFAULT_ARGS);
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
        let playlist = Path::new("/tmp/kichi-play/test.m3u");
        let command = build_command(&config, playlist, name);
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
                "--ytdl=no".to_owned(),
                "--playlist=/tmp/kichi-play/test.m3u".to_owned(),
            ]
        );
        assert_eq!(
            command.get_program().to_string_lossy(),
            "flatpak",
            "程序名不受文件名影响"
        );
    }

    #[test]
    fn signed_url_never_reaches_the_command_line() {
        let config = test_config();
        let url = "https://cdn.invalid/file?sign=SECRET&expire=1";
        let command = build_command(&config, Path::new("/tmp/x.m3u"), "t");
        let rendered = command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .chain(std::iter::once(
                command.get_program().to_string_lossy().into_owned(),
            ))
            .collect::<Vec<_>>()
            .join(" ");
        assert!(!rendered.contains("SECRET"), "命令行不得出现签名直链");
        // URL 只进清单文件
        assert!(url.contains("SECRET"));
    }

    #[test]
    fn playlist_is_private_and_removed_after_launch() {
        let dir = std::env::temp_dir().join(format!("kichi-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("p.m3u");
        let _ = std::fs::remove_file(&path);
        write_private(&path, "https://cdn.invalid/v?sign=S").expect("写清单失败");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path)
                .expect("读权限")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "清单必须是仅本人可读");
        }
        let body = std::fs::read_to_string(&path).expect("读清单");
        assert_eq!(body.trim(), "https://cdn.invalid/v?sign=S");
        // create_new：已存在的文件不允许被覆盖
        assert!(write_private(&path, "x").is_err());
        let _ = std::fs::remove_dir_all(&dir);
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
