//! 传输记录的落盘：`~/.config/kichi/transfers.json` 的读写、清理与恢复规则。
//!
//! 从 `transfers.rs` 拆出来守行数上限：那边管状态机与记录表，这边只管
//! 「一条记录怎么变成一行 JSON、读回来怎么挂回记录表、什么时候该丢」。
//! 决定见 `docs/adr/0014-download-history-on-disk.md`：
//!
//! - 整张记录表落盘（排队 / 下载中 / 完成 / 失败 / 取消都存），不设条数上限；
//! - 上一轮没跑完的行读回为「已中断」，保留「重试」入口；
//! - 清理：同批次超过 90 天、且不是已完成的整批删；文件超过 10 MB 时按整批
//!   丢最旧的已结束批次，直到 5 MB 以内；
//! - 写盘按 2 秒节流（结构变化与下载中的字节推进共用节拍），退出 / 登出强制写一次；
//!   写失败只记 stderr，不阻断下载。
//!
//! 不落盘：速度窗口、卡住时钟、说明存活状态——重启后从零开始。
//!
//! 锁顺序固定为「先 records，再 history」：记录表锁由调用方持有，写盘在锁外
//! 做（`std::fs` 是毫秒级 IO，不能挡住下载线程）。

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use kichi_core::EntryKind;
use serde::{Deserialize, Serialize};

use super::transfers::{
    is_folder_ledger, restore_row, with_records, Transfer, TransferDirection, TransferJob,
    TransferState, UploadDir, UploadJob,
};

/// 落盘格式版本。读取不看版本号：高版本也只按已知字段解析，未知字段忽略。
pub(crate) const HISTORY_VERSION: u32 = 3;
/// 两次写盘至少间隔这么久（进度回调很密，不能每次写盘）。
pub(crate) const WRITE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);
/// 失败 / 取消 / 已中断旧批次保留的天数。
pub(crate) const RETENTION_DAYS: i64 = 90;
const DAY_SECS: i64 = 24 * 3600;
/// 文件超过这个大小就要在启动时缩小。
pub(crate) const MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;
/// 缩小到这个大小以内（按整批丢最旧的已结束批次）。
pub(crate) const SHRINK_TO_BYTES: u64 = 5 * 1024 * 1024;

/// 磁盘上的一个条目：一条记录的全部可恢复字段。
///
/// 缺失字段一律取默认值（`#[serde(default)]`），手改坏了或版本更高的文件都不会挡住启动。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct EntryRecord {
    #[serde(default)]
    pub(crate) batch: u64,
    #[serde(default)]
    pub(crate) index: usize,
    #[serde(default)]
    pub(crate) child: bool,
    #[serde(default)]
    pub(crate) source_id: String,
    #[serde(default)]
    pub(crate) job: JobRecord,
    /// 状态名（`queued` / `running` / `done` …）；认不出的一律当已中断
    #[serde(default)]
    pub(crate) state: String,
    #[serde(default)]
    pub(crate) bytes: u64,
    #[serde(default)]
    pub(crate) total: Option<u64>,
    #[serde(default)]
    pub(crate) detail: String,
    #[serde(default)]
    pub(crate) path: Option<String>,
    #[serde(default)]
    pub(crate) empty_folder: bool,
    #[serde(default)]
    pub(crate) skipped: usize,
    /// 文件夹批次的目录树（相对下载根目录，含根文件夹名）
    #[serde(default)]
    pub(crate) dirs: Vec<Vec<String>>,
    /// 上传文件夹批次已经建好的云端目录（只写在父行上）
    #[serde(default)]
    pub(crate) upload_dirs: Vec<UploadDirRecord>,
    /// 上传文件夹批次当前创建失败的云端目录路径（只写在父行上）
    #[serde(default)]
    pub(crate) upload_failed_dirs: Vec<Vec<String>>,
    /// 批次入队时刻（Unix 秒）：90 天清理与整批重试保留原时刻
    #[serde(default)]
    pub(crate) created_unix: i64,
    /// 进程启动序号：识别「上一轮没跑完的行」
    #[serde(default)]
    pub(crate) session: u64,
    /// 正常退出时仍在进行，下一轮恢复登录态后自动续跑
    #[serde(default)]
    pub(crate) auto_resume: bool,
}

