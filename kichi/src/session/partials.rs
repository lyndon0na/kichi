//! 下载断点索引：跨重启继续未完成的 `.kichi-part`。
//!
//! 位置 `~/.config/kichi/partials.json`（`KICHI_PARTIALS_FILE` 可覆盖），0600，
//! 先写 `.tmp` 再 rename。键是**云端文件 id**，不是文件名：同名文件可以来自
//! 不同目录；`part` / `final` 存绝对路径，续传只接受 part 位于本次下载根目录
//! 之下的条目（含文件夹批次的子目录），避免把断点续到别的目录。
//!
//! 这个模块只管理索引与纯决策，不碰网络、不碰 Qt；真正的下载循环在
//! `session/download.rs`。索引写失败不阻断下载（只是失去跨重启能力），
//! 调用方选择记日志而不是把用户挡在下载前面。

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::{paths, private_fs};

/// 索引格式版本。
///
/// v1 只有顺序追加的 part；v2 增加 `shards`（单文件多分片的逐片进度）。
/// 读取不检查版本号：v1 条目 `shards` 为空，继续走原来的顺序续传。
pub const PARTIALS_VERSION: u32 = 2;

/// 一个分片的进度；区间是 `[start, end)`（不包含 end）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShardSpan {
    pub start: u64,
    pub end: u64,
    /// 这一片已经写进 part 的字节数（相对 `start`）
    #[serde(default)]
    pub done: u64,
}

impl ShardSpan {
    pub fn len(&self) -> u64 {
        self.end.saturating_sub(self.start)
    }

    pub fn is_empty(&self) -> bool {
        self.start >= self.end
    }
}

/// 索引里的一条断点。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartialEntry {
    /// 云端文件 id（判重键）
    pub id: String,
    /// `.kichi-part` 的绝对路径
    pub part: String,
    /// 完成后的正式路径（续传完成时仍不覆盖，冲突会改序号）
    #[serde(rename = "final")]
    pub final_path: String,
    /// 上次已知的完整大小；`None` = 当时不知道
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
    /// 最后更新（Unix 秒）；只用于排障，不做自动过期
    pub updated_unix: i64,
    /// v2 分片进度；空 = v1 式顺序追加（旧断点照旧能续）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shards: Vec<ShardSpan>,
}

impl PartialEntry {
    /// 建一条断点记录（`updated_unix` 取当前时间）。
    pub fn new(id: &str, part: &Path, final_path: &Path, total: Option<u64>) -> Self {
        Self {
            id: id.to_owned(),
            part: part.display().to_string(),
            final_path: final_path.display().to_string(),
            total,
            updated_unix: now_unix(),
            shards: Vec::new(),
        }
    }

    /// 建一条带分片进度的 v2 断点记录。
    pub fn new_sharded(
        id: &str,
        part: &Path,
        final_path: &Path,
        total: u64,
        shards: Vec<ShardSpan>,
    ) -> Self {
        Self {
            id: id.to_owned(),
            part: part.display().to_string(),
            final_path: final_path.display().to_string(),
            total: Some(total),
            updated_unix: now_unix(),
            shards,
        }
    }

    pub fn part_path(&self) -> PathBuf {
        PathBuf::from(&self.part)
    }

    pub fn final_path(&self) -> PathBuf {
        PathBuf::from(&self.final_path)
    }
}

/// 落盘格式。
#[derive(Debug, Serialize, Deserialize)]
struct PartialFile {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    entries: Vec<PartialEntry>,
}

/// 续传决策结果（纯函数输出，下载循环照此执行）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResumePlan {
    /// 从 part 的哪个字节继续（调用方还要以磁盘实际长度为准复核）
    pub offset: u64,
    /// 是否把 part 截断到 0 后重下
    pub truncate: bool,
    /// 已知的完整文件大小
    pub total: Option<u64>,
    /// part 已经等于完整大小：只需 sync + rename，不必再发请求
    pub complete: bool,
}

/// 决定一个断点怎么接着下。
///
/// 规则顺序（见 `docs/adr/0011-resumable-downloads.md`）：
/// 1. 没有断点 → 从 0 新建；
/// 2. 索引里的 `total` 与本次详情 `size` 都存在且不等 → 云端文件变了，截断重下；
/// 3. part 比已知总大小还长 → 截断重下；
/// 4. part 正好等于已知总大小 → 视为已下完（崩溃在 rename 前）；
/// 5. 其余 → 从 part 末尾追加；
/// 6. 前后都不知道大小 → 也按 part 长度发 Range，服务器 416 时再重来。
pub fn plan_resume(
    entry: Option<&PartialEntry>,
    remote_size: Option<u64>,
    part_len: u64,
) -> ResumePlan {
    let Some(entry) = entry else {
        return ResumePlan {
            offset: 0,
            truncate: false,
            total: remote_size,
            complete: false,
        };
    };

    if let (Some(indexed), Some(remote)) = (entry.total, remote_size) {
        if indexed != remote {
            return ResumePlan {
                offset: 0,
                truncate: true,
                total: Some(remote),
                complete: false,
            };
        }
    }

    let total = remote_size.or(entry.total);
    match total {
        Some(total) if part_len > total => ResumePlan {
            offset: 0,
            truncate: true,
            total: Some(total),
            complete: false,
        },
        Some(total) if total > 0 && part_len == total => ResumePlan {
            offset: part_len,
            truncate: false,
            total: Some(total),
            complete: true,
        },
        _ => ResumePlan {
            offset: part_len,
            truncate: false,
            total,
            complete: false,
        },
    }
}

