//! mpv 的 IPC 客户端（只用标准库 + 已依赖的 `serde_json`）。
//!
//! 用法（见 [`crate::player::session`]）：spawn mpv 时加
//! `--idle=yes --input-ipc-server=<sock>`，连上 socket 后先写
//! `observe_property` 与 `loadfile`，再用 [`watch`] 等「窗口真的出现」：
//!
//! - `vo-configured` 变 true：窗口已创建，正是可以收起提示条的时机；
//! - `audio-params` 非空：纯音频永远没有窗口，用解码参数代替；
//! - `end-file(reason=error)`：链接坏了 / 被拒，播放器随后自己退出；
//! - 就绪后必须写 `set_property idle no`，否则 `--idle=yes` 会让进程在播完后
//!   一直留着不退出。
//!
//! 直链只在 IPC 里出现：不进 argv、不落盘（`docs/adr/0008-mpv-idle-ipc.md`）。

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::session::download::Cancel;

use super::{expand, PlayerConfig};

/// 连 socket 的总宽限（mpv 实测 0.13–0.15 秒就绪）。
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// 每次重连的间隔。
const CONNECT_RETRY: Duration = Duration::from_millis(20);
/// 读循环的切片：每轮查一次取消标志，`quit` 后也不会卡死。
const READ_TIMEOUT: Duration = Duration::from_millis(200);
/// 连上之后这么久还一个 JSON 都没读到，就当播放器不吭声（老 mpv 等）。
const FIRST_SIGNAL_TIMEOUT: Duration = Duration::from_secs(5);

/// `observe_property` 的请求号：回包 `request_id` 用它区分是哪个属性的结果。
const VO_CONF_REQ: u64 = 1;
const AUDIO_REQ: u64 = 2;

/// 链接打不开时的自动重试：PikPak 的签名直链偶发第一次被 CDN 拒（403），
/// 同一个 URL 立刻再 `loadfile` 就能播——实测复现过（见 ADR 0007）。
const LOAD_RETRY_DELAYS: [Duration; 2] = [Duration::from_millis(300), Duration::from_millis(900)];
/// 链接打不开时最多自动重试几次（= 上面那个表的长度）。
pub const LOAD_RETRIES: usize = LOAD_RETRY_DELAYS.len();

/// 第 `attempt` 次重试前要等多久（`attempt` 从 0 起）。
pub fn retry_delay(attempt: usize) -> Duration {
    LOAD_RETRY_DELAYS.get(attempt).copied().unwrap_or(
        *LOAD_RETRY_DELAYS
            .last()
            .unwrap_or(&Duration::from_millis(900)),
    )
}

/// 一次播放从启动到「窗口出现 / 失败 / 取消」的判定结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartOutcome {
    /// `vo-configured=true`，或 `audio-params` 非空（纯音频）
    Ready,
    /// 播放器自己报告这个链接打不开（`end-file(reason=error)`）。
    /// 可能只是 CDN 第一次没准备好，值得用同一个直链再试
    LoadFailed(String),
    /// 进程在就绪前退出 / 读状态出错：重试没有意义
    Failed(String),
    /// 连不上 / 老 mpv 拒绝 observe / 一直没有信号：当作已交给播放器，但没法取消
    Unknown,
    Cancelled,
}

/// `<temp_dir>/kichi-play/<pid>-<毫秒>-<计数器>.sock`。
///
/// 与旧清单共用目录；每次播放独立文件名，绝不复用（mpv 自己会以 0600 建它）。
pub fn socket_path() -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    let name = format!(
        "{}-{}-{}.sock",
        std::process::id(),
        stamp,
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let dir = super::play_dir();
    let path = dir.join(name);
    // unix socket 路径上限约 108 字节：默认临时目录异常长时退回 /tmp
    if path.as_os_str().len() < 100 {
        path
    } else {
        Path::new("/tmp/kichi-play").join(path.file_name().unwrap_or_default())
    }
}

/// 一条与 mpv 的连接。
pub struct Connection {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    /// 最近一次从 mpv 日志里抓到的 HTTP 失败原因（`403 Forbidden` 之类）
    pub last_http_error: Option<String>,
}

impl Connection {
    /// 写一行 JSON（mpv 的 IPC 协议：一行一条命令 / 事件）。
    pub fn send(&mut self, value: &Value) -> std::io::Result<()> {
        let mut line = value.to_string();
        line.push('\n');
        self.writer.write_all(line.as_bytes())?;
        self.writer.flush()
    }

