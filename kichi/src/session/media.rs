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
///
/// `cancel` 有值时，链接全空的 5 秒重试可被取消打断（播放提示条的「取消」走这里）；
/// 被取消返回「下载已取消」，调用方应据取消标志静默收尾，不显示这条错误。
pub fn play_options(
    session: &Session,
    file_id: &str,
    cancel: Option<&Cancel>,
) -> Result<Vec<PlayOption>, Failure> {
    let info = file_info(session, file_id, cancel)?;
    Ok(info.play_options())
}

/// 链接还没生成好时的等待：有取消信号就切成可打断的等待（播放提示条的
/// 「取消」靠它提前返回），没有就照旧睡满。
///
/// 抽出来是为了能单测「取消后不睡满 5 秒」而不必搭假服务器。
fn wait_before_retry(cancel: Option<&Cancel>) -> Result<(), Failure> {
    match cancel {
        Some(cancel) if !sleep_cancellable(cancel, RETRY_DELAY) => {
            Err(Failure::Other("下载已取消".to_owned()))
        }
        Some(_) => Ok(()),
        None => {
            std::thread::sleep(RETRY_DELAY);
            Ok(())
        }
    }
}

/// 同 [`play_options`]，但返回完整详情（需要链接过期时间等）。
///
/// `cancel` 有值时，5 秒重试等待可被取消打断；被取消就返回「下载已取消」，
/// 让调用方立刻收尾（下载线程与播放提示条共用这条路）。
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
    wait_before_retry(cancel)?;
    let client = session.drive()?;
    let retry = client.file_detail(file_id).map_err(Failure::from)?;
    if retry.all_links_empty {
        return Err(Failure::Other(
            "官方还没生成播放链接，请稍后再试".to_owned(),
        ));
    }
    Ok(retry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn cancelled_play_retry_does_not_wait_the_full_five_seconds() {
        let cancel = std::sync::Arc::new(Cancel::default());
        let flag = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(60));
            flag.cancel();
        });
        let started = Instant::now();
        let result = wait_before_retry(Some(&cancel));
        assert!(result.is_err(), "取消后要返回错误，交给调用方静默收尾");
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "播放路径的 5 秒等待也必须能被取消打断：{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn already_cancelled_returns_at_once() {
        let cancel = Cancel::default();
        cancel.cancel();
        let started = Instant::now();
        assert!(wait_before_retry(Some(&cancel)).is_err());
        assert!(started.elapsed() < Duration::from_millis(200));
    }
}
