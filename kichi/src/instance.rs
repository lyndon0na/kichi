//! 单实例保护：同一用户会话只允许一个 kichi 进程。
//!
//! 首个进程取得运行时锁并监听 Unix socket；再次启动时第二个进程连上 socket
//! 发送 `show`，由已有实例把窗口唤回（与托盘「显示」同一条路径），然后自己退出。
//!
//! 为什么不用 D-Bus：托盘已经依赖 D-Bus，但这里只需要「同用户、同会话、点对点」
//! 的一次唤醒，Unix socket + flock 用标准库就够，不增加依赖，也不受 session bus
//! 重连 / 超时影响。socket 放在 `$XDG_RUNTIME_DIR`（0700），退化目录用
//! `/tmp/kichi-<uid>`，socket 文件本身 0600。
//!
//! 崩溃遗留的 socket 文件不会挡住下一次启动：锁随进程消失自动释放，新进程拿到锁后
//! 只删除「确实是 socket」的旧路径，再重新 bind。锁优先保证同一时刻只有一个注册者，
//! socket 只负责唤回；即使监听线程起不来，后来的进程也会因为拿不到锁而退出。

use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, ErrorKind, Read, Write};
use std::os::unix::fs::{FileTypeExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

/// 二次启动请求：一行文本，方便排查 socket 流量。
const ACTIVATION: &[u8] = b"show\n";
const ACK: &[u8] = b"ok\n";
/// 二次启动最多等这么久：覆盖「首个实例已拿锁、还没 bind」和「旧实例正在退出」
/// 两种窗口，同时保证已有实例异常时第二个进程也不会长时间挂着。
const SECONDARY_WAIT: Duration = Duration::from_secs(2);
const SECONDARY_RETRY: Duration = Duration::from_millis(50);
const LISTENER_TICK: Duration = Duration::from_millis(100);

/// 本进程是否已经作为唯一实例启动。
pub enum Claim {
    /// 取得了实例锁；必须一直持有到进程退出。
    Primary(Instance),
    /// 已有实例在运行，已请求它唤回窗口。
    Secondary,
    /// 锁 / socket 路径异常，无法提供保护；按普通启动继续（只打 stderr）。
    Unprotected,
}

/// 首个实例的运行时资源。Drop 时释放锁并删除自己的 socket 文件。
pub struct Instance {
    activations: Arc<AtomicU32>,
    stop: Arc<AtomicBool>,
    socket_path: PathBuf,
    owns_socket: bool,
    #[allow(dead_code)] // 字段不被读取，但必须一直持有：flock 随文件关闭释放
    lock_file: File,
}

impl Drop for Instance {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if self.owns_socket {
            let _ = fs::remove_file(&self.socket_path);
        }
        // 锁文件故意不删：unlink 后另一个进程会在同一路径新建一把锁并锁住它，
        // 两边各持一把锁，单实例保护就失效了。socket 文件删除没有这个问题。
    }
}

/// 主线程 `pump()` 读取唤回请求数。没有取得实例锁时恒为 0。
pub fn take_activations() -> u32 {
    let slot = activations_slot()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    slot.as_ref()
        .map(|count| count.swap(0, Ordering::SeqCst))
        .unwrap_or(0)
}

/// 启动最早期调用：先判断自己是不是唯一实例，再做任何清理 / 建窗口。
pub fn claim() -> Claim {
    let path = socket_path();
    let deadline = Instant::now() + SECONDARY_WAIT;
    while Instant::now() < deadline {
        match try_claim_once(&path) {
            Ok(Some(instance)) => {
                let mut slot = activations_slot()
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                *slot = Some(instance.activations.clone());
                return Claim::Primary(instance);
            }
            Ok(None) => {
                // 别人持有锁，但可能还没 bind 好 socket：稍等再通知。
                if notify_existing(&path).is_ok() {
                    return Claim::Secondary;
                }
            }
            Err(err) => {
                eprintln!("[kichi] 单实例保护不可用，按普通启动继续：{err}");
                return Claim::Unprotected;
            }
        }
        thread::sleep(SECONDARY_RETRY);
    }
    // 锁始终被别人持有，通知又一直失败：宁可退出，也不能起第二个进程。
    eprintln!("[kichi] 已有实例在运行，但未能请求它显示窗口");
    Claim::Secondary
}

