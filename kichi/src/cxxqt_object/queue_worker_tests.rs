//! `queue_worker.rs` 的单元测试：进度节流、批次结束的通知与状态栏、建目录。
//!
//! 下载循环本身要真实会话，属于 `live_tests` 的范围；目录遍历的边界在
//! `queue_collect_tests.rs`；显示路径的测试在 `transfers_tests.rs`。

use super::*;

fn report(
    name: &str,
    folder: bool,
    done: usize,
    failed: usize,
    failure: Option<&str>,
) -> BatchReport {
    report_with(name, folder, done, failed, 0, false, failure)
}

fn report_with(
    name: &str,
    folder: bool,
    done: usize,
    failed: usize,
    skipped: usize,
    empty_folder: bool,
    failure: Option<&str>,
) -> BatchReport {
    BatchReport {
        name: name.to_owned(),
        folder,
        done,
        failed,
        cancelled: 0,
        skipped,
        empty_folder,
        failure: failure.map(str::to_owned),
    }
}

#[test]
fn throttle_sends_first_then_waits_within_same_percent() {
    let throttle = Throttle::default();
    assert!(throttle.should_send(0, Some(10_000)), "第一次一定发");
    assert!(
        !throttle.should_send(50, Some(10_000)),
        "仍是 0%、未到 250ms：不发"
    );
    assert!(throttle.should_send(500, Some(10_000)), "进到 5% 就发");
}

#[test]
fn throttle_without_total_uses_time_only() {
    let throttle = Throttle::default();
    assert!(throttle.should_send(1, None));
    assert!(!throttle.should_send(2, None), "没有总大小时按时间节流");
}

#[test]
fn cancelled_batch_has_no_notice() {
    let report = report("剧集", true, 3, 0, None);
    assert!(
        end_notice(&report, &BatchEnd::Cancelled).is_none(),
        "用户主动取消不弹系统通知"
    );
}

#[test]
fn interrupted_batch_has_one_error_notice() {
    let notice = end_notice(&report("剧集", true, 3, 0, None), &BatchEnd::Interrupted)
        .expect("中断要弹一条");
    assert_eq!(notice.summary, "下载失败");
    assert_eq!(notice.icon, "dialog-error");
    assert!(notice.body.contains("下载线程异常中断"));
}

#[test]
fn batch_level_failure_has_its_reason_in_the_notice() {
    let end = BatchEnd::Failed("创建下载目录失败：权限不足".to_owned());
    let notice = end_notice(&report("剧集", true, 0, 0, None), &end).expect("批次级失败要弹");
    assert_eq!(notice.summary, "下载失败");
    assert!(notice.body.contains("创建下载目录失败：权限不足"));
}

#[test]
fn completed_batch_uses_summary_builder() {
    let notice = end_notice(&report("a.mkv", false, 1, 0, None), &BatchEnd::Completed)
        .expect("正常完成要弹一条");
    assert_eq!(notice.summary, "下载完成");
    assert_eq!(notice.body, "a.mkv");
    assert!(!notice.body.contains('/'), "通知不写保存路径");
}

#[test]
fn empty_folder_is_a_success_without_a_notice() {
    let report = report_with("空文件夹", true, 0, 0, 0, true, None);
    assert!(
        end_notice(&report, &BatchEnd::Completed).is_none(),
        "空文件夹建完就完了，不值得弹系统通知"
    );
}

#[test]
fn empty_folder_status_says_what_was_created() {
    let report = report_with("空文件夹", true, 0, 0, 0, true, None);
    assert_eq!(
        status_text(&report, &BatchEnd::Completed).as_deref(),
        Some("已创建空文件夹：空文件夹")
    );
}

#[test]
fn completed_status_mentions_skipped_directories() {
    let report = report_with("剧集", true, 12, 0, 3, false, None);
    assert_eq!(
        status_text(&report, &BatchEnd::Completed).as_deref(),
        Some("下载完成：12 个文件（跳过 3 个目录）")
    );

    let clean = report_with("剧集", true, 12, 0, 0, false, None);
    assert_eq!(
        status_text(&clean, &BatchEnd::Completed).as_deref(),
        Some("下载完成：12 个文件"),
        "没有跳过时不写后缀"
    );
}

