//! 播放与下载的会话门面：取文件详情（含「链接还没生成」的重试）。
//!
//! 官方偶发在文件详情里下发空链接（rclone 遇到会睡 5 秒重试），这里照做一次。
//! 但有些情况重试注定无用：详情明确 `size=0`（空文件），或审核标记异常
//! （违规 / 受限）。这类直接返回，不再白等 5 秒。
//! 延时只发生在调用方给的后台线程里，不阻塞界面。全部函数阻塞。

use std::time::Duration;

use kichi_core::{FileInfo, NoLinkReason, PlayOption};

use super::download::{sleep_cancellable, Cancel};
use super::{Failure, Session};

/// 链接全空且原因不明时的重试间隔（rclone 同为 5 秒）。
const RETRY_DELAY: Duration = Duration::from_secs(5);

/// 官方明确不下发链接（违规 / 受限），重试无用。
pub const RESTRICTED_NO_LINK: &str = "官方没有提供这个文件的链接（可能是违规或受限文件）";
/// 链接还在生成：两次都没拿到。
pub const LINK_NOT_READY: &str = "官方还没生成这个文件的链接，请稍后再试";
/// 0 字节文件没有可播放的内容。
pub const ZERO_BYTE_PLAY: &str = "这个文件是 0 字节，没有可播放的内容";

/// 一次详情请求的链接状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LinkState {
    /// 有可用链接
    Ready,
    /// 官方明确说是空文件
    ZeroByte,
    /// 官方审核异常，不会给链接
    Restricted,
    /// 原因不明：等一会儿再问一次
    Waiting,
}

/// 判定这次详情属于哪种情况。
///
/// `list_audit_flagged` 是列表里的旧标记（列表请求带 `with_audit=true`）：
/// 详情明确 `STATUS_OK` 时以详情为准，详情没说才拿列表标记当线索。
fn link_state(info: &FileInfo, list_audit_flagged: bool) -> LinkState {
    if !info.all_links_empty() {
        return LinkState::Ready;
    }
    match info.no_link_reason() {
        Some(NoLinkReason::ZeroByte) => LinkState::ZeroByte,
        Some(NoLinkReason::Restricted) => LinkState::Restricted,
        None => {
            if info.audit_flagged() != Some(false) && list_audit_flagged {
                LinkState::Restricted
            } else {
                LinkState::Waiting
            }
        }
    }
}

/// 取一次 → 判定 →（原因不明才）等待 → 再取一次。
///
/// 抽成纯逻辑（`fetch` / `wait` 都是闭包）是为了能单测重试矩阵，
/// 不必搭假服务器。
fn resolve_info<F, W>(mut fetch: F, list_audit_flagged: bool, wait: W) -> Result<FileInfo, Failure>
where
    F: FnMut() -> Result<FileInfo, Failure>,
    W: FnOnce() -> Result<(), Failure>,
{
    let first = fetch()?;
    match link_state(&first, list_audit_flagged) {
        // 0 字节由调用方落空文件 / 报专门文案，这里只负责把详情带回去
        LinkState::Ready | LinkState::ZeroByte => return Ok(first),
        LinkState::Restricted => return Err(Failure::Other(RESTRICTED_NO_LINK.to_owned())),
        LinkState::Waiting => {}
    }
    // 原因不明：等一会儿再问一次；仍为空就如实报错，不给假档位。
    wait()?;
    let retry = fetch()?;
    match link_state(&retry, list_audit_flagged) {
        LinkState::Ready | LinkState::ZeroByte => Ok(retry),
        LinkState::Restricted => Err(Failure::Other(RESTRICTED_NO_LINK.to_owned())),
        LinkState::Waiting => Err(Failure::Other(LINK_NOT_READY.to_owned())),
    }
}