/// 单文件多分片的续传计划。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShardPlan {
    /// 已知的完整文件大小（分片必须知道大小）
    pub total: u64,
    /// 是否先把 part 截断到 0（云端大小变化 / 分片记录损坏）
    pub truncate: bool,
    /// 每片的 `[start, end)` 与已写字节；永远从 0 连续覆盖到 total
    pub spans: Vec<ShardSpan>,
    /// 所有片都已写完：只需 sync + rename，不必发请求
    pub complete: bool,
}

impl ShardPlan {
    /// 全部片已写字节之和（用于进度 / 完成判定）。
    pub fn done_bytes(&self) -> u64 {
        self.spans.iter().map(|span| span.done).sum()
    }
}

/// 决定一个文件要不要分片、每片从哪继续（纯函数）。
///
/// 返回 `None` 表示**不分片**，调用方走原来的单连接路径：
/// - 详情 / 列表都不知道总大小（`remote_size == None`）；
/// - 文件小于 `threshold`；
/// - `wanted < 2`（`KICHI_DOWNLOAD_CONNECTIONS=1`）；
/// - 已有 v1 顺序断点（`shards` 为空）：旧断点继续顺序追加，不冒险转换。
///
/// 以下情况会返回 `truncate = true` 的全新分片计划（整个文件重下）：
/// - 索引里的 total 与本次详情不一致（云端文件变了）；
/// - part 比云端文件还长；
/// - v2 分片记录不连续 / 区间不合法。
pub fn plan_shards(
    entry: Option<&PartialEntry>,
    remote_size: Option<u64>,
    part_len: u64,
    threshold: u64,
    wanted: usize,
) -> Option<ShardPlan> {
    // 已在分片中的旧 part：即使这次关了分片（wanted=1）或阈值被调高，也要按
    // 原分片记录继续，否则顺序追加会把稀疏文件里的空洞当成已下载字节。
    let resuming_shards = entry.is_some_and(|entry| !entry.shards.is_empty());
    let total = remote_size.or_else(|| entry.and_then(|entry| entry.total))?;
    if total == 0 {
        return None;
    }
    if !resuming_shards && (total < threshold || wanted < 2) {
        return None;
    }
    // v1 断点：保持顺序续传，不把已写的前缀转成分片
    if entry.is_some_and(|entry| entry.shards.is_empty()) {
        return None;
    }
    if let Some(entry) = entry {
        if entry.total != Some(total) || part_len > total {
            return Some(fresh_shard_plan(total, wanted));
        }
        if resuming_shards {
            return Some(
                reuse_shard_plan(entry, total, part_len)
                    .unwrap_or_else(|| fresh_shard_plan(total, wanted)),
            );
        }
    }
    Some(fresh_shard_plan(total, wanted))
}

/// 全新分片：按 `wanted` 均分，最后一片可能不足一片大小。
fn fresh_shard_plan(total: u64, wanted: usize) -> ShardPlan {
    let wanted = u64::try_from(wanted).unwrap_or(u64::MAX).max(2);
    let shard_size = total.div_ceil(wanted).max(1);
    let mut spans = Vec::new();
    let mut start = 0;
    while start < total {
        let end = start.saturating_add(shard_size).min(total);
        spans.push(ShardSpan {
            start,
            end,
            done: 0,
        });
        start = end;
    }
    ShardPlan {
        total,
        truncate: true,
        spans,
        complete: false,
    }
}

/// 复用 v2 分片记录；记录不合法时返回 `None`（调用方整文件重下）。
fn reuse_shard_plan(entry: &PartialEntry, total: u64, part_len: u64) -> Option<ShardPlan> {
    let spans = &entry.shards;
    if spans.is_empty() || spans[0].start != 0 || spans.last()?.end != total {
        return None;
    }
    let mut expected_start = 0;
    let mut clamped = Vec::with_capacity(spans.len());
    for span in spans {
        if span.is_empty() || span.start != expected_start || span.done > span.len() {
            return None;
        }
        // 崩溃可能让索引比磁盘多记一点：以磁盘实际能覆盖到的位置为准
        let done = span
            .done
            .min(part_len.saturating_sub(span.start))
            .min(span.len());
        clamped.push(ShardSpan {
            start: span.start,
            end: span.end,
            done,
        });
        expected_start = span.end;
    }
    let complete = part_len >= total && clamped.iter().all(|span| span.done == span.len());
    Some(ShardPlan {
        total,
        truncate: false,
        spans: clamped,
        complete,
    })
}

