//! 播放会话测试：登记表 / mpv / VLC / 字幕 / 失败路径。

use std::process::Child;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::*;

mod failures;
mod mpv;
mod registry;
mod subtitles;
mod vlc;

fn serial() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

fn sleeper() -> Child {
    std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .expect("启动 sleep 失败")
}

fn wait_for_no_sessions() {
    let deadline = Instant::now() + Duration::from_secs(2);
    while active_count() > 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
}

const FAKE_MPV: &str = r#"import json, os, socket, sys, time
sock = None
mode = "ready"
log = None
for arg in sys.argv[1:]:
    if arg.startswith("--input-ipc-server="):
        sock = arg.split("=", 1)[1]
    elif arg.startswith("--fake-mode="):
        mode = arg.split("=", 1)[1]
    elif arg.startswith("--fake-log="):
        log = arg.split("=", 1)[1]
if not sock:
    sys.exit(2)
if os.path.exists(sock):
    os.remove(sock)
server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
server.bind(sock)
server.listen(1)
server.settimeout(20)
try:
    conn, _ = server.accept()
except socket.timeout:
    sys.exit(3)
conn.settimeout(0.2)
def send(obj):
    conn.sendall((json.dumps(obj) + "\n").encode())
buffer = b""
loads = 0
deadline = time.time() + 30
while time.time() < deadline:
    try:
        data = conn.recv(65536)
    except socket.timeout:
        continue
    if not data:
        break
    buffer += data
    while b"\n" in buffer:
        line, buffer = buffer.split(b"\n", 1)
        msg = json.loads(line)
        cmd = msg.get("command") or []
        if cmd and cmd[0] == "observe_property":
            send({"request_id": 0, "error": "success"})
        elif cmd and cmd[0] == "loadfile":
            if mode in ("ready", "sub-fail", "progress", "complete"):
                time.sleep(0.2)
                if mode == "progress":
                    send({"event": "property-change", "name": "duration", "data": 120.0})
                    send({"event": "property-change", "name": "time-pos", "data": 42.0})
                elif mode == "complete":
                    send({"event": "property-change", "name": "duration", "data": 120.0})
                    send({"event": "property-change", "name": "time-pos", "data": 119.2})
                send({"event": "property-change", "name": "vo-configured", "data": True})
            elif mode == "fail":
                time.sleep(0.1)
                send({"event": "end-file", "reason": "error"})
            elif mode == "fail-once":
                # 第一次拒绝（CDN 还没准备好），第二次同一个链接就成功
                loads += 1
                time.sleep(0.05)
                if loads == 1:
                    send({"event": "log-message", "level": "warn", "text": "http: HTTP error 403 Forbidden"})
                    send({"event": "end-file", "reason": "error", "file_error": "loading failed"})
                else:
                    time.sleep(0.05)
                    send({"event": "property-change", "name": "vo-configured", "data": True})
            elif mode == "never":
                pass  # 一直不出窗口：测「取消关掉还没出窗口的 mpv」
        elif cmd and cmd[0] == "sub-add":
            if log:
                with open(log, "a") as f:
                    f.write("%s\t%s\n" % (cmd[2] if len(cmd) > 2 else "",
                                          cmd[3] if len(cmd) > 3 else ""))
            if mode == "sub-fail":
                send({"request_id": msg.get("request_id"), "error": "loading failed"})
            else:
                send({"request_id": msg.get("request_id"), "error": "success"})
        elif cmd and cmd[0] == "seek":
            if log:
                with open(log, "a") as f:
                    f.write("seek\t%s\n" % (cmd[1] if len(cmd) > 1 else ""))
            send({"request_id": 0, "error": "success"})
        elif cmd and cmd[0] == "quit":
            os.remove(sock)
            sys.exit(0)
        elif cmd and cmd[0] == "set_property":
            if mode == "complete":
                send({"event": "end-file", "reason": "eof"})
                time.sleep(0.01)
                sys.exit(0)
            else:
                send({"request_id": 0, "error": "success"})
"#;

/// 假 VLC：只实现 Kichi 用到的 oldrc 数值命令。
const FAKE_VLC: &str = r#"import os, socket, sys
sock = None
log = None
mode = "ready"
for arg in sys.argv[1:]:
    if arg.startswith("--rc-unix="):
        sock = arg.split("=", 1)[1]
    elif arg.startswith("--fake-log="):
        log = arg.split("=", 1)[1]
    elif arg.startswith("--fake-mode="):
        mode = arg.split("=", 1)[1]
if not sock:
    sys.exit(2)
if os.path.exists(sock):
    os.remove(sock)
server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
server.bind(sock)
server.listen(1)
server.settimeout(20)
try:
    conn, _ = server.accept()
except socket.timeout:
    sys.exit(3)
conn.settimeout(0.2)
buffer = b""
position = 0
while True:
    try:
        data = conn.recv(65536)
    except socket.timeout:
        continue
    if not data:
        break
    buffer += data
    while b"\n" in buffer:
        line, buffer = buffer.split(b"\n", 1)
        cmd = line.decode(errors="replace").strip()
        if cmd == "is_playing":
            if mode == "fail":
                sys.exit(4)
            conn.sendall(b"1\n")
        elif cmd == "get_time":
            position += 2
            conn.sendall((str(position) + "\n").encode())
        elif cmd == "get_length":
            conn.sendall(b"120\n")
        elif cmd.startswith("seek "):
            if log:
                with open(log, "a") as f:
                    f.write("seek\t%s\n" % cmd.split(" ", 1)[1])
            conn.sendall(b"seek: returned 0 (no error)\n")
        elif cmd == "quit":
            os.remove(sock)
            sys.exit(0)
"#;

/// 一条「参数里带 mpv、实际跑假 mpv 脚本」的播放器命令。
///
/// `is_mpv` 看的是程序名和前置参数，脚本名 `fake_mpv.py` 就含 mpv；
/// 真正执行的是 python3 脚本，不依赖可执行位。
fn fake_mpv_config(mode: &str) -> PlayerConfig {
    let dir = std::env::temp_dir().join(format!("kichi-fake-mpv-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let script = dir.join("fake_mpv.py");
    std::fs::write(&script, FAKE_MPV).expect("写假 mpv 脚本");
    PlayerConfig {
        program: "python3".to_owned(),
        pre_args: vec![
            script.to_string_lossy().into_owned(),
            format!("--fake-mode={mode}"),
        ],
        // access_args 是真正会传给播放器的参数；这里留空，避免 python 把
        // `--force-media-title=` 当自己的选项
        access_args: Vec::new(),
    }
}

/// 同 [`fake_mpv_config`]，但把收到的 `sub-add`（flags + title）追加写到 `log`。
fn fake_mpv_config_with_log(mode: &str, log: &std::path::Path) -> PlayerConfig {
    let mut config = fake_mpv_config(mode);
    config
        .pre_args
        .push(format!("--fake-log={}", log.display()));
    config
}

fn fake_vlc_config(mode: &str, log: &std::path::Path) -> PlayerConfig {
    let dir = std::env::temp_dir().join(format!("kichi-fake-vlc-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let script = dir.join("fake_vlc.py");
    std::fs::write(&script, FAKE_VLC).expect("写假 VLC 脚本");
    PlayerConfig {
        program: "python3".to_owned(),
        pre_args: vec![
            script.to_string_lossy().into_owned(),
            format!("--fake-mode={mode}"),
            format!("--fake-log={}", log.display()),
        ],
        access_args: Vec::new(),
    }
}
