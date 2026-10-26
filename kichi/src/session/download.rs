//! 下载到本地：取链接 → 流式写盘。
//!
//! 职责边界（守「会话状态只放 session」的纪律）：
//! - 本模块只做「一个文件的一次下载」，不排队、不弹通知、不碰界面；
//! - 队列、进度汇总、通知属于界面侧调度，放 `cxxqt_object::downloads`；
//! - 全部函数阻塞，调用方负责丢到后台线程。
//!
//! 落盘约定：
//! - 先写 `<名>.kichi-part`，**下载完整成功后才 rename** 成正式文件名；
//!   中途取消/断网只删掉临时文件，绝不留下半个文件冒充成品；
//! - 同名冲突自动改名 `name (1).ext`，**绝不覆盖**已有文件
//!   （安全纪律：不破坏用户本地数据）。

use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use super::media::file_info;
use super::{Failure, Session};

/// 每次读的块大小（64 KiB：够大够快，进度回调也不至于太密）。
const CHUNK: usize = 64 * 1024;
/// 临时文件后缀。
const PART_SUFFIX: &str = ".kichi-part";

/// 一次下载的结果。
#[derive(Debug, Clone)]
pub struct Downloaded {
    /// 最终落盘路径（已改名，不含临时后缀）
    pub path: PathBuf,
    pub bytes: u64,
}

/// 下载进度回调（在下载线程里调用，调用方自己保证线程安全）。
pub type Progress<'a> = dyn Fn(u64, Option<u64>) + Send + 'a;

/// 取消信号：下载线程每块检查一次。
#[derive(Debug, Default)]
pub struct Cancel {
    flag: std::sync::atomic::AtomicBool,
}

