//! 播放的会话门面：取播放档位。
//!
//! 官方偶发在文件详情里下发空链接（rclone 遇到会睡 5 秒重试），
//! 这里照做一次。延时只发生在调用方给的后台线程里，不阻塞界面。
//! 全部函数阻塞，调用方负责丢到后台线程。

use std::time::Duration;

use kichi_core::{FileInfo, PlayOption};

use super::download::{sleep_cancellable, Cancel};
use super::{Failure, Session};

/// 链接全空时的重试间隔（rclone 同为 5 秒）。
const RETRY_DELAY: Duration = Duration::from_secs(5);

/// 取一个文件的可播放档位。阻塞。
pub fn play_options(session: &Session, file_id: &str) -> Result<Vec<PlayOption>, Failure> {
    // 播放路径没有取消入口：保持原来的「一定等满 5 秒」
    let info = file_info(session, file_id, None)?;
    Ok(info.play_options())
}

/// 同 [`play_options`]，但返回完整详情（需要链接过期时间等）。
///
/// `cancel` 有值时，5 秒重试等待可被取消打断；被取消就返回「下载已取消」，
/// 让下载线程立刻收尾（播放路径传 `None`，行为与以前一致）。
pub fn file_info(
    session: &Session,
    file_id: &str,
    cancel: Option<&Cancel>,
) -> Result<FileInfo, Failure> {
    let client = session.drive()?;
    let first = client.file_detail(file_id).map_err(Failure::from)?;
    if !first.all_links_empty {
        return Ok(first);
    }
    // 链接还没生成好，等一会儿再问一次；仍为空就如实报错，不给假档位。
    if let Some(cancel) = cancel {
        if !sleep_cancellable(cancel, RETRY_DELAY) {
            return Err(Failure::Other("下载已取消".to_owned()));
        }
    } else {
        std::thread::sleep(RETRY_DELAY);
    }
    let client = session.drive()?;
    let retry = client.file_detail(file_id).map_err(Failure::from)?;
    if retry.all_links_empty {
        return Err(Failure::Other(
            "官方还没生成播放链接，请稍后再试".to_owned(),
        ));
    }
    Ok(retry)
}
