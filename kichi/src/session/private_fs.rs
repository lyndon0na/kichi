//! 只有本人可读写的文件与目录：统一 0600 / 0700 的实现。
//!
//! 设置、断点索引、传输历史与各类缓存都往用户目录写内部状态，
//! 同一台机器的其他用户不应该看到。权限代码只放在这里，避免各处口径分叉。
//!
//! 写入分两种语义：覆盖写（`.tmp` 文件被复用）与 `create_new`（缓存临时文件，
//! 已存在即失败）。两种都先按 0600 打开，再显式收紧一次权限，防止复用的旧文件
//! 带着宽松权限。非 Unix 平台只保留打开 / 写入语义，不处理权限位。

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

/// 打开方式：覆盖已有文件，或要求文件不存在。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OpenMode {
    /// 覆盖写（不存在则创建）
    Overwrite,
    /// 仅新建（已存在则失败）
    CreateNew,
}

/// 按 [`OpenMode`] 打开一个 0600 文件。
pub(crate) fn open_private(path: &Path, mode: OpenMode) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true);
    match mode {
        OpenMode::Overwrite => {
            options.create(true).truncate(true);
        }
        OpenMode::CreateNew => {
            options.create_new(true);
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    Ok(file)
}

/// 覆盖写一个 0600 私有文件。
pub(crate) fn write_private(path: &Path, body: &[u8]) -> io::Result<()> {
    let mut file = open_private(path, OpenMode::Overwrite)?;
    file.write_all(body)?;
    file.flush()
}

/// 创建并收紧一个 0700 私有目录（父目录按需创建）。
pub(crate) fn create_private_dir(dir: &Path) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    restrict_private_dir(dir)
}

/// 只收紧已有目录的权限到 0700，不创建目录。
pub(crate) fn restrict_private_dir(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(not(unix))]
    let _ = dir;
    Ok(())
}

/// 把同目录临时文件替换成目标文件；rename 失败时清理临时文件。
pub(crate) fn replace_file(temp: &Path, target: &Path) -> io::Result<()> {
    if let Err(err) = fs::rename(temp, target) {
        let _ = fs::remove_file(temp);
        return Err(err);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn temp_dir(tag: &str) -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "kichi-private-fs-{tag}-{}-{sequence}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("建测试目录");
        dir
    }

    #[cfg(unix)]
    fn mode_of(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path).expect("读权限").permissions().mode() & 0o777
    }

    #[test]
    fn overwrite_writes_private_file() {
        let dir = temp_dir("overwrite");
        let path = dir.join("state.json");
        write_private(&path, b"one").expect("写私有文件");
        assert_eq!(fs::read(&path).expect("读"), b"one".as_slice());
        write_private(&path, b"two").expect("覆盖写");
        assert_eq!(fs::read(&path).expect("读"), b"two".as_slice());
        #[cfg(unix)]
        assert_eq!(mode_of(&path), 0o600);
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn overwrite_tightens_an_existing_loose_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("tighten");
        let path = dir.join("old.tmp");
        fs::write(&path, b"old").expect("写旧文件");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("放宽权限");
        write_private(&path, b"new").expect("覆盖写");
        assert_eq!(mode_of(&path), 0o600);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn create_new_refuses_to_overwrite() {
        let dir = temp_dir("create-new");
        let path = dir.join("cache.tmp");
        let mut file = open_private(&path, OpenMode::CreateNew).expect("新建");
        file.write_all(b"first").expect("写");
        drop(file);
        assert!(
            open_private(&path, OpenMode::CreateNew).is_err(),
            "create_new 不能覆盖已有文件"
        );
        assert_eq!(fs::read(&path).expect("读"), b"first".as_slice());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn private_dir_is_created_with_0700() {
        let dir = temp_dir("dir");
        let nested = dir.join("a").join("b");
        create_private_dir(&nested).expect("建私有目录");
        assert!(nested.is_dir());
        #[cfg(unix)]
        assert_eq!(mode_of(&nested), 0o700);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn replace_file_swaps_contents_and_removes_temp() {
        let dir = temp_dir("replace");
        let temp = dir.join(".tmp");
        let target = dir.join("target");
        fs::write(&target, b"old").expect("写旧目标");
        write_private(&temp, b"new").expect("写临时文件");
        replace_file(&temp, &target).expect("替换");
        assert!(!temp.exists());
        assert_eq!(fs::read(&target).expect("读"), b"new".as_slice());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn failed_replace_cleans_up_temp() {
        let dir = temp_dir("replace-fail");
        let temp = dir.join(".tmp");
        let target = dir.join("target");
        fs::create_dir(&target).expect("建同名目录让 rename 失败");
        write_private(&temp, b"body").expect("写临时文件");
        assert!(replace_file(&temp, &target).is_err());
        assert!(!temp.exists(), "失败后不能留下临时文件");
        let _ = fs::remove_dir_all(&dir);
    }
}