/// 取一个文件的可播放档位。阻塞。
///
/// `cancel` 有值时，链接全空的 5 秒重试可被取消打断（播放提示条的「取消」走这里）；
/// 被取消返回「下载已取消」，调用方应据取消标志静默收尾，不显示这条错误。
///
/// `list_audit_flagged` 来自文件列表（列表请求带 `with_audit=true`），
/// 详情没下发 audit 时作为「违规 / 受限」的兜底线索。
pub fn play_options(
    session: &Session,
    file_id: &str,
    cancel: Option<&Cancel>,
    list_audit_flagged: bool,
) -> Result<Vec<PlayOption>, Failure> {
    let info = file_info(session, file_id, cancel, list_audit_flagged)?;
    if info.is_zero_byte() {
        return Err(Failure::Other(ZERO_BYTE_PLAY.to_owned()));
    }
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
///
/// 明确 0 字节 / 违规时直接返回 `Ok(info)` 或 `RESTRICTED_NO_LINK`，不等待。
pub fn file_info(
    session: &Session,
    file_id: &str,
    cancel: Option<&Cancel>,
    list_audit_flagged: bool,
) -> Result<FileInfo, Failure> {
    resolve_info(
        || session.drive()?.file_detail(file_id).map_err(Failure::from),
        list_audit_flagged,
        || wait_before_retry(cancel),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::time::Instant;

    /// 构造一份只关心「链接 / size / audit」的详情。
    fn info(size: Option<u64>, audit: Option<&str>, link: &str) -> FileInfo {
        FileInfo {
            id: "f1".into(),
            name: "a.mkv".into(),
            size,
            audit_status: audit.map(str::to_owned),
            octet_stream: link.to_owned(),
            ..FileInfo::default()
        }
    }

    #[test]
    fn link_state_matrix() {
        use LinkState::*;
        assert_eq!(
            link_state(&info(None, None, "https://cdn.invalid/x"), true),
            Ready,
            "有链接就是 Ready，列表标记不拦"
        );
        assert_eq!(link_state(&info(Some(0), None, ""), false), ZeroByte);
        assert_eq!(
            link_state(&info(None, Some("STATUS_BANNED"), ""), false),
            Restricted
        );
        assert_eq!(link_state(&info(None, None, ""), true), Restricted);
        assert_eq!(
            link_state(&info(None, Some("STATUS_OK"), ""), true),
            Waiting,
            "详情明确 OK 时以详情为准，列表旧标记不作数"
        );
        assert_eq!(link_state(&info(None, None, ""), false), Waiting);
    }

    #[test]
    fn zero_byte_returns_after_one_fetch_without_waiting() {
        let fetches = Cell::new(0);
        let waited = Cell::new(false);
        let result = resolve_info(
            || {
                fetches.set(fetches.get() + 1);
                Ok(info(Some(0), None, ""))
            },
            false,
            || {
                waited.set(true);
                Ok(())
            },
        )
        .expect("0 字节不是错误，落空文件由调用方处理");
        assert_eq!(fetches.get(), 1);
        assert!(!waited.get(), "0 字节不该再等 5 秒");
        assert!(result.is_zero_byte());
    }

    #[test]
    fn detail_restricted_fails_after_one_fetch_without_waiting() {
        let fetches = Cell::new(0);
        let waited = Cell::new(false);
        let err = resolve_info(
            || {
                fetches.set(fetches.get() + 1);
                Ok(info(None, Some("STATUS_BANNED"), ""))
            },
            false,
            || {
                waited.set(true);
                Ok(())
            },
        )
        .expect_err("详情明确违规时必须报错");
        assert_eq!(err.to_string(), RESTRICTED_NO_LINK);
        assert_eq!(fetches.get(), 1);
        assert!(!waited.get(), "违规重试没用，不该等 5 秒");
    }

    #[test]
    fn list_restricted_fails_after_one_fetch_without_waiting() {
        let fetches = Cell::new(0);
        let waited = Cell::new(false);
        let err = resolve_info(
            || {
                fetches.set(fetches.get() + 1);
                Ok(info(None, None, ""))
            },
            true,
            || {
                waited.set(true);
                Ok(())
            },
        )
        .expect_err("详情没说、列表标了违规：按受限处理");
        assert_eq!(err.to_string(), RESTRICTED_NO_LINK);
        assert_eq!(fetches.get(), 1);
        assert!(!waited.get());
    }

    #[test]
    fn unknown_empty_retries_once_then_reports_not_ready() {
        let fetches = Cell::new(0);
        let waits = Cell::new(0);
        let err = resolve_info(
            || {
                fetches.set(fetches.get() + 1);
                Ok(info(None, None, ""))
            },
            false,
            || {
                waits.set(waits.get() + 1);
                Ok(())
            },
        )
        .expect_err("两次都全空且原因不明：报链接未就绪");
        assert_eq!(err.to_string(), LINK_NOT_READY);
        assert_eq!(fetches.get(), 2);
        assert_eq!(waits.get(), 1);
    }

    #[test]
    fn link_appearing_on_retry_is_returned() {
        let fetches = Cell::new(0);
        let waits = Cell::new(0);
        let result = resolve_info(
            || {
                let round = fetches.get() + 1;
                fetches.set(round);
                Ok(info(
                    None,
                    None,
                    if round == 1 {
                        ""
                    } else {
                        "https://cdn.invalid/x"
                    },
                ))
            },
            false,
            || {
                waits.set(waits.get() + 1);
                Ok(())
            },
        )
        .expect("第二次拿到链接就该成功");
        assert_eq!(fetches.get(), 2);
        assert_eq!(waits.get(), 1);
        assert!(!result.all_links_empty());
    }

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

    #[test]
    fn zero_byte_link_text_is_specific() {
        assert!(ZERO_BYTE_PLAY.contains("0 字节"));
        assert!(RESTRICTED_NO_LINK.contains("违规"));
        assert!(LINK_NOT_READY.contains("链接"));
    }
}
