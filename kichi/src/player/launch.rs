//! 既不是 mpv 也不是 VLC 的播放器：写 0600 清单 → spawn → 后台收尾。
//!
//! 这条路径**没有** IPC 观测能力：spawn 成功就当作开播，进程退出后回收；
//! 用户按「取消」时已经 spawn 的播放器没法关掉（见 `docs/adr/0008-mpv-idle-ipc.md`
//! 的「代价与后果」）。

use std::path::Path;
use std::process::{Command, Stdio};

use super::{expand, PlayerConfig};

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

/// 启动通用降级播放器；成功返回后立刻返回，不等待退出。
///
/// 直链写进临时清单（0600）而不放命令行，避免 `ps` / journal 暴露签名 URL。
/// 清单要等播放器真正读入后才能删：先是 3 秒宽限，等不到就等它退出再删。
/// 播放器退出并回收后调用 `on_exit`，让界面清掉「播放中」。
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
    write_private(&playlist, url).map_err(|err| format!("写入播放清单失败：{err}"))?;

    let mut command = build_command(config, &playlist, title);
    let program = config.program.clone();
    let mut child = command
        .spawn()
        .map_err(|err| format!("启动播放器 {program} 失败：{err}"))?;
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
            let _ = std::fs::remove_file(&playlist);
            let status = match early_status {
                Some(status) => Ok(status),
                None => child.wait(),
            };
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::{split_words, DEFAULT_ARGS, DEFAULT_PLAYER};

    fn test_config() -> PlayerConfig {
        let mut config = PlayerConfig {
            program: "flatpak".to_owned(),
            pre_args: vec!["run".to_owned(), "io.mpv.Mpv".to_owned()],
            access_args: Vec::new(),
        };
        config.access_args = split_words(DEFAULT_ARGS);
        assert_eq!(PlayerConfig::default().program, config.program);
        assert!(DEFAULT_PLAYER.contains("io.mpv.Mpv"));
        config
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
    fn empty_url_is_rejected_before_spawn() {
        let config = test_config();
        assert!(spawn(&config, "  ", "x", || {}).is_err());
    }

    #[test]
    fn non_mpv_players_still_get_a_playlist() {
        let config = PlayerConfig {
            program: "vlc".to_owned(),
            pre_args: Vec::new(),
            access_args: vec!["--title={title}".to_owned()],
        };
        let command = build_command(&config, Path::new("/tmp/kichi-play/x.m3u"), "影·片");
        let args: Vec<_> = command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            vec![
                "--title=影·片".to_owned(),
                "--playlist=/tmp/kichi-play/x.m3u".to_owned()
            ],
            "非 mpv 退回清单方式"
        );
    }
}
