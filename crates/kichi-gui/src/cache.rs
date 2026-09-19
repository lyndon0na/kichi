//! 磁盘缓存淘汰: `~/.cache/kichi` 下的预览缓存与缩略图缓存共用一套按 mtime 的 LRU。
//!
//! 与内存里的目录缓存(`app::DIR_CACHE_CAP` 那套)口径一致: 有上限, 超限按
//! 「最久未用」淘汰。差别是这里只能靠文件 mtime 表达「最近使用」(Linux 默认
//! relatime, atime 不可信), 因此命中缓存时要显式调用 [`touch`]。
//!
//! **上限是软上限**: 处于豁免窗口内、正被使用、或自身就超过上限的条目不删;
//! 且淘汰只扫一轮即止 —— 淘汰绝不能把缓存删到「下次一定又重下」的死循环里。

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// 预览缓存上限: 512 MiB / 200 条。
///
/// 非媒体预览是「整份下载」, 条目单价高、复用率低, 条目数不必给太多。
pub const PREVIEW_CAPS: Caps = Caps {
    max_bytes: 512 << 20,
    max_entries: 200,
    min_age: Duration::from_secs(300),
};

/// 缩略图缓存上限: 128 MiB / 1000 条(实测均值约 150 KB/张, 约合 870 张)。
pub const THUMB_CAPS: Caps = Caps {
    max_bytes: 128 << 20,
    max_entries: 1000,
    min_age: Duration::from_secs(300),
};

/// 淘汰口径。
#[derive(Clone, Copy, Debug)]
pub struct Caps {
    /// 缓存目录整体字节上限。
    pub max_bytes: u64,
    /// 条目数上限(预览按 `<file_id>/` 目录计一条, 缩略图按文件计一条)。
    pub max_entries: usize,
    /// 豁免窗口: 最近这么久内改动过的条目不删(正在写入 / 刚交给查看器的文件)。
    pub min_age: Duration,
}

/// 一次扫描的结果。
#[derive(Clone, Copy, Debug, Default)]
pub struct Sweep {
    /// 扫描后的占用。
    pub bytes: u64,
    pub entries: usize,
    /// 本次删除的条目数与释放的字节数。
    pub removed: usize,
    pub freed: u64,
}

/// 一级缓存条目。
struct Entry {
    path: PathBuf,
    bytes: u64,
    used: SystemTime,
    is_dir: bool,
}

/// 扫描 `root` 下的一级条目, 超限时按 mtime 从最旧开始删, 直到不再超限。
///
/// `in_use` 里的路径(以及包含它们的目录条目)一律跳过; 豁免窗口内的条目也跳过。
/// 只扫一轮, 不做多轮反复扫描, 因此**超限是允许保留的状态**。
pub fn evict_lru(root: &Path, caps: &Caps, in_use: &HashSet<PathBuf>) -> Sweep {
    let mut entries = scan(root);
    let mut bytes: u64 = entries.iter().map(|e| e.bytes).sum();
    let mut removed = 0usize;
    let mut freed = 0u64;
    let now = SystemTime::now();
    // 最久未用的排在前面, 先淘汰它们。
    entries.sort_by_key(|e| e.used);

    for e in &entries {
        if bytes <= caps.max_bytes && entries.len() - removed <= caps.max_entries {
            break;
        }
        if blocked(&e.path, e.is_dir, in_use) {
            continue;
        }
        if now.duration_since(e.used).unwrap_or_default() < caps.min_age {
            continue;
        }
        if remove_entry(e) {
            bytes = bytes.saturating_sub(e.bytes);
            removed += 1;
            freed += e.bytes;
        }
    }

    Sweep {
        bytes,
        entries: entries.len() - removed,
        removed,
        freed,
    }
}

/// 清空缓存(用户在设置页显式操作)。
///
/// 与 [`evict_lru`] 的区别: 不受上限与豁免窗口限制, 除 `in_use` 外全删。
pub fn purge(root: &Path, in_use: &HashSet<PathBuf>) -> Sweep {
    let entries = scan(root);
    let mut bytes: u64 = entries.iter().map(|e| e.bytes).sum();
    let mut removed = 0usize;
    let mut freed = 0u64;

    for e in &entries {
        if blocked(&e.path, e.is_dir, in_use) {
            continue;
        }
        if remove_entry(e) {
            bytes = bytes.saturating_sub(e.bytes);
            removed += 1;
            freed += e.bytes;
        }
    }

    Sweep {
        bytes,
        entries: entries.len() - removed,
        removed,
        freed,
    }
}