#[test]
fn failed_and_partial_batches_stay_out_of_the_status_bar() {
    let partial = report_with("剧集", true, 10, 2, 1, false, Some("断网"));
    assert_eq!(status_text(&partial, &BatchEnd::Completed), None);

    let failed = report_with("剧集", true, 0, 5, 0, false, Some("断网"));
    assert_eq!(status_text(&failed, &BatchEnd::Completed), None);

    let dir_failed = BatchEnd::Failed("创建下载目录失败".to_owned());
    assert_eq!(status_text(&partial, &dir_failed), None);
    assert_eq!(status_text(&partial, &BatchEnd::Interrupted), None);

    assert_eq!(
        status_text(&partial, &BatchEnd::Cancelled).as_deref(),
        Some("下载已取消（已完成 10 个，失败 2 个）")
    );
}

#[test]
fn completed_batch_reports_individually_cancelled_children() {
    let mut mixed = report_with("剧集", true, 3, 0, 1, false, None);
    mixed.cancelled = 2;
    assert_eq!(
        status_text(&mixed, &BatchEnd::Completed).as_deref(),
        Some("下载结束：完成 3 个，取消 2 个（跳过 1 个目录）")
    );

    let mut all_cancelled = report_with("剧集", true, 0, 0, 0, false, None);
    all_cancelled.cancelled = 4;
    assert_eq!(
        status_text(&all_cancelled, &BatchEnd::Completed).as_deref(),
        Some("下载已取消：4 个文件")
    );
}

// —— 建目录（本地空目录树）——

fn temp_root(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kichi-dirs-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建临时目录");
    dir
}

#[test]
fn local_dir_sanitises_every_part() {
    let path = local_dir(
        Path::new("/home/me/Downloads"),
        &["剧集".to_owned(), ".piko".to_owned()],
    );
    assert_eq!(
        path,
        PathBuf::from("/home/me/Downloads")
            .join("剧集")
            .join("_.piko"),
        "建目录与文件落盘共用 safe_file_name"
    );
}