/// 重试种子在磁盘上的样子（与 `TransferJob` 一一对应）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct JobRecord {
    #[serde(default)]
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) name: String,
    /// `folder` / `file`；认不出当文件
    #[serde(default)]
    pub(crate) kind: String,
    #[serde(default)]
    pub(crate) size: u64,
    #[serde(default)]
    pub(crate) audit_flagged: bool,
    #[serde(default)]
    pub(crate) sub_dir: Vec<String>,
    #[serde(default)]
    pub(crate) dir: String,
    /// `download` / `upload`；旧记录缺字段时按下载处理
    #[serde(default)]
    pub(crate) direction: String,
    #[serde(default)]
    pub(crate) local_path: String,
    #[serde(default)]
    pub(crate) parent_id: String,
    #[serde(default)]
    pub(crate) parent_name: String,
    /// 上传文件夹根目录的云端 id
    #[serde(default)]
    pub(crate) batch_root_id: String,
    /// 上传子文件直接父目录的云端 id
    #[serde(default)]
    pub(crate) cloud_parent_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct UploadDirRecord {
    #[serde(default)]
    pub(crate) path: Vec<String>,
    #[serde(default)]
    pub(crate) id: String,
}

/// 落盘格式：`{ version, entries }`。
#[derive(Debug, Serialize, Deserialize)]
struct HistoryFile {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    entries: Vec<EntryRecord>,
}

/// 这一轮运行写哪个文件、用哪个会话号（启动时定下，之后不变）。
#[derive(Debug, Clone)]
pub(crate) struct StoreTarget {
    pub(crate) path: PathBuf,
    pub(crate) session: u64,
}

impl StoreTarget {
    /// 读到目标（`transfers.json`）；`KICHI_TRANSFERS_FILE` 可覆盖路径。
    pub(crate) fn from_env() -> Self {
        Self {
            path: crate::session::transfers_path(),
            session: 0,
        }
    }
}

/// 进程启动序号：以前写下的最大会话号 + 1。
///
/// 不用时间戳做标识：同一秒内重启两次会撞成同一个号，旧行就认不出是「上一轮」。
pub(crate) fn next_session(previous_max: u64) -> u64 {
    previous_max.saturating_add(1)
}

/// 该不该现在写盘：首次写、或者离上次写够 [`WRITE_INTERVAL`]。
pub(crate) fn write_due(last: Option<Instant>, now: Instant) -> bool {
    match last {
        None => true,
        Some(previous) => now.saturating_duration_since(previous) >= WRITE_INTERVAL,
    }
}

fn job_kind_name(kind: EntryKind) -> &'static str {
    match kind {
        EntryKind::Folder => "folder",
        EntryKind::File => "file",
    }
}

fn job_kind_of(name: &str) -> EntryKind {
    if name == "folder" {
        EntryKind::Folder
    } else {
        EntryKind::File
    }
}

/// 状态名 → 状态；认不出当已中断。
pub(crate) fn state_of(name: &str) -> TransferState {
    match name {
        "queued" => TransferState::Queued,
        "preparing" => TransferState::Preparing,
        "running" => TransferState::Running,
        "pausing" => TransferState::Pausing,
        "paused" => TransferState::Paused,
        "done" => TransferState::Done,
        "failed" => TransferState::Failed,
        "cancelled" => TransferState::Cancelled,
        _ => TransferState::Interrupted,
    }
}

// —— 记录 → 条目 ——

