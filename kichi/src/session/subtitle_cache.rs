//! 外挂字幕的持久缓存：下载过的字幕留在本地，再播同一集不再联网。
//!
//! 位置：`$XDG_CACHE_HOME/kichi/subtitles/<文件 id>/<净化后的原名>`
//! （`KICHI_SUBTITLE_CACHE` 可覆盖；目录 0700、文件 0600）。
//!
//! 缓存是加速用的：**任何一步失败都退回联网下载**，不影响播放。
//! 写入走「同目录临时文件 + rename」，并发下载或 mpv 不会读到半截文件。
//! 清理只在启动时做一次：删 30 天前的，再按总大小 100 MiB 从旧到新删。
//!
//! 为什么按「文件 id + 名字」：云端文件被替换时 id 会变，旧的缓存自然作废；
//! 同名不同目录的文件 id 不同，不会互相串。字幕被原地改名也会换一个缓存名。

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use super::file_cache::{self, MAX_AGE};
use super::subtitles::MAX_SUBTITLE_BYTES;

/// 缓存目录名与错误文案里的称呼。
const LABEL: &str = "字幕缓存";
/// 缓存总大小上限；超过就从最旧的开始删。
const MAX_BYTES: u64 = 100 * 1024 * 1024;
/// 净化后的文件名超过这个**字节数**就不缓存（Linux 单个文件名上限 255 字节）。
const MAX_NAME_BYTES: usize = 180;

/// 缓存根目录（`KICHI_SUBTITLE_CACHE` > `$XDG_CACHE_HOME/kichi/subtitles`）。
pub fn base_dir() -> PathBuf {
    file_cache::base_dir("KICHI_SUBTITLE_CACHE", "subtitles")
}

/// 读缓存；没有 / 太大 / 读失败都当未命中。
pub fn load(id: &str, name: &str) -> Option<Vec<u8>> {
    load_from(&base_dir(), id, name)
}

/// 写缓存；失败只返回原因，调用方只记日志，不影响播放。
pub fn store(id: &str, name: &str, bytes: &[u8]) -> Result<(), String> {
    store_in(&base_dir(), id, name, bytes)
}

fn load_from(base: &Path, id: &str, name: &str) -> Option<Vec<u8>> {
    let path = cache_path(base, id, name)?;
    let metadata = fs::metadata(&path).ok()?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_SUBTITLE_BYTES {
        return None;
    }
    fs::read(&path).ok()
}

fn store_in(base: &Path, id: &str, name: &str, bytes: &[u8]) -> Result<(), String> {
    if bytes.is_empty() || bytes.len() as u64 > MAX_SUBTITLE_BYTES {
        return Err("字幕大小不在缓存范围内".to_owned());
    }
    let Some(path) = cache_path(base, id, name) else {
        return Err("字幕文件名过长，不缓存".to_owned());
    };
    let dir = path
        .parent()
        .ok_or_else(|| "缓存路径没有父目录".to_owned())?;
    file_cache::create_cache_dir(dir, LABEL)?;
    file_cache::store_atomic(dir, &path, bytes, LABEL)
}

/// 缓存路径；名字太长返回 `None`（这条字幕不缓存）。
fn cache_path(base: &Path, id: &str, name: &str) -> Option<PathBuf> {
    let name = safe_component(name);
    if name.len() > MAX_NAME_BYTES {
        return None;
    }
    Some(base.join(safe_component(id)).join(name))
}

