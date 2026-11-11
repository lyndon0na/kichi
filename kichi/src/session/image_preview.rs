//! 图片预览：把网盘图片按文件 id 下载到本地临时文件，QML 只读 `file://`。
//!
//! 与头像 / 缩略图同一条纪律：签名直链永远留在 Rust，界面拿不到远程地址。
//! 预览用的是**原图**（不是列表缩略图），因此单独下载并设 64 MiB 上限、
//! 20 秒空闲读超时和 120 秒总时限；超限 / 超时 / 无直链都返回可读原因。
//!
//! 临时文件放在 `$TMPDIR/kichi-preview/`（0700 目录、0600 文件）。
//! 单次预览的文件由桥接层在关闭 / 换图 / 登出时删除；进程崩溃留下的旧文件
//! 由启动时的 [`sweep_stale`] 按 24 小时清理。

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

use crate::session::media::{file_info_for_download, KnownHint};
use crate::session::private_fs::{self, OpenMode};
use crate::session::{Failure, Session};

/// 单张预览图大小上限：超过即拒绝，不把大图整份读进内存 / 磁盘。
pub const MAX_PREVIEW_BYTES: u64 = 64 * 1024 * 1024;
/// 连续这么久没收到字节就中止（下载 client 的空闲读超时）。
const IDLE_TIMEOUT: Duration = Duration::from_secs(20);
/// 整个取图过程（详情 + 直链 + 下载）的总时限。
const TOTAL_TIMEOUT: Duration = Duration::from_secs(120);

const DIR_NAME: &str = "kichi-preview";
const READ_BUFFER: usize = 64 * 1024;

/// 0 字节文件没有可预览的内容（与下载 / 播放的判定分开写，界面文案更准确）。
pub const ZERO_BYTE_PREVIEW: &str = "这个文件是 0 字节，没有可预览的内容";
/// 详情两次都没给直链。
pub const LINK_MISSING_PREVIEW: &str = "官方没有提供这个文件的图片链接";

pub fn base_dir() -> PathBuf {
    std::env::temp_dir().join(DIR_NAME)
}

/// 下载一张原图到本地临时文件，返回路径。阻塞，必须放后台线程。
pub fn download(
    session: &Session,
    file_id: &str,
    name: &str,
    hint: KnownHint,
) -> Result<PathBuf, Failure> {
    let started = Instant::now();
    let info = file_info_for_download(session, file_id, hint, None)?;
    if info.is_zero_byte() {
        return Err(Failure::Other(ZERO_BYTE_PREVIEW.to_owned()));
    }
    if let Some(size) = info.size.filter(|size| *size > MAX_PREVIEW_BYTES) {
        return Err(Failure::Other(too_large_text(size)));
    }

    let client = session.drive()?;
    let mut stream = client
        .open_download_with(&info, IDLE_TIMEOUT)
        .map_err(Failure::from)?;
    if stream.start_offset() != 0 {
        return Err(Failure::Other(
            "下载服务器返回了意外的数据范围，无法预览".to_owned(),
        ));
    }
    if let Some(total) = stream
        .total_bytes()
        .filter(|total| *total > MAX_PREVIEW_BYTES)
    {
        return Err(Failure::Other(too_large_text(total)));
    }

    let path = fresh_path(name);
    if let Some(parent) = path.parent() {
        private_fs::create_private_dir(parent)
            .map_err(|err| Failure::Other(format!("创建图片临时目录失败：{err}")))?;
    }
    let mut file = private_fs::open_private(&path, OpenMode::CreateNew)
        .map_err(|err| Failure::Other(format!("创建图片临时文件失败：{err}")))?;

    let result = copy_stream(&mut stream, &mut file, started);
    drop(file);
    match result {
        Ok(()) => Ok(path),
        Err(err) => {
            let _ = fs::remove_file(&path);
            Err(err)
        }
    }
}

