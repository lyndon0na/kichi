//! 文件列表缩略图缓存：Rust 下载字节，QML 只读取本地文件。
//!
//! 位置：`$XDG_CACHE_HOME/kichi/thumbnails`（`KICHI_THUMBNAIL_CACHE` 可覆盖）。
//! 文件按云端文件 id 编码成安全文件名；写入走同目录临时文件 + rename。
//! 启动时删 30 天前条目，并把总大小收到 64 MiB 以内。

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use super::file_cache::{self, MAX_AGE};

/// 缓存目录名与错误文案里的称呼。
const LABEL: &str = "缩略图缓存";
const MAX_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_THUMBNAIL_BYTES: usize = 4 * 1024 * 1024;
const SUFFIXES: &[&str] = &[".jpg", ".png", ".webp", ".gif", ".bmp", ".img"];

pub fn base_dir() -> PathBuf {
    file_cache::base_dir("KICHI_THUMBNAIL_CACHE", "thumbnails")
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
    file_cache::create_cache_dir(base, LABEL)?;
    let stem = cache_stem(id).ok_or_else(|| "缩略图文件 id 无效".to_owned())?;
    for suffix in SUFFIXES {
        let _ = fs::remove_file(base.join(format!("{stem}{suffix}")));
    }

    let path = base.join(format!("{stem}{}", suffix_for(bytes)));
    file_cache::store_atomic(base, &path, bytes, LABEL)?;
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
        if file_cache::is_stale(now, modified, max_age) {
            let _ = fs::remove_file(entry.path());
            continue;
        }
        files.push((entry.path(), metadata.len(), modified));
    }

    file_cache::evict_to_limit(&mut files, max_bytes);
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