/// 把云端名字净化成单层文件名：保留字母 / 数字（含中文）、`.` `_` `-`，其余变 `_`；
/// 去掉开头的点（避免变成隐藏文件或 `..`），空的用 `_`。
fn safe_component(value: &str) -> String {
    let mut cleaned = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_alphanumeric() || matches!(ch, '.' | '_' | '-') {
            cleaned.push(ch);
        } else {
            cleaned.push('_');
        }
    }
    let trimmed = cleaned.trim_start_matches('.');
    if trimmed.is_empty() {
        "_".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// 启动时清一次缓存：先删超过 [`MAX_AGE`] 的，再按总大小收到 [`MAX_BYTES`] 以内。
///
/// 只动本模块自己写出的文件；读目录失败就当没有缓存（不值得打扰用户）。
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
        if !metadata.is_dir() {
            continue;
        }
        let Ok(inner) = fs::read_dir(entry.path()) else {
            continue;
        };
        for file in inner.flatten() {
            let Ok(meta) = file.metadata() else {
                continue;
            };
            if !meta.is_file() {
                continue;
            }
            let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            if file_cache::is_stale(now, modified, max_age) {
                let _ = fs::remove_file(file.path());
                continue;
            }
            files.push((file.path(), meta.len(), modified));
        }
    }
    file_cache::evict_to_limit(&mut files, max_bytes);
    // 空掉的 id 目录顺手收掉（非空 remove_dir 会失败，忽略即可）
    if let Ok(entries) = fs::read_dir(base) {
        for entry in entries.flatten() {
            let _ = fs::remove_dir(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn temp_base(tag: &str) -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "kichi-subcache-{tag}-{}-{sequence}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn store_then_load_round_trips() {
        let base = temp_base("roundtrip");
        assert!(load_from(&base, "id1", "剧集.zh.ass").is_none());
        store_in(&base, "id1", "剧集.zh.ass", b"ass-bytes").expect("写缓存");
        assert_eq!(
            load_from(&base, "id1", "剧集.zh.ass").as_deref(),
            Some(&b"ass-bytes"[..])
        );
        // 不同 id / 不同名字互不影响
        assert!(load_from(&base, "id2", "剧集.zh.ass").is_none());
        assert!(load_from(&base, "id1", "另一集.zh.ass").is_none());
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn cache_files_are_private_and_nested_per_id() {
        let base = temp_base("perms");
        store_in(&base, "id9", "a/b:c.ass", b"x").expect("写缓存");
        let path = cache_path(&base, "id9", "a/b:c.ass").expect("有缓存路径");
        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some("a_b_c.ass"),
            "路径分隔符要净化"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let file_mode = fs::metadata(&path)
                .map(|meta| meta.permissions().mode() & 0o777)
                .unwrap_or(0);
            assert_eq!(file_mode, 0o600);
            let dir_mode = path
                .parent()
                .and_then(|dir| fs::metadata(dir).ok())
                .map(|meta| meta.permissions().mode() & 0o777)
                .unwrap_or(0);
            assert_eq!(dir_mode, 0o700);
        }
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn oversized_names_are_not_cached() {
        let base = temp_base("longname");
        let long = format!("{}.ass", "字".repeat(200));
        assert!(cache_path(&base, "id", &long).is_none());
        assert!(store_in(&base, "id", &long, b"x").is_err());
    }

    #[test]
    fn an_empty_or_oversized_cache_file_is_a_miss() {
        let base = temp_base("badfile");
        let path = cache_path(&base, "id", "x.ass").expect("有路径");
        file_cache::create_cache_dir(path.parent().expect("有父目录"), LABEL).expect("建目录");
        fs::write(&path, b"").expect("写空文件");
        assert!(load_from(&base, "id", "x.ass").is_none(), "空文件不命中");
        fs::write(&path, vec![7u8; MAX_SUBTITLE_BYTES as usize + 1]).expect("写超大文件");
        assert!(load_from(&base, "id", "x.ass").is_none(), "超限不命中");
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn sweep_drops_stale_files() {
        let base = temp_base("sweep-age");
        store_in(&base, "id", "a.ass", b"a").expect("写缓存");
        // max_age = 0：任何早于「现在」的文件都算过期
        sweep_in(&base, Duration::ZERO, u64::MAX);
        assert!(load_from(&base, "id", "a.ass").is_none(), "过期文件要被删");
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn sweep_honours_the_size_cap() {
        let base = temp_base("sweep-size");
        store_in(&base, "a", "a.ass", b"aaaa").expect("写第一个");
        store_in(&base, "b", "b.ass", b"bbbb").expect("写第二个");
        sweep_in(&base, MAX_AGE, 4);
        let sizes = [
            load_from(&base, "a", "a.ass").map(|bytes| bytes.len()),
            load_from(&base, "b", "b.ass").map(|bytes| bytes.len()),
        ];
        let total: usize = sizes.iter().flatten().sum();
        assert!(total <= 4, "超过总上限要从旧的删：{sizes:?}");
        assert_eq!(sizes.iter().flatten().count(), 1, "只留一份：{sizes:?}");
        let _ = fs::remove_dir_all(&base);
    }
}
