//! `queue.rs` 的单元测试：按源 id 占用、串行推进、释放与取消。
//!
//! 只测纯调度逻辑（不碰网络、不起真线程）：`collect_folder` 与
//! `run_jobs` 都需要真实会话，属于 `live_tests` 的范围。

use super::*;
use chrono::Utc;

fn entry(id: &str, kind: EntryKind, name: &str) -> FileEntry {
    FileEntry {
        id: id.to_owned(),
        name: name.to_owned(),
        kind,
        size: 0,
        modified_at: Some(Utc::now()),
        md5: None,
        audit_flagged: false,
        media: false,
    }
}

fn reserve_id(id: &str) -> Option<u64> {
    reserve(entry(id, EntryKind::Folder, id), PathBuf::from("/tmp"))
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
        assert!(scheduler.current.is_none());
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
            scheduler.current.as_ref().map(|pending| pending.state),
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
    cancel_all();
    let (_, queued) = counts();
    assert_eq!(queued, 0, "排队中的批次直接放掉，不再占位");
    assert!(!is_pending("a") && !is_pending("b"), "释放后可以重新下");
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
fn only_one_batch_runs_at_a_time() {
    let _guard = lock();
    reset_for_test();
    reserve_id("a").expect("a");
    reserve_id("b").expect("b");
    let first = next_start().expect("第一批开跑");
    assert!(
        next_start().is_none(),
        "正在跑的那批没结束前，绝不取下一批（串行执行）"
    );
    // 它结束后才轮到 b
    assert!(release(first.batch).is_some());
    let second = next_start().expect("第二批顶上");
    assert_ne!(second.batch, first.batch);
    assert_eq!(second.entry.id, "b");
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
