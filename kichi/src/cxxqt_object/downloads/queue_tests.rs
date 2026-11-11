//! `queue.rs` 的单元测试：按源 id 占用、并发推进、文件夹独占、释放与取消。
//!
//! 只测纯调度逻辑（不碰网络、不起真线程）：`queue_collect::collect_folder` 与
//! `run_jobs` 都需要真实会话，属于 `live_tests` 的范围。

use super::*;
use crate::cxxqt_object::transfers::TransferDirection;
use chrono::Utc;

fn entry(id: &str, kind: EntryKind, name: &str) -> FileEntry {
    FileEntry {
        id: id.to_owned(),
        name: name.to_owned(),
        kind,
        size: 0,
        created_at: None,
        modified_at: Some(Utc::now()),
        parent_id: String::new(),
        md5: None,
        audit_flagged: false,
        starred: false,
        media: false,
        video: false,
        thumbnail_url: String::new(),
    }
}

fn reserve_id(id: &str) -> Option<u64> {
    reserve(entry(id, EntryKind::Folder, id), PathBuf::from("/tmp"))
}

fn reserve_file_id(id: &str) -> Option<u64> {
    reserve(
        entry(id, EntryKind::File, &format!("{id}.mkv")),
        PathBuf::from("/tmp"),
    )
}

/// 重试种子（单文件 / 文件夹总账行都造得出来）。
fn retry_job(id: &str, kind: EntryKind, sub_dir: &[&str]) -> TransferJob {
    TransferJob {
        id: id.to_owned(),
        name: format!("{id}.mkv"),
        kind,
        size: 123,
        audit_flagged: false,
        sub_dir: sub_dir.iter().map(|part| (*part).to_owned()).collect(),
        dir: PathBuf::from("/tmp/kichi-retry"),
        direction: TransferDirection::Download,
        upload: None,
    }
}

/// 调度器是进程级单例：测试默认并行跑，必须串起来，否则互相清状态。
fn lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// 测试共用一个进程级调度器：每条用例结束都要清干净，否则互相污染。
fn reset_for_test() {
    // 新语义下 reset 会保留「已起线程」的 active；测试残留必须先 release。
    let residual = with_scheduler(|scheduler| {
        scheduler
            .active
            .iter()
            .map(|pending| pending.batch)
            .collect::<Vec<_>>()
    });
    for batch in residual {
        let _ = release(batch);
    }
    reset();
    transfers::reset();
}

#[test]
fn same_source_id_is_rejected_while_pending() {
    let _guard = lock();
    reset_for_test();
    let first = reserve_id("folder-a").expect("第一次登记");
    assert!(
        reserve_id("folder-a").is_none(),
        "同一源条目未结束前不能再入队（连点只会建一个批次）"
    );
    assert!(is_pending("folder-a"));
    assert!(!is_pending("folder-b"));

    let _ = release(first);
    assert!(!is_pending("folder-a"), "结束后立即释放，可以再下");
    assert!(reserve_id("folder-a").is_some());
    reset_for_test();
}

#[test]
fn different_sources_queue_up_in_order() {
    let _guard = lock();
    reset_for_test();
    let first = reserve_id("a").expect("a");
    let second = reserve_id("b").expect("b");
    assert_ne!(first, second, "不同条目各占一个批次");
    with_scheduler(|scheduler| {
        assert!(scheduler.active.is_empty());
        let ids: Vec<&str> = scheduler
            .queue
            .iter()
            .map(|pending| pending.source_id.as_str())
            .collect();
        assert_eq!(ids, vec!["a", "b"], "登记顺序就是执行顺序");
    });
    let (running, queued) = counts();
    assert!(!running, "还没起线程");
    assert_eq!(queued, 2);
    reset_for_test();
}

#[test]
fn preparing_batch_still_owns_its_id() {
    let _guard = lock();
    reset_for_test();
    let batch = reserve_id("folder").expect("登记");
    // 模拟 advance 已经把它取成 current 并进入「准备中」
    let start = next_start().expect("轮到这一批");
    assert_eq!(start.batch, batch);
    assert!(start.folder, "文件夹批次要读目录");
    assert!(
        reserve_id("folder").is_none(),
        "准备中（正在读目录）也算占用"
    );
    let (running, queued) = counts();
    assert!(running);
    assert_eq!(queued, 0);
    transfers::reset();
    reset();
}

