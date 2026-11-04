//! 全量建库 + canary 探针。
//!
//! ## 全量建库
//!
//! `parent_id=*` 流式翻页 → 内存收集 → 单事务替换。事务提交前旧库完全不受
//! 影响；网络失败、取消、翻页异常都发生在替换之前，所以「建库失败不能破坏
//! 旧库」是结构保证，而不是靠回滚补救。全盘列举官方不给总数，进度只能报已
//! 扫描条数（界面显示 `x/?`）。
//!
//! ## canary 探针
//!
//! 每次只发两个请求（配额用量 + `parent_id=*` 最近修改第一页），用三样东西
//! 对账：官方 `usage`、本地未删除文件大小和、最近页。对得上就是 fresh；
//! 差值能被最近页的变化完全解释就打补丁；对不上才怀疑并请求全量重建。
//!
//! ⚠️ **未验证假设**：`order=modified_time desc` 是否被官方真正执行还没有用
//! 真实账号确认过（未知查询参数历史上会被静默忽略）。因此这里不信任该参数：
//! 每次先校验第一页确实按修改时间倒序，校验不过就把探针标为「不可用」，
//! **绝不**据此触发全量重建。真账号验证前，这个分支是默认的保守路径。

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use kichi_core::{DriveClient, EntryKind, FileEntry};

use crate::session::download::Cancel;
use crate::session::Failure;

use super::apply::ProbeRecord;
use super::query::KnownNode;
use super::{IndexError, TreeIndex, SUSPECT_REBUILD_COOLDOWN_SECS};

/// 一次全量建库的结局。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildOutcome {
    /// 建成，值为入库条目数
    Built(usize),
    /// 用户取消 / 索引被关闭：旧库保持原样
    Cancelled,
}

/// 一轮 canary 探针的结论。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeOutcome {
    /// 配额与本地大小和一致
    Fresh { patched: usize },
    /// 差值能由最近页的变化完全解释，已原地补丁
    Patched { patched: usize },
    /// 对不上；`rebuild` 表示本次应触发全量重建（受冷却约束）
    Suspect {
        reason: SuspectReason,
        rebuild: bool,
    },
    /// 最近页顺序不可信 / 差值无法归因：只记录，不重建
    Unavailable {
        patched: usize,
        /// 第一次发现不可用（界面只提示一次）
        first_time: bool,
    },
    /// 用户取消 / 索引关闭
    Cancelled,
}

/// 为什么怀疑本地索引已经落后。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuspectReason {
    /// 本地未删除文件大小和大于配额用量：外部删除了文件
    ExternalDeletion,
    /// 最近页整页都比上次同步新，窗口可能装不下全部变化
    WindowOverflow,
    /// 用量差无法由最近页的变化解释
    UnexplainedDelta,
}

impl SuspectReason {
    /// 给界面 / 日志用的中文说明。
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ExternalDeletion => "本地索引里的文件比网盘还多，可能是在别处删除了文件",
            Self::WindowOverflow => "最近修改第一页可能装不下上次同步以来的全部变化",
            Self::UnexplainedDelta => "网盘用量与本地索引的差值无法由最近修改条目解释",
        }
    }
}

/// 探针的网络输入；测试用假数据直接构造。
pub(crate) struct ProbeInput {
    /// 官方配额用量（字节）
    pub usage: u64,
    /// `parent_id=*` 按修改时间倒序的第一页
    pub recent: Vec<FileEntry>,
    /// 这一页之后还有下一页
    pub has_next: bool,
}

/// 全盘翻页的保护上限（约 200 万条，防止异常分页吃光内存）。
const MAX_FULL_BUILD_PAGES: usize = 5000;

type Page = (Vec<FileEntry>, Option<String>);
type PageFetch<'a> = dyn FnMut(Option<&str>) -> Result<Page, Failure> + 'a;

/// 从网盘全量建库（阻塞；调用方放后台线程）。
pub fn full_build(
    index: &TreeIndex,
    client: &DriveClient,
    cancel: &Cancel,
    on_progress: &mut dyn FnMut(usize),
) -> Result<BuildOutcome, Failure> {
    let mut fetch = |token: Option<&str>| -> Result<(Vec<FileEntry>, Option<String>), Failure> {
        client
            .list_all_page(token)
            .map(|(hits, next)| (hits.into_iter().map(|hit| hit.entry).collect(), next))
            .map_err(Failure::from)
    };
    build_with_pages(index, cancel, &mut fetch, on_progress)
}

