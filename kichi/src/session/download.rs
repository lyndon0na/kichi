//! 下载到本地：取链接 → 流式写盘。
//!
//! 职责边界（守「会话状态只放 session」的纪律）：
//! - 本模块只做「一个文件的一次下载」，不排队、不弹通知、不碰界面；
//! - 队列、进度汇总、通知属于界面侧调度，放 `cxxqt_object::queue`；
//! - 全部函数阻塞，调用方负责丢到后台线程。
//!
//! 落盘约定见 [`download_naming`]：先写 `<名>.kichi-part`，**下载完整成功后才
//! rename** 成正式文件名；中途取消/断网只删掉临时文件，绝不留下半个文件冒充成品。

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::download_naming::claim_part;
// 文件名策略在 `download_naming`（那里有单测），这里转出去给队列层用。
pub use super::download_naming::safe_file_name;
use super::media::file_info;
use super::{Failure, Session};
use crate::format::size as readable_size;

/// 每次读的块大小（64 KiB：够大够快，进度回调也不至于太密）。
const CHUNK: usize = 64 * 1024;
/// 可取消等待的检查间隔（最坏多等这么久就发现取消）。
const CANCEL_POLL: Duration = Duration::from_millis(50);

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
    /// 是否已请求取消。
    pub fn is_cancelled(&self) -> bool {
        self.flag.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// 请求取消（任意线程可调）。
    pub fn cancel(&self) {
        self.flag.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// 可被取消打断的等待：等到 `total` 或期间被取消。`false` = 被取消。
///
/// 按 50 ms 切片查标志，取消最坏多等一个切片；不用条件变量是因为
/// `Cancel` 只是个原子标志，调用方（界面线程）不该为此持有锁。
pub fn sleep_cancellable(cancel: &Cancel, total: Duration) -> bool {
    let started = Instant::now();
    loop {
        if cancel.is_cancelled() {
            return false;
        }
        let remaining = total.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return true;
        }
        std::thread::sleep(remaining.min(CANCEL_POLL));
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
        let info = file_info(self, file_id, Some(cancel))?;
        let name = safe_file_name(if name_hint.trim().is_empty() {
            &info.name
        } else {
            name_hint
        });
        // 取消后不再建连接：这一步可能白等响应头
        if cancel.is_cancelled() {
            return Err(Failure::Other("下载已取消".to_owned()));
        }
        let stream = client.open_download(&info).map_err(Failure::from)?;

        std::fs::create_dir_all(target_dir)
            .map_err(|err| Failure::Other(format!("创建下载目录失败：{err}")))?;
        // 选名 + 原子占位一步做完：两个任务同时选中同名时，只有一个能抢到
        let (mut file, part_path, final_path) = claim_part(target_dir, &name)?;

        let mut source = stream;
        let total = source.total_bytes();

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
                    // 卡在读超时的那几十秒里按的取消仍然算「已取消」，
                    // 不能因为网络先返回超时就被记成失败
                    if cancel.is_cancelled() {
                        return Err(Failure::Other("下载已取消".to_owned()));
                    }
                    if err.kind() == io::ErrorKind::TimedOut {
                        return Err(Failure::Other(format!(
                            "下载超时：{err}（已下载 {}）",
                            readable_size(written)
                        )));
                    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_is_shared_across_threads() {
        let cancel = std::sync::Arc::new(Cancel::default());
        let clone = cancel.clone();
        let handle = std::thread::spawn(move || clone.cancel());
        handle.join().expect("线程正常结束");
        assert!(cancel.is_cancelled());
    }

    #[test]
    fn cancellable_sleep_returns_early_when_cancelled() {
        let cancel = std::sync::Arc::new(Cancel::default());
        let clone = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(60));
            clone.cancel();
        });
        let started = Instant::now();
        assert!(
            !sleep_cancellable(&cancel, Duration::from_secs(5)),
            "期间被取消要返回 false"
        );
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "不能等满 5 秒：{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn cancellable_sleep_finishes_normally_without_cancel() {
        let cancel = Cancel::default();
        let started = Instant::now();
        assert!(sleep_cancellable(&cancel, Duration::from_millis(80)));
        assert!(started.elapsed() >= Duration::from_millis(80));
    }
}