#[test]
fn single_file_skips_preparing() {
    let _guard = lock();
    reset_for_test();
    let batch = reserve(
        entry("file-1", EntryKind::File, "a.mkv"),
        PathBuf::from("/tmp"),
    )
    .expect("登记");
    let start = next_start().expect("轮到这一批");
    assert_eq!(start.batch, batch);
    assert!(!start.folder, "单文件不用读目录");
    with_scheduler(|scheduler| {
        assert_eq!(
            scheduler.active.first().map(|pending| pending.state),
            Some(PendingState::Running),
            "单文件跳过「准备中」，直接下载中"
        );
    });
    transfers::reset();
    reset();
}

#[test]
fn cancel_all_drops_queued_batches() {
    let _guard = lock();
    reset_for_test();
    reserve_id("a").expect("a");
    reserve_id("b").expect("b");
    let queued_batches = cancel_all();
    assert_eq!(queued_batches.len(), 2, "返回直接移除的排队批次");
    let (_, queued) = counts();
    assert_eq!(queued, 0, "排队中的批次直接放掉，不再占位");
    assert!(!is_pending("a") && !is_pending("b"), "释放后可以重新下");
    reset_for_test();
}

#[test]
fn cancel_transfer_drops_a_queued_batch() {
    let _guard = lock();
    reset_for_test();
    let batch = reserve_id("queued-folder").expect("登记");
    assert_eq!(cancel_transfer(batch, 0, true), Some(CancelOutcome::Queued));
    let (running, queued) = counts();
    assert!(!running && queued == 0, "排队批次直接移出队列");
    assert!(!is_pending("queued-folder"), "取消后不再占用来源 id");
    reset_for_test();
}

#[test]
fn pause_drops_a_queued_batch_and_resume_reuses_its_identity() {
    let _guard = lock();
    reset_for_test();
    let batch = reserve_id("queued-pause").expect("登记");
    assert_eq!(pause_transfer(batch), PauseOutcome::Queued);
    transfers::pause_queued(batch);
    assert!(
        is_pending("queued-pause"),
        "暂停后虽不占调度槽，但仍占用同一源条目"
    );
    assert!(
        reserve_id("queued-pause").is_none(),
        "已暂停的源条目不能再排第二批"
    );
    assert_eq!(counts(), (false, 0));

    let files = vec![Job {
        id: "child".to_owned(),
        index: 3,
        sub_dir: vec!["合集".to_owned()],
        name: "child.mkv".to_owned(),
        size: 10,
        audit_flagged: false,
    }];
    assert!(reserve_resume(ResumeBatch {
        batch,
        source_id: "queued-pause".to_owned(),
        entry: entry("queued-pause", EntryKind::Folder, "合集"),
        dir: PathBuf::from("/tmp"),
        files: Some(files),
        dirs: vec![vec!["合集".to_owned()]],
        skipped: 2,
        done: 1,
        failed: 1,
        cancelled: 0,
        failure: Some("旧失败".to_owned()),
    }));

    let start = next_start().expect("继续后重新入队");
    assert_eq!(start.batch, batch, "继续复用原批次号");
    let collected = start.resume_collected.expect("文件夹子项直接复用");
    assert_eq!(collected.files[0].index, 3, "原 index / 行 key 不变");
    assert_eq!(collected.skipped, 2);
    assert_eq!(
        release(batch).map(|report| (report.done, report.failed, report.cancelled)),
        Some((1, 1, 0)),
        "续跑后的通知汇总要包含暂停前已完成 / 失败的子项"
    );
    assert!(
        start
            .job_cancels
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains_key(&3),
        "继续后的子取消信号也按原 index 挂回"
    );
    transfers::reset();
    reset_for_test();
}

