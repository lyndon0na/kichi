//! 进度、说明、完成状态与实时视图字段。

use std::path::{Path, PathBuf};
use std::time::Instant;

use cxx_qt_lib::QVariant;
use kichi_core::EntryKind;

use crate::cxxqt_object::transfers_rate::{NoteState, SpeedMeter};
use crate::cxxqt_object::transfers_view::live_map_at;
use crate::cxxqt_object::QVariantList;

use super::*;

/// 进度更新（只在 Running 时接受，取消后迟到的回调不复活记录）。
pub(crate) fn progress(batch: u64, index: usize, bytes: u64, total: Option<u64>) {
    with_records(|records| progress_in(records, batch, index, bytes, total));
}

/// 上传阶段与进度。进度条按当前阶段自己的总量计算；上传阶段的总量就是文件大小。
pub(crate) fn upload_progress(batch: u64, index: usize, progress: UploadProgressState) {
    with_records(|records| upload_progress_in(records, batch, index, progress));
}

/// 下载中的状态提示（续传 / 重试原因）；只写进行中行的 detail。
///
/// 走 `post_live()` 下发（见 `queue_worker`）：它是下载中的高频说明，
/// 不属于结构变化，跟着 `transferLive` 贴到行上，不重建列表。
pub(crate) fn note(batch: u64, index: usize, text: &str) {
    with_records(|records| note_in(records, batch, index, text));
}

/// 下载成功：记下落盘路径。
pub(crate) fn finish_ok(batch: u64, index: usize, path: &Path) {
    with_records(|records| finish_ok_in(records, batch, index, path));
}

/// 下载失败：记下原因。
pub(crate) fn finish_err(batch: u64, index: usize, reason: &str) {
    with_records(|records| finish_err_in(records, batch, index, reason));
}

/// 所有正在下载文件的实时进度（各文件行 + 文件夹父行聚合）。
///
/// 单独成一路属性：进度每 250ms 变一次，若跟着整个模型重建，`ListView`
/// 会整列表重灌、滚动位置回到顶部（2026-10-27 实测），下载期间就没法翻看了。
/// QML 按 key 取对应行；父行聚合和展开的子行都从这一份取速度 / 进度。
pub(crate) fn live_snapshot() -> QVariantList {
    live_snapshot_at(Instant::now())
}

/// [`live_snapshot`] 的注入时间版本（测试用；生产走 `Instant::now()`）。
///
/// 速度 / 卡住 / 说明都由 `now` 现算：卡住秒数没有进度回调也能靠主线程
/// 1Hz 心跳（[`super::downloads`] 的 `maybe_heartbeat_live`）继续走。
pub(crate) fn live_snapshot_at(now: Instant) -> QVariantList {
    with_records(|records| {
        let mut list = QVariantList::default();
        // 批内并发与跨批并发都可能同时出现多条「下载中」：每一条都要下发自己的
        // 实时进度，否则除第一条外的父行 / 子行会停在开始时的值。
        let mut parent_batches = Vec::new();
        for running in records.iter().filter(|record| {
            record.state == TransferState::Running && record.job.kind == EntryKind::File
        }) {
            list.append(QVariant::from(&live_map_at(running, now)));
            if !running.child || parent_batches.contains(&running.batch) {
                continue;
            }
            parent_batches.push(running.batch);
            if let Some(parent) = records
                .iter()
                .find(|record| record.batch == running.batch && is_folder_ledger(record))
            {
                list.append(QVariant::from(
                    &crate::cxxqt_object::transfers_view::parent_live_map_at(parent, records, now),
                ));
            }
        }
        list
    })
}

/// 按 `(batch, index)` 找文件行：必须过滤掉文件夹总账行。
///
/// 父行与第一个子行都可能是 `(batch, 0)`；只看坐标会误把父行当文件。
/// 已中断的行不再收下载线程的迟到回调（那一轮下载已经不存在了）。
pub(crate) fn find_file(
    records: &mut [Transfer],
    batch: u64,
    index: usize,
) -> Option<&mut Transfer> {
    records.iter_mut().find(|record| {
        record.job.kind == EntryKind::File
            && record.batch == batch
            && record.index == index
            && record.state != TransferState::Interrupted
    })
}

/// 改一条文件行，并把它的状态 / 字节变化增量并入父行缓存。
///
/// `allow_interrupted` 只给「重试」用；下载线程的迟到回调仍不能复活已中断行。
pub(crate) fn update_child_record(
    records: &mut [Transfer],
    batch: u64,
    index: usize,
    allow_interrupted: bool,
    change: impl FnOnce(&mut Transfer) -> bool,
) -> bool {
    let Some(position) = records.iter().position(|record| {
        record.job.kind == EntryKind::File
            && record.batch == batch
            && record.index == index
            && (allow_interrupted || record.state != TransferState::Interrupted)
    }) else {
        return false;
    };
    let before = ChildStats::of(&records[position]);
    if !change(&mut records[position]) {
        return false;
    }
    let after = ChildStats::of(&records[position]);
    let state_changed = before.state != after.state;
    update_child_stats(records, batch, before, after);
    if state_changed {
        refresh_parent_state(records, batch);
    }
    true
}

/// 一条新记录（排队态）。`index` 在文件夹展开时由外层决定。
pub(crate) fn row_from(
    batch: u64,
    index: usize,
    source_id: &str,
    job: &TransferJob,
    created_unix: i64,
    session: u64,
) -> Transfer {
    Transfer {
        batch,
        index,
        child: false,
        job: job.clone(),
        source_id: source_id.to_owned(),
        state: TransferState::Queued,
        bytes: 0,
        total: (job.size > 0).then_some(job.size),
        detail: String::new(),
        path: None,
        speed: SpeedMeter::default(),
        note: NoteState::default(),
        empty_folder: false,
        skipped: 0,
        created_unix,
        session,
        auto_resume: false,
        upload_progress: None,
        upload_dirs: Vec::new(),
        upload_failed_dirs: Vec::new(),
        batch_stats: BatchStats::default(),
    }
}