#[test]
fn create_dirs_builds_the_whole_tree() {
    let root = temp_root("tree");
    let dirs = vec![
        vec!["剧集".to_owned()],
        vec!["剧集".to_owned(), "第一季".to_owned()],
        vec!["剧集".to_owned(), "空".to_owned()],
    ];
    let (skipped, fatal) = create_dirs(&root, &dirs, &Cancel::default());
    assert_eq!(skipped, 0);
    assert!(fatal.is_none());
    for parts in &dirs {
        assert!(local_dir(&root, parts).is_dir(), "每一层都要真的建出来");
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn create_dirs_counts_non_root_failures_as_skips() {
    let root = temp_root("blocked");
    // 二级目录的位置先占成普通文件，它下面的目录就建不出来
    std::fs::create_dir_all(root.join("剧集")).expect("建一级目录");
    std::fs::write(root.join("剧集").join("挡住"), b"x").expect("写占位文件");
    let dirs = vec![
        vec!["剧集".to_owned()],
        vec!["剧集".to_owned(), "挡住".to_owned()],
    ];
    let (skipped, fatal) = create_dirs(&root, &dirs, &Cancel::default());
    assert_eq!(skipped, 1, "非根目录建不出来只并入跳过");
    assert!(fatal.is_none(), "根目录建好了，不算整批失败");
    assert!(local_dir(&root, &dirs[0]).is_dir());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn create_dirs_root_failure_is_fatal() {
    let root = temp_root("fatal");
    // 下载根目录位置被一个普通文件占住 → 根目录本身建不出来
    std::fs::write(root.join("剧集"), b"x").expect("写占位文件");
    let dirs = vec![
        vec!["剧集".to_owned()],
        vec!["剧集".to_owned(), "第一季".to_owned()],
    ];
    let (skipped, fatal) = create_dirs(&root, &dirs, &Cancel::default());
    assert_eq!(skipped, 0, "根目录失败不能算跳过");
    assert!(
        fatal
            .as_deref()
            .is_some_and(|reason| reason.contains("创建下载目录失败")),
        "根目录建不出来要整批失败：{fatal:?}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn create_dirs_stops_on_cancel() {
    let root = temp_root("cancel");
    let cancel = Cancel::default();
    cancel.cancel();
    let dirs = vec![vec!["剧集".to_owned()]];
    let (skipped, fatal) = create_dirs(&root, &dirs, &cancel);
    assert_eq!((skipped, fatal), (0, None));
    assert!(!local_dir(&root, &dirs[0]).exists(), "取消后不再建目录");
    let _ = std::fs::remove_dir_all(&root);
}

// —— 批内并发游标（2026-10-28 下载提速）——

#[test]
fn parallel_cursor_visits_each_job_exactly_once() {
    let items: Vec<usize> = (0..64).collect();
    let hits: Vec<AtomicUsize> = (0..64).map(|_| AtomicUsize::new(0)).collect();
    let interrupted = run_parallel(
        &items,
        4,
        |index, item| {
            assert_eq!(index, *item, "游标下标必须与任务一一对应");
            hits[index].fetch_add(1, Ordering::SeqCst);
        },
        || false,
    );
    assert!(!interrupted, "没有 panic / 取消时不算中断");
    assert!(
        hits.iter().all(|hit| hit.load(Ordering::SeqCst) == 1),
        "每个 job 必须恰好被处理一次（不重不漏）"
    );
}

#[test]
fn parallel_panic_waits_for_every_worker_before_returning() {
    struct Active<'a>(&'a AtomicUsize);
    impl Drop for Active<'_> {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }

    let active = AtomicUsize::new(0);
    let items: Vec<usize> = (0..32).collect();
    let interrupted = run_parallel(
        &items,
        4,
        |_index, item| {
            active.fetch_add(1, Ordering::SeqCst);
            let _guard = Active(&active);
            std::thread::sleep(Duration::from_millis(20));
            assert_ne!(*item, 0, "第一条任务模拟 panic，收尾必须等所有线程退出");
        },
        || false,
    );
    assert!(interrupted, "有线程 panic 时批次要按中断收尾");
    assert_eq!(
        active.load(Ordering::SeqCst),
        0,
        "函数返回时所有 worker 必须已经结束，不能再往记录表里写"
    );
}

#[test]
fn parallel_env_defaults_and_serial_rollback() {
    static ENV_LOCK: Mutex<()> = Mutex::new(());
    let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let cases = [
        ("1", 1),
        (" 4 ", 4),
        ("0", DEFAULT_PARALLEL),
        ("", DEFAULT_PARALLEL),
        ("abc", DEFAULT_PARALLEL),
        ("-2", DEFAULT_PARALLEL),
        ("999", MAX_PARALLEL),
    ];
    for (raw, expected) in cases {
        std::env::set_var(PARALLEL_ENV, raw);
        assert_eq!(download_parallel(), expected, "输入 {raw:?}");
    }
    std::env::remove_var(PARALLEL_ENV);
    assert_eq!(download_parallel(), DEFAULT_PARALLEL);
}

#[test]
fn a_saved_file_is_recorded_even_after_logout() {
    // rename 与登出作废互斥，成品一旦落盘就要照实记完成——否则历史写「已中断」，
    // 用户点重试会再下一份 `名字 (1).ext`
    assert!(records_result(true, false, true), "正常完成");
    assert!(records_result(true, false, false), "登出瞬间刚下完");
    assert!(records_result(true, true, false), "取消刚好下完也记完成");
}

#[test]
fn unsaved_results_are_recorded_only_in_the_same_session() {
    assert!(records_result(false, false, true), "同一会话的失败要记原因");
    assert!(!records_result(false, false, false), "旧会话的失败不改记录");
    assert!(!records_result(false, true, true), "取消不记失败");
    assert!(!records_result(false, true, false));
}
