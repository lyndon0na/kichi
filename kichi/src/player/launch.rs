//! 既不是 mpv 也不是 VLC 的播放器：准备输入 → spawn → 后台收尾。
//!
//! 这条路径**没有** IPC 观测能力：spawn 成功就当作开播，进程退出后回收；
//! 用户按「取消」时已经 spawn 的播放器没法关掉（见 `docs/adr/0008-mpv-idle-ipc.md`
//! 的「代价与后果」）。命令 / 参数可以用 `{title}`、`{playlist}`、`{url}`
//! 自己拼输入方式；没有输入占位符时把清单路径作为裸参数追加。

use std::path::Path;
use std::process::{Command, Stdio};

use super::{expand_play_arg, PlayerConfig};

/// 拼出要执行的命令（不启动）。
///
/// `playlist` 是一份 0600 权限的临时清单文件路径：URL 走清单而不走命令行，
/// 避免 `ps` / `journalctl` 之类把带签名的直链整条暴露出去。
///
/// 参数里出现 `{url}` / `{playlist}` 时由用户自己决定播放器从哪里拿输入；
/// 两者都没有时把清单路径作为裸参数追加。需要 `--playlist=<清单>` 这类
/// 形式的播放器，自己写 `--playlist={playlist}` 即可。
pub fn build_command(
    config: &PlayerConfig,
    playlist: Option<&Path>,
    url: &str,
    title: &str,
) -> Command {
    let mut command = Command::new(&config.program);
    let playlist_for_expand = playlist.unwrap_or_else(|| Path::new(""));
    for arg in &config.pre_args {
        command.arg(expand_play_arg(arg, title, playlist_for_expand, url));
    }
    for arg in &config.access_args {
        command.arg(expand_play_arg(arg, title, playlist_for_expand, url));
    }
    if !config.uses_input_placeholder() {
        if let Some(playlist) = playlist {
            command.arg(playlist);
        }
    }
    command.stdin(Stdio::null());
    // 播放器自己的输出一律丢弃：mpv 默认会把直链打到 stderr，
    // 继承过来就会进 kichi 的日志。
    command.stdout(Stdio::null());
    command.stderr(Stdio::null());
    command
}

/// 启动通用降级播放器；成功返回后立刻返回，不等待退出。
///
/// 默认把直链写进临时清单（0600）而不放命令行，避免 `ps` / journal 暴露签名 URL；
/// 用户显式使用 `{url}` 时按配置直接传 URL。清单要等播放器真正读入后才能删：
/// 先是 3 秒宽限，等不到就等它退出再删。播放器退出并回收后调用 `on_exit`，
/// 让界面清掉「播放中」。
pub fn spawn(
    config: &PlayerConfig,
    url: &str,
    title: &str,
    on_exit: impl FnOnce() + Send + 'static,
) -> Result<(), String> {
    if url.trim().is_empty() {
        return Err("播放链接为空".to_owned());
    }
    let dir = super::ensure_play_dir()?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    let playlist = dir.join(format!("{}-{}.m3u", std::process::id(), stamp));
    let wants_url = config.uses_url_placeholder();
    let wants_playlist = config.uses_playlist_placeholder();
    let playlist = if wants_url && !wants_playlist {
        None
    } else {
        write_playlist(&playlist, url, title).map_err(|err| format!("写入播放清单失败：{err}"))?;
        Some(playlist)
    };

    let mut command = build_command(config, playlist.as_deref(), url, title);
    let program = config.program.clone();
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(err) => {
            if let Some(playlist) = playlist.as_deref() {
                let _ = std::fs::remove_file(playlist);
            }
            return Err(format!("启动播放器 {program} 失败：{err}"));
        }
    };
    // 交给后台线程收尾：等播放器读入清单后删掉它，再见机回收进程
    std::thread::Builder::new()
        .name("kichi-player".into())
        .spawn(move || {
            // 宽限是为了让播放器读完清单；进程提前退出时没必要把
            // 「播放中」多挂满 3 秒，轮询到退出就立刻收尾。
            let deadline = std::time::Instant::now() + PLAYLIST_GRACE;
            let mut early_status = None;
            while std::time::Instant::now() < deadline {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        early_status = Some(status);
                        break;
                    }
                    Ok(None) => {
                        std::thread::sleep(std::time::Duration::from_millis(50));
                    }
                    Err(_) => break,
                }
            }
            let status = match early_status {
                Some(status) => Ok(status),
                None => child.wait(),
            };
            // 通用播放器没有就绪信号；冷启动可能超过宽限期，清单要留到进程退出
            // 再删，避免播放器还没读到文件就被删掉。目录 0700 / 文件 0600。
            if let Some(playlist) = playlist.as_deref() {
                let _ = std::fs::remove_file(playlist);
            }
            match status {
                Ok(status) if !status.success() => {
                    eprintln!("[kichi] 播放器退出码：{status}");
                }
                Ok(_) => {}
                Err(err) => eprintln!("[kichi] 等待播放器退出失败：{err}"),
            }
            on_exit();
        })
        .map_err(|err| format!("启动播放器守护线程失败：{err}"))?;
    Ok(())
}

/// 播放器读入清单的宽限时间（mpv 实测启动即读；留足余量）。
const PLAYLIST_GRACE: std::time::Duration = std::time::Duration::from_secs(3);