/// 与网络解耦的建库主体：测试用假分页数据直接驱动。
pub(super) fn build_with_pages(
    index: &TreeIndex,
    cancel: &Cancel,
    fetch: &mut PageFetch<'_>,
    on_progress: &mut dyn FnMut(usize),
) -> Result<BuildOutcome, Failure> {
    let mut collected = Vec::new();
    let mut token: Option<String> = None;
    let mut pages = 0usize;
    loop {
        if stopped(index, cancel) {
            return Ok(BuildOutcome::Cancelled);
        }
        pages += 1;
        if pages > MAX_FULL_BUILD_PAGES {
            return Err(Failure::Other(
                "全盘列举页数超过上限，已停止建库（旧索引保留）".to_owned(),
            ));
        }
        let (entries, next) = fetch(token.as_deref())?;
        collected.extend(entries);
        on_progress(collected.len());
        match next {
            None => break,
            Some(next) => {
                if token.as_deref() == Some(next.as_str()) {
                    return Err(Failure::Other(
                        "官方分页异常：全盘列举的翻页令牌原地打转，已停止建库（旧索引保留）"
                            .to_owned(),
                    ));
                }
                token = Some(next);
            }
        }
    }
    if stopped(index, cancel) {
        return Ok(BuildOutcome::Cancelled);
    }
    let count = collected.len();
    index
        .replace_all(&collected)
        .map_err(|err| Failure::Other(format!("本地索引写入失败：{err}")))?;
    Ok(BuildOutcome::Built(count))
}

/// 跑一次 canary 探针（阻塞；调用方放后台线程）。
///
/// 网络失败只向上报错，不写节点；调用方应调 [`TreeIndex::note_probe_attempt`]
/// 让冷却照样生效，避免每次触发都立刻重试。
pub fn run_canary(
    index: &TreeIndex,
    client: &DriveClient,
    cancel: &Cancel,
) -> Result<ProbeOutcome, Failure> {
    if stopped(index, cancel) {
        return Ok(ProbeOutcome::Cancelled);
    }
    let usage = client.usage().map_err(Failure::from)?;
    if stopped(index, cancel) {
        return Ok(ProbeOutcome::Cancelled);
    }
    let (recent, next) = client.list_recent_page(None).map_err(Failure::from)?;
    if stopped(index, cancel) {
        return Ok(ProbeOutcome::Cancelled);
    }
    let input = ProbeInput {
        usage,
        recent,
        has_next: next.is_some(),
    };
    probe_with_data(index, input, Utc::now().timestamp())
}

/// 与网络解耦的探针主体：读本地快照 → 判定 → 落补丁与冷却标记。
pub(super) fn probe_with_data(
    index: &TreeIndex,
    input: ProbeInput,
    now: i64,
) -> Result<ProbeOutcome, Failure> {
    if index.is_closed() {
        return Ok(ProbeOutcome::Cancelled);
    }
    let ids: Vec<String> = input.recent.iter().map(|entry| entry.id.clone()).collect();
    let snapshot = index.probe_snapshot(&ids).map_err(index_failure)?;
    let last_sync = latest(snapshot.last_probe_at, snapshot.last_full_build_at);
    let decision = decide(
        input.usage,
        snapshot.local_bytes,
        &input.recent,
        input.has_next,
        &snapshot.known,
        last_sync,
    );
    let rebuild_allowed = match snapshot.last_suspect_rebuild_at {
        Some(last) => now.saturating_sub(last) >= SUSPECT_REBUILD_COOLDOWN_SECS,
        None => true,
    };
    let unavailable_first_time = !snapshot.unavailable;

    let (entries, outcome, unavailable, suspect_rebuild) = match decision {
        Decision::Fresh { upserts } => {
            let patched = upserts.len();
            (upserts, ProbeOutcome::Fresh { patched }, false, false)
        }
        Decision::Patch { upserts } => {
            let patched = upserts.len();
            (upserts, ProbeOutcome::Patched { patched }, false, false)
        }
        Decision::Unavailable { upserts } => {
            let patched = upserts.len();
            (
                upserts,
                ProbeOutcome::Unavailable {
                    patched,
                    first_time: unavailable_first_time,
                },
                true,
                false,
            )
        }
        Decision::Suspect { upserts, reason } => (
            upserts,
            ProbeOutcome::Suspect {
                reason,
                rebuild: rebuild_allowed,
            },
            false,
            rebuild_allowed,
        ),
    };

    index
        .record_probe(&ProbeRecord {
            entries: &entries,
            now,
            usage: input.usage,
            unavailable,
            suspect_rebuild,
        })
        .map_err(index_failure)?;
    Ok(outcome)
}