#[test]
fn pause_running_batch_waits_for_release_then_resumes() {
    let _guard = lock();
    reset_for_test();
    let batch = reserve_id("running-pause").expect("登记");
    let start = next_start().expect("文件夹开始准备");
    assert_eq!(pause_transfer(batch), PauseOutcome::Running);
    assert!(start.cancel.is_paused(), "运行中的批次先收到暂停信号");
    assert!(start.cancel.is_cancelled(), "暂停也必须让下载线程尽快停下");
    assert_eq!(counts(), (true, 0), "线程收尾前仍占调度槽");

    assert!(release(batch).is_some());
    assert!(reserve_resume(ResumeBatch {
        batch,
        source_id: "running-pause".to_owned(),
        entry: entry("running-pause", EntryKind::Folder, "running-pause"),
        dir: PathBuf::from("/tmp"),
        files: Some(Vec::new()),
        dirs: Vec::new(),
        skipped: 0,
        done: 0,
        failed: 0,
        cancelled: 0,
        failure: None,
    }));
    let resumed = next_start().expect("线程收尾后可继续");
    assert_eq!(resumed.batch, batch);
    transfers::reset();
    reset_for_test();
}

#[test]
fn cancel_after_pause_switches_back_to_delete_semantics() {
    let _guard = lock();
    reset_for_test();
    let batch = reserve_id("pause-then-cancel").expect("登记");
    let start = next_start().expect("文件夹开始准备");
    assert_eq!(pause_transfer(batch), PauseOutcome::Running);
    assert!(start.cancel.is_paused());

    assert_eq!(
        cancel_transfer(batch, 0, true),
        Some(CancelOutcome::Running)
    );
    assert!(start.cancel.is_cancelled());
    assert!(
        !start.cancel.is_paused(),
        "暂停后改取消必须让下载线程删除断点"
    );
    assert!(release(batch).is_some());
    transfers::reset();
    reset_for_test();
}

#[test]
fn child_cancel_leaves_the_batch_and_siblings_running() {
    let _guard = lock();
    reset_for_test();
    let batch = reserve_id("running-folder").expect("登记");
    let start = next_start().expect("文件夹轮到准备");
    assert!(start.folder);

    assert_eq!(
        cancel_transfer(batch, 0, false),
        Some(CancelOutcome::Running)
    );
    let child = start
        .job_cancels
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&0)
        .cloned()
        .expect("子取消信号已登记");
    assert!(child.is_cancelled(), "目标子文件收到取消");
    assert!(!start.cancel.is_cancelled(), "整批父信号不受影响");
    assert!(is_pending("running-folder"));

    assert_eq!(
        cancel_transfer(batch, 0, true),
        Some(CancelOutcome::Running)
    );
    assert!(start.cancel.is_cancelled(), "父行取消会向下传递整批");
    assert!(child.is_cancelled());
    let _ = release(batch);
    reset_for_test();
}

#[test]
fn reset_clears_queue_and_bumps_epoch() {
    let _guard = lock();
    reset_for_test();
    reserve_id("a").expect("a");
    let before = current_epoch();
    reset();
    assert_eq!(current_epoch(), before + 1, "登出作废在途结果");
    let (running, queued) = counts();
    assert!(!running && queued == 0);
    assert!(reserve_id("a").is_some(), "reset 之后同名可以重新入队");
    reset_for_test();
}

#[test]
fn reset_keeps_started_batch_until_worker_finishes() {
    let _guard = lock();
    reset_for_test();
    let batch = reserve_id("folder-a").expect("登记");
    let start = next_start().expect("轮到这一批");
    assert_eq!(start.batch, batch);
    mark_started(batch);
    reset();

    assert!(
        !is_pending("folder-a"),
        "abandoned 只是等旧线程退出，不再参与判重"
    );
    let (running, queued) = counts();
    assert!(running, "旧线程未退出前调度槽不能被提前放开");
    assert_eq!(queued, 0);
    assert!(
        reserve_id("folder-a").is_some(),
        "重新登录后同一文件可以再入队"
    );
    assert!(next_start().is_none(), "旧线程未退出，新批次只能排队");

    assert!(release(batch).is_some(), "旧线程收尾释放槽位");
    let next = next_start().expect("旧线程退出后新批次自动开跑");
    assert_eq!(next.entry.id, "folder-a");
    transfers::reset();
    reset_for_test();
}