/// 一条记录 → 磁盘条目。`dirs` 只有文件夹总账行用得上。
pub(crate) fn entry_of(record: &Transfer, dirs: Vec<Vec<String>>) -> EntryRecord {
    EntryRecord {
        batch: record.batch,
        index: record.index,
        child: record.child,
        source_id: record.source_id.clone(),
        job: JobRecord {
            id: record.job.id.clone(),
            name: record.job.name.clone(),
            kind: job_kind_name(record.job.kind).to_owned(),
            size: record.job.size,
            audit_flagged: record.job.audit_flagged,
            sub_dir: record.job.sub_dir.clone(),
            dir: record.job.dir.display().to_string(),
            direction: record.job.direction.name().to_owned(),
            local_path: record
                .job
                .upload
                .as_ref()
                .map(|upload| upload.local_path.display().to_string())
                .unwrap_or_default(),
            parent_id: record
                .job
                .upload
                .as_ref()
                .map(|upload| upload.parent_id.clone())
                .unwrap_or_default(),
            parent_name: record
                .job
                .upload
                .as_ref()
                .map(|upload| upload.parent_name.clone())
                .unwrap_or_default(),
            batch_root_id: record
                .job
                .upload
                .as_ref()
                .map(|upload| upload.batch_root_id.clone())
                .unwrap_or_default(),
            cloud_parent_id: record
                .job
                .upload
                .as_ref()
                .map(|upload| upload.cloud_parent_id.clone())
                .unwrap_or_default(),
        },
        state: record.state.name().to_owned(),
        bytes: record.bytes,
        total: record.total,
        detail: record.detail.clone(),
        path: record.path.as_ref().map(|path| path.display().to_string()),
        empty_folder: record.empty_folder,
        skipped: record.skipped,
        dirs,
        upload_dirs: if is_folder_ledger(record) {
            record
                .upload_dirs
                .iter()
                .map(|dir| UploadDirRecord {
                    path: dir.path.clone(),
                    id: dir.id.clone(),
                })
                .collect()
        } else {
            Vec::new()
        },
        upload_failed_dirs: if is_folder_ledger(record) {
            record.upload_failed_dirs.clone()
        } else {
            Vec::new()
        },
        created_unix: record.created_unix,
        session: record.session,
        auto_resume: record.auto_resume,
    }
}