/// 纯判定：给定三样对账数据，得出这一轮该怎么处理。
pub(super) fn decide(
    usage: u64,
    local_bytes: u64,
    recent: &[FileEntry],
    has_next: bool,
    known: &[KnownNode],
    last_sync: Option<i64>,
) -> Decision {
    let known_by_id: HashMap<&str, &KnownNode> = known
        .iter()
        .map(|node| (node.entry.id.as_str(), node))
        .collect();
    let upserts = changed_entries(recent, &known_by_id);

    // 本地文件比配额还大：只能是在别处删掉了文件
    if local_bytes > usage {
        return Decision::Suspect {
            upserts,
            reason: SuspectReason::ExternalDeletion,
        };
    }
    if local_bytes == usage {
        return Decision::Fresh { upserts };
    }
    // 顺序不可信时绝不触发重建：否则每次探针都会全盘重建一次
    if !ordered_desc(recent) {
        return Decision::Unavailable { upserts };
    }
    if window_overflow(recent, has_next, last_sync) {
        return Decision::Suspect {
            upserts,
            reason: SuspectReason::WindowOverflow,
        };
    }
    let delta = usage - local_bytes;
    let explained = explained_bytes(recent, &known_by_id);
    if delta <= explained {
        return Decision::Patch { upserts };
    }
    if upserts.is_empty() {
        // 差值既解释不了又没有可见变化：多半是系统性口径差异（例如配额含
        // 回收站）。重建也只会得到同样的结论，标记不可用即可，绝不反复重建。
        return Decision::Unavailable { upserts };
    }
    Decision::Suspect {
        upserts,
        reason: SuspectReason::UnexplainedDelta,
    }
}

/// 判定结果；`upserts` 是无论结论如何都应落到索引里的可见变化。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Decision {
    Fresh {
        upserts: Vec<FileEntry>,
    },
    Patch {
        upserts: Vec<FileEntry>,
    },
    Suspect {
        upserts: Vec<FileEntry>,
        reason: SuspectReason,
    },
    Unavailable {
        upserts: Vec<FileEntry>,
    },
}

/// 最近页里与本地索引不一致的条目（新条目 / 改名 / 换目录 / 大小或时间变化）。
///
/// 全盘列举偶尔不给 `parent_id` / `modified_time`：这类空值不能拿来冲掉本地
/// 已知值，否则补丁会把条目错挂到根目录、并丢掉排序依据。
fn changed_entries(recent: &[FileEntry], known: &HashMap<&str, &KnownNode>) -> Vec<FileEntry> {
    let mut changes = Vec::new();
    for entry in recent {
        let mut effective = entry.clone();
        let Some(node) = known.get(entry.id.as_str()) else {
            changes.push(effective);
            continue;
        };
        if effective.parent_id.trim().is_empty() && !node.entry.parent_id.trim().is_empty() {
            effective.parent_id = node.entry.parent_id.clone();
        }
        if effective.modified_at.is_none() {
            effective.modified_at = node.entry.modified_at;
        }
        let changed = node.trashed
            || node.entry.name != effective.name
            || node.entry.parent_id != effective.parent_id
            || node.entry.kind != effective.kind
            || node.entry.size != effective.size
            || !same_second(node.entry.modified_at, effective.modified_at);
        if changed {
            changes.push(effective);
        }
    }
    changes
}