#[test]
fn reset_drops_batch_without_live_worker() {
    let _guard = lock();
    reset_for_test();
    let batch = reserve_id("folder-b").expect("登记");
    let start = next_start().expect("取成 current");
    assert_eq!(start.batch, batch);
    // 不 mark_started：模拟线程根本没起来（测试里的假批次）
    reset();

    let (running, queued) = counts();
    assert!(!running && queued == 0, "没起线程的批次直接丢掉");
    assert!(next_start().is_none(), "队列已空");
    assert!(reserve_id("folder-b").is_some(), "同一 id 可以重新入队");
    transfers::reset();
    reset_for_test();
}

#[test]
fn cancelling_while_preparing_releases_at_once() {
    let _guard = lock();
    reset_for_test();
    let batch = reserve_id("folder").expect("登记");
    let _ = next_start().expect("轮到这一批");
    assert!(is_pending("folder"));

    cancel_all();
    // 准备中（读目录）的批次只置标志，等线程收尾；这里直接用 release 模拟
    assert!(release(batch).is_some());
    assert!(!is_pending("folder"), "取消后占用立刻释放");
    assert!(reserve_id("folder").is_some(), "同一目取消后可以马上再下");
    transfers::reset();
    reset();
}

#[test]
fn queued_batch_dropped_by_cancel_still_advances() {
    let _guard = lock();
    reset_for_test();
    let first = reserve_id("a").expect("a");
    reserve_id("b").expect("b");
    // a 正在跑，b 还在排队；此时取消全部
    let start = next_start().expect("a 开跑");
    assert_eq!(start.batch, first);
    cancel_all();
    // 排队的 b 被直接丢掉（不留 current，才能让后续批次顶上）
    assert!(!is_pending("b"));
    assert_eq!(counts().1, 0, "排队列表已清空");
    // 线程侧的 release 后，下一批可以起来
    assert!(release(first).is_some());
    assert!(next_start().is_none(), "没有更多批次了");
    transfers::reset();
    reset();
}

#[test]
fn a_panicked_worker_still_frees_its_slot() {
    // 线程 panic 也必须走到 release（收尾在 run_jobs 的 catch_unwind 之后），
    // 否则这个条目 id 永远占着、后面的批次也永远不动。
    let _guard = lock();
    reset_for_test();
    let batch = reserve_id("folder").expect("登记");
    let start = next_start().expect("轮到这一批");
    assert_eq!(start.batch, batch);
    // run_jobs 的收尾无论成功 / 失败 / panic 都调 release：
    assert!(release(batch).is_some(), "收尾必须释放占用");
    assert!(!is_pending("folder"));
    assert!(next_start().is_none(), "释放后队列空了，不会再起一批");
    transfers::reset();
    reset();
}

#[test]
fn single_file_batches_fill_the_parallel_slots() {
    let _guard = lock();
    reset_for_test();
    let limit = download_parallel();
    for index in 0..=limit {
        reserve_file_id(&format!("file-{index}")).expect("登记单文件");
    }

    let mut started = Vec::new();
    for _ in 0..limit {
        started.push(next_start().expect("并发槽逐个启动").batch);
    }
    assert_eq!(started.len(), limit, "独立单文件填满并发槽");
    assert!(next_start().is_none(), "超过并发数的文件继续排队");
    assert_eq!(counts(), (true, 1));

    assert!(release(started[0]).is_some());
    let next = next_start().expect("释放一个槽后，队首下一批顶上");
    assert_ne!(next.batch, started[0]);
    assert_eq!(next.entry.id, format!("file-{limit}"));
    assert_eq!(counts(), (true, 0));
    transfers::reset();
    reset();
}

