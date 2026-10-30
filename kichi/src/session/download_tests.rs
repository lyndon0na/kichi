//! `download.rs` 的单元测试：取消、空文件、续传重试决策。
//!
//! 下载循环本身需要真实会话，属于 `live_tests.rs` 的范围。

use super::super::download_resume::{
    classify_open_error, download_retries, read_into_part, resume_failure, retry_delay,
    save_empty_file, OpenAction, Part, ReadOutcome, RetryBudget, DEFAULT_RETRIES, RETRIES_ENV,
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
fn abandon_sets_stop_and_keep() {
    let cancel = Cancel::default();
    cancel.abandon();
    assert!(cancel.is_cancelled(), "登出也要立刻停止下载");
    assert!(cancel.is_abandoned(), "登出要保留断点");

    let plain = Cancel::default();
    plain.cancel();
    assert!(plain.is_cancelled());
    assert!(!plain.is_abandoned(), "普通取消不保留断点");
}

#[test]
fn cancel_after_pause_clears_the_preserve_flag() {
    let cancel = Cancel::default();
    cancel.pause();
    assert!(cancel.is_cancelled() && cancel.is_paused());
    assert!(cancel.preserves_part(), "暂停先保留断点");

    cancel.cancel();
    assert!(cancel.is_cancelled());
    assert!(!cancel.is_paused(), "暂停后改取消必须切回删除断点");
    assert!(!cancel.preserves_part());
    assert_eq!(cancel.stop_reason(), "下载已取消");
}

#[test]
fn child_cancel_stops_only_itself_and_inherits_parent() {
    let parent = std::sync::Arc::new(Cancel::default());
    let first = Cancel::child(parent.clone());
    let second = Cancel::child(parent.clone());

    first.cancel();
    assert!(first.is_cancelled(), "子项取消后自己立即停止");
    assert!(!second.is_cancelled(), "不能误伤同批其它文件");
    assert!(!parent.is_cancelled(), "取消一个文件不能取消整批");
    assert!(
        !first.is_abandoned() && !second.is_abandoned(),
        "普通取消不保留断点"
    );

    parent.abandon();
    assert!(second.is_cancelled(), "登出也要向下停止每个子项");
    assert!(second.is_abandoned(), "登出必须让子项保留断点");
}

#[test]
fn empty_file_is_saved_without_part_leftover() {
    let dir = temp_dir("empty");
    let downloaded =
        save_empty_file(&dir.join("nested"), "empty.mkv", &Cancel::default()).expect("落空文件");
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
    let cancel = Cancel::default();
    let first = save_empty_file(&dir, "same.mkv", &cancel).expect("第一次");
    let second = save_empty_file(&dir, "same.mkv", &cancel).expect("第二次");
    assert_eq!(first.path, dir.join("same.mkv"));
    assert_eq!(second.path, dir.join("same (1).mkv"), "同名要加序号");
    assert_eq!(second.bytes, 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn empty_file_abandoned_leaves_nothing() {
    let dir = temp_dir("empty-abandoned");
    let cancel = Cancel::default();
    cancel.abandon();
    let result = save_empty_file(&dir, "empty.mkv", &cancel);
    assert!(result.is_err(), "登出命中时不能创建成品");
    assert!(!dir.join("empty.mkv").exists(), "登出绝不能落正式文件");
    assert!(
        !dir.join("empty.mkv.kichi-part").exists(),
        "0 字节 part 没有续传价值，直接删掉"
    );
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

// —— 重试预算 ——

#[test]
fn retry_budget_stops_after_consecutive_failures() {
    let mut budget = RetryBudget::new(3);
    assert_eq!(budget.grant(), Some(0));
    assert_eq!(budget.grant(), Some(1));
    assert_eq!(budget.grant(), Some(2));
    assert_eq!(budget.grant(), None, "连续三次都失败就该放弃");
    assert!(budget.exhausted());
}

#[test]
fn retry_budget_resets_after_progress() {
    // 长下载里零星抖动：每次都下了点数据再断，不该累积耗尽（known-issues §4.1）
    let mut budget = RetryBudget::new(3);
    for round in 0..50 {
        budget.progressed();
        assert_eq!(budget.grant(), Some(0), "第 {round} 轮应该还有满额");
    }
}

#[test]
fn retry_budget_has_a_total_cap() {
    // 服务器每次只给一点就断：连续计数一直被清零，得靠总额度收口
    let mut budget = RetryBudget::new(1);
    let mut granted = 0;
    while budget.grant().is_some() {
        granted += 1;
        budget.progressed();
        assert!(granted < 10_000, "总额度必须有限");
    }
    assert_eq!(granted, 20, "总额度 = 连续上限 × 20");
    assert!(budget.exhausted());
}

#[test]
fn retry_budget_zero_disables_retries() {
    let mut budget = RetryBudget::new(0);
    assert_eq!(budget.grant(), None);
    budget.progressed();
    assert_eq!(
        budget.grant(),
        None,
        "KICHI_DOWNLOAD_RETRIES=0 关掉重试后不该再试"
    );
}

#[test]
fn retry_budget_reports_consecutive_count_for_messages() {
    let mut budget = RetryBudget::new(3);
    assert_eq!(budget.consecutive(), 0);
    assert_eq!(budget.max_consecutive(), 3);
    budget.grant();
    budget.grant();
    assert_eq!(budget.consecutive(), 2, "提示里的 n/max 用连续计数");
    budget.progressed();
    assert_eq!(budget.consecutive(), 0);
}

#[test]
fn resume_failure_only_mentions_bytes_when_there_are_some() {
    let zero = resume_failure("下载中断".into(), 0).to_string();
    assert_eq!(zero, "下载中断");
    let some = resume_failure("下载中断".into(), 45 * 1024 * 1024).to_string();
    assert!(some.contains("45.0 MB"), "要说明续传起点：{some}");
    assert!(some.contains("可重试续传"), "{some}");
}

/// 读一个假字节流，返回（结果, 已写字节数）。
fn read_body(body: &[u8], total: Option<u64>, tag: &str) -> (ReadOutcome, u64) {
    let dir = temp_dir(tag);
    let part_path = dir.join("a.part");
    let file = std::fs::File::create(&part_path).expect("建 part 文件");
    let mut part = Part {
        file,
        part_path: part_path.clone(),
        final_path: dir.join("a.mkv"),
        offset: 0,
        total,
    };
    let mut stream = std::io::Cursor::new(body.to_vec());
    let cancel = Cancel::default();
    let progress = |_: u64, _: Option<u64>| {};
    let outcome = read_into_part(&mut stream, &mut part, &progress, &cancel);
    let offset = part.offset;
    drop(part);
    let _ = std::fs::remove_dir_all(&dir);
    (outcome, offset)
}

#[test]
fn known_total_early_eof_is_retryable_and_keeps_the_part() {
    let (outcome, offset) = read_body(b"0123456789", Some(20), "eof-early");
    assert!(
        matches!(outcome, ReadOutcome::Retryable(_)),
        "已知大小没下完必须可重试"
    );
    assert_eq!(offset, 10, "已收到的字节要保留给续传");
}

#[test]
fn known_total_exact_length_completes() {
    let (outcome, offset) = read_body(b"0123456789", Some(10), "exact");
    assert!(matches!(outcome, ReadOutcome::Complete));
    assert_eq!(offset, 10);
}

#[test]
fn known_total_overflow_is_a_local_error() {
    // 服务器发得比声明多：不能把多余字节当成正常文件
    let (outcome, _) = read_body(b"0123456789", Some(5), "overflow");
    assert!(
        matches!(outcome, ReadOutcome::Local(_)),
        "超出声明长度要放弃临时文件"
    );
}

#[test]
fn unknown_total_eof_is_complete() {
    // 未知总大小：HTTP 层能报的截断（chunked 不完整 / Content-Length 发不满）
    // 都在读时报错；能走到 EOF 的只有「服务器没说长度、正常发完」
    let (outcome, offset) = read_body(b"0123456789", None, "unknown");
    assert!(matches!(outcome, ReadOutcome::Complete));
    assert_eq!(offset, 10);
}