/// 最近页里能解释多少用量增长。
///
/// 本地没有的文件按整份大小算；已知文件只有「变大」才可能贡献用量（变小不会
/// 让用量增加，差值由别的变化解释）。文件夹不计大小。
fn explained_bytes(recent: &[FileEntry], known: &HashMap<&str, &KnownNode>) -> u64 {
    recent
        .iter()
        .filter(|entry| entry.kind == EntryKind::File)
        .map(|entry| match known.get(entry.id.as_str()) {
            None => entry.size,
            Some(node) if node.trashed => entry.size,
            Some(node) => entry.size.saturating_sub(node.entry.size),
        })
        .sum()
}

/// 最近页确实按修改时间倒序吗？
///
/// 任何一条缺 `modified_at` 都算不可信：宁可不用探针，也不能拿乱序页去推断
/// 「窗口够不够」。
fn ordered_desc(recent: &[FileEntry]) -> bool {
    let mut previous: Option<i64> = None;
    for entry in recent {
        let Some(modified) = entry.modified_at else {
            return false;
        };
        let seconds = modified.timestamp();
        if let Some(previous) = previous {
            if seconds > previous {
                return false;
            }
        }
        previous = Some(seconds);
    }
    true
}

/// 最近页是否可能漏掉上次同步以来的变化。
///
/// 只有「这一页装满且还有下一页」才可能漏；再比较页尾（最旧一条）与上次
/// 同步时间：页尾都比上次同步新，说明一页装不下全部变化。
fn window_overflow(recent: &[FileEntry], has_next: bool, last_sync: Option<i64>) -> bool {
    if !has_next {
        return false;
    }
    let Some(last_sync) = last_sync else {
        return true;
    };
    match recent.last().and_then(|entry| entry.modified_at) {
        Some(oldest) => oldest.timestamp() > last_sync,
        None => true,
    }
}

fn same_second(left: Option<DateTime<Utc>>, right: Option<DateTime<Utc>>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => left.timestamp() == right.timestamp(),
        (None, None) => true,
        _ => false,
    }
}

fn latest(left: Option<i64>, right: Option<i64>) -> Option<i64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

fn index_failure(err: IndexError) -> Failure {
    Failure::Other(format!("本地索引读写失败：{err}"))
}