fn activations_slot() -> &'static Mutex<Option<Arc<AtomicU32>>> {
    static SLOT: OnceLock<Mutex<Option<Arc<AtomicU32>>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

/// 尝试一次成为首个实例。
///
/// - `Ok(Some)`：拿到锁并（尽力）开始监听；
/// - `Ok(None)`：锁在别的进程手里；
/// - `Err`：锁文件 / socket 路径无法使用，调用方退化为无保护启动。
fn try_claim_once(path: &Path) -> io::Result<Option<Instance>> {
    let lock_path = path.with_extension("lock");
    let lock_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(&lock_path)?;
    match lock_file.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => return Ok(None),
        Err(TryLockError::Error(err)) => return Err(err),
    }

    // 锁已经在自己手里，这个路径上的 socket 只可能属于已经退出的旧实例。
    // 只删除 socket 文件本身，别碰同名普通文件 / 符号链接。
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_socket() => match fs::remove_file(path) {
            Ok(()) => {}
            Err(err) if err.kind() == ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        },
        Ok(_) => {
            return Err(io::Error::new(
                ErrorKind::AlreadyExists,
                "实例 socket 路径已被非 socket 文件占用",
            ))
        }
        Err(err) if err.kind() == ErrorKind::NotFound => {}
        Err(err) => return Err(err),
    }

    let listener = UnixListener::bind(path)?;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));

    let activations = Arc::new(AtomicU32::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    if let Err(err) = spawn_listener(listener, activations.clone(), stop.clone()) {
        // 监听失败只影响「唤回窗口」，锁仍然有效，后来者依旧会被挡下。
        eprintln!("[kichi] 单实例唤回通道不可用：{err}");
    }

    Ok(Some(Instance {
        activations,
        stop,
        socket_path: path.to_path_buf(),
        owns_socket: true,
        lock_file,
    }))
}

fn spawn_listener(
    listener: UnixListener,
    activations: Arc<AtomicU32>,
    stop: Arc<AtomicBool>,
) -> io::Result<()> {
    listener.set_nonblocking(true)?;
    thread::Builder::new()
        .name("kichi-instance".to_owned())
        .spawn(move || {
            loop {
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                match listener.accept() {
                    Ok((stream, _)) => handle_connection(stream, &activations),
                    // WouldBlock 是常态（非阻塞轮询）；其它错误也按下一拍重试。
                    Err(_) => thread::sleep(LISTENER_TICK),
                }
            }
        })
        .map(|_| ())
}

fn handle_connection(mut stream: UnixStream, activations: &AtomicU32) {
    let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(500)));
    let mut buf = [0u8; 32];
    let read = match stream.read(&mut buf) {
        Ok(read) => read,
        Err(_) => return,
    };
    let line = buf[..read]
        .split(|byte| *byte == b'\n')
        .next()
        .unwrap_or(&[]);
    if line != b"show" {
        return;
    }
    activations.fetch_add(1, Ordering::SeqCst);
    let _ = stream.write_all(ACK);
}

/// 通知已有实例显示窗口。写入后必须收到回执：监听线程可能已经随旧进程停掉，
/// 这时调用方应该继续重试，等锁释放后由本次启动接管。
fn notify_existing(path: &Path) -> io::Result<()> {
    let mut stream = UnixStream::connect(path)?;
    stream.set_read_timeout(Some(Duration::from_secs(1)))?;
    stream.set_write_timeout(Some(Duration::from_secs(1)))?;
    stream.write_all(ACTIVATION)?;
    // 必须等到回执：监听线程可能在旧进程收尾时已经停了，这时应该重试
    // （锁会在旧进程退出后释放，本次启动还能接管），而不是静默退出。
    let mut ack = [0u8; 8];
    let read = stream.read(&mut ack)?;
    if ack[..read].starts_with(b"ok") {
        Ok(())
    } else {
        Err(io::Error::new(ErrorKind::InvalidData, "实例回执无效"))
    }
}