pub(crate) fn progress_in(
    records: &mut [Transfer],
    batch: u64,
    index: usize,
    bytes: u64,
    total: Option<u64>,
) {
    progress_in_at(records, batch, index, bytes, total, Instant::now());
}

/// [`progress_in`] 的注入时间版本（测试用）。
pub(crate) fn progress_in_at(
    records: &mut [Transfer],
    batch: u64,
    index: usize,
    bytes: u64,
    total: Option<u64>,
    now: Instant,
) {
    update_child_record(records, batch, index, false, |record| {
        progress_record(record, now, bytes, total)
    });
}

/// 进行中行的字节指纹：判断「上次写盘后有没有真的走到新字节」。
///
/// 只用于进度写盘的节流决策，不参与序列化，也不需要抗碰撞：下载卡住时
/// 心跳会反复调 `sync_transfer_live()`，指纹没变就不必重写整张表。
pub(crate) fn progress_stamp_in(records: &[Transfer]) -> u64 {
    records.iter().fold(0u64, |stamp, record| {
        if record.state != TransferState::Running {
            return stamp;
        }
        stamp
            .wrapping_mul(1_000_003)
            .wrapping_add(record.bytes)
            .wrapping_add(record.index as u64)
            .wrapping_add(record.batch)
    })
}

/// 当前记录表的进度指纹（见 [`progress_stamp_in`]）。
pub(crate) fn progress_stamp() -> u64 {
    with_records(|records| progress_stamp_in(records))
}

/// 单条记录的进度：写字节数，同时喂速度窗口与说明状态。
pub(crate) fn progress_record(
    record: &mut Transfer,
    now: Instant,
    bytes: u64,
    total: Option<u64>,
) -> bool {
    if record.state != TransferState::Running {
        return false;
    }
    let before = (record.bytes, record.total);
    record.bytes = bytes;
    record.total = total.or(record.total);
    record.speed.observe(now, bytes);
    record.note.observe(now, bytes);
    before != (record.bytes, record.total)
}

pub(crate) fn note_in(records: &mut [Transfer], batch: u64, index: usize, text: &str) {
    note_in_at(records, batch, index, text, Instant::now());
}

/// [`note_in`] 的注入时间版本（测试用）：写 `detail` 的同时重置说明存活计时。
pub(crate) fn note_in_at(
    records: &mut [Transfer],
    batch: u64,
    index: usize,
    text: &str,
    now: Instant,
) {
    if let Some(record) = find_file(records, batch, index) {
        note_record(record, now, text);
    }
}

/// 单条记录的重试 / 续传说明：未恢复前一直显示，恢复后 5 秒消失。
pub(crate) fn note_record(record: &mut Transfer, now: Instant, text: &str) {
    if record.state != TransferState::Running {
        return;
    }
    record.detail = text.to_owned();
    record.note.write(now, record.bytes);
}

pub(crate) fn finish_ok_in(records: &mut [Transfer], batch: u64, index: usize, path: &Path) {
    // 用户点取消后，当前文件仍可能刚好下载完整：文件已在磁盘上，照实记成完成
    update_child_record(records, batch, index, false, |record| {
        if record.state != TransferState::Done {
            record.state = TransferState::Done;
            record.bytes = record.total.unwrap_or(record.bytes).max(record.bytes);
            record.detail.clear();
            record.path = Some(path.to_path_buf());
            true
        } else {
            false
        }
    });
}

pub(crate) fn finish_err_in(records: &mut [Transfer], batch: u64, index: usize, reason: &str) {
    update_child_record(records, batch, index, false, |record| {
        if matches!(
            record.state,
            TransferState::Queued | TransferState::Preparing | TransferState::Running
        ) {
            record.state = TransferState::Failed;
            record.detail = reason.to_owned();
            true
        } else {
            false
        }
    });
}

pub(crate) fn cancel_active_in(records: &mut [Transfer], batch: Option<u64>, reason: &str) {
    for record in records.iter_mut() {
        if batch.is_some_and(|batch| record.batch != batch) {
            continue;
        }
        if record.state.is_cancellable() {
            record.state = TransferState::Cancelled;
            record.detail = reason.to_owned();
        }
    }
    rebuild_all_batch_stats(records);
}

pub(crate) fn cancel_active_for_in(
    records: &mut [Transfer],
    direction: TransferDirection,
    reason: &str,
) {
    for record in records
        .iter_mut()
        .filter(|record| record.job.direction == direction)
    {
        if record.state.is_cancellable() {
            record.state = TransferState::Cancelled;
            record.detail = reason.to_owned();
        }
    }
    rebuild_all_batch_stats(records);
}

/// 文件夹总账行对应的落盘文件夹：下载目录 + 安全化后的文件夹名。
pub(crate) fn folder_download_path(record: &Transfer) -> Option<PathBuf> {
    (is_folder_ledger(record) && !record.job.is_upload()).then(|| {
        record
            .job
            .dir
            .join(crate::session::download::safe_file_name(&record.job.name))
    })
}

/// 这个批次是否已经有子文件落盘（父行的「打开位置」按此给出文件夹）。
pub(crate) fn has_done_child(records: &[Transfer], batch: u64) -> bool {
    records
        .iter()
        .any(|record| record.batch == batch && record.child && record.state == TransferState::Done)
}