/// 整张记录表 → 条目（原样导出，清理不在这里做）。
///
/// 目录树属于整个文件夹批次，**只写在总账行上**：读回时的孤儿校验只认总账行
/// 那一份（[`batch_is_consistent`]）。给每条子行都克隆一份的话，300 个子文件
/// 的文件夹就往文件里写 300 份树，主线程每 2 秒整表重写一次，文件被撑到 10 MB
/// 后启动清理反而去丢更早的已完成批次。
pub(crate) fn entries_of(
    records: &[Transfer],
    dirs: &[(u64, Vec<Vec<String>>)],
) -> Vec<EntryRecord> {
    records
        .iter()
        .map(|record| {
            let tree = if is_folder_ledger(record) {
                dirs.iter()
                    .find(|(batch, _)| *batch == record.batch)
                    .map(|(_, tree)| tree.clone())
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            entry_of(record, tree)
        })
        .collect()
}

/// 落盘条目里的目录树（每个文件夹批次一份，挂在总账行上）。
///
/// 读回后挂进内存：写盘时还要用（否则重启后的第一次写盘会把它们丢掉），
/// 也保住孤儿校验的数据。整批重试本身不读它——重试会重新读一遍目录。
fn trees_of(entries: &[EntryRecord]) -> Vec<(u64, Vec<Vec<String>>)> {
    entries
        .iter()
        .filter(|entry| !entry.child && !entry.dirs.is_empty())
        .map(|entry| (entry.batch, entry.dirs.clone()))
        .collect()
}

// —— 条目 → 记录 ——

/// 批次里的行是不是与父行绑得上（孤儿修复用）。
///
/// 规则（`docs/adr/0014-download-history-on-disk.md`）：
/// - 同批次恰好一条顶层行（文件夹总账行或单文件行）；
/// - 文件夹总账行的每个子行都要落在它的目录树之下；目录树缺失时（旧文件 /
///   手改）退一步只要求子行有 `sub_dir`；
/// - 单文件行的批次不能出现子行。
fn batch_is_consistent(rows: &[&EntryRecord]) -> bool {
    if rows.iter().filter(|row| !row.child).count() != 1 {
        return false;
    }
    let Some(ledger) = rows.iter().find(|row| !row.child) else {
        return false;
    };
    let children: Vec<&&EntryRecord> = rows.iter().filter(|row| row.child).collect();
    if ledger.job.kind != "folder" {
        return children.is_empty();
    }
    children.iter().all(|child| {
        !child.job.sub_dir.is_empty()
            && (ledger.dirs.is_empty() || ledger.dirs.contains(&child.job.sub_dir))
    })
}

/// 恢复规则：下载的暂停请求（含线程尚未来得及收尾的「正在暂停」）保持暂停；
/// 上传暂停不跨进程，暂停态读回为已中断；其余没跑完或状态认不出的行 → 已中断。
pub(crate) fn recovered_state(stored: TransferState) -> TransferState {
    if matches!(stored, TransferState::Pausing | TransferState::Paused) {
        return TransferState::Paused;
    }
    if stored.is_active() || !stored.is_finished() {
        return TransferState::Interrupted;
    }
    stored
}

/// 读回并整理：孤儿批次丢弃、没跑完的行标已中断、批内排序稳定。
///
/// `session` 是这一轮的会话号；上一轮的 `queued / preparing / running` 一律读回
/// 已中断（不管它写的会话号是多少——上一次运行都已经结束了）。
pub(crate) fn restore(entries: Vec<EntryRecord>, session: u64) -> Vec<Transfer> {
    let mut batches: Vec<u64> = entries.iter().map(|entry| entry.batch).collect();
    batches.sort_unstable();
    batches.dedup();

    let mut restored = Vec::new();
    for batch in batches {
        let rows: Vec<&EntryRecord> = entries
            .iter()
            .filter(|entry| entry.batch == batch)
            .collect();
        if !batch_is_consistent(&rows) {
            continue;
        }
        let mut batch_rows: Vec<Transfer> = rows
            .iter()
            .map(|entry| restore_entry(entry, session))
            .collect();
        // 父行在前，子行按 index（统一列表顺序由视图层重新排）
        batch_rows.sort_by_key(|record| (record.child, record.index));
        restored.extend(batch_rows);
    }
    restored
}

fn restore_entry(entry: &EntryRecord, session: u64) -> Transfer {
    let direction = TransferDirection::from_name(&entry.job.direction);
    let job = TransferJob {
        id: entry.job.id.clone(),
        name: entry.job.name.clone(),
        kind: job_kind_of(&entry.job.kind),
        size: entry.job.size,
        audit_flagged: entry.job.audit_flagged,
        sub_dir: entry.job.sub_dir.clone(),
        dir: PathBuf::from(&entry.job.dir),
        direction,
        upload: (direction == TransferDirection::Upload).then(|| UploadJob {
            local_path: PathBuf::from(&entry.job.local_path),
            parent_id: entry.job.parent_id.clone(),
            parent_name: entry.job.parent_name.clone(),
            batch_root_id: entry.job.batch_root_id.clone(),
            cloud_parent_id: entry.job.cloud_parent_id.clone(),
        }),
    };
    let stored = state_of(&entry.state);
    let state = match (direction, recovered_state(stored)) {
        (TransferDirection::Upload, TransferState::Paused) => TransferState::Interrupted,
        (_, state) => state,
    };
    // 已中断行的说明由视图层按「上次退出时未完成」拼，这里不覆盖用户能看到的旧原因
    let detail = if state == TransferState::Interrupted && stored.is_active() {
        String::new()
    } else {
        entry.detail.clone()
    };
    let mut row = restore_row(
        entry.batch,
        entry.index,
        entry.child,
        &job,
        &entry.source_id,
        state,
        entry.bytes,
        entry.total,
        detail,
        entry.path.as_ref().map(PathBuf::from),
        entry.empty_folder,
        entry.skipped,
        entry.created_unix,
        session,
        entry.auto_resume,
    );
    row.upload_dirs = entry
        .upload_dirs
        .iter()
        .map(|dir| UploadDir {
            path: dir.path.clone(),
            id: dir.id.clone(),
        })
        .collect();
    row.upload_failed_dirs = entry.upload_failed_dirs.clone();
    row
}

// —— 清理规则（全是纯函数） ——

/// 现在（Unix 秒）。
pub(crate) fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// 这批的入队时刻（顶层行的 `created_unix`）。
pub(crate) fn created_of(records: &[Transfer], batch: u64) -> i64 {
    records
        .iter()
        .filter(|record| record.batch == batch)
        .map(|record| record.created_unix)
        .filter(|created| *created > 0)
        .min()
        .unwrap_or(0)
}

/// 超过保留期、且不包含任何已完成 / 暂停行的整批（升序）。
///
/// 已完成的批次是用户看得见的结果，不因为放了三个月就消失；
/// 暂停的批次还要让用户继续，也不能悄悄丢；只有失败 / 取消 / 已中断的旧批次
/// 在启动时整批删。没有时间戳（手改 / 更早的文件）的批次不因缺信息被误删。
pub(crate) fn expired_batches(records: &[Transfer], now: i64, days: i64) -> Vec<u64> {
    let cutoff = now.saturating_sub(days.saturating_mul(DAY_SECS));
    let mut batches: Vec<u64> = records.iter().map(|record| record.batch).collect();
    batches.sort_unstable();
    batches.dedup();
    batches.retain(|batch| {
        let created = created_of(records, *batch);
        let has_done = records
            .iter()
            .any(|record| record.batch == *batch && record.state == TransferState::Done);
        let has_paused = records.iter().any(|record| {
            record.batch == *batch
                && matches!(record.state, TransferState::Pausing | TransferState::Paused)
        });
        created > 0 && created < cutoff && !has_done && !has_paused
    });
    batches
}

/// 记录表删掉这些整批（过期清理 / 启动时缩小）。
pub(crate) fn drop_batches(records: &mut Vec<Transfer>, batches: &[u64]) {
    if batches.is_empty() {
        return;
    }
    records.retain(|record| !batches.contains(&record.batch));
}

/// 条目序列化后的大致字节数（每条固定开销 + 名字长度估算，够做阈值判断）。
fn estimate_size<'a>(entries: impl Iterator<Item = &'a EntryRecord>) -> u64 {
    // 每条记录的固定字段开销（键名 + 数字 + 标点）；名字与目录树另算
    const PER_ENTRY: u64 = 220;
    entries
        .map(|entry| {
            let chars: usize = entry.job.id.len()
                + entry.job.name.len()
                + entry.job.dir.len()
                + entry.source_id.len()
                + entry.detail.len()
                + entry.path.as_deref().map(str::len).unwrap_or(0)
                + entry.job.sub_dir.iter().map(String::len).sum::<usize>()
                + entry
                    .dirs
                    .iter()
                    .map(|dir| dir.iter().map(String::len).sum::<usize>())
                    .sum::<usize>();
            PER_ENTRY.saturating_add(u64::try_from(chars).unwrap_or(u64::MAX))
        })
        .fold(0u64, u64::saturating_add)
}

