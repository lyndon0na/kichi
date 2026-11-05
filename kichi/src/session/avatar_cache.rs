//! 账号头像缓存：Rust 下载，QML 只读本地文件。
//!
//! 位置：`$XDG_CACHE_HOME/kichi/avatars`（`KICHI_AVATAR_CACHE` 可覆盖）。
//! 文件名由账号 id 与头像 URL 的稳定指纹组成，因此换账号或换头像不会命中旧图。
//! 写入走同目录临时文件 + rename；启动时删 30 天前条目，并把总量收到 8 MiB。

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use super::file_cache::{self, MAX_AGE};

/// 缓存目录名与错误文案里的称呼。
const LABEL: &str = "头像缓存";
const MAX_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_AVATAR_BYTES: usize = 2 * 1024 * 1024;
const SUFFIXES: &[&str] = &[".jpg", ".png", ".webp", ".gif", ".bmp", ".img"];

pub fn base_dir() -> PathBuf {
    file_cache::base_dir("KICHI_AVATAR_CACHE", "avatars")
}

/// 找到这个账号与头像地址对应的本地缓存。
pub fn find(user_id: &str, url: &str) -> Option<PathBuf> {
    find_in(&base_dir(), user_id, url)
}

pub fn store(user_id: &str, url: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    store_in(&base_dir(), user_id, url, bytes)
}

/// 官方确认账号没有头像时，删掉这个账号的全部旧缓存。
pub fn remove_for_user(user_id: &str) -> Result<(), String> {
    remove_for_user_in(&base_dir(), user_id)
}

fn find_in(base: &Path, user_id: &str, url: &str) -> Option<PathBuf> {
    let stem = cache_stem(user_id, url)?;
    for suffix in SUFFIXES {
        let path = base.join(format!("{stem}{suffix}"));
        let Ok(metadata) = fs::metadata(&path) else {
            continue;
        };
        if metadata.is_file() && metadata.len() > 0 && metadata.len() <= MAX_AVATAR_BYTES as u64 {
            return Some(path);
        }
        let _ = fs::remove_file(path);
    }
    None
}

fn store_in(base: &Path, user_id: &str, url: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    if bytes.is_empty() || bytes.len() > MAX_AVATAR_BYTES {
        return Err("头像大小不在缓存范围内".to_owned());
    }
    file_cache::create_cache_dir(base, LABEL)?;
    let stem = cache_stem(user_id, url).ok_or_else(|| "头像账号 id 或地址无效".to_owned())?;
    for suffix in SUFFIXES {
        let _ = fs::remove_file(base.join(format!("{stem}{suffix}")));
    }

    let path = base.join(format!("{stem}{}", suffix_for(bytes)));
    file_cache::store_atomic(base, &path, bytes, LABEL)?;
    Ok(path)
}

fn remove_for_user_in(base: &Path, user_id: &str) -> Result<(), String> {
    let prefix = user_prefix(user_id).ok_or_else(|| "头像账号 id 无效".to_owned())?;
    let entries = match fs::read_dir(base) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(format!("读取头像缓存目录失败：{err}")),
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with(&prefix) {
            continue;
        }
        match fs::remove_file(entry.path()) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(format!("删除旧头像缓存失败：{err}")),
        }
    }
    Ok(())
}

fn cache_stem(user_id: &str, url: &str) -> Option<String> {
    let user = user_prefix(user_id)?;
    let url = url.trim();
    if url.is_empty() || url.len() > 4096 {
        return None;
    }
    Some(format!("{user}-{:016x}", fnv1a64(url.as_bytes())))
}

fn user_prefix(user_id: &str) -> Option<String> {
    if user_id.is_empty() || user_id.len() > 512 {
        return None;
    }
    let mut out = String::with_capacity(user_id.len() * 2 + 1);
    for byte in user_id.bytes() {
        out.push_str(&format!("{byte:02x}"));
    }
    out.push('-');
    Some(out)
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
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

/// 启动时清一次头像缓存。
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

    const JPEG: &[u8] = b"\xff\xd8\xff\xe0kichi-avatar";
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nkichi-avatar";

    fn temp_base(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("kichi-avatar-cache-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn store_then_find_round_trips_and_keeps_private_permissions() {
        let base = temp_base("roundtrip");
        let url = "https://user.mypikpak.com/avatar/1";
        let path = store_in(&base, "user/with/slash", url, JPEG).expect("写头像缓存应该成功");
        assert_eq!(find_in(&base, "user/with/slash", url), Some(path.clone()));
        assert_eq!(fs::read(&path).expect("读头像缓存"), JPEG);
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
    fn changed_avatar_url_does_not_reuse_the_old_picture() {
        let base = temp_base("changed");
        let first = store_in(&base, "user-1", "https://user.mypikpak.com/avatar/1", JPEG)
            .expect("写旧头像");
        let second =
            store_in(&base, "user-1", "https://user.mypikpak.com/avatar/2", PNG).expect("写新头像");
        assert_ne!(first, second);
        assert_eq!(
            find_in(&base, "user-1", "https://user.mypikpak.com/avatar/1"),
            Some(first)
        );
        assert_eq!(
            find_in(&base, "user-1", "https://user.mypikpak.com/avatar/2"),
            Some(second)
        );
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn removing_an_account_drops_all_of_its_avatar_versions() {
        let base = temp_base("remove");
        store_in(&base, "user-1", "https://user.mypikpak.com/avatar/1", JPEG).expect("写旧头像");
        store_in(&base, "user-1", "https://user.mypikpak.com/avatar/2", PNG).expect("写新头像");
        store_in(&base, "user-2", "https://user.mypikpak.com/avatar/1", JPEG)
            .expect("写别人的头像");
        remove_for_user_in(&base, "user-1").expect("删除账号头像");
        assert_eq!(
            find_in(&base, "user-1", "https://user.mypikpak.com/avatar/1"),
            None
        );
        assert_eq!(
            find_in(&base, "user-1", "https://user.mypikpak.com/avatar/2"),
            None
        );
        assert!(
            find_in(&base, "user-2", "https://user.mypikpak.com/avatar/1").is_some(),
            "不能误删其他账号的缓存"
        );
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn empty_or_oversized_bytes_are_rejected() {
        let base = temp_base("limits");
        let url = "https://user.mypikpak.com/avatar/1";
        assert!(store_in(&base, "u", url, b"").is_err());
        assert!(store_in(&base, "u", url, &vec![1; MAX_AVATAR_BYTES + 1]).is_err());
        assert!(find_in(&base, "", url).is_none());
        assert!(find_in(&base, "u", "").is_none());
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn sweep_drops_old_files_and_honors_the_size_cap() {
        let base = temp_base("sweep");
        let url = "https://user.mypikpak.com/avatar/1";
        store_in(&base, "old", url, JPEG).expect("写旧图");
        sweep_in(&base, Duration::ZERO, u64::MAX);
        assert!(find_in(&base, "old", url).is_none());

        store_in(&base, "a", url, JPEG).expect("写 a");
        store_in(&base, "b", url, JPEG).expect("写 b");
        sweep_in(&base, MAX_AGE, JPEG.len() as u64);
        let remaining = [find_in(&base, "a", url), find_in(&base, "b", url)];
        assert_eq!(remaining.iter().flatten().count(), 1);
        let _ = fs::remove_dir_all(&base);
    }
}