/// 删除一次预览的临时文件；不存在也算成功。
pub fn remove_file(path: &Path) {
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => eprintln!("[kichi] 清理图片预览临时文件失败：{err}"),
    }
}

/// 启动时清掉崩溃残留：只删超过 `max_age` 的文件，避免误删另一个实例正在用的。
pub fn sweep_stale(max_age: Duration) {
    sweep_in(&base_dir(), max_age);
}

fn sweep_in(base: &Path, max_age: Duration) {
    let Ok(entries) = fs::read_dir(base) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let stale = now
            .duration_since(modified)
            .map(|age| age > max_age)
            .unwrap_or(false);
        if stale {
            let _ = fs::remove_file(entry.path());
        }
    }
}

fn copy_stream(
    stream: &mut impl Read,
    file: &mut impl Write,
    started: Instant,
) -> Result<(), Failure> {
    let mut buffer = vec![0u8; READ_BUFFER];
    let mut copied: u64 = 0;
    loop {
        if started.elapsed() > TOTAL_TIMEOUT {
            return Err(Failure::Other(format!(
                "下载图片超时：{} 秒内没有完成",
                TOTAL_TIMEOUT.as_secs()
            )));
        }
        let read = stream
            .read(&mut buffer)
            .map_err(|err| Failure::Other(format!("下载图片失败：{err}")))?;
        if read == 0 {
            break;
        }
        copied += read as u64;
        if copied > MAX_PREVIEW_BYTES {
            return Err(Failure::Other(too_large_text(copied)));
        }
        file.write_all(&buffer[..read])
            .map_err(|err| Failure::Other(format!("写入图片临时文件失败：{err}")))?;
    }
    if copied == 0 {
        return Err(Failure::Other("这个文件没有可预览的图片内容".to_owned()));
    }
    Ok(())
}

fn too_large_text(size: u64) -> String {
    format!(
        "图片太大（{}），超过 {} 的预览上限",
        crate::format::size(size),
        crate::format::size(MAX_PREVIEW_BYTES)
    )
}

/// 每次预览一个独立文件名：同一秒内多次打开也不会互相覆盖。
fn fresh_path(name: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let millis = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0);
    let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
    let extension = extension_for(name);
    base_dir().join(format!(
        "preview-{}-{millis}-{sequence}.{extension}",
        std::process::id()
    ))
}

/// 只接受字母数字的短后缀；认不出就用 `img`（Qt 仍按内容识别格式）。
fn extension_for(name: &str) -> String {
    let extension = name
        .rsplit_once('.')
        .map(|(_, extension)| extension.trim().to_ascii_lowercase())
        .unwrap_or_default();
    if !extension.is_empty()
        && extension.len() <= 8
        && extension.bytes().all(|byte| byte.is_ascii_alphanumeric())
    {
        extension
    } else {
        "img".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_is_lowercased_and_sanitized() {
        assert_eq!(extension_for("photo.JPG"), "jpg");
        assert_eq!(extension_for("a.b.webp"), "webp");
        assert_eq!(extension_for("no-extension"), "img");
        assert_eq!(extension_for("bad.ex*tension"), "img");
        assert_eq!(extension_for("bad.averylongextension"), "img");
    }

    #[test]
    fn oversized_declared_size_is_rejected_before_download() {
        let text = too_large_text(MAX_PREVIEW_BYTES + 1);
        assert!(text.contains("超过"), "{text}");
        assert!(text.contains("预览上限"), "{text}");
    }

    #[test]
    fn sweep_removes_only_old_files() {
        let dir =
            std::env::temp_dir().join(format!("kichi-image-preview-sweep-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("建测试目录");
        let path = dir.join("preview-old.jpg");
        fs::write(&path, b"x").expect("写测试文件");
        // 0 时长把所有文件都算过期；用足够长的时长则一个都不删。
        sweep_in(&dir, Duration::ZERO);
        assert!(!path.exists());
        fs::write(&path, b"x").expect("重写测试文件");
        sweep_in(&dir, Duration::from_secs(3600));
        assert!(path.exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