/// 所有行都已结束的批次（升序）：完成 / 失败 / 取消。
fn finished_batches_of(entries: &[&EntryRecord]) -> Vec<u64> {
    let mut batches: Vec<u64> = entries.iter().map(|entry| entry.batch).collect();
    batches.sort_unstable();
    batches.dedup();
    batches.retain(|batch| {
        entries
            .iter()
            .filter(|entry| entry.batch == *batch)
            .all(|entry| state_of(&entry.state).is_finished())
    });
    batches
}

/// 文件超过上限时该丢哪些整批：按批号从旧到新丢已结束批次，直到估算大小 `shrink_to` 以内。
///
/// 进行中 / 已中断的批次不丢（用户还要重试）；丢到一个批次都不剩时停手，
/// 不把唯一的记录删干净。
pub(crate) fn oversized_batches(
    entries: &[&EntryRecord],
    max_bytes: u64,
    shrink_to: u64,
) -> Vec<u64> {
    if estimate_size(entries.iter().copied()) <= max_bytes {
        return Vec::new();
    }
    let mut remaining: Vec<&EntryRecord> = entries.to_vec();
    let mut dropped = Vec::new();
    for batch in finished_batches_of(entries) {
        if estimate_size(remaining.iter().copied()) <= shrink_to {
            break;
        }
        let kept: Vec<&EntryRecord> = remaining
            .iter()
            .copied()
            .filter(|entry| entry.batch != batch)
            .collect();
        if kept.is_empty() {
            break;
        }
        remaining = kept;
        dropped.push(batch);
    }
    dropped
}

// —— 读盘 / 写盘 ——

/// 读一个历史文件；不存在 / 损坏 / 读失败都回空表（不让历史文件挡住启动）。
pub(crate) fn load(path: &Path) -> Vec<EntryRecord> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(err) => {
            eprintln!("[kichi] 读取传输历史失败（本次按空历史启动）：{err}");
            return Vec::new();
        }
    };
    match serde_json::from_slice::<HistoryFile>(&bytes) {
        Ok(file) => file.entries,
        Err(err) => {
            eprintln!("[kichi] 传输历史文件损坏（本次按空历史启动）：{err}");
            Vec::new()
        }
    }
}

