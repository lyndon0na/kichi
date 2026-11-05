//! VLC 的 oldrc 控制通道。
//!
//! VLC 用 `--extraintf oldrc --rc-unix=<临时 sock>` 额外开一个本地控制接口，
//! 主界面仍是用户熟悉的 Qt 窗口。直链只写进 0600 的临时播放清单，RC 里只发
//! 控制命令，不把签名 URL 放进 argv 或日志。

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crate::session::download::Cancel;

use super::connection::{connect_with, is_timeout, CONNECT_RETRY, CONNECT_TIMEOUT};
use super::{expand, PlayerConfig};

const READ_TIMEOUT: Duration = Duration::from_millis(200);
/// RC 一般在一个轮询周期内回应；超时就先当作本轮未知，不阻塞播放。
const QUERY_TIMEOUT: Duration = Duration::from_millis(900);

/// 一条持久化的 VLC oldrc 连接。
pub struct Connection {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
}

impl Connection {
    fn send(&mut self, command: &str) -> std::io::Result<()> {
        self.writer.write_all(command.as_bytes())?;
        self.writer.write_all(b"\n")?;
        self.writer.flush()
    }

    /// RC 的数值命令（`is_playing` / `get_time` / `get_length`）只返回一个数字。
    /// 中间可能夹着 `status change:` 异步通知，跳过它们。
    fn query_number(&mut self, command: &str) -> std::io::Result<Option<f64>> {
        self.send(command)?;
        let deadline = Instant::now() + QUERY_TIMEOUT;
        let mut line = String::new();
        while Instant::now() < deadline {
            line.clear();
            match self.reader.read_line(&mut line) {
                Ok(0) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "VLC 控制连接已关闭",
                    ))
                }
                Ok(_) => {
                    if let Some(value) = parse_number(&line) {
                        return Ok(Some(value));
                    }
                }
                Err(err) if is_timeout(&err) => continue,
                Err(err) => return Err(err),
            }
        }
        Ok(None)
    }

    pub fn is_playing(&mut self) -> std::io::Result<Option<bool>> {
        Ok(self.query_number("is_playing")?.map(|value| value >= 0.5))
    }

    pub fn position(&mut self) -> std::io::Result<Option<f64>> {
        Ok(self
            .query_number("get_time")?
            .filter(|value| value.is_finite() && *value >= 0.0))
    }

    pub fn duration(&mut self) -> std::io::Result<Option<f64>> {
        Ok(self
            .query_number("get_length")?
            .filter(|value| value.is_finite() && *value > 0.0))
    }

    pub fn seek_absolute(&mut self, seconds: u64) -> std::io::Result<()> {
        self.send(&format!("seek {seconds}"))
    }

    pub fn quit(&mut self) -> std::io::Result<()> {
        self.send("quit")
    }
}

/// 连上 VLC 的 RC socket；取消或超时返回 `None`。
pub fn connect(socket: &Path, cancel: &Cancel) -> Option<Connection> {
    connect_with(cancel, CONNECT_TIMEOUT, CONNECT_RETRY, || {
        try_connect(socket)
    })
}

fn try_connect(socket: &Path) -> Option<Connection> {
    let stream = UnixStream::connect(socket).ok()?;
    stream.set_read_timeout(Some(READ_TIMEOUT)).ok()?;
    stream.set_write_timeout(Some(READ_TIMEOUT)).ok()?;
    let reader = BufReader::new(stream.try_clone().ok()?);
    Some(Connection {
        reader,
        writer: stream,
    })
}