    /// 订阅「窗口已创建」与「音频参数」两个属性。
    pub fn observe_start(&mut self) -> std::io::Result<()> {
        self.send(&json!({"command": ["observe_property", VO_CONF_REQ, "vo-configured"]}))?;
        self.send(&json!({"command": ["observe_property", AUDIO_REQ, "audio-params"]}))
    }

    /// 让 mpv 在出错时把日志经 IPC 送过来：HTTP 状态码只在日志里。
    pub fn observe_log(&mut self) -> std::io::Result<()> {
        self.send(&json!({"command": ["request_log_messages", "warn"]}))
    }

    /// 让 mpv 播放这个直链。
    pub fn loadfile(&mut self, url: &str) -> std::io::Result<()> {
        self.send(&json!({"command": ["loadfile", url]}))
    }

    /// 就绪后关掉 idle：播完自动退出，不留下一个空窗口的进程。
    pub fn disable_idle(&mut self) -> std::io::Result<()> {
        self.send(&json!({"command": ["set_property", "idle", false]}))
    }

    /// 让 mpv 退出（取消 / kichi 退出）。
    pub fn quit(&mut self) -> std::io::Result<()> {
        self.send(&json!({"command": ["quit"]}))
    }
}

/// 连上 socket（最多等 [`CONNECT_TIMEOUT`]，每 [`CONNECT_RETRY`] 一次）。
///
/// 连不上或期间被取消返回 `None`。
pub fn connect(socket: &Path, cancel: &Cancel) -> Option<Connection> {
    let deadline = Instant::now() + CONNECT_TIMEOUT;
    loop {
        if cancel.is_cancelled() {
            return None;
        }
        if let Some(connection) = try_connect(socket) {
            return Some(connection);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(CONNECT_RETRY);
    }
}

/// 只试一次（退出收尾用，不能在那里等 5 秒）。
pub fn try_connect(socket: &Path) -> Option<Connection> {
    let stream = UnixStream::connect(socket).ok()?;
    stream.set_read_timeout(Some(READ_TIMEOUT)).ok()?;
    // 读写两条句柄：读超时挂在同一个 socket 上，不影响写
    let reader = BufReader::new(stream.try_clone().ok()?);
    Some(Connection {
        reader,
        writer: stream,
        last_http_error: None,
    })
}

/// 等「窗口出现 / 失败 / 取消」。调用前连接上必须已经发过 observe + loadfile。
///
/// 每 [`READ_TIMEOUT`] 查一次取消标志；连上后一直没有任何信号超过
/// [`FIRST_SIGNAL_TIMEOUT`] 也算 `Unknown`（不让用户永远等一个不吭声的播放器）。
pub fn watch(conn: &mut Connection, cancel: &Cancel) -> StartOutcome {
    let started = Instant::now();
    let mut signals = 0usize;
    let mut line = String::new();
    loop {
        if cancel.is_cancelled() {
            return StartOutcome::Cancelled;
        }
        line.clear();
        match conn.reader.read_line(&mut line) {
            Ok(0) => {
                return if cancel.is_cancelled() {
                    StartOutcome::Cancelled
                } else {
                    // 进程退出了：重试没有意义（可能是 spawn 出来的播放器根本没起来）
                    StartOutcome::Failed("播放器还没开始播放就退出了".to_owned())
                };
            }
            Ok(_) => {}
            Err(err) if is_timeout(&err) => {
                if signals == 0 && started.elapsed() >= FIRST_SIGNAL_TIMEOUT {
                    return StartOutcome::Unknown;
                }
                continue;
            }
            Err(err) => return StartOutcome::Failed(format!("读取播放器状态失败：{err}")),
        }
        let Ok(value) = serde_json::from_str::<Value>(line.trim()) else {
            continue;
        };
        signals += 1;
        // HTTP 层的真实原因（403 / 404 / 超时）只出现在 log-message 里：
        // 记下来给用户看，否则只能笼统说「无法播放」
        if let Some(reason) = http_failure_from_log(&value) {
            conn.last_http_error = Some(reason);
        }
        if let Some(outcome) = interpret(&value) {
            return outcome;
        }
    }
}

/// mpv 的 log-message 里有没有 HTTP 层的失败（`http: HTTP error 403 Forbidden`）。
///
/// 只用于给用户一句人话解释，不参与播放判定。
fn http_failure_from_log(value: &Value) -> Option<String> {
    if value.get("event").and_then(Value::as_str) != Some("log-message") {
        return None;
    }
    let text = value.get("text").and_then(Value::as_str)?.trim();
    let rest = text.split_once("HTTP error ")?.1;
    // 只留状态码与短语（"403 Forbidden"），后面的 URL 一律丢掉
    let reason = rest
        .split_whitespace()
        .take(2)
        .collect::<Vec<_>>()
        .join(" ");
    (!reason.is_empty()).then_some(reason)
}

/// `end-file(reason=error)` + 已知 HTTP 原因 → 给用户看的文案。
fn load_failure_text(detail: &str) -> String {
    let base = "播放器打不开这个链接";
    match detail.trim() {
        "" | "loading failed" => format!("{base}（播放器报告加载失败）"),
        other => format!("{base}：{other}"),
    }
}

/// 单条 mpv 消息 → 判定（`None` = 继续等）。
fn interpret(value: &Value) -> Option<StartOutcome> {
    if let Some(event) = value.get("event").and_then(Value::as_str) {
        match event {
            "property-change" => {
                let name = value
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let data = value.get("data").unwrap_or(&Value::Null);
                let ready = match name {
                    // 窗口创建：mpv 实测本地文件 0.29 秒、慢服务器 3.25 秒
                    "vo-configured" => data.as_bool() == Some(true),
                    // 纯音频永远没有窗口，解码参数一有值就算就绪
                    "audio-params" => !data.is_null(),
                    _ => false,
                };
                return ready.then_some(StartOutcome::Ready);
            }
            "end-file" => {
                let reason = value
                    .get("reason")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if reason == "error" {
                    // mpv 会把真正的原因放在 file_error（如 "loading failed"），
                    // 但 HTTP 状态码只走 log-message；这里按「可重试」上报
                    let detail = value
                        .get("file_error")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    return Some(StartOutcome::LoadFailed(load_failure_text(detail)));
                }
            }
            _ => {}
        }
        return None;
    }
    // 命令回包：老 mpv 没有这两个属性会回 error，这时观测不了，只能当已开播
    if let Some(request) = value.get("request_id").and_then(Value::as_u64) {
        let error = value
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("success");
        if matches!(request, VO_CONF_REQ | AUDIO_REQ) && error != "success" {
            return Some(StartOutcome::Unknown);
        }
    }
    None
}

/// 拼 mpv 的启动命令（不含直链：直链只走 IPC 的 `loadfile`）。
///
/// flatpak 情形额外插 `--die-with-parent`：只杀 `flatpak run` 父进程会在沙箱里
/// 留下孤儿的 `mpv-bin` / `bwrap`（本机实测），加了这个参数 2 秒内全退。
/// 参数必须紧跟 `run`——插错位置会被 flatpak 当成应用自己的参数。
pub fn build_command(config: &PlayerConfig, socket: &Path, title: &str) -> std::process::Command {
    use std::process::{Command, Stdio};

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
        .arg("--idle=yes")
        .arg(format!("--input-ipc-server={}", socket.display()))
        .stdin(Stdio::null())
        // 播放器输出一律丢弃：mpv 默认会把直链打到 stderr
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

/// `program` 是不是 flatpak（`flatpak` 或 `/usr/bin/flatpak`）。
fn is_flatpak(config: &PlayerConfig) -> bool {
    Path::new(&config.program)
        .file_name()
        .is_some_and(|name| name == "flatpak")
}

fn is_timeout(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::split_words;
    use std::os::unix::net::UnixListener;

    /// 假 mpv：接一次连接，按脚本发事件，返回从客户端收到的命令行。
    ///
    /// `linger = None`：发完脚本立刻断开，模拟「播放器没开播就退出了」。
    fn fake_mpv(
        socket: &Path,
        script: &[&str],
        linger: Option<Duration>,
    ) -> std::thread::JoinHandle<Vec<String>> {
        let listener = UnixListener::bind(socket).expect("bind 假 mpv socket");
        let script: Vec<String> = script.iter().map(|line| (*line).to_owned()).collect();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("接受连接");
            let mut writer = stream.try_clone().expect("复制 socket");
            let mut received = Vec::new();
            std::thread::sleep(Duration::from_millis(30));
            for line in &script {
                let _ = writeln!(writer, "{line}");
                let _ = writer.flush();
                std::thread::sleep(Duration::from_millis(10));
            }
            let Some(linger) = linger else {
                // 断开连接：客户端 read_line 立刻读到 EOF
                drop(writer);
                drop(stream);
                return received;
            };
            // 收客户端发来的命令（loadfile / observe），顺便把连接多留一会儿
            let mut reader = BufReader::new(stream);
            let started = Instant::now();
            while started.elapsed() < linger {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) => break,
                    Ok(_) => received.push(line.trim().to_owned()),
                    Err(err) if is_timeout(&err) => {}
                    Err(_) => break,
                }
            }
            received
        })
    }

    fn temp_socket(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kichi-ipc-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(format!("{name}.sock"));
        let _ = std::fs::remove_file(&path);
        path
    }

    fn run_watch(
        name: &str,
        script: &[&str],
        linger: Option<Duration>,
    ) -> (StartOutcome, Vec<String>) {
        let socket = temp_socket(name);
        let server = fake_mpv(&socket, script, linger);
        let cancel = Cancel::default();
        let mut conn = connect(&socket, &cancel).expect("连上假 mpv");
        conn.observe_start().expect("发 observe");
        conn.loadfile("https://cdn.invalid/file?sign=SECRET")
            .expect("发 loadfile");
        let outcome = watch(&mut conn, &cancel);
        drop(conn);
        let received = server.join().expect("假 mpv 线程结束");
        let _ = std::fs::remove_file(&socket);
        (outcome, received)
    }

    #[test]
    fn window_created_means_ready() {
        let (outcome, received) = run_watch(
            "ready",
            &[r#"{"event":"property-change","name":"vo-configured","data":true}"#],
            Some(Duration::from_millis(200)),
        );
        assert_eq!(outcome, StartOutcome::Ready);
        // 直链只经 IPC，命令里确实发了 loadfile
        assert!(
            received.iter().any(|line| line.contains("loadfile")),
            "应当发 loadfile：{received:?}"
        );
        assert!(received
            .iter()
            .any(|line| line.contains("observe_property")));
    }

    #[test]
    fn audio_params_mean_ready_for_audio_files() {
        let (outcome, _) = run_watch(
            "audio",
            &[r#"{"event":"property-change","name":"audio-params","data":{"samplerate":44100}}"#],
            Some(Duration::from_millis(200)),
        );
        assert_eq!(outcome, StartOutcome::Ready);
    }

    #[test]
    fn null_audio_params_do_not_count_as_ready() {
        // 先来一个 null（还没解码），再来窗口：不能因为 null 就收提示，
        // 也不能因为「有消息」就误判就绪
        let (outcome, _) = run_watch(
            "audio-null",
            &[
                r#"{"event":"property-change","name":"audio-params","data":null}"#,
                r#"{"event":"property-change","name":"vo-configured","data":true}"#,
            ],
            Some(Duration::from_millis(200)),
        );
        assert_eq!(outcome, StartOutcome::Ready);
    }

    #[test]
    fn end_file_error_is_a_retryable_load_failure() {
        let (outcome, _) = run_watch(
            "bad-link",
            &[r#"{"event":"end-file","reason":"error","file_error":"loading failed"}"#],
            Some(Duration::from_millis(200)),
        );
        // 链接打不开 ≠ 播放器坏了：调用方会用同一个直链重试
        let StartOutcome::LoadFailed(text) = outcome else {
            panic!("应当是可重试的加载失败：{outcome:?}");
        };
        assert!(text.contains("加载失败"), "{text}");
    }

    #[test]
    fn http_status_from_the_mpv_log_is_reported() {
        let (outcome, _) = run_watch(
            "http-403",
            &[
                r#"{"event":"log-message","level":"warn","text":"http: HTTP error 403 Forbidden\n"}"#,
                r#"{"event":"end-file","reason":"error","file_error":"loading failed"}"#,
            ],
            Some(Duration::from_millis(200)),
        );
        assert!(
            matches!(outcome, StartOutcome::LoadFailed(_)),
            "{outcome:?}"
        );
    }

    #[test]
    fn http_status_is_not_taken_from_logs_that_are_not_http() {
        assert_eq!(
            http_failure_from_log(&json!({"event": "log-message", "text": "vo: x"})),
            None
        );
        assert_eq!(
            http_failure_from_log(&json!({
                "event": "log-message",
                "text": "http: HTTP error 403 Forbidden\n"
            })),
            Some("403 Forbidden".to_owned())
        );
        // 不是日志事件的一律不认
        assert_eq!(
            http_failure_from_log(&json!({"event": "end-file", "text": "HTTP error 500"})),
            None
        );
    }

    #[test]
    fn observe_error_means_unknown() {
        // 老 mpv 没有 vo-configured / audio-params
        let (outcome, _) = run_watch(
            "observe-error",
            &[r#"{"request_id":1,"error":"property not found"}"#],
            Some(Duration::from_millis(200)),
        );
        assert_eq!(outcome, StartOutcome::Unknown);
    }

    #[test]
    fn successful_reply_is_not_unknown() {
        let (outcome, _) = run_watch(
            "observe-ok",
            &[
                r#"{"request_id":1,"error":"success"}"#,
                r#"{"event":"property-change","name":"vo-configured","data":true}"#,
            ],
            Some(Duration::from_millis(200)),
        );
        assert_eq!(outcome, StartOutcome::Ready);
    }

    #[test]
    fn socket_eof_before_ready_is_a_failure() {
        // 脚本为空、连接很快断开 = mpv 没开播就退出了
        let (outcome, _) = run_watch("eof", &[], None);
        assert!(matches!(outcome, StartOutcome::Failed(_)), "{outcome:?}");
    }

    #[test]
    fn cancellation_returns_quickly() {
        let socket = temp_socket("cancel");
        let server = fake_mpv(&socket, &[], Some(Duration::from_millis(900)));
        let cancel = std::sync::Arc::new(Cancel::default());
        let mut conn = connect(&socket, &cancel).expect("连上假 mpv");
        conn.observe_start().expect("发 observe");

        let flag = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            flag.cancel();
        });
        let started = Instant::now();
        let outcome = watch(&mut conn, &cancel);
        let elapsed = started.elapsed();
        assert_eq!(outcome, StartOutcome::Cancelled);
        assert!(
            elapsed < Duration::from_millis(300),
            "取消后最多等一个读切片：{elapsed:?}"
        );
        drop(conn);
        let _ = server.join();
        let _ = std::fs::remove_file(&socket);
    }

    #[test]
    fn connect_gives_up_when_nothing_listens() {
        let socket = temp_socket("nobody-home");
        let cancel = Cancel::default();
        let started = Instant::now();
        assert!(try_connect(&socket).is_none());
        // connect() 会重试到 5 秒，这里只验证「取消立刻放弃」
        let flag = cancel;
        flag.cancel();
        assert!(connect(&socket, &flag).is_none());
        assert!(started.elapsed() < CONNECT_TIMEOUT);
    }

    #[test]
    fn mpv_command_keeps_the_url_out_of_argv() {
        let config = PlayerConfig {
            program: "flatpak".to_owned(),
            pre_args: vec!["run".to_owned(), "io.mpv.Mpv".to_owned()],
            access_args: split_words("--force-media-title={title} --ytdl=no"),
        };
        let socket = Path::new("/tmp/kichi-play/1-2-3.sock");
        let command = build_command(&config, socket, "影·片");
        let args: Vec<_> = command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            vec![
                "run".to_owned(),
                "--die-with-parent".to_owned(),
                "io.mpv.Mpv".to_owned(),
                "--force-media-title=影·片".to_owned(),
                "--ytdl=no".to_owned(),
                "--idle=yes".to_owned(),
                "--input-ipc-server=/tmp/kichi-play/1-2-3.sock".to_owned(),
            ]
        );
        let rendered = args.join(" ");
        assert!(!rendered.contains("https://"), "直链不进命令行");
    }

    #[test]
    fn die_with_parent_only_for_flatpak_and_only_once() {
        // 普通 mpv：不插 flatpak 专用参数
        let plain = PlayerConfig {
            program: "mpv".to_owned(),
            pre_args: Vec::new(),
            access_args: Vec::new(),
        };
        let args: Vec<_> = build_command(&plain, Path::new("/tmp/a.sock"), "t")
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(!args.iter().any(|arg| arg == "--die-with-parent"));

        // 用户自己已经写了：不重复插
        let custom = PlayerConfig {
            program: "/usr/bin/flatpak".to_owned(),
            pre_args: vec![
                "run".to_owned(),
                "--die-with-parent".to_owned(),
                "io.mpv.Mpv".to_owned(),
            ],
            access_args: Vec::new(),
        };
        let args: Vec<_> = build_command(&custom, Path::new("/tmp/a.sock"), "t")
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args.iter()
                .filter(|arg| *arg == "--die-with-parent")
                .count(),
            1
        );
        assert_eq!(args[0], "run");
    }
}
