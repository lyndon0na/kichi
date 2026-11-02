//! 本地文件夹扫描：后台遍历、可取消、不跟随符号链接。
//!
//! 扫描只产出相对路径和普通文件大小，不碰 Qt。完成后回投主线程，由
//! `uploads.rs` 把结果展开成传输记录并交给上传调度器。

use std::collections::HashMap;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use super::transfers::TransferJob;
use super::upload_queue::{MAX_NAME_BYTES, MAX_PATH_BYTES};
use crate::session::download::Cancel;

const MAX_SCAN_FILES: usize = 100_000;
const MAX_SCAN_DIRS: usize = 20_000;
const MAX_SCAN_ENTRIES: usize = MAX_SCAN_FILES + MAX_SCAN_DIRS;
const MAX_SCAN_RELATIVE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_SCAN_ABSOLUTE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_SCAN_DEPTH: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScannedFile {
    pub(crate) path: PathBuf,
    /// 相对目标目录的父级路径，包含选中的根文件夹名。
    pub(crate) parent_path: Vec<String>,
    pub(crate) name: String,
    pub(crate) size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScanResult {
    pub(crate) root_name: String,
    pub(crate) files: Vec<ScannedFile>,
    /// 相对目标目录的目录树，包含根文件夹和所有空目录。
    pub(crate) dirs: Vec<Vec<String>>,
    /// 符号链接、特殊文件、非 UTF-8 名称和读失败的子目录数量。
    pub(crate) skipped: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ScanOutcome {
    Done(ScanResult),
    Cancelled,
    Failed(String),
}

struct ScanBudget {
    files: usize,
    dirs: usize,
    entries: usize,
    relative_bytes: u64,
    absolute_bytes: u64,
}

impl ScanBudget {
    fn new(root_name: &str, root: &Path) -> Result<Self, String> {
        check_name(root_name)?;
        check_absolute_path(root)?;
        let root_bytes = u64::try_from(root.as_os_str().as_bytes().len()).unwrap_or(u64::MAX);
        Ok(Self {
            files: 0,
            dirs: 1,
            entries: 0,
            relative_bytes: u64::try_from(root_name.len()).unwrap_or(u64::MAX),
            absolute_bytes: root_bytes,
        })
    }

    fn observe_entry(&mut self) -> Result<(), String> {
        if self.entries >= MAX_SCAN_ENTRIES {
            return Err(format!(
                "文件夹里的条目太多（最多 {MAX_SCAN_ENTRIES} 个），已取消整批上传"
            ));
        }
        self.entries = self.entries.saturating_add(1);
        Ok(())
    }

    fn reserve_dir(&mut self, path: &[String], absolute: &Path) -> Result<(), String> {
        check_depth(path.len())?;
        check_absolute_path(absolute)?;
        if self.dirs >= MAX_SCAN_DIRS {
            return Err(format!(
                "文件夹目录太多（最多 {MAX_SCAN_DIRS} 个），已取消整批上传"
            ));
        }
        self.check_relative(path)?;
        self.check_absolute_total(absolute)?;
        self.dirs = self.dirs.saturating_add(1);
        self.relative_bytes = self
            .relative_bytes
            .saturating_add(relative_path_bytes(path));
        self.absolute_bytes = self.absolute_bytes.saturating_add(
            u64::try_from(absolute.as_os_str().as_bytes().len()).unwrap_or(u64::MAX),
        );
        Ok(())
    }

    fn add_file(&mut self, parent: &[String], name: &str, path: &Path) -> Result<(), String> {
        check_name(name)?;
        check_depth(parent.len().saturating_add(1))?;
        check_absolute_path(path)?;
        if self.files >= MAX_SCAN_FILES {
            return Err(format!(
                "文件夹里的文件太多（最多 {MAX_SCAN_FILES} 个），已取消整批上传"
            ));
        }
        let mut full = parent.to_vec();
        full.push(name.to_owned());
        self.check_relative(&full)?;
        self.check_absolute_total(path)?;
        self.files = self.files.saturating_add(1);
        self.relative_bytes = self
            .relative_bytes
            .saturating_add(relative_path_bytes(&full));
        self.absolute_bytes = self
            .absolute_bytes
            .saturating_add(u64::try_from(path.as_os_str().as_bytes().len()).unwrap_or(u64::MAX));
        Ok(())
    }

    fn check_relative(&self, path: &[String]) -> Result<(), String> {
        if self
            .relative_bytes
            .saturating_add(relative_path_bytes(path))
            > MAX_SCAN_RELATIVE_BYTES
        {
            return Err(format!(
                "文件夹路径总量超过 {} MiB，已取消整批上传",
                MAX_SCAN_RELATIVE_BYTES / (1024 * 1024)
            ));
        }
        Ok(())
    }

    fn check_absolute_total(&self, path: &Path) -> Result<(), String> {
        let bytes = u64::try_from(path.as_os_str().as_bytes().len()).unwrap_or(u64::MAX);
        if self.absolute_bytes.saturating_add(bytes) > MAX_SCAN_ABSOLUTE_BYTES {
            return Err(format!(
                "本地路径总量超过 {} MiB，已取消整批上传",
                MAX_SCAN_ABSOLUTE_BYTES / (1024 * 1024)
            ));
        }
        Ok(())
    }
}

fn check_name(name: &str) -> Result<(), String> {
    if name.len() > MAX_NAME_BYTES {
        return Err(format!(
            "文件或文件夹名太长（最多 {MAX_NAME_BYTES} 字节）：{name}"
        ));
    }
    Ok(())
}

fn check_depth(depth: usize) -> Result<(), String> {
    if depth > MAX_SCAN_DEPTH {
        return Err(format!(
            "文件夹层级太深（最多 {MAX_SCAN_DEPTH} 层），已取消整批上传"
        ));
    }
    Ok(())
}

fn check_absolute_path(path: &Path) -> Result<(), String> {
    if path.as_os_str().as_bytes().len() > MAX_PATH_BYTES {
        return Err(format!(
            "本地路径太长（最多 {MAX_PATH_BYTES} 字节），已取消整批上传：{}",
            path.display()
        ));
    }
    Ok(())
}

fn relative_path_bytes(path: &[String]) -> u64 {
    let bytes: usize = path.iter().map(String::len).sum();
    u64::try_from(bytes.saturating_add(path.len().saturating_sub(1))).unwrap_or(u64::MAX)
}

struct ScanEntry {
    source_id: String,
    cancel: Arc<Cancel>,
    abandoned: bool,
}

fn scans() -> &'static Mutex<HashMap<u64, ScanEntry>> {
    static SCANS: OnceLock<Mutex<HashMap<u64, ScanEntry>>> = OnceLock::new();
    SCANS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn with_scans<T>(f: impl FnOnce(&mut HashMap<u64, ScanEntry>) -> T) -> T {
    let mut guard = scans()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    f(&mut guard)
}

fn epoch() -> &'static AtomicU64 {
    static EPOCH: OnceLock<AtomicU64> = OnceLock::new();
    EPOCH.get_or_init(|| AtomicU64::new(0))
}

fn current_epoch() -> u64 {
    epoch().load(Ordering::SeqCst)
}

pub(crate) fn is_taken(source_id: &str) -> bool {
    with_scans(|scans| {
        scans
            .values()
            .any(|entry| !entry.abandoned && entry.source_id == source_id)
    })
}

/// 启动一个后台扫描。调用方负责先确认同一路径没有在扫描 / 排队。
pub(crate) fn start(batch: u64, parent: TransferJob) -> Result<(), String> {
    let root = parent
        .upload
        .as_ref()
        .map(|upload| upload.local_path.clone())
        .ok_or_else(|| "上传文件夹缺少本地路径".to_owned())?;
    let source_id = parent.id.clone();
    let cancel = Arc::new(Cancel::default());
    let thread_cancel = Arc::clone(&cancel);
    let my_epoch = current_epoch();
    with_scans(|scans| {
        scans.insert(
            batch,
            ScanEntry {
                source_id,
                cancel: Arc::clone(&cancel),
                abandoned: false,
            },
        );
    });

    let spawned = std::thread::Builder::new()
        .name("kichi-upload-scan".into())
        .spawn(move || {
            let outcome = scan_folder(&root, &thread_cancel);
            let current = take_if_current(batch, my_epoch);
            if !current {
                return;
            }
            if my_epoch != current_epoch() {
                return;
            }
            super::actions::enqueue_ui(
                move |mut qichi: std::pin::Pin<&mut super::qobject::Kichi>| {
                    qichi.as_mut().on_upload_folder_scanned(batch, outcome);
                },
            );
        });
    if let Err(err) = spawned {
        with_scans(|scans| {
            scans.remove(&batch);
        });
        return Err(format!("无法创建文件夹扫描线程：{err}"));
    }
    Ok(())
}

fn take_if_current(batch: u64, my_epoch: u64) -> bool {
    if my_epoch != current_epoch() {
        return false;
    }
    with_scans(|scans| scans.remove(&batch).is_some())
}

pub(crate) fn cancel(batch: u64) -> bool {
    with_scans(|scans| {
        let Some(entry) = scans.get(&batch) else {
            return false;
        };
        entry.cancel.cancel();
        true
    })
}

pub(crate) fn cancel_all() {
    with_scans(|scans| {
        for entry in scans.values() {
            entry.cancel.cancel();
        }
    });
}

pub(crate) fn reset() {
    epoch().fetch_add(1, Ordering::SeqCst);
    with_scans(|scans| {
        for entry in scans.values_mut() {
            entry.cancel.cancel();
            entry.cancel.abandon();
            entry.abandoned = true;
        }
        scans.clear();
    });
}

fn scan_folder(root: &Path, cancel: &Cancel) -> ScanOutcome {
    let metadata = match std::fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(err) => return ScanOutcome::Failed(format!("无法读取本地文件夹：{err}")),
    };
    if metadata.file_type().is_symlink() {
        return ScanOutcome::Failed("不跟随符号链接，请选择真实文件夹".to_owned());
    }
    if !metadata.is_dir() {
        return ScanOutcome::Failed("选择的不是文件夹".to_owned());
    }
    let Some(root_name) = root.file_name().and_then(|name| name.to_str()) else {
        return ScanOutcome::Failed("文件夹名不是有效的 UTF-8".to_owned());
    };
    if root_name.trim().is_empty() {
        return ScanOutcome::Failed("文件夹名不能为空".to_owned());
    }
    if let Err(reason) = check_absolute_path(root) {
        return ScanOutcome::Failed(reason);
    }
    let mut budget = match ScanBudget::new(root_name, root) {
        Ok(budget) => budget,
        Err(reason) => return ScanOutcome::Failed(reason),
    };

    let mut result = ScanResult {
        root_name: root_name.to_owned(),
        files: Vec::new(),
        dirs: vec![vec![root_name.to_owned()]],
        skipped: 0,
    };
    let mut stack = vec![(root.to_path_buf(), vec![root_name.to_owned()], true)];
    while let Some((dir, relative, is_root)) = stack.pop() {
        if cancel.is_cancelled() {
            return ScanOutcome::Cancelled;
        }
        // 子目录从发现到真正读取之间可能被替换成符号链接；这里按 lstat
        // 语义再确认一次，绝不顺着链接读出去。
        let metadata = match std::fs::symlink_metadata(&dir) {
            Ok(metadata) => metadata,
            Err(_) if !is_root => {
                result.skipped += 1;
                continue;
            }
            Err(err) => return ScanOutcome::Failed(format!("无法读取本地文件夹：{err}")),
        };
        if metadata.file_type().is_symlink() {
            if is_root {
                return ScanOutcome::Failed("不跟随符号链接，请选择真实文件夹".to_owned());
            }
            result.skipped += 1;
            continue;
        }
        if !metadata.is_dir() {
            if is_root {
                return ScanOutcome::Failed("选择的不是文件夹".to_owned());
            }
            result.skipped += 1;
            continue;
        }
        let dir_entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) if !is_root => {
                result.skipped += 1;
                continue;
            }
            Err(err) => return ScanOutcome::Failed(format!("无法读取本地文件夹：{err}")),
        };
        // 目录成功打开后才记入上传树；预算在发现时就已经占用，避免一个超大
        // 目录先把栈撑爆才在读取阶段报错。
        if !is_root {
            result.dirs.push(relative.clone());
        }
        let mut entries = Vec::new();
        for entry in dir_entries {
            if let Err(reason) = budget.observe_entry() {
                return ScanOutcome::Failed(reason);
            }
            match entry {
                Ok(entry) => entries.push(entry),
                Err(_) => result.skipped += 1,
            }
        }
        entries.sort_by_cached_key(|entry| entry.file_name());
        let mut child_dirs = Vec::new();
        for entry in entries {
            if cancel.is_cancelled() {
                return ScanOutcome::Cancelled;
            }
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                result.skipped += 1;
                continue;
            };
            if let Err(reason) = check_name(&name) {
                return ScanOutcome::Failed(reason);
            }
            if let Err(reason) = check_absolute_path(&entry.path()) {
                return ScanOutcome::Failed(reason);
            }
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(_) => {
                    result.skipped += 1;
                    continue;
                }
            };
            if file_type.is_symlink() || (!file_type.is_file() && !file_type.is_dir()) {
                result.skipped += 1;
                continue;
            }
            if file_type.is_dir() {
                let mut child = relative.clone();
                child.push(name);
                if let Err(reason) = budget.reserve_dir(&child, &entry.path()) {
                    return ScanOutcome::Failed(reason);
                }
                child_dirs.push((entry.path(), child, false));
                continue;
            }
            // 不用 `entry.metadata()`：目录项被替换成符号链接时它会跟随。
            // 扫描承诺不跟随符号链接，这里再以 lstat 语义确认一次。
            let metadata = match std::fs::symlink_metadata(entry.path()) {
                Ok(metadata) => metadata,
                Err(_) => {
                    result.skipped += 1;
                    continue;
                }
            };
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                result.skipped += 1;
                continue;
            }
            if let Err(reason) = budget.add_file(&relative, &name, &entry.path()) {
                return ScanOutcome::Failed(reason);
            }
            result.files.push(ScannedFile {
                path: entry.path(),
                parent_path: relative.clone(),
                name,
                size: metadata.len(),
            });
        }
        // 反着压栈，保持目录名升序遍历；文件最终再统一按相对路径排序。
        for child in child_dirs.into_iter().rev() {
            stack.push(child);
        }
    }
    if cancel.is_cancelled() {
        return ScanOutcome::Cancelled;
    }
    result.files.sort_by(|a, b| {
        a.parent_path
            .cmp(&b.parent_path)
            .then_with(|| a.name.cmp(&b.name))
    });
    result
        .dirs
        .sort_by(|a, b| a.len().cmp(&b.len()).then_with(|| a.cmp(b)));
    ScanOutcome::Done(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{symlink, FileTypeExt};

    fn temp_dir(name: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("kichi-upload-scan-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("建测试目录");
        path
    }

    #[test]
    fn scan_budget_rejects_size_path_and_depth_edges() {
        assert!(check_name(&"a".repeat(MAX_NAME_BYTES)).is_ok());
        assert!(check_name(&"a".repeat(MAX_NAME_BYTES + 1)).is_err());
        assert!(check_depth(MAX_SCAN_DEPTH).is_ok());
        assert!(check_depth(MAX_SCAN_DEPTH + 1).is_err());

        let long_path = PathBuf::from("/").join("a".repeat(MAX_PATH_BYTES));
        assert!(check_absolute_path(&long_path).is_err());

        let mut budget = ScanBudget::new("root", Path::new("/tmp/root")).expect("短根名合法");
        budget.relative_bytes.clone_from(&MAX_SCAN_RELATIVE_BYTES);
        assert!(budget.check_relative(&["x".to_owned()]).is_err());

        let mut files = ScanBudget::new("root", Path::new("/tmp/root")).expect("短根名合法");
        files.files = MAX_SCAN_FILES;
        assert!(files
            .add_file(&[], "a.txt", Path::new("/tmp/a.txt"))
            .is_err());

        let mut dirs = ScanBudget::new("root", Path::new("/tmp/root")).expect("短根名合法");
        dirs.dirs = MAX_SCAN_DIRS;
        assert!(dirs
            .reserve_dir(
                &["root".to_owned(), "child".to_owned()],
                Path::new("/tmp/root/child")
            )
            .is_err());

        let mut entries = ScanBudget::new("root", Path::new("/tmp/root")).expect("短根名合法");
        entries.entries = MAX_SCAN_ENTRIES;
        assert!(entries.observe_entry().is_err());

        let mut absolute = ScanBudget::new("root", Path::new("/tmp/root")).expect("短根名合法");
        absolute.absolute_bytes = MAX_SCAN_ABSOLUTE_BYTES;
        assert!(absolute
            .add_file(&[], "a.txt", Path::new("/tmp/a.txt"))
            .is_err());
    }

    #[test]
    fn scan_keeps_empty_dirs_and_skips_symlinks_and_specials() {
        let root = temp_dir("tree");
        std::fs::create_dir_all(root.join("空目录/更深")).expect("建空目录");
        std::fs::write(root.join("a.txt"), b"hello").expect("写文件");
        std::fs::create_dir(root.join("sub")).expect("建子目录");
        std::fs::write(root.join("sub/b.txt"), b"world").expect("写文件");
        symlink(root.join("a.txt"), root.join("link.txt")).expect("建符号链接");
        symlink(root.join("sub"), root.join("link-dir")).expect("建目录符号链接");

        let outcome = scan_folder(&root, &Cancel::default());
        let ScanOutcome::Done(result) = outcome else {
            panic!("扫描应成功：{outcome:?}");
        };
        let root_name = root
            .file_name()
            .and_then(|name| name.to_str())
            .expect("测试根目录名")
            .to_owned();
        assert!(result
            .dirs
            .contains(&vec![root_name.clone(), "空目录".to_owned()]));
        assert!(result.dirs.contains(&vec![
            root_name.clone(),
            "空目录".to_owned(),
            "更深".to_owned()
        ]));
        assert!(result.files.iter().any(|file| file.parent_path
            == vec![root_name.clone(), "sub".to_owned()]
            && file.name == "b.txt"));
        assert_eq!(result.skipped, 2, "符号链接文件和目录都不跟随");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn root_symlink_is_rejected() {
        let root = temp_dir("root-link");
        let target = root.join("target");
        std::fs::create_dir(&target).expect("建目标目录");
        let link = root.join("link");
        symlink(&target, &link).expect("建符号链接");
        match scan_folder(&link, &Cancel::default()) {
            ScanOutcome::Failed(reason) => assert!(reason.contains("符号链接")),
            other => panic!("根目录符号链接应拒绝：{other:?}"),
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn cancelled_scan_stops_without_publishing_a_partial_tree() {
        let root = temp_dir("cancel");
        std::fs::write(root.join("a.txt"), b"x").expect("写文件");
        let cancel = Cancel::default();
        cancel.cancel();
        assert_eq!(scan_folder(&root, &cancel), ScanOutcome::Cancelled);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn special_file_is_skipped() {
        let root = temp_dir("special");
        let fifo = root.join("pipe");
        let status = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .expect("运行 mkfifo");
        if !status.success() {
            return;
        }
        let meta = std::fs::symlink_metadata(&fifo).expect("读 fifo");
        assert!(meta.file_type().is_fifo());
        let ScanOutcome::Done(result) = scan_folder(&root, &Cancel::default()) else {
            panic!("扫描应成功");
        };
        assert_eq!(result.skipped, 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn reset_stops_an_inflight_scan() {
        let batch = 9_900_001;
        let cancel = Arc::new(Cancel::default());
        with_scans(|scans| {
            scans.insert(
                batch,
                ScanEntry {
                    source_id: "upload-folder:/tmp/reset".to_owned(),
                    cancel: Arc::clone(&cancel),
                    abandoned: false,
                },
            );
        });
        reset();
        assert!(cancel.is_cancelled());
        assert!(cancel.is_abandoned());
        assert!(!is_taken("upload-folder:/tmp/reset"));
    }
}
