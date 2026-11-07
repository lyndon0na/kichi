//! 排队、文件夹展开、开始、失败收尾与自动续跑。

use std::path::PathBuf;
use std::time::Instant;

use crate::cxxqt_object::transfers_rate::{NoteState, SpeedMeter};

use super::*;

/// 整批登记。真正下载的一行一个 [`TransferJob`]（文件夹总账行只有一条）。
///
/// 排队阶段只登记传给它的行；文件夹批次轮到读目录后用 [`expand_folder`]
/// 展开成整批。
pub(crate) fn start_batch(batch: u64, source_id: &str, job: &TransferJob) {
    let created = crate::cxxqt_object::transfers_history::now_unix();
    let session = current_session();
    with_records(|records| push_job(records, batch, source_id, job, created, session));
    // 记录数超上限时 `push_job` 会淘汰整批，被淘汰批次的目录树跟着收走
    prune_trees();
}

/// 轮到读目录：把这一批的总账行标成「准备中」。
pub(crate) fn mark_preparing(batch: u64) {
    with_records(|records| mark_preparing_in(records, batch));
}

/// 整个文件夹批次读完了：在总账行后面挂上整批的子文件行。
///
/// 父行留在原位置（统一列表一个文件夹只占一行）；子行从它后面依次插入，
/// 排队中的其他批次会自然地排在它下面。`dirs` 是相对下载根目录的目录树，
/// 随总账行落盘（读回时的孤儿校验用；整批重试会重新读目录）。
pub(crate) fn expand_folder(batch: u64, jobs: &[TransferJob], dirs: &[Vec<String>]) {
    let created = crate::cxxqt_object::transfers_history::now_unix();
    let session = current_session();
    // 锁顺序：先 records、再 trees
    with_records(|records| {
        if let Some(created_unix) = records
            .iter()
            .find(|record| record.batch == batch && is_folder_ledger(record))
            .map(|record| record.created_unix)
        {
            expand_folder_in(records, batch, jobs, created_unix, session);
        } else {
            expand_folder_in(records, batch, jobs, created, session);
        }
    });
    with_trees(|trees| {
        trees.insert(batch, dirs.to_vec());
    });
    // `expand_folder_in` 也会 trim；被淘汰的批次目录树跟着收走（新批次在记录表里，留着）
    prune_trees();
}

/// 上传文件夹扫描完成、等上传调度器接手：父行从「准备中」落成「排队中」。
pub(crate) fn mark_folder_queued(batch: u64) {
    with_records(|records| {
        for record in records
            .iter_mut()
            .filter(|record| record.batch == batch && is_folder_ledger(record))
        {
            if record.state == TransferState::Preparing {
                record.state = TransferState::Queued;
                record.detail.clear();
            }
        }
    });
}

/// 读回一条历史行（`transfers_history::restore` 用）：不跑任何状态迁移。
#[allow(clippy::too_many_arguments)]
pub(crate) fn restore_row(
    batch: u64,
    index: usize,
    child: bool,
    job: &TransferJob,
    source_id: &str,
    state: TransferState,
    bytes: u64,
    total: Option<u64>,
    detail: String,
    path: Option<PathBuf>,
    empty_folder: bool,
    skipped: usize,
    created_unix: i64,
    session: u64,
    auto_resume: bool,
) -> Transfer {
    Transfer {
        batch,
        index,
        child,
        job: job.clone(),
        source_id: source_id.to_owned(),
        state,
        bytes,
        total: total.or((job.size > 0).then_some(job.size)),
        detail,
        path,
        speed: SpeedMeter::default(),
        note: NoteState::default(),
        empty_folder,
        skipped,
        created_unix,
        session,
        auto_resume,
        upload_progress: None,
        upload_dirs: Vec::new(),
        upload_failed_dirs: Vec::new(),
        batch_stats: BatchStats::default(),
    }
}

/// 一批整体没跑起来（收集失败 / 线程起不来 / 根目录建不出来）：标失败。
pub(crate) fn fail_batch(batch: u64, reason: &str) {
    with_records(|records| fail_batch_in(records, batch, reason));
}

/// 文件夹收集阶段记下的跳过数（展开子行之后写，空文件夹也写）。
pub(crate) fn set_skipped(batch: u64, skipped: usize) {
    with_records(|records| set_skipped_in(records, batch, skipped));
}

/// 下载阶段又跳过了几个目录（本地空目录创建失败）。
pub(crate) fn add_skipped(batch: u64, extra: usize) {
    if extra == 0 {
        return;
    }
    with_records(|records| add_skipped_in(records, batch, extra));
}

