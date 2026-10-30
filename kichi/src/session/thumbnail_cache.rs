//! 文件列表缩略图缓存：Rust 下载字节，QML 只读取本地文件。
//!
//! 位置：`$XDG_CACHE_HOME/kichi/thumbnails`（`KICHI_THUMBNAIL_CACHE` 可覆盖）。
//! 文件按云端文件 id 编码成安全文件名；写入走同目录临时文件 + rename。
//! 启动时删 30 天前条目，并把总大小收到 64 MiB 以内。

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

const MAX_AGE: Duration = Duration::from_secs(30 * 24 * 60 * 60);
const MAX_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_THUMBNAIL_BYTES: usize = 4 * 1024 * 1024;
const SUFFIXES: &[&str] = &[".jpg", ".png", ".webp", ".gif", ".bmp", ".img"];

pub fn base_dir() -> PathBuf {
    if let Some(dir) = read_env("KICHI_THUMBNAIL_CACHE") {
        return PathBuf::from(dir);
    }
    let home = read_env("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(|home| PathBuf::from(home).join(".cache"))
                .unwrap_or_else(std::env::temp_dir)
        });
    home.join("kichi").join("thumbnails")
}

fn read_env(name: &str) -> Option<String> {
    let value = std::env::var(name).ok()?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

/// 找到可用的本地缓存；空文件、超限文件都按未命中处理。
pub fn find(id: &str) -> Option<PathBuf> {
    find_in(&base_dir(), id)
}

pub fn store(id: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    store_in(&base_dir(), id, bytes)
}

fn find_in(base: &Path, id: &str) -> Option<PathBuf> {
    let stem = cache_stem(id)?;
    for suffix in SUFFIXES {
        let path = base.join(format!("{stem}{suffix}"));
        let Ok(metadata) = fs::metadata(&path) else {
            continue;
        };
        if metadata.is_file() && metadata.len() > 0 && metadata.len() <= MAX_THUMBNAIL_BYTES as u64
        {
            return Some(path);
        }
        let _ = fs::remove_file(path);
    }
    None
}

fn store_in(base: &Path, id: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    if bytes.is_empty() || bytes.len() > MAX_THUMBNAIL_BYTES {
        return Err("缩略图大小不在缓存范围内".to_owned());
    }
    create_private_dir(base)?;
    let stem = cache_stem(id).ok_or_else(|| "缩略图文件 id 无效".to_owned())?;
    for suffix in SUFFIXES {
        let _ = fs::remove_file(base.join(format!("{stem}{suffix}")));
    }

    let path = base.join(format!("{stem}{}", suffix_for(bytes)));
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
    let tmp = base.join(format!(".tmp-{}-{sequence}", std::process::id()));
    if let Err(reason) = write_private(&tmp, bytes) {
        let _ = fs::remove_file(&tmp);
        return Err(reason);
    }
    if let Err(err) = fs::rename(&tmp, &path) {
        let _ = fs::remove_file(&tmp);
        return Err(format!("替换缩略图缓存失败：{err}"));
    }
    Ok(path)
}

fn cache_stem(id: &str) -> Option<String> {
    if id.is_empty() || id.len() > 512 {
        return None;
    }
    let mut out = String::with_capacity(id.len() * 2);
    for byte in id.bytes() {
        out.push_str(&format!("{byte:02x}"));
    }
    Some(out)
}

fn suffix_for(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(b"\xff\xd8\xff") {
        ".jpg"
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        ".png"
    } else if bytes.starts_with(b"GIF8") {
        ".gif"
    } else if bytes.starts_with(b"BM") {
        ".bmp"
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        ".webp"
    } else {
        ".img"
    }
}

fn create_private_dir(dir: &Path) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|err| format!("创建缩略图缓存目录失败：{err}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
            .map_err(|err| format!("收紧缩略图缓存目录权限失败：{err}"))?;
    }
    Ok(())
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|err| format!("创建缩略图缓存文件失败：{err}"))?;
    file.write_all(bytes)
        .map_err(|err| format!("写缩略图缓存失败：{err}"))?;
    Ok(())
}

/// 启动时清一次缓存。
pub fn sweep() {
    sweep_in(&base_dir(), MAX_AGE, MAX_BYTES);
}

fn sweep_in(base: &Path, max_age: Duration, max_bytes: u64) {
    let Ok(entries) = fs::read_dir(base) else {
        return;
    };
    let now = SystemTime::now();
    let mut files: Vec<(PathBuf, u64, SystemTime)> = Vec::new();
    for entry in entries.flatten() {
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        if is_stale(now, modified, max_age) {
            let _ = fs::remove_file(entry.path());
            continue;
        }
        files.push((entry.path(), metadata.len(), modified));
    }

    let mut total: u64 = files.iter().map(|(_, len, _)| len).sum();
    if total > max_bytes {
        files.sort_by_key(|(_, _, modified)| *modified);
        for (path, len, _) in files {
            if total <= max_bytes {
                break;
            }
            if fs::remove_file(path).is_ok() {
                total = total.saturating_sub(len);
            }
        }
    }
}

fn is_stale(now: SystemTime, modified: SystemTime, older_than: Duration) -> bool {
    now.duration_since(modified)
        .map(|age| age > older_than)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    const JPEG: &[u8] = b"\xff\xd8\xff\xe0kichi";
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nkichi";

    fn temp_base(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("kichi-thumb-cache-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn store_then_find_round_trips_and_keeps_private_permissions() {
        let base = temp_base("roundtrip");
        let path = store_in(&base, "id/with/slash", JPEG).expect("写缓存");
        assert_eq!(find_in(&base, "id/with/slash"), Some(path.clone()));
        assert_eq!(fs::read(&path).expect("读缓存"), JPEG);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path)
                    .map(|meta| meta.permissions().mode() & 0o777)
                    .unwrap_or(0),
                0o600
            );
            assert_eq!(
                fs::metadata(&base)
                    .map(|meta| meta.permissions().mode() & 0o777)
                    .unwrap_or(0),
                0o700
            );
        }
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn rewrite_removes_the_old_extension() {
        let base = temp_base("rewrite");
        let old = store_in(&base, "id", JPEG).expect("写 JPEG");
        let new = store_in(&base, "id", PNG).expect("写 PNG");
        assert!(!old.exists());
        assert_eq!(find_in(&base, "id"), Some(new));
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn sweep_drops_old_files_and_honors_the_size_cap() {
        let base = temp_base("sweep");
        store_in(&base, "old", JPEG).expect("写旧图");
        sweep_in(&base, Duration::ZERO, u64::MAX);
        assert!(find_in(&base, "old").is_none());

        store_in(&base, "a", JPEG).expect("写 a");
        store_in(&base, "b", PNG).expect("写 b");
        // 容量上限至少能容纳一个现有文件，否则按设计会把两份都清掉。
        sweep_in(&base, MAX_AGE, PNG.len() as u64);
        let remaining = [find_in(&base, "a"), find_in(&base, "b")];
        assert_eq!(remaining.iter().flatten().count(), 1);
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn empty_or_oversized_bytes_are_rejected() {
        let base = temp_base("limits");
        assert!(store_in(&base, "empty", b"").is_err());
        assert!(store_in(&base, "large", &vec![1; MAX_THUMBNAIL_BYTES + 1]).is_err());
        let _ = fs::remove_dir_all(&base);
    }
}