/// 把条目写成 `{ version, entries }`：先写 `.json.tmp` 再 rename，失败删 tmp。
pub(crate) fn save(path: &Path, entries: &[EntryRecord]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| format!("创建配置目录失败：{err}"))?;
    }
    // 紧凑格式：这是内部状态镜像，没人手读；缩进实测每条约多 230 字节，
    // 而 10 MB 的清理线是按实际文件大小卡的。
    let body = serde_json::to_vec(&HistoryFile {
        version: HISTORY_VERSION,
        entries: entries.to_vec(),
    })
    .map_err(|err| format!("序列化传输历史失败：{err}"))?;
    let temp = path.with_extension("json.tmp");
    write_private(&temp, &body).map_err(|err| format!("写入传输历史失败：{err}"))?;
    std::fs::rename(&temp, path).map_err(|err| {
        let _ = std::fs::remove_file(&temp);
        format!("保存传输历史失败：{err}")
    })
}

/// 写一个只有本人可读写的文件（0600）。与 `partials.rs` 同一做法。
fn write_private(path: &Path, body: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(body)?;
    file.flush()
}

// —— 记录表 ↔ 磁盘 ——

/// 记录表 → QML 顶层统一列表模型 + 进行中条数。
pub(crate) fn snapshot() -> (crate::cxxqt_object::QVariantList, i32) {
    with_records(|records| crate::cxxqt_object::transfers_view::build_snapshot(records))
}

/// 全部历史里有没有「所有行都已结束」的批次（清空按钮据此置灰，不按日期收窄）。
pub(crate) fn has_finished() -> bool {
    with_records(|records| {
        let mut batches: Vec<u64> = records.iter().map(|record| record.batch).collect();
        batches.sort_unstable();
        batches.dedup();
        batches.iter().any(|batch| {
            records
                .iter()
                .filter(|record| record.batch == *batch)
                .all(|record| record.state.is_finished())
        })
    })
}

/// 写盘节流用的「上次写盘时刻」（主线程独占）。
fn last_write() -> &'static Mutex<Option<Instant>> {
    static LAST: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();
    LAST.get_or_init(|| Mutex::new(None))
}

fn with_last_write<T>(f: impl FnOnce(&mut Option<Instant>) -> T) -> T {
    let mut guard: MutexGuard<'static, Option<Instant>> = last_write()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    f(&mut guard)
}

/// 当前记录表 → 磁盘（强制写一次，不节流）。写失败只返回原因。
pub(crate) fn write_now(target: &StoreTarget) -> Result<(), String> {
    // 锁顺序：先 records、再 forest（见 `transfers.rs` 的锁顺序说明）
    let entries = with_records(|records| {
        let trees: Vec<(u64, Vec<Vec<String>>)> = super::transfers::with_trees(|trees| {
            trees
                .iter()
                .map(|(batch, tree)| (*batch, tree.clone()))
                .collect()
        });
        entries_of(records, &trees)
    });
    save(&target.path, &entries)
}

/// 主线程节流写盘：结构同步时调用，最多 [`WRITE_INTERVAL`] 写一次。
pub(crate) fn maybe_flush(target: &StoreTarget) {
    let now = Instant::now();
    let due = with_last_write(|last| {
        if !write_due(*last, now) {
            return false;
        }
        *last = Some(now);
        true
    });
    if !due {
        return;
    }
    if let Err(err) = write_now(target) {
        eprintln!("[kichi] 传输历史未能落盘（本次仅失去跨重启记录）：{err}");
    }
}

/// 上次写盘时的进度指纹（见 [`super::transfers::progress_stamp_in`]）。
fn last_progress_stamp() -> &'static Mutex<Option<u64>> {
    static LAST: OnceLock<Mutex<Option<u64>>> = OnceLock::new();
    LAST.get_or_init(|| Mutex::new(None))
}

fn with_last_progress_stamp<T>(f: impl FnOnce(&mut Option<u64>) -> T) -> T {
    let mut guard: MutexGuard<'static, Option<u64>> = last_progress_stamp()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    f(&mut guard)
}

