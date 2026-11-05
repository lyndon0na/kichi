//! 各类本地缓存（字幕 / 缩略图 / 头像）的公用件。
//!
//! 只抽三处完全一致的策略：环境变量覆盖、缓存根目录、0700 目录、0600 原子写入、
//! 按 mtime 年龄与总大小淘汰。目录遍历结构各不相同（字幕按文件 id 分二级目录，
//! 缩略图 / 头像平铺），仍留在各自模块里。
//!
//! 缓存出错只记日志、不影响主流程；这里的错误文案带 `label`，由调用方传入
//! 「字幕缓存」「缩略图缓存」「头像缓存」等称呼，保持原有措辞。

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use super::private_fs;

/// 缓存条目最多留多久（按文件 mtime）。
pub(crate) const MAX_AGE: Duration = Duration::from_secs(30 * 24 * 60 * 60);

/// 读取环境变量：首尾空白去掉，空串视为未设置。
pub(crate) fn read_env(name: &str) -> Option<String> {
    let value = std::env::var(name).ok()?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

/// 缓存根目录：`<override_var>` > `$XDG_CACHE_HOME/kichi/<subdir>` >
/// `$HOME/.cache/kichi/<subdir>` > 系统临时目录。
pub(crate) fn base_dir(override_var: &str, subdir: &str) -> PathBuf {
    if let Some(dir) = read_env(override_var) {
        return PathBuf::from(dir);
    }
    let home = read_env("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(|home| PathBuf::from(home).join(".cache"))
                .unwrap_or_else(std::env::temp_dir)
        });
    home.join("kichi").join(subdir)
}

/// 建缓存目录并收紧到 0700；两步分开报错，保留各缓存原有的提示。
pub(crate) fn create_cache_dir(dir: &Path, label: &str) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|err| format!("创建{label}目录失败：{err}"))?;
    private_fs::restrict_private_dir(dir).map_err(|err| format!("收紧{label}目录权限失败：{err}"))
}

/// 同目录临时文件 + rename 写入缓存；失败清理临时文件。
pub(crate) fn store_atomic(
    dir: &Path,
    target: &Path,
    bytes: &[u8],
    label: &str,
) -> Result<(), String> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
    let temp = dir.join(format!(".tmp-{}-{sequence}", std::process::id()));

    let mut file = match private_fs::open_private(&temp, private_fs::OpenMode::CreateNew) {
        Ok(file) => file,
        Err(err) => {
            let _ = fs::remove_file(&temp);
            return Err(format!("创建{label}文件失败：{err}"));
        }
    };
    if let Err(err) = file.write_all(bytes) {
        drop(file);
        let _ = fs::remove_file(&temp);
        return Err(format!("写{label}失败：{err}"));
    }
    drop(file);
    private_fs::replace_file(&temp, target).map_err(|err| format!("替换{label}失败：{err}"))
}

/// 文件是否早于 `older_than`；时钟回拨（mtime 在未来）不算过期。
pub(crate) fn is_stale(now: SystemTime, modified: SystemTime, older_than: Duration) -> bool {
    now.duration_since(modified)
        .map(|age| age > older_than)
        .unwrap_or(false)
}

/// 超过总大小上限时，从最旧的文件开始删，直到不超过 `max_bytes`。
pub(crate) fn evict_to_limit(files: &mut [(PathBuf, u64, SystemTime)], max_bytes: u64) {
    let mut total: u64 = files.iter().map(|(_, len, _)| *len).sum();
    if total <= max_bytes {
        return;
    }
    files.sort_by_key(|(_, _, modified)| *modified);
    for (path, len, _) in files.iter() {
        if total <= max_bytes {
            break;
        }
        if fs::remove_file(path).is_ok() {
            total = total.saturating_sub(*len);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "kichi-file-cache-{tag}-{}-{sequence}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("建测试目录");
        dir
    }

    #[test]
    fn read_env_trims_and_treats_empty_as_unset() {
        let name = format!("KICHI_TEST_FILE_CACHE_ENV_{}", std::process::id());
        std::env::set_var(&name, "  /tmp/kichi-cache-test  ");
        assert_eq!(read_env(&name).as_deref(), Some("/tmp/kichi-cache-test"));
        std::env::set_var(&name, "   ");
        assert!(read_env(&name).is_none(), "空白视为未设置");
        std::env::remove_var(&name);
    }

    #[test]
    fn base_dir_prefers_the_override_variable() {
        let name = format!("KICHI_TEST_FILE_CACHE_DIR_{}", std::process::id());
        std::env::set_var(&name, "/tmp/kichi-override");
        assert_eq!(
            base_dir(&name, "subtitles"),
            PathBuf::from("/tmp/kichi-override")
        );
        std::env::remove_var(&name);
    }

    #[test]
    fn store_atomic_writes_private_file_and_replaces_old_content() {
        let dir = temp_dir("store");
        let target = dir.join("thumb.jpg");
        store_atomic(&dir, &target, b"first", "测试缓存").expect("写缓存");
        assert_eq!(fs::read(&target).expect("读"), b"first".as_slice());
        store_atomic(&dir, &target, b"second", "测试缓存").expect("覆盖缓存");
        assert_eq!(fs::read(&target).expect("读"), b"second".as_slice());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&target).expect("读权限").permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn failed_store_cleans_up_the_temp_file() {
        let dir = temp_dir("store-fail");
        let target = dir.join("occupied");
        fs::create_dir(&target).expect("建同名目录让 rename 失败");
        assert!(store_atomic(&dir, &target, b"body", "测试缓存").is_err());
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .expect("读目录")
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().starts_with(".tmp-"))
            .collect();
        assert!(leftovers.is_empty(), "失败后不能留下临时文件");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn create_cache_dir_is_private() {
        let dir = temp_dir("dir");
        let nested = dir.join("cache");
        create_cache_dir(&nested, "测试缓存").expect("建缓存目录");
        assert!(nested.is_dir());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&nested).expect("读权限").permissions().mode() & 0o777;
            assert_eq!(mode, 0o700);
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn eviction_removes_files_until_under_the_cap() {
        let dir = temp_dir("evict");
        let first = dir.join("a");
        let second = dir.join("b");
        fs::write(&first, b"aaaa").expect("写 a");
        fs::write(&second, b"bbbb").expect("写 b");
        let mut files = vec![
            (first.clone(), 4, SystemTime::now()),
            (second.clone(), 4, SystemTime::now()),
        ];

        evict_to_limit(&mut files, 100);
        assert!(first.exists() && second.exists(), "未超限不删");

        evict_to_limit(&mut files, 0);
        assert!(!first.exists() && !second.exists(), "上限为零时全删");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_age_boundary_is_strict() {
        let now = SystemTime::now();
        assert!(is_stale(
            now,
            now - Duration::from_secs(40 * 24 * 60 * 60),
            MAX_AGE
        ));
        assert!(!is_stale(
            now,
            now - Duration::from_secs(29 * 24 * 60 * 60),
            MAX_AGE
        ));
        assert!(!is_stale(now - MAX_AGE, now, MAX_AGE), "时钟回拨不该误删");
    }
}