#[test]
fn folder_batch_stays_exclusive() {
    let _guard = lock();
    reset_for_test();
    reserve_file_id("file").expect("单文件排队");
    reserve_id("folder").expect("文件夹排队");
    reserve_file_id("tail").expect("文件夹后面的单文件排队");

    let file = next_start().expect("先启动队首单文件");
    assert!(!file.folder);
    assert!(
        next_start().is_none(),
        "队首轮到文件夹时，必须等已有单文件全部结束"
    );

    assert!(release(file.batch).is_some());
    let folder = next_start().expect("槽空后文件夹独占启动");
    assert!(folder.folder);
    assert!(next_start().is_none(), "文件夹运行期间不能启动外面的单文件");
    assert!(release(folder.batch).is_some());
    let tail = next_start().expect("文件夹结束后才继续后面的单文件");
    assert_eq!(tail.entry.id, "tail");
    transfers::reset();
    reset();
}

#[test]
fn release_unknown_batch_is_a_no_op() {
    let _guard = lock();
    reset_for_test();
    assert!(release(999).is_none());
    reset_for_test();
}

// —— 失败重试（2026-10-27 传输页逐条动作）——

#[test]
fn retry_single_file_keeps_identity_and_sub_dir() {
    let _guard = lock();
    reset_for_test();
    let batch = reserve_retry(retry_job("file-1", EntryKind::File, &["剧集", "第一季"]))
        .expect("失败的单个文件可以重试");

    let start = next_start().expect("轮到这一批");
    assert_eq!(start.batch, batch);
    assert!(!start.folder, "单个文件重试不用读目录");
    assert_eq!(start.entry.id, "file-1", "重试的是文件 id");
    assert_eq!(start.sub_dir, vec!["剧集", "第一季"], "保留原批次的子目录");
    assert_eq!(
        start.dir,
        PathBuf::from("/tmp/kichi-retry"),
        "落回原批次的目录快照"
    );
    assert_eq!(start.entry.size, 123, "大小原样带给记录表");
    transfers::reset();
    reset();
}

#[test]
fn retry_keeps_the_audit_flag_for_the_link_probe() {
    let _guard = lock();
    reset_for_test();
    let mut job = retry_job("file-1", EntryKind::File, &[]);
    job.audit_flagged = true;
    reserve_retry(job).expect("审核标记的文件也能重试");
    let start = next_start().expect("轮到这一批");
    assert!(
        start.entry.audit_flagged,
        "审核标记随重试保留，避免再白等 5 秒取链"
    );
    transfers::reset();
    reset();
}

#[test]
fn retry_folder_ledger_reads_the_folder_again() {
    let _guard = lock();
    reset_for_test();
    reserve_retry(retry_job("folder-1", EntryKind::Folder, &[])).expect("总账行可重试");
    let start = next_start().expect("轮到这一批");
    assert!(start.folder, "文件夹总账行重试 = 重下整个文件夹");
    assert_eq!(start.entry.id, "folder-1");
    assert!(start.sub_dir.is_empty());
    transfers::reset();
    reset();
}

#[test]
fn retry_is_rejected_while_the_same_file_is_pending() {
    let _guard = lock();
    reset_for_test();
    let batch = reserve_retry(retry_job("file-1", EntryKind::File, &[])).expect("第一次重试");
    assert!(
        reserve_retry(retry_job("file-1", EntryKind::File, &[])).is_none(),
        "同一个文件 id 已在队列里，不能再入队"
    );
    assert!(is_pending("file-1"));
    let _ = release(batch);
    assert!(!is_pending("file-1"));
    assert!(
        reserve_retry(retry_job("file-1", EntryKind::File, &[])).is_some(),
        "释放后可以再重试"
    );
    transfers::reset();
    reset();
}

#[test]
fn retry_shares_the_duplicate_key_with_a_normal_download() {
    let _guard = lock();
    reset_for_test();
    reserve(
        entry("file-1", EntryKind::File, "a.mkv"),
        PathBuf::from("/tmp"),
    )
    .expect("普通下载先入队");
    assert!(
        reserve_retry(retry_job("file-1", EntryKind::File, &[])).is_none(),
        "文件页的下载与传输页的重试共用同一个云端条目 id"
    );
    transfers::reset();
    reset();
}