impl PartialEntry {
    /// 按云端 id 查断点；只接受 part 仍存在、且位于本次下载根目录之下的条目。
    ///
    /// 用 `starts_with`（组件级比较）而不是 `parent() == target_dir`：文件夹批次
    /// 会把文件写进子目录；重启后单独点其中一个文件时，断点就在
    /// `Downloads/剧集/第 1 集.mkv.kichi-part`，仍然属于当前下载根目录。
    /// 换过下载根目录（`/old` → `/new`）不会命中，旧 part 留在原处。
    pub fn matches_dir(&self, target_dir: &Path) -> bool {
        let part = self.part_path();
        part.is_file() && part.starts_with(target_dir)
    }
}

/// 测试里改 `KICHI_PARTIALS_FILE` 时共用这把锁（跨测试模块）。
#[cfg(test)]
pub(crate) static ENV_LOCK: Mutex<()> = Mutex::new(());

/// 进程内串行化索引读改写（下载是串行的，这里只需要防界面线程同时查）。
fn lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

/// 查一个文件的断点。part 不在 `target_dir` 之下时视为没有（不续到别的目录）。
pub fn lookup(id: &str, target_dir: &Path) -> Option<PartialEntry> {
    let _guard = lock();
    lookup_in(&paths::partials_path(), id, target_dir)
}

/// 记下一个断点（同 id 旧条目会被替换）。写失败只记日志，不阻断下载。
pub fn remember(entry: &PartialEntry) {
    let _guard = lock();
    let path = paths::partials_path();
    let mut entries = load(&path);
    entries.retain(|old| old.id != entry.id);
    entries.push(entry.clone());
    if let Err(err) = save(&path, &entries) {
        eprintln!("[kichi] 下载断点索引未能落盘（本次仅失去跨重启续传）：{err}");
    }
}

/// 忘掉一个文件的断点（成功 / 取消 / 本地写坏时调用）。不删 part 文件。
pub fn forget(id: &str) {
    let _guard = lock();
    let path = paths::partials_path();
    let mut entries = load(&path);
    let before = entries.len();
    entries.retain(|entry| entry.id != id);
    if entries.len() != before {
        if let Err(err) = save(&path, &entries) {
            eprintln!("[kichi] 下载断点索引未能落盘（本次仅失去跨重启续传）：{err}");
        }
    }
}

/// 用户取消一条未完成下载：删掉指定下载根目录下的 `.kichi-part` 和索引项。
///
/// 只处理 `id` 相同且 part 位于 `target_dir` 之下的条目，避免同名 id 或换过
/// 下载目录时误删别处文件。已完成文件没有 part，传进来的 id 自然不会命中。
pub fn discard(id: &str, target_dir: &Path) {
    let _guard = lock();
    let path = paths::partials_path();
    let mut entries = load(&path);
    let before = entries.len();
    entries.retain(|entry| {
        if entry.id != id || !entry.part_path().starts_with(target_dir) {
            return true;
        }
        let part = entry.part_path();
        if let Err(err) = std::fs::remove_file(&part) {
            if err.kind() != std::io::ErrorKind::NotFound {
                eprintln!("[kichi] 删除下载断点失败（{}）：{err}", part.display());
            }
        }
        false
    });
    if entries.len() != before {
        if let Err(err) = save(&path, &entries) {
            eprintln!("[kichi] 更新断点索引失败：{err}");
        }
    }
}

fn lookup_in(path: &Path, id: &str, target_dir: &Path) -> Option<PartialEntry> {
    load(path)
        .into_iter()
        .find(|entry| entry.id == id && entry.matches_dir(target_dir))
}

fn load(path: &Path) -> Vec<PartialEntry> {
    let Ok(bytes) = std::fs::read(path) else {
        return Vec::new();
    };
    let Ok(file) = serde_json::from_slice::<PartialFile>(&bytes) else {
        // 损坏 / 手改坏了：当空索引，不 panic；part 文件本身还在，用户不会丢数据
        return Vec::new();
    };
    file.entries
        .into_iter()
        .filter(|entry| {
            !entry.id.is_empty() && !entry.part.is_empty() && Path::new(&entry.part).is_file()
        })
        .collect()
}

fn save(path: &Path, entries: &[PartialEntry]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| format!("创建配置目录失败：{err}"))?;
    }
    let body = serde_json::to_vec_pretty(&PartialFile {
        version: PARTIALS_VERSION,
        entries: entries.to_vec(),
    })
    .map_err(|err| format!("序列化下载断点失败：{err}"))?;
    let temp = path.with_extension("json.tmp");
    private_fs::write_private(&temp, &body).map_err(|err| format!("写入下载断点失败：{err}"))?;
    private_fs::replace_file(&temp, path).map_err(|err| format!("保存下载断点失败：{err}"))
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
#[path = "partials_tests.rs"]
mod tests;