fn stopped(index: &TreeIndex, cancel: &Cancel) -> bool {
    cancel.is_cancelled() || index.is_closed()
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};

    use super::super::store;
    use super::super::test_support::{file, folder};

    use super::*;

    fn at(seconds: i64) -> Option<DateTime<Utc>> {
        DateTime::from_timestamp(seconds, 0)
    }

    fn file_at(id: &str, size: u64, parent: &str, modified: Option<DateTime<Utc>>) -> FileEntry {
        let mut entry = file(id, &format!("{id}.mkv"), parent);
        entry.size = size;
        entry.modified_at = modified;
        entry
    }

    fn folder_at(id: &str, modified: Option<DateTime<Utc>>) -> FileEntry {
        let mut entry = folder(id, id, "");
        entry.modified_at = modified;
        entry
    }

    /// 建好库，并把「上次同步」时间钉在 `last_sync`（测试不能依赖真实时钟）。
    fn index_with(entries: &[FileEntry], last_sync: Option<i64>) -> TreeIndex {
        let index = TreeIndex::open_in_memory("u1").expect("打开内存库");
        index.replace_all(entries).expect("整库替换");
        index
            .store
            .with_conn(|conn| {
                match last_sync {
                    Some(sync) => {
                        store::meta_set(conn, "last_probe_at", &sync.to_string())?;
                        store::meta_set(conn, "last_full_build_at", &sync.to_string())?;
                    }
                    None => {
                        conn.execute(
                            "DELETE FROM meta WHERE key IN ('last_probe_at', 'last_full_build_at')",
                            [],
                        )?;
                    }
                }
                Ok(())
            })
            .expect("写同步时间");
        index
    }

    fn pages(pages: Vec<Vec<FileEntry>>) -> impl FnMut(Option<&str>) -> Result<Page, Failure> {
        let mut queue: std::collections::VecDeque<Vec<FileEntry>> = pages.into();
        let mut served = 0usize;
        move |_token| {
            let Some(entries) = queue.pop_front() else {
                return Ok((Vec::new(), None));
            };
            served += 1;
            let next = (!queue.is_empty()).then(|| format!("page-{served}"));
            Ok((entries, next))
        }
    }

    #[test]
    fn matching_usage_is_fresh_and_patches_renames() {
        let mut original = file_at("a", 100, "", at(1_000));
        original.name = "old.mkv".to_owned();
        let index = index_with(&[original], Some(1_000));

        let mut renamed = file_at("a", 100, "", at(1_000));
        renamed.name = "new.mkv".to_owned();
        let outcome = probe_with_data(
            &index,
            ProbeInput {
                usage: 100,
                recent: vec![renamed],
                has_next: false,
            },
            1_060,
        )
        .expect("探针");
        assert_eq!(outcome, ProbeOutcome::Fresh { patched: 1 });
        assert_eq!(index.last_probe_at().expect("读探针时间"), Some(1_060));
        let root = index
            .listing_if_fresh("", 0, i64::MAX)
            .expect("读根目录")
            .expect("根目录仍在");
        assert_eq!(root[0].name, "new.mkv", "改名要原地补上");
    }

    #[test]
    fn explainable_addition_is_patched_in_place() {
        let index = index_with(&[file_at("a", 100, "", at(1_000))], Some(1_000));
        let recent = vec![
            file_at("b", 50, "", at(2_000)),
            file_at("a", 100, "", at(1_000)),
        ];
        let outcome = probe_with_data(
            &index,
            ProbeInput {
                usage: 150,
                recent,
                has_next: false,
            },
            2_060,
        )
        .expect("探针");
        assert_eq!(outcome, ProbeOutcome::Patched { patched: 1 });
        let root = index
            .listing_if_fresh("", 0, i64::MAX)
            .expect("读根目录")
            .expect("根目录仍在");
        assert!(root.iter().any(|entry| entry.id == "b"), "新文件要补进索引");
    }

    #[test]
    fn local_usage_above_quota_suspects_external_deletion() {
        let index = index_with(&[file_at("a", 200, "", at(1_000))], Some(1_000));
        let input = || ProbeInput {
            usage: 100,
            recent: vec![file_at("a", 200, "", at(1_000))],
            has_next: false,
        };
        let first = probe_with_data(&index, input(), 1_060).expect("探针");
        assert_eq!(
            first,
            ProbeOutcome::Suspect {
                reason: SuspectReason::ExternalDeletion,
                rebuild: true,
            }
        );
        // 冷却写回：紧接着再探一次不会重复触发重建
        let second = probe_with_data(&index, input(), 1_080).expect("再探");
        assert_eq!(
            second,
            ProbeOutcome::Suspect {
                reason: SuspectReason::ExternalDeletion,
                rebuild: false,
            }
        );
    }

    #[test]
    fn window_overflow_suspects_a_rebuild() {
        let index = index_with(&[file_at("a", 100, "", at(1_000))], Some(1_000));
        let outcome = probe_with_data(
            &index,
            ProbeInput {
                usage: 150,
                recent: vec![file_at("a", 100, "", at(2_000))],
                has_next: true,
            },
            2_060,
        )
        .expect("探针");
        assert_eq!(
            outcome,
            ProbeOutcome::Suspect {
                reason: SuspectReason::WindowOverflow,
                rebuild: true,
            }
        );
    }

    #[test]
    fn untrusted_order_marks_probe_unavailable_only_once() {
        let index = index_with(&[file_at("a", 100, "", at(1_000))], Some(1_000));
        // 升序页：官方没有按 modified_time desc 排序
        let recent = vec![
            file_at("b", 10, "", at(2_000)),
            file_at("c", 10, "", at(3_000)),
        ];
        let input = || ProbeInput {
            usage: 150,
            recent: recent.clone(),
            has_next: false,
        };
        let first = probe_with_data(&index, input(), 2_060).expect("探针");
        assert_eq!(
            first,
            ProbeOutcome::Unavailable {
                patched: 2,
                first_time: true,
            }
        );
        let snapshot = index.probe_snapshot(&[]).expect("读快照");
        assert!(snapshot.unavailable, "要记下「探针不可用」只提示一次");
        assert!(
            snapshot.last_suspect_rebuild_at.is_none(),
            "排序不可信绝不能触发重建"
        );

        let second = probe_with_data(&index, input(), 2_080).expect("再探");
        assert_eq!(
            second,
            ProbeOutcome::Unavailable {
                patched: 0,
                first_time: false,
            },
            "同样的不可用状态不重复提示"
        );
    }

    #[test]
    fn unexplained_delta_without_changes_does_not_rebuild() {
        let index = index_with(&[file_at("a", 100, "", at(1_000))], Some(1_000));
        let outcome = probe_with_data(
            &index,
            ProbeInput {
                usage: 200,
                recent: vec![file_at("a", 100, "", at(1_000))],
                has_next: false,
            },
            1_060,
        )
        .expect("探针");
        assert_eq!(
            outcome,
            ProbeOutcome::Unavailable {
                patched: 0,
                first_time: true,
            }
        );
        let snapshot = index.probe_snapshot(&[]).expect("读快照");
        assert!(
            snapshot.last_suspect_rebuild_at.is_none(),
            "无法归因的差值不能触发重建"
        );
        assert!(snapshot.unavailable, "记为不可用，避免反复提示 / 重建");
    }

    #[test]
    fn folders_without_sizes_do_not_explain_byte_deltas() {
        let index = index_with(&[file_at("a", 100, "", at(1_000))], Some(1_000));
        let recent = vec![folder_at("f", at(2_000)), file_at("a", 100, "", at(1_000))];
        let outcome = probe_with_data(
            &index,
            ProbeInput {
                usage: 200,
                recent,
                has_next: false,
            },
            2_060,
        )
        .expect("探针");
        assert_eq!(
            outcome,
            ProbeOutcome::Suspect {
                reason: SuspectReason::UnexplainedDelta,
                rebuild: true,
            },
            "新增文件夹不占空间，不能拿它解释用量差"
        );
    }

    #[test]
    fn network_failure_can_note_an_attempt_without_touching_nodes() {
        let index = index_with(&[file_at("a", 100, "", at(1_000))], Some(1_000));
        index.note_probe_attempt(2_000).expect("记一次尝试");
        assert_eq!(index.last_probe_at().expect("读探针时间"), Some(2_000));
        assert_eq!(index.local_file_bytes().expect("大小和"), 100);
        let root = index
            .listing_if_fresh("", 0, i64::MAX)
            .expect("读根目录")
            .expect("根目录仍在");
        assert_eq!(root.len(), 1, "失败的探针不能动节点");
    }

    #[test]
    fn closed_index_cancels_the_probe() {
        let index = TreeIndex::open_in_memory("u1").expect("打开内存库");
        index.close();
        let outcome = probe_with_data(
            &index,
            ProbeInput {
                usage: 0,
                recent: Vec::new(),
                has_next: false,
            },
            1_000,
        )
        .expect("取消");
        assert_eq!(outcome, ProbeOutcome::Cancelled);
    }

    #[test]
    fn full_build_refreshes_probe_metadata() {
        let index = TreeIndex::open_in_memory("u1").expect("打开内存库");
        index
            .store
            .with_conn(|conn| store::meta_set(conn, "probe_unavailable", "1"))
            .expect("写不可用标记");
        index
            .replace_all(&[
                file_at("a", 40, "", at(1_000)),
                file_at("b", 60, "dir", at(2_000)),
            ])
            .expect("整库替换");
        assert!(
            index.last_probe_at().expect("读探针时间").is_some(),
            "全量建库本身就是一次完整同步"
        );
        let snapshot = index.probe_snapshot(&[]).expect("读快照");
        assert_eq!(snapshot.local_bytes, 100, "用量和要跟新快照一起刷新");
        assert!(!snapshot.unavailable, "可信的完整同步要清掉不可用标记");
    }

    // —— 全量建库（P0.2 起保留）——

    #[test]
    fn full_build_replaces_the_old_snapshot_in_one_transaction() {
        let index = TreeIndex::open_in_memory("u1").expect("打开内存库");
        index
            .put_listing("", &[file("old", "old.mkv", "")], true)
            .expect("旧快照");
        let mut fetch = pages(vec![
            vec![folder("dir", "动画", ""), file("a", "a.mkv", "")],
            vec![file("b", "b.mkv", "dir")],
        ]);
        let mut progress = Vec::new();
        let outcome = build_with_pages(&index, &Cancel::default(), &mut fetch, &mut |scanned| {
            progress.push(scanned)
        })
        .expect("建库");
        assert_eq!(outcome, BuildOutcome::Built(3));
        assert_eq!(progress, vec![2, 3], "每页回调一次已扫描条数");
        assert!(
            index.path_to("old").expect("读路径").is_none(),
            "旧快照应被整体替换"
        );
        let root = index.listing_if_fresh("", 0, i64::MAX).expect("读根目录");
        let root = root.expect("根目录已建成");
        assert!(
            root.iter().any(|entry| entry.id == "dir"),
            "根目录下的文件夹应在索引里"
        );
        let child = index
            .listing_if_fresh("dir", 0, i64::MAX)
            .expect("读子目录");
        assert!(
            child
                .expect("子目录已建成")
                .iter()
                .any(|entry| entry.id == "b"),
            "文件夹自身的子目录也要标记完整"
        );
    }

    #[test]
    fn network_failure_keeps_the_old_snapshot() {
        let index = TreeIndex::open_in_memory("u1").expect("打开内存库");
        index
            .put_listing("", &[file("old", "old.mkv", "")], true)
            .expect("旧快照");
        let mut calls = 0;
        let mut fetch = |_: Option<&str>| -> Result<(Vec<FileEntry>, Option<String>), Failure> {
            calls += 1;
            if calls == 1 {
                Ok((vec![file("new", "new.mkv", "")], Some("next".to_owned())))
            } else {
                Err(Failure::Other("网络中断".to_owned()))
            }
        };
        let result = build_with_pages(&index, &Cancel::default(), &mut fetch, &mut |_| {});
        assert!(result.is_err(), "网络失败要向上报错");
        let live = index
            .listing_if_fresh("", 0, i64::MAX)
            .expect("读旧库")
            .expect("旧库仍在");
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].id, "old");
        assert!(
            index.path_to("new").expect("读新节点").is_none(),
            "失败前收集的新数据不能落库"
        );
    }

    #[test]
    fn cancellation_before_replace_keeps_the_old_snapshot() {
        let index = TreeIndex::open_in_memory("u1").expect("打开内存库");
        index
            .put_listing("", &[file("old", "old.mkv", "")], true)
            .expect("旧快照");
        let cancel = Cancel::default();
        let mut calls = 0;
        let mut fetch = |_: Option<&str>| -> Result<(Vec<FileEntry>, Option<String>), Failure> {
            calls += 1;
            // 第一页读完就取消：替换事务不能开始
            cancel.cancel();
            Ok((vec![file("new", "new.mkv", "")], Some("next".to_owned())))
        };
        let outcome = build_with_pages(&index, &cancel, &mut fetch, &mut |_| {}).expect("取消");
        assert_eq!(outcome, BuildOutcome::Cancelled);
        assert_eq!(calls, 1);
        let live = index
            .listing_if_fresh("", 0, i64::MAX)
            .expect("读旧库")
            .expect("旧库仍在");
        assert_eq!(live[0].id, "old");
    }

    #[test]
    fn broken_paging_is_reported_without_touching_the_index() {
        let index = TreeIndex::open_in_memory("u1").expect("打开内存库");
        index
            .put_listing("", &[file("old", "old.mkv", "")], true)
            .expect("旧快照");
        let mut fetch = |_: Option<&str>| -> Result<(Vec<FileEntry>, Option<String>), Failure> {
            Ok((vec![file("a", "a", "")], Some("stuck".to_owned())))
        };
        let result = build_with_pages(&index, &Cancel::default(), &mut fetch, &mut |_| {});
        assert!(result.is_err(), "token 原地打转要报错");
        let live = index
            .listing_if_fresh("", 0, i64::MAX)
            .expect("读旧库")
            .expect("旧库仍在");
        assert_eq!(live[0].id, "old");
    }

    #[test]
    fn closed_index_cancels_without_writing() {
        let index = TreeIndex::open_in_memory("u1").expect("打开内存库");
        index.close();
        let mut fetch = |_: Option<&str>| -> Result<(Vec<FileEntry>, Option<String>), Failure> {
            panic!("关闭后不应再发请求");
        };
        let outcome =
            build_with_pages(&index, &Cancel::default(), &mut fetch, &mut |_| {}).expect("取消");
        assert_eq!(outcome, BuildOutcome::Cancelled);
    }
}