impl Cancel {
    fn is_cancelled(&self) -> bool {
        self.flag.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// 请求取消（任意线程可调）。
    pub fn cancel(&self) {
        self.flag.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

impl Session {
    /// 下载一个文件到 `target_dir`（目录不存在会自动创建）。阻塞。
    ///
    /// `name_hint` 是列表里的文件名；官方详情里也可能给名字，优先用列表的
    /// （列表是用户眼前看到的那个）。`progress` 每读一块调一次。
    pub fn download_file(
        &self,
        file_id: &str,
        name_hint: &str,
        target_dir: &Path,
        progress: &Progress<'_>,
        cancel: &Cancel,
    ) -> Result<Downloaded, Failure> {
        let client = self.drive()?;
        let info = file_info(self, file_id)?;
        let name = safe_file_name(if name_hint.trim().is_empty() {
            &info.name
        } else {
            name_hint
        });
        let stream = client.open_download(&info).map_err(Failure::from)?;

        std::fs::create_dir_all(target_dir)
            .map_err(|err| Failure::Other(format!("创建下载目录失败：{err}")))?;
        let final_path = unique_path(target_dir, &name);
        let part_path = part_path_for(&final_path);

        let mut source = stream;
        let total = source.total_bytes();
        let mut file = File::create(&part_path)
            .map_err(|err| Failure::Other(format!("无法创建下载文件：{err}")))?;

        let mut written: u64 = 0;
        let mut buffer = vec![0u8; CHUNK];
        progress(0, total);
        loop {
            if cancel.is_cancelled() {
                drop(file);
                let _ = std::fs::remove_file(&part_path);
                return Err(Failure::Other("下载已取消".to_owned()));
            }
            let read = match source.read(&mut buffer) {
                Ok(read) => read,
                Err(err) => {
                    drop(file);
                    let _ = std::fs::remove_file(&part_path);
                    return Err(Failure::Other(format!("下载中断：{err}")));
                }
            };
            if read == 0 {
                break;
            }
            if let Err(err) = file.write_all(&buffer[..read]) {
                drop(file);
                let _ = std::fs::remove_file(&part_path);
                return Err(Failure::Other(format!("写入本地文件失败：{err}")));
            }
            written += read as u64;
            progress(written, total);
        }
        // flush + sync 都成功才算落盘，再改名
        if let Err(err) = file.flush().and_then(|()| file.sync_all()) {
            drop(file);
            let _ = std::fs::remove_file(&part_path);
            return Err(Failure::Other(format!("写入本地文件失败：{err}")));
        }
        drop(file);

        if let Some(total) = total {
            if written != total {
                let _ = std::fs::remove_file(&part_path);
                return Err(Failure::Other(format!(
                    "下载不完整：收到 {written} / {total} 字节"
                )));
            }
        }
        std::fs::rename(&part_path, &final_path).map_err(|err| {
            let _ = std::fs::remove_file(&part_path);
            Failure::Other(format!("保存下载文件失败：{err}"))
        })?;
        Ok(Downloaded {
            path: final_path,
            bytes: written,
        })
    }
}

/// 临时文件名：`<正式名>.kichi-part`。
fn part_path_for(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(PART_SUFFIX);
    path.with_file_name(name)
}

/// 不覆盖已有文件：`a.mkv` 已存在 → `a (1).mkv`，依此类推。
///
/// 临时文件也算「已存在」：另一个任务正在下同一个名字时，不能两个线程写同一份。
pub fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !exists(&candidate) {
        return candidate;
    }
    let (stem, ext) = split_ext(name);
    for index in 1..10_000 {
        let candidate = dir.join(format!("{stem} ({index}){ext}"));
        if !exists(&candidate) {
            return candidate;
        }
    }
    // 一万个同名文件还撞上：加时间戳兜底，总比覆盖强
    dir.join(format!("{stem}-{}{ext}", std::process::id()))
}

fn exists(path: &Path) -> bool {
    path.exists() || part_path_for(path).exists()
}

/// 拆成（不含扩展名的主体，含点的扩展名）。
fn split_ext(name: &str) -> (String, String) {
    match name.rsplit_once('.') {
        // 以点开头的隐藏文件（`.bashrc`）没有扩展名
        Some((stem, ext)) if !stem.is_empty() => (stem.to_owned(), format!(".{ext}")),
        _ => (name.to_owned(), String::new()),
    }
}

/// 把官方文件名收敛成安全的本地文件名。
///
/// 官方允许 `/`、`\` 等字符出现在名字里（在它那边是合法文件名），
/// 落到本地可能被当成路径分隔符写到别的目录去——一律替换掉。
pub fn safe_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | '\0' => '_',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    // 去掉末尾的点（Windows/部分挂载点会吃掉它，留着容易变成隐藏或空名）
    let trimmed = cleaned.trim().trim_end_matches('.').to_owned();
    if trimmed.is_empty() || trimmed.chars().all(|c| c == '.') {
        "未命名文件".to_owned()
    } else {
        // 以点开头会被当成隐藏文件；官方名字很少这样，加个下划线前缀保住可见性
        if trimmed.starts_with('.') {
            format!("_{trimmed}")
        } else {
            trimmed
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kichi-dl-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建临时目录");
        dir
    }

    #[test]
    fn same_name_never_overwrites() {
        let dir = temp_dir("unique");
        let first = unique_path(&dir, "ep1.mkv");
        assert_eq!(first, dir.join("ep1.mkv"));
        std::fs::write(&first, b"x").expect("写文件");
        assert_eq!(unique_path(&dir, "ep1.mkv"), dir.join("ep1 (1).mkv"));
        std::fs::write(dir.join("ep1 (1).mkv"), b"x").expect("写文件");
        assert_eq!(unique_path(&dir, "ep1.mkv"), dir.join("ep1 (2).mkv"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pending_part_file_counts_as_taken() {
        // 另一个任务正在下同一个名字：不能选同一个路径
        let dir = temp_dir("part");
        std::fs::write(part_path_for(&dir.join("a.bin")), b"x").expect("写临时文件");
        assert_eq!(unique_path(&dir, "a.bin"), dir.join("a (1).bin"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn extension_split_handles_hidden_and_dotted() {
        assert_eq!(split_ext("a.mkv"), ("a".into(), ".mkv".into()));
        assert_eq!(
            split_ext("archive.tar.gz"),
            ("archive.tar".into(), ".gz".into())
        );
        assert_eq!(split_ext(".bashrc"), (".bashrc".into(), "".into()));
        assert_eq!(split_ext("noext"), ("noext".into(), "".into()));
    }

    #[test]
    fn file_names_cannot_escape_the_target_dir() {
        assert_eq!(
            safe_file_name("../../etc/passwd"),
            "_.._.._etc_passwd",
            "路径分隔符必须替掉，开头点加下划线保可见"
        );
        assert_eq!(safe_file_name("a\\b"), "a_b");
        assert_eq!(safe_file_name("   "), "未命名文件");
        assert_eq!(safe_file_name("正常 名字.mkv"), "正常 名字.mkv");
        assert_eq!(safe_file_name(".."), "未命名文件", "纯点会变空名");
        assert_eq!(safe_file_name("truncated."), "truncated");
    }

    #[test]
    fn cancel_is_shared_across_threads() {
        let cancel = std::sync::Arc::new(Cancel::default());
        let clone = cancel.clone();
        let handle = std::thread::spawn(move || clone.cancel());
        handle.join().expect("线程正常结束");
        assert!(cancel.is_cancelled());
    }
}