/// 空文件夹批次结束：父行直接标完成，落盘路径 = 下载目录 + 文件夹名。
///
/// 这一批没有子行，[`refresh_parent_state`] 不会碰它；取消过的行不改。
pub(crate) fn finish_folder_empty(batch: u64) {
    with_records(|records| finish_folder_empty_in(records, batch));
}

/// 一条开始下载。
pub(crate) fn begin(batch: u64, index: usize) {
    with_records(|records| begin_in(records, batch, index));
}

pub(crate) fn push_job(
    records: &mut Vec<Transfer>,
    batch: u64,
    source_id: &str,
    job: &TransferJob,
    created_unix: i64,
    session: u64,
) {
    records.push(row_from(batch, 0, source_id, job, created_unix, session));
    trim(records);
}

/// 把整批子行挂到文件夹总账行后面（总账行留在原位，状态仍是「准备中」）。
///
/// 不删父行：统一列表永远只有一条文件夹批次行；子行只在展开时展示。
/// 子行从父行后面依次插入，后面排队的批次位置不受影响。
pub(crate) fn expand_folder_in(
    records: &mut Vec<Transfer>,
    batch: u64,
    jobs: &[TransferJob],
    created_unix: i64,
    session: u64,
) {
    let Some(at) = records
        .iter()
        .position(|record| record.batch == batch && is_folder_ledger(record))
    else {
        return;
    };
    let source_id = records[at].source_id.clone();
    let rows: Vec<Transfer> = jobs
        .iter()
        .enumerate()
        .map(|(index, job)| {
            let mut row = row_from(batch, index, &source_id, job, created_unix, session);
            row.child = true;
            row
        })
        .collect();
    records.splice(at + 1..at + 1, rows);
    rebuild_batch_stats(records, batch);
    trim(records);
}

pub(crate) fn begin_in(records: &mut [Transfer], batch: u64, index: usize) {
    begin_in_at(records, batch, index, Instant::now());
}

/// [`begin_in`] 的注入时间版本（测试用）。
pub(crate) fn begin_in_at(records: &mut [Transfer], batch: u64, index: usize, now: Instant) {
    update_child_record(records, batch, index, false, |record| {
        begin_record(record, now)
    });
}

/// 一条记录开始下载：状态转「下载中」，卡住时钟落在 `now`（还没收到字节也算）。
pub(crate) fn begin_record(record: &mut Transfer, now: Instant) -> bool {
    if record.state != TransferState::Queued {
        return false;
    }
    record.state = TransferState::Running;
    record.speed.begin(now);
    true
}

pub(crate) fn mark_preparing_in(records: &mut [Transfer], batch: u64) {
    // 只有文件夹总账行需要「准备中」；单文件批次直接 排队中 → 下载中
    for record in records
        .iter_mut()
        .filter(|record| record.batch == batch && is_folder_ledger(record))
    {
        if record.state == TransferState::Queued {
            record.state = TransferState::Preparing;
        }
    }
}

pub(crate) fn fail_batch_in(records: &mut [Transfer], batch: u64, reason: &str) {
    for record in records.iter_mut().filter(|record| record.batch == batch) {
        if matches!(
            record.state,
            TransferState::Queued
                | TransferState::Preparing
                | TransferState::Running
                | TransferState::Pausing
        ) {
            record.state = TransferState::Failed;
            record.detail = reason.to_owned();
        }
    }
    rebuild_batch_stats(records, batch);
}

/// 跳过数只记在文件夹总账行上（QML 的说明行从这一行取）。
pub(crate) fn set_skipped_in(records: &mut [Transfer], batch: u64, skipped: usize) {
    for record in records.iter_mut().filter(|record| record.batch == batch) {
        if is_folder_ledger(record) {
            record.skipped = skipped;
        }
    }
}

pub(crate) fn add_skipped_in(records: &mut [Transfer], batch: u64, extra: usize) {
    for record in records.iter_mut().filter(|record| record.batch == batch) {
        if is_folder_ledger(record) {
            record.skipped += extra;
        }
    }
}

pub(crate) fn finish_folder_empty_in(records: &mut [Transfer], batch: u64) {
    let Some(parent) = records
        .iter_mut()
        .find(|record| record.batch == batch && is_folder_ledger(record))
    else {
        return;
    };
    if parent.state == TransferState::Cancelled || parent.state == TransferState::Interrupted {
        return;
    }
    // 路径要在改字段前算好（函数借的是记录本身）
    let path = folder_download_path(parent);
    parent.state = TransferState::Done;
    parent.empty_folder = true;
    parent.path = path;
    parent.detail.clear();
}