/// 该不该为进度写盘：节流到点，**且**字节指纹确实变了。
///
/// 下载卡住时心跳会反复刷新进度行，指纹没变就不该重写整张表。抽成纯函数
/// 方便单测；真实调用只发生在主线程（`sync_transfer_live`）。
pub(crate) fn progress_write_due(
    last: Option<Instant>,
    now: Instant,
    saved_stamp: Option<u64>,
    stamp: u64,
) -> bool {
    write_due(last, now) && saved_stamp != Some(stamp)
}

/// 进度触发的节流写盘（下载中由 `sync_transfer_live` 调用）。
///
/// 结构事件之外的唯一写盘口。不写的话，纯进度阶段被 SIGKILL / 直接注销后，
/// 重启的「已中断」行会退回最后一次结构事件的进度（常见停在 0）。
pub(crate) fn maybe_flush_progress(target: &StoreTarget) {
    let now = Instant::now();
    let stamp = super::transfers::progress_stamp();
    let saved = with_last_progress_stamp(|slot| *slot);
    let due = with_last_write(|last| {
        if !progress_write_due(*last, now, saved, stamp) {
            return false;
        }
        *last = Some(now);
        true
    });
    if !due {
        return;
    }
    with_last_progress_stamp(|slot| *slot = Some(stamp));
    if let Err(err) = write_now(target) {
        eprintln!("[kichi] 传输历史未能落盘（本次仅失去跨重启记录）：{err}");
    }
}

/// 强制写一次（退出 / 登出前调用），并记住这一刻。
pub(crate) fn flush_now(target: &StoreTarget) {
    with_last_write(|last| *last = Some(Instant::now()));
    if let Err(err) = write_now(target) {
        eprintln!("[kichi] 传输历史未能落盘（本次仅失去跨重启记录）：{err}");
    }
}

/// 启动时加载一次：孤儿修复 → 过期清理 → 挂回记录表 → 批次计数器接上历史最大值。
///
/// 返回这一轮运行时用的目标（路径 + 会话号）。
pub(crate) fn boot(target: &StoreTarget) -> StoreTarget {
    let entries = load(&target.path);
    let previous_max = entries.iter().map(|entry| entry.session).max().unwrap_or(0);
    let session = next_session(previous_max);
    // 目录树随总账行落盘；`restore` 会吃掉 entries，先把树取出来
    let trees = trees_of(&entries);
    let restored = restore(entries, session);
    let now = now_unix();

    let max_batch = with_records(|records| {
        if !records.is_empty() {
            // 理论上启动时表是空的；真不空就以内存为准（历史文件只是镜像）
            return records.iter().map(|record| record.batch).max().unwrap_or(0);
        }
        let mut restored = restored;
        let expired = expired_batches(&restored, now, RETENTION_DAYS);
        drop_batches(&mut restored, &expired);
        let max_batch = restored
            .iter()
            .map(|record| record.batch)
            .max()
            .unwrap_or(0);
        // 锁顺序：先 records、再 trees（与 `write_now` 一致）
        super::transfers::restore_trees(trees, &restored);
        records.extend(restored);
        max_batch
    });
    // 批次号从历史最大值之后接着走，避免 key 冲突
    super::transfers::raise_batch_counter(max_batch);

    let session_target = StoreTarget {
        path: target.path.clone(),
        session,
    };
    shrink_oversized(&session_target);
    session_target
}

/// 文件太大：按整批丢最旧的已结束批次，直到 `SHRINK_TO_BYTES` 以内并写回。
///
/// 只在启动做：清理不在下载节拍里跑。
fn shrink_oversized(target: &StoreTarget) {
    let size = std::fs::metadata(&target.path)
        .map(|meta| meta.len())
        .unwrap_or(0);
    if size <= MAX_FILE_BYTES {
        return;
    }
    let entries = load(&target.path);
    let refs: Vec<&EntryRecord> = entries.iter().collect();
    let dropped = oversized_batches(&refs, MAX_FILE_BYTES, SHRINK_TO_BYTES);
    if dropped.is_empty() {
        return;
    }
    with_records(|records| drop_batches(records, &dropped));
    // 整批没了，目录树也别留着（锁外调用：prune 自己先取 records 再取 trees）
    super::transfers::prune_trees();
    if let Err(err) = write_now(target) {
        eprintln!("[kichi] 整理过大的传输历史失败：{err}");
    }
}

#[cfg(test)]
#[path = "transfers_history_tests.rs"]
mod tests;
