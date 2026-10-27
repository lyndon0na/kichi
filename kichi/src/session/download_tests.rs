//! `download.rs` 的单元测试：取消、空文件、续传重试决策。
//!
//! 下载循环本身需要真实会话，属于 `live_tests.rs` 的范围。

use super::super::download_resume::{
    classify_open_error, download_retries, resume_failure, retry_delay, save_empty_file,
    OpenAction, DEFAULT_RETRIES, RETRIES_ENV,
};
use super::*;
use kichi_core::Error as CoreError;

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kichi-download-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建临时目录");
    dir
}

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

#[test]
fn empty_file_is_saved_without_part_leftover() {
    let dir = temp_dir("empty");
    let downloaded = save_empty_file(&dir.join("nested"), "empty.mkv").expect("落空文件");
    assert_eq!(downloaded.bytes, 0);
    assert_eq!(downloaded.path, dir.join("nested").join("empty.mkv"));
    assert_eq!(
        std::fs::metadata(&downloaded.path)
            .expect("文件应存在")
            .len(),
        0,
        "不能留下任何内容"
    );
    assert!(
        !dir.join("nested").join("empty.mkv.kichi-part").exists(),
        "完成后退名，不能残留 .kichi-part"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn empty_file_never_overwrites_an_existing_one() {
    let dir = temp_dir("empty-twice");
    let first = save_empty_file(&dir, "same.mkv").expect("第一次");
    let second = save_empty_file(&dir, "same.mkv").expect("第二次");
    assert_eq!(first.path, dir.join("same.mkv"));
    assert_eq!(second.path, dir.join("same (1).mkv"), "同名要加序号");
    assert_eq!(second.bytes, 0);
    let _ = std::fs::remove_dir_all(&dir);
}

// —— 重试决策（纯函数矩阵）——

#[test]
fn network_errors_are_retryable_but_auth_and_quota_are_not() {
    assert_eq!(
        classify_open_error(&CoreError::Network("连接被重置".into()), 10),
        OpenAction::Retry { refresh: false }
    );
    assert_eq!(
        classify_open_error(&CoreError::DownloadTimeout("下载超时".into()), 10),
        OpenAction::Retry { refresh: false }
    );
    assert_eq!(
        classify_open_error(
            &CoreError::Api {
                code: 500,
                reason: "download_http_500".into(),
                message: String::new()
            },
            10
        ),
        OpenAction::Retry { refresh: false }
    );
    assert_eq!(
        classify_open_error(
            &CoreError::Api {
                code: 429,
                reason: "download_http_429".into(),
                message: String::new()
            },
            10
        ),
        OpenAction::Retry { refresh: false }
    );
    assert_eq!(
        classify_open_error(&CoreError::Auth { reason: "x".into() }, 10),
        OpenAction::Fatal
    );
    assert_eq!(
        classify_open_error(&CoreError::RateLimited { reason: "x".into() }, 10),
        OpenAction::Fatal
    );
}

#[test]
fn expired_link_refreshes_detail_on_retry() {
    for code in [403, 410] {
        assert_eq!(
            classify_open_error(
                &CoreError::Api {
                    code,
                    reason: format!("download_http_{code}"),
                    message: String::new()
                },
                10
            ),
            OpenAction::Retry { refresh: true },
            "签名链接过期要重取详情"
        );
    }
}

#[test]
fn range_errors_restart_only_when_there_is_an_offset_to_restart_from() {
    let err = CoreError::RangeNotSatisfiable { total: Some(10) };
    assert_eq!(classify_open_error(&err, 6), OpenAction::RestartFromZero);
    assert_eq!(
        classify_open_error(&err, 0),
        OpenAction::Fatal,
        "从 0 还报 Range 错，不能再循环重来"
    );
}

#[test]
fn retry_delays_are_one_three_eight_then_capped() {
    assert_eq!(retry_delay(0), Duration::from_secs(1));
    assert_eq!(retry_delay(1), Duration::from_secs(3));
    assert_eq!(retry_delay(2), Duration::from_secs(8));
    assert_eq!(retry_delay(3), Duration::from_secs(8));
    assert_eq!(retry_delay(u32::MAX), Duration::from_secs(8));
}

#[test]
fn retry_count_env_parses_and_can_be_disabled() {
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let cases = [
        ("5", 5),
        (" 0 ", 0),
        ("abc", DEFAULT_RETRIES),
        ("", DEFAULT_RETRIES),
    ];
    for (raw, expected) in cases {
        std::env::set_var(RETRIES_ENV, raw);
        assert_eq!(download_retries(), expected, "输入 {raw:?}");
    }
    std::env::remove_var(RETRIES_ENV);
    assert_eq!(download_retries(), DEFAULT_RETRIES);
}

#[test]
fn resume_failure_only_mentions_bytes_when_there_are_some() {
    let zero = resume_failure("下载中断".into(), 0).to_string();
    assert_eq!(zero, "下载中断");
    let some = resume_failure("下载中断".into(), 45 * 1024 * 1024).to_string();
    assert!(some.contains("45.0 MB"), "要说明续传起点：{some}");
    assert!(some.contains("可重试续传"), "{some}");
}