/// 生成一份 VLC 能直接读取的私有播放清单。
///
/// 标题压成单行，避免云端文件名里的换行改变清单结构；URL 同样拒绝控制字符。
pub fn write_playlist(url: &str, title: &str) -> Result<PathBuf, String> {
    let url = url.trim();
    if url.is_empty() {
        return Err("播放链接为空".to_owned());
    }
    if url.chars().any(char::is_control) {
        return Err("播放链接包含换行或控制字符".to_owned());
    }
    let dir = super::ensure_play_dir()?;
    let path = unique_path(&dir, "vlc", "m3u");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&path)
        .map_err(|err| format!("创建 VLC 播放清单失败：{err}"))?;
    let title = one_line(title);
    writeln!(file, "#EXTM3U\n#EXTINF:-1,{title}\n{url}")
        .map_err(|err| format!("写入 VLC 播放清单失败：{err}"))?;
    file.flush()
        .map_err(|err| format!("保存 VLC 播放清单失败：{err}"))?;
    Ok(path)
}

/// 拼 VLC 命令。URL 只经 playlist 文件，不进入 argv。
pub fn build_command(
    config: &PlayerConfig,
    socket: &Path,
    playlist: &Path,
    title: &str,
) -> Command {
    let mut command = Command::new(&config.program);
    let mut pre_args = config.pre_args.clone();
    let flatpak_run = is_flatpak(config) && pre_args.first().is_some_and(|arg| arg == "run");
    if flatpak_run && !pre_args.iter().any(|arg| arg == "--die-with-parent") {
        pre_args.insert(1, "--die-with-parent".to_owned());
    }
    command.args(&pre_args);
    for arg in &config.access_args {
        command.arg(expand(arg, title));
    }
    command
        .arg("--extraintf")
        .arg("oldrc")
        .arg("--rc-fake-tty")
        .arg(format!("--rc-unix={}", socket.display()))
        .arg("--no-one-instance")
        .arg("--play-and-exit")
        .arg("--no-media-library")
        .arg("--no-video-title-show")
        // VLC 没有 `--playlist=` 选项，清单路径必须作为位置参数传入。
        .arg(playlist)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

fn unique_path(dir: &Path, kind: &str, extension: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    dir.join(format!(
        "{kind}-{}-{stamp}-{}.{extension}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ))
}

fn parse_number(line: &str) -> Option<f64> {
    line.trim()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
}

fn one_line(text: &str) -> String {
    let title = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.is_empty() {
        "video".to_owned()
    } else {
        title
    }
}

fn is_flatpak(config: &PlayerConfig) -> bool {
    Path::new(&config.program)
        .file_name()
        .is_some_and(|name| name == "flatpak")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::split_words;

    #[test]
    fn command_keeps_url_out_of_argv_and_enables_rc() {
        let config = PlayerConfig {
            program: "flatpak".to_owned(),
            pre_args: vec!["run".to_owned(), "org.videolan.VLC".to_owned()],
            access_args: split_words("--fullscreen --meta-title={title}"),
        };
        let command = build_command(
            &config,
            Path::new("/tmp/kichi-play/1.sock"),
            Path::new("/tmp/kichi-play/1.m3u"),
            "影·片",
        );
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        let rendered = args.join(" ");
        assert!(rendered.contains("--die-with-parent"));
        assert!(rendered.contains("--extraintf oldrc"));
        assert!(rendered.contains("--rc-unix=/tmp/kichi-play/1.sock"));
        assert!(rendered.contains("--play-and-exit"));
        assert!(args.iter().any(|arg| arg == "/tmp/kichi-play/1.m3u"));
        assert!(rendered.contains("影·片"));
        assert!(!rendered.contains("https://"), "直链不进命令行");
    }

    #[test]
    fn playlist_is_private_single_line_and_has_title() {
        let path = write_playlist("https://cdn.invalid/v?sign=SECRET", "  第 1 集\n第二行  ")
            .expect("写清单失败");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path)
                .expect("读权限")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let body = std::fs::read_to_string(&path).expect("读清单");
        assert!(body.contains("#EXTINF:-1,第 1 集 第二行"));
        assert!(body.contains("https://cdn.invalid/v?sign=SECRET"));
        assert_eq!(body.lines().count(), 3);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn numeric_parser_ignores_rc_status_lines() {
        assert_eq!(parse_number(" 12 \r\n"), Some(12.0));
        assert_eq!(parse_number("status change: ( play state: 3 )"), None);
        assert_eq!(parse_number("NaN"), None);
    }
}