/// 命中缓存时把 mtime 推到当前时间, 作为 LRU 的「最近使用」。
///
/// 尽力而为: 打不开(已删除 / 无权限)就忽略。目录条目不用单独 touch ——
/// 目录的「最近使用」取的是子树里最新的 mtime(见 [`scan`])。
pub fn touch(path: &Path) {
    let Ok(f) = std::fs::File::open(path) else {
        return;
    };
    let _ = f.set_modified(SystemTime::now());
}

/// 该条目是否被「正在使用」挡下: 路径本身在集合里, 或(目录条目)含有集合中的路径。
fn blocked(path: &Path, is_dir: bool, in_use: &HashSet<PathBuf>) -> bool {
    if in_use.contains(path) {
        return true;
    }
    is_dir && in_use.iter().any(|p| p.starts_with(path))
}

fn remove_entry(e: &Entry) -> bool {
    let r = if e.is_dir {
        std::fs::remove_dir_all(&e.path)
    } else {
        std::fs::remove_file(&e.path)
    };
    match r {
        Ok(()) => true,
        Err(err) => {
            // 尽力而为: 删不掉(权限 / 仍被占用)就留着, 下次再试。
            tracing::debug!("缓存条目删除失败 {}: {err}", e.path.display());
            false
        }
    }
}