fn socket_path() -> PathBuf {
    if let Some(path) = std::env::var_os("KICHI_INSTANCE_SOCKET") {
        return PathBuf::from(path);
    }
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        return PathBuf::from(dir).join("kichi.sock");
    }
    // 没有 XDG_RUNTIME_DIR 的退化路径：目录只给当前用户，socket 文件本身 0600。
    let dir = std::env::temp_dir().join(format!("kichi-{}", current_uid()));
    if fs::create_dir_all(&dir).is_ok() {
        let _ = fs::set_permissions(&dir, fs::Permissions::from_mode(0o700));
    }
    dir.join("instance.sock")
}

/// `/proc/self/status` 里的 real uid。拿不到时退回 0；socket 文件本身仍是 0600，
/// 最坏情况只是锁路径不可用、退化成无保护启动，不会 panic。
fn current_uid() -> u32 {
    fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status
                .lines()
                .find(|line| line.starts_with("Uid:"))
                .and_then(|line| line.split_whitespace().nth(1))
                .and_then(|value| value.parse().ok())
        })
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{notify_existing, try_claim_once, Instance};
    use std::fs;
    use std::os::unix::fs::{FileTypeExt, PermissionsExt};
    use std::path::PathBuf;
    use std::sync::atomic::Ordering;
    use std::thread;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    fn temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "kichi-instance-{}-{nanos}-{label}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn wait_for_activation(instance: &Instance) -> u32 {
        for _ in 0..40 {
            let count = instance.activations.swap(0, Ordering::SeqCst);
            if count > 0 {
                return count;
            }
            thread::sleep(Duration::from_millis(50));
        }
        0
    }

    #[test]
    fn second_launch_reaches_the_first_instance() {
        let dir = temp_dir("notify");
        let path = dir.join("instance.sock");
        let first = try_claim_once(&path)
            .expect("lock path usable")
            .expect("first claim wins");

        notify_existing(&path).expect("second process can notify");
        assert_eq!(wait_for_activation(&first), 1);

        let meta = fs::symlink_metadata(&path).expect("socket exists");
        assert!(meta.file_type().is_socket(), "路径应是 socket");
        assert_eq!(
            meta.permissions().mode() & 0o777,
            0o600,
            "socket 只允许当前用户访问"
        );

        drop(first);
        assert!(!path.exists(), "退出后删除自己的 socket");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn live_instance_blocks_a_second_claim() {
        let dir = temp_dir("blocked");
        let path = dir.join("instance.sock");
        let first = try_claim_once(&path)
            .expect("lock path usable")
            .expect("first claim wins");

        assert!(
            try_claim_once(&path).expect("lock path usable").is_none(),
            "锁被持有期间不能再成为首个实例"
        );

        drop(first);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn stale_socket_from_a_crash_is_replaced() {
        let dir = temp_dir("stale");
        let path = dir.join("instance.sock");
        {
            // 模拟崩溃遗留：bind 后直接丢掉 listener，不删除 socket 文件。
            let listener = std::os::unix::net::UnixListener::bind(&path).expect("bind stale");
            drop(listener);
        }
        assert!(path.exists(), "前置条件：残留 socket 文件");

        let instance = try_claim_once(&path)
            .expect("stale socket is recoverable")
            .expect("new instance becomes primary");
        notify_existing(&path).expect("new socket is reachable");
        assert_eq!(wait_for_activation(&instance), 1);

        drop(instance);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn non_socket_path_is_not_deleted() {
        let dir = temp_dir("regular");
        let path = dir.join("instance.sock");
        fs::write(&path, b"do not touch").expect("write regular file");

        assert!(
            try_claim_once(&path).is_err(),
            "同名普通文件必须让保护退化为无保护启动"
        );
        assert_eq!(fs::read(&path).expect("file still exists"), b"do not touch");

        let _ = fs::remove_dir_all(dir);
    }
}