#[test]
fn release_reports_single_file_result() {
    let _guard = lock();
    reset_for_test();
    let batch = reserve(
        entry("file-1", EntryKind::File, "a.mkv"),
        PathBuf::from("/tmp"),
    )
    .expect("登记");
    let _ = next_start().expect("轮到这一批");
    add_result(batch, JobOutcome::Saved);
    let report = release(batch).expect("批次结束后应有汇总");
    assert_eq!(report.name, "a.mkv");
    assert!(!report.folder);
    assert_eq!((report.done, report.failed), (1, 0));
    assert_eq!(report.failure, None, "成功时不带失败原因");
    transfers::reset();
    reset();
}

#[test]
fn release_reports_last_failure() {
    let _guard = lock();
    reset_for_test();
    let batch = reserve(
        entry("file-2", EntryKind::File, "b.mkv"),
        PathBuf::from("/tmp"),
    )
    .expect("登记");
    let _ = next_start().expect("轮到这一批");
    add_result(batch, JobOutcome::Failed { reason: "断网" });
    add_result(
        batch,
        JobOutcome::Failed {
            reason: "磁盘已满"
        },
    );
    let report = release(batch).expect("批次结束后应有汇总");
    assert_eq!(report.done, 0);
    assert_eq!(report.failed, 2);
    assert_eq!(
        report.failure.as_deref(),
        Some("磁盘已满"),
        "单文件批次的通知用最后一条失败原因"
    );
    transfers::reset();
    reset();
}

// —— 空文件夹 / 跳过计数（2026-10-27 文件夹下载边界）——

#[test]
fn release_defaults_to_no_skips_and_not_empty() {
    let _guard = lock();
    reset_for_test();
    let batch = reserve(
        entry("file-1", EntryKind::File, "a.mkv"),
        PathBuf::from("/tmp"),
    )
    .expect("登记");
    let _ = next_start().expect("轮到这一批");
    let report = release(batch).expect("批次结束后应有汇总");
    assert_eq!(report.skipped, 0, "默认没有跳过的目录");
    assert!(!report.empty_folder, "默认不是空文件夹批次");
    transfers::reset();
    reset();
}

#[test]
fn add_skipped_only_touches_the_current_batch() {
    let _guard = lock();
    reset_for_test();
    let first = reserve_id("a").expect("a");
    let second = reserve_id("b").expect("b");
    let _ = next_start().expect("a 开跑");

    add_skipped(second, 5);
    add_skipped(first, 2);
    add_skipped(first, 3);
    let report = release(first).expect("a 结束");
    assert_eq!(report.skipped, 5, "只累计当前批次，且在同一个计数上叠加");

    let _ = next_start().expect("b 顶上");
    let report = release(second).expect("b 结束");
    assert_eq!(report.skipped, 0, "排队中的批次不会被写到");
    transfers::reset();
    reset();
}

// —— 按次选目录（「下载到…」）——

#[test]
fn reserve_keeps_the_given_directory_for_each_batch() {
    let _guard = lock();
    reset_for_test();
    let dir = PathBuf::from("/tmp/kichi-picked");
    reserve(entry("a", EntryKind::File, "a.mkv"), dir.clone()).expect("a");
    reserve(entry("b", EntryKind::File, "b.mkv"), dir.clone()).expect("b");

    let first = next_start().expect("a 开跑");
    let second = next_start().expect("b 接上");
    assert_eq!(first.dir, dir, "自定义目录进入批次快照");
    assert_eq!(second.dir, dir, "多选共用同一个目录，但各自一批");
    assert_ne!(first.batch, second.batch, "每个条目仍是独立批次");
    reset_for_test();
}

#[test]
fn reserve_still_uses_the_global_directory_when_asked() {
    let _guard = lock();
    reset_for_test();
    // 现有「下载」入口继续传全局目录；这条锁住新入口没有偷偷改掉它。
    let global = PathBuf::from("/tmp/kichi-global");
    reserve(entry("a", EntryKind::File, "a.mkv"), global.clone()).expect("a");
    let start = next_start().expect("a 开跑");
    assert_eq!(start.dir, global);
    reset_for_test();
}