/// 写一份仅本人可读的文件（0600），内容是带标题的单条 m3u。
fn write_playlist(path: &Path, url: &str, title: &str) -> std::io::Result<()> {
    use std::io::Write;
    let url = url.trim();
    if url.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "播放链接为空",
        ));
    }
    if url.chars().any(char::is_control) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "播放链接包含换行或控制字符",
        ));
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    let title = one_line(title);
    writeln!(file, "#EXTM3U\n#EXTINF:-1,{title}\n{url}")?;
    file.flush()
}

fn one_line(text: &str) -> String {
    let title = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.is_empty() {
        "video".to_owned()
    } else {
        title
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::{split_words, DEFAULT_ARGS};

    fn test_config() -> PlayerConfig {
        let mut config = PlayerConfig {
            program: "mpv".to_owned(),
            pre_args: Vec::new(),
            access_args: Vec::new(),
        };
        config.access_args = split_words(DEFAULT_ARGS);
        assert_eq!(PlayerConfig::default().program, config.program);
        config
    }

    #[test]
    fn malicious_file_name_stays_a_plain_argument() {
        let config = test_config();
        let name = "--fs \"quoted\" $(touch pwned)";
        let playlist = Path::new("/tmp/kichi-play/test.m3u");
        let command = build_command(
            &config,
            Some(playlist),
            "https://cdn.invalid/v?sign=S",
            name,
        );
        let args: Vec<_> = command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            vec![
                format!("--force-media-title={name}"),
                "--ytdl=no".to_owned(),
                "/tmp/kichi-play/test.m3u".to_owned(),
            ]
        );
        assert_eq!(
            command.get_program().to_string_lossy(),
            "mpv",
            "程序名不受文件名影响"
        );
    }

    #[test]
    fn signed_url_never_reaches_the_command_line() {
        let config = test_config();
        let url = "https://cdn.invalid/file?sign=SECRET&expire=1";
        let command = build_command(&config, Some(Path::new("/tmp/x.m3u")), url, "t");
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
        write_playlist(&path, "https://cdn.invalid/v?sign=S", "第 1 集\n第二行")
            .expect("写清单失败");
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
        assert_eq!(
            body,
            "#EXTM3U\n#EXTINF:-1,第 1 集 第二行\nhttps://cdn.invalid/v?sign=S\n"
        );
        // create_new：已存在的文件不允许被覆盖
        assert!(write_playlist(&path, "x", "t").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_url_is_rejected_before_spawn() {
        let config = test_config();
        assert!(spawn(&config, "  ", "x", || {}).is_err());
    }

    #[test]
    fn players_without_placeholders_get_the_playlist_path() {
        let config = PlayerConfig {
            program: "vlc".to_owned(),
            pre_args: Vec::new(),
            access_args: vec!["--title={title}".to_owned()],
        };
        let command = build_command(
            &config,
            Some(Path::new("/tmp/kichi-play/x.m3u")),
            "https://cdn.invalid/v",
            "影·片",
        );
        let args: Vec<_> = command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            vec![
                "--title=影·片".to_owned(),
                "/tmp/kichi-play/x.m3u".to_owned()
            ],
            "没有输入占位符时把清单路径作为裸参数传入"
        );
    }

    #[test]
    fn playlist_placeholder_is_user_assembled_without_auto_option() {
        let config = PlayerConfig {
            program: "totem".to_owned(),
            pre_args: vec!["--title={title}".to_owned()],
            access_args: vec!["{playlist}".to_owned()],
        };
        let command = build_command(
            &config,
            Some(Path::new("/tmp/kichi-play/custom.m3u")),
            "https://cdn.invalid/v",
            "影·片",
        );
        let args: Vec<_> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args, vec!["--title=影·片", "/tmp/kichi-play/custom.m3u"]);
        assert!(
            !args.iter().any(|arg| arg.starts_with("--playlist=")),
            "用户自己放 {{playlist}} 时不能再追加 --playlist="
        );
    }

    #[test]
    fn explicit_playlist_option_is_still_possible() {
        let config = PlayerConfig {
            program: "custom-player".to_owned(),
            pre_args: Vec::new(),
            access_args: vec!["--playlist={playlist}".to_owned()],
        };
        let command = build_command(
            &config,
            Some(Path::new("/tmp/kichi-play/custom.m3u")),
            "https://cdn.invalid/v",
            "影·片",
        );
        let args: Vec<_> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args, vec!["--playlist=/tmp/kichi-play/custom.m3u"]);
    }

    #[test]
    fn url_placeholder_passes_the_signed_url_explicitly() {
        let config = PlayerConfig {
            program: "haruna".to_owned(),
            pre_args: Vec::new(),
            access_args: vec!["--fullscreen".to_owned(), "{url}".to_owned()],
        };
        let command = build_command(&config, None, "https://cdn.invalid/v?sign=SECRET", "影·片");
        let args: Vec<_> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            vec![
                "--fullscreen".to_owned(),
                "https://cdn.invalid/v?sign=SECRET".to_owned()
            ]
        );
        assert!(
            !args.iter().any(|arg| arg.starts_with("--playlist=")),
            "使用 {{url}} 时不能自动追加 --playlist="
        );
    }
}