/// 一级条目: 路径、占用字节、最近使用时间(mtime)。
///
/// 目录条目(预览缓存是「每个文件一个目录」)的最近使用时间取**子树里最新的
/// mtime** —— 目录自身的 mtime 只在增删文件时变, 命中缓存时我们 touch 的是
/// 目录里的文件, 只看目录 mtime 会把「刚看过」的预览误判成最旧的。
fn scan(root: &Path) -> Vec<Entry> {
    let Ok(rd) = std::fs::read_dir(root) else {
        // 目录不存在(首次运行 / 被 tmpfiles 清掉)视为空缓存。
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in rd.flatten() {
        let Ok(md) = e.metadata() else { continue };
        let path = e.path();
        let is_dir = md.is_dir();
        let mut used = md.modified().ok();
        let bytes = if is_dir {
            let (bytes, newest) = dir_stat(&path);
            if newest > used {
                used = newest;
            }
            bytes
        } else {
            md.len()
        };
        out.push(Entry {
            path,
            bytes,
            used: used.unwrap_or(SystemTime::UNIX_EPOCH),
            is_dir,
        });
    }
    out
}

/// 目录的 (递归占用, 子树里最新的 mtime)。
fn dir_stat(dir: &Path) -> (u64, Option<SystemTime>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return (0, None);
    };
    let mut bytes = 0u64;
    let mut newest: Option<SystemTime> = None;
    for e in rd.flatten() {
        let Ok(md) = e.metadata() else { continue };
        let mut at = md.modified().ok();
        if md.is_dir() {
            let (sub_bytes, sub_at) = dir_stat(&e.path());
            bytes += sub_bytes;
            if sub_at > at {
                at = sub_at;
            }
        } else {
            bytes += md.len();
        }
        if at > newest {
            newest = at;
        }
    }
    (bytes, newest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kichi-cache-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("建临时目录");
        dir
    }

    /// 把某个条目的 mtime 推到 `age_secs` 秒之前(目录条目也可以用)。
    fn set_age(path: &Path, age_secs: u64) {
        // 只读句柄也能改时间戳, 目录同样适用。
        let f = fs::File::open(path).expect("打开缓存条目");
        f.set_modified(SystemTime::now() - Duration::from_secs(age_secs))
            .expect("设置 mtime");
    }

    /// 建一个指定大小、指定「年龄」的缓存条目(年龄越大越先被淘汰)。
    fn write_at(path: &Path, bytes: usize, age_secs: u64) {
        if let Some(p) = path.parent() {
            let _ = fs::create_dir_all(p);
        }
        fs::write(path, vec![b'x'; bytes]).expect("写缓存文件");
        set_age(path, age_secs);
    }

    fn caps(max_bytes: u64, max_entries: usize, min_age_secs: u64) -> Caps {
        Caps {
            max_bytes,
            max_entries,
            min_age: Duration::from_secs(min_age_secs),
        }
    }

    #[test]
    fn evicts_oldest_until_under_byte_cap() {
        let root = tmp_root("bytes");
        write_at(&root.join("a"), 10, 300);
        write_at(&root.join("b"), 10, 200);
        write_at(&root.join("c"), 10, 100);

        let s = evict_lru(&root, &caps(25, 100, 0), &HashSet::new());

        assert!(!root.join("a").exists(), "最旧的应被淘汰");
        assert!(root.join("b").exists() && root.join("c").exists());
        assert_eq!(s.removed, 1);
        assert_eq!(s.freed, 10);
        assert_eq!(s.bytes, 20);
        assert_eq!(s.entries, 2);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn evicts_until_under_entry_cap() {
        let root = tmp_root("entries");
        write_at(&root.join("a"), 1, 300);
        write_at(&root.join("b"), 1, 200);
        write_at(&root.join("c"), 1, 100);

        let s = evict_lru(&root, &caps(1 << 20, 2, 0), &HashSet::new());

        assert!(!root.join("a").exists());
        assert_eq!(s.entries, 2);
        assert_eq!(s.removed, 1);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn dir_entry_uses_newest_child_mtime() {
        let root = tmp_root("dirmtime");
        // 预览缓存每个文件一个目录: 目录里刚用过的文件应让它「变新」,
        // 即便目录自身的 mtime 很旧。
        let dir = root.join("fileid");
        write_at(&dir.join("just-previewed.bin"), 10, 0);
        set_age(&dir, 3600);
        write_at(&root.join("stale"), 10, 1800);

        let s = evict_lru(&root, &caps(10, 100, 0), &HashSet::new());

        assert!(!root.join("stale").exists(), "该淘汰目录 mtime 更旧的那个");
        assert!(
            dir.join("just-previewed.bin").exists(),
            "目录内有刚用过的文件, 整体应算较新"
        );
        assert_eq!(s.removed, 1);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn fresh_entries_are_exempt() {
        let root = tmp_root("fresh");
        write_at(&root.join("just-written"), 100, 0);

        // 上限远小于单个条目, 但它在豁免窗口内: 不删, 超限保留。
        let s = evict_lru(&root, &caps(10, 0, 300), &HashSet::new());

        assert!(root.join("just-written").exists(), "豁免窗口内不删");
        assert_eq!(s.removed, 0);
        assert_eq!(s.entries, 1);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn in_use_path_and_its_parent_dir_are_exempt() {
        let root = tmp_root("inuse");
        let dir = root.join("fileid");
        write_at(&dir.join("movie.zip"), 100, 600);

        // 条目是 `fileid/` 目录, 集合里只有目录内的文件: 也要跳过。
        let mut in_use = HashSet::new();
        in_use.insert(dir.join("movie.zip"));
        let s = evict_lru(&root, &caps(0, 0, 0), &in_use);
        assert!(dir.join("movie.zip").exists());
        assert_eq!(s.removed, 0);

        // 注销后即可正常淘汰。
        let s = evict_lru(&root, &caps(0, 0, 0), &HashSet::new());
        assert_eq!(s.removed, 1);
        assert!(!dir.exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn purge_clears_all_but_in_use() {
        let root = tmp_root("purge");
        write_at(&root.join("a"), 5, 1);
        write_at(&root.join("b"), 5, 1);

        let mut in_use = HashSet::new();
        in_use.insert(root.join("a"));
        let s = purge(&root, &in_use);

        assert!(root.join("a").exists(), "使用中的条目不清");
        assert!(!root.join("b").exists());
        assert_eq!(s.removed, 1);
        assert_eq!(s.entries, 1);
        assert_eq!(s.freed, 5);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_root_is_treated_as_empty() {
        let s = evict_lru(
            Path::new("/nonexistent/kichi-cache-test"),
            &PREVIEW_CAPS,
            &HashSet::new(),
        );
        assert_eq!(s.bytes, 0);
        assert_eq!(s.entries, 0);
        assert_eq!(s.removed, 0);
    }

    #[test]
    fn touch_refreshes_used_time() {
        let root = tmp_root("touch");
        let f = root.join("thumb.jpg");
        write_at(&f, 4, 600);

        touch(&f);

        let used = fs::metadata(&f).unwrap().modified().unwrap();
        let age = SystemTime::now().duration_since(used).unwrap_or_default();
        assert!(age < Duration::from_secs(5), "mtime 应被推到当前时间");
        let _ = fs::remove_dir_all(&root);
    }
}
