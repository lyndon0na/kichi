//! 本地目录树索引门面：账号生命周期 + 读写入口。
//!
//! 索引只是可重建的缓存（ADR 0024）：库文件删掉、坏了、版本不认识都会重建，
//! 所以这里不提供迁移，也不在内存里保留第二份权威状态。所有读写错误都原样
//! 冒泡，由调用方决定是提示用户还是静默降级到网络。纯追加表是唯一例外：
//! 打开时用 `IF NOT EXISTS` 原地补建，不提升版本（见 `store`）。

pub(crate) mod apply;
pub(crate) mod query;
pub(crate) mod store;
pub(crate) mod sync;

use std::path::Path;

use chrono::Utc;
use kichi_core::{EntryKind, FileEntry, TrashEntry};
use rusqlite::params;

pub(crate) use apply::Change;
pub(crate) use query::{IndexSearchHit, MoveTargets};
pub(crate) use store::IndexError;

/// 目录快照多新才算「可以直接渲染」。
///
/// 30 秒与 ADR P0.1 一致：再旧就回网络，避免用户看到明显过期的列表。
pub(crate) const FRESH_TTL_SECS: i64 = 30;

/// canary 探针冷却：这段时间内的重复触发直接跳过。
pub(crate) const PROBE_COOLDOWN_SECS: i64 = 60;

/// 搜索路径看到探针超过这么久没跑过，就在后台补一次（不阻塞本次搜索）。
pub(crate) const PROBE_SEARCH_STALE_SECS: i64 = 300;

/// suspect 触发的全量重建冷却。
///
/// 防止系统性对不上（例如官方配额把回收站也算进去）时每次探针都全盘重建。
pub(crate) const SUSPECT_REBUILD_COOLDOWN_SECS: i64 = 900;

/// 一个账号的本地索引。
pub(crate) struct TreeIndex {
    store: store::Store,
    owner: String,
}

impl TreeIndex {
    /// 打开（必要时重建）指定路径的索引库。
    pub(crate) fn open(path: &Path, owner: &str) -> Result<Self, IndexError> {
        Ok(Self {
            store: store::Store::open(path, owner)?,
            owner: owner.to_owned(),
        })
    }

    #[cfg(test)]
    pub(crate) fn open_in_memory(owner: &str) -> Result<Self, IndexError> {
        Ok(Self {
            store: store::Store::open_in_memory(owner)?,
            owner: owner.to_owned(),
        })
    }

    /// 库里记录的是哪个账号（`KICHI_INDEX_FILE` 指向共享路径时用来校验）。
    pub(crate) fn owner(&self) -> &str {
        &self.owner
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.store.is_closed()
    }

    /// 逻辑关闭；后台若正持有连接不会阻塞等待。
    pub(crate) fn close(&self) {
        self.store.close();
    }

    /// 这个库是否还需要一次全量建库。
    ///
    /// 判据只有「有没有成功建过库」：打开时版本不认识 / 损坏都会重建出空库，
    /// 空库同样没有标记，因此自然触发全量建库。
    pub(crate) fn needs_full_build(&self) -> Result<bool, IndexError> {
        self.store
            .with_conn(|conn| Ok(store::meta_get(conn, "last_full_build_at")?.is_none()))
    }

    /// 一条目录列表落库；`complete` 语义见 [`apply::put_listing`]。
    pub(crate) fn put_listing(
        &self,
        dir_id: &str,
        entries: &[FileEntry],
        complete: bool,
    ) -> Result<(), IndexError> {
        let now = Utc::now().timestamp();
        self.store
            .with_conn(|conn| apply::put_listing(conn, dir_id, entries, complete, now))
    }

    pub(crate) fn apply_change(&self, change: Change<'_>) -> Result<(), IndexError> {
        let now = Utc::now().timestamp();
        self.store
            .with_conn(|conn| apply::apply_change(conn, change, now))
    }

    /// 目录内容已不可信（新建 / 上传 / 离线任务产物等没有具体行可改的场景）。
    pub(crate) fn mark_dir_dirty(&self, dir_id: &str) -> Result<(), IndexError> {
        self.store
            .with_conn(|conn| apply::mark_dir_dirty(conn, dir_id))
    }

    /// 用服务端回收站替换本地快照。
    pub(crate) fn replace_trash(&self, entries: &[TrashEntry]) -> Result<(), IndexError> {
        let now = Utc::now().timestamp();
        self.store
            .with_conn(|conn| apply::replace_trash(conn, entries, now))
    }

    /// 回收站快照仍新鲜时返回条目；否则 `None` 走网络。
    pub(crate) fn trash_listing_if_fresh(
        &self,
        now: i64,
        max_age_secs: i64,
    ) -> Result<Option<Vec<TrashEntry>>, IndexError> {
        self.store
            .with_conn(|conn| query::trash_listing_if_fresh(conn, now, max_age_secs))
    }

    /// 页面快照：重启后先渲染上次成功结果，再后台校准。
    pub(crate) fn page_snapshot(&self, kind: &str) -> Result<Option<String>, IndexError> {
        self.store.with_conn(|conn| {
            Ok(store::page_snapshot(conn, kind)?.map(|(payload, _updated_at)| payload))
        })
    }

    pub(crate) fn put_page_snapshot(&self, kind: &str, payload: &str) -> Result<(), IndexError> {
        let now = Utc::now().timestamp();
        self.store
            .with_conn(|conn| store::put_page_snapshot(conn, kind, payload, now))
    }

    /// 网络父链回填（folder_trail 找不到本地链时用）。
    pub(crate) fn backfill_trail(&self, trail: &[(String, String)]) -> Result<(), IndexError> {
        self.store
            .with_conn(|conn| apply::backfill_trail(conn, trail))
    }

    /// 目录快照仍新鲜时返回条目；否则 `None` 让调用方回网络。
    pub(crate) fn listing_if_fresh(
        &self,
        dir_id: &str,
        now: i64,
        max_age_secs: i64,
    ) -> Result<Option<Vec<FileEntry>>, IndexError> {
        self.store
            .with_conn(|conn| query::listing_if_fresh(conn, dir_id, now, max_age_secs))
    }

    /// 不管新鲜度，返回索引里该目录现有的条目（用于先渲染旧列表）。
    pub(crate) fn listing_any(&self, dir_id: &str) -> Result<Option<Vec<FileEntry>>, IndexError> {
        self.store
            .with_conn(|conn| query::listing_any(conn, dir_id))
    }

    /// 移动 / 复制选择器的一层：已知子文件夹 + 能否 0 网络直接渲染。
    pub(crate) fn move_targets(&self, dir_id: &str) -> Result<MoveTargets, IndexError> {
        self.store
            .with_conn(|conn| query::move_targets(conn, dir_id))
    }

    /// 目标是否落在某个源条目内部（含自身）；索引缺链时返回 `None`。
    ///
    /// `None` 表示判断不了，调用方应放行并让服务端错误冒出来，不能静默失败。
    pub(crate) fn target_inside_sources(
        &self,
        target: &str,
        sources: &[String],
    ) -> Result<Option<bool>, IndexError> {
        if target.trim().is_empty() {
            return Ok(Some(false));
        }
        self.store.with_conn(|conn| {
            let Some(trail) = query::path_to(conn, target)? else {
                return Ok(None);
            };
            Ok(Some(
                trail
                    .iter()
                    .any(|(id, _)| sources.iter().any(|source| source == id)),
            ))
        })
    }

    /// 本地未删除文件大小和（探针对账用）。
    pub(crate) fn local_file_bytes(&self) -> Result<u64, IndexError> {
        self.store.with_conn(query::local_file_bytes)
    }

    /// 最近一次探针尝试 / 成功时间（unix 秒）。
    pub(crate) fn last_probe_at(&self) -> Result<Option<i64>, IndexError> {
        self.store.with_conn(|conn| {
            Ok(store::meta_get(conn, "last_probe_at")?.and_then(|value| value.parse::<i64>().ok()))
        })
    }

    /// 探针需要的本地状态：大小和、最近页条目的本地版本、探针时间与冷却信息。
    pub(crate) fn probe_snapshot(
        &self,
        recent_ids: &[String],
    ) -> Result<query::ProbeSnapshot, IndexError> {
        self.store
            .with_conn(|conn| query::probe_snapshot(conn, recent_ids))
    }

    /// 探针结果落库（补丁 + 时间戳 + 冷却标记）。
    fn record_probe(&self, record: &apply::ProbeRecord<'_>) -> Result<(), IndexError> {
        self.store
            .with_conn(|conn| apply::record_probe(conn, record))
    }

    /// 网络失败的探针也要让冷却生效，避免每次窗口获得焦点都重试。
    pub(crate) fn note_probe_attempt(&self, now: i64) -> Result<(), IndexError> {
        self.store
            .with_conn(|conn| apply::note_probe_attempt(conn, now))
    }

    /// 根 → 目标文件夹自身的 `(id, 名称)`；索引缺链时为 `None`。
    /// 在库条目的名称与父目录 id；索引缺行时为 `None`。
    pub(crate) fn parent_location(
        &self,
        entry_id: &str,
    ) -> Result<Option<(String, String)>, IndexError> {
        self.store
            .with_conn(|conn| query::parent_location(conn, entry_id))
    }

    pub(crate) fn path_to(
        &self,
        folder_id: &str,
    ) -> Result<Option<Vec<(String, String)>>, IndexError> {
        self.store.with_conn(|conn| query::path_to(conn, folder_id))
    }

    pub(crate) fn search(&self, text: &str, limit: usize) -> Result<Vec<FileEntry>, IndexError> {
        self.store
            .with_conn(|conn| query::search(conn, text, limit))
    }

    /// 本地全盘搜索：包含回收站行，没有条数上限。
    pub(crate) fn search_with_trash(&self, text: &str) -> Result<Vec<IndexSearchHit>, IndexError> {
        self.store
            .with_conn(|conn| query::search_with_trash(conn, text))
    }

    /// 一批文件夹 id → 真实路径（索引内解析 + 共享父目录缓存）。
    pub(crate) fn folder_paths(
        &self,
        folder_ids: &[String],
    ) -> Result<std::collections::HashMap<String, String>, IndexError> {
        self.store
            .with_conn(|conn| query::folder_paths(conn, folder_ids))
    }

    /// 最近一次全量建库时间（unix 秒）；没有建过库时为 `None`。
    pub(crate) fn last_full_build_at(&self) -> Result<Option<i64>, IndexError> {
        self.store.with_conn(|conn| {
            Ok(store::meta_get(conn, "last_full_build_at")?
                .and_then(|value| value.parse::<i64>().ok()))
        })
    }

    pub(crate) fn starred(&self, limit: usize) -> Result<Vec<FileEntry>, IndexError> {
        self.store.with_conn(|conn| query::starred(conn, limit))
    }

    /// 全量建库的最后一步：单事务整库替换。
    ///
    /// 调用方已经把所有页收集在内存里；这里一旦开始就是原子操作，失败回滚后
    /// 旧库原样保留。根目录、每个条目的父目录、每个文件夹自身都标记为完整，
    /// 因为 `parent_id=*` 枚举的本来就是全盘条目。
    pub(crate) fn replace_all(&self, entries: &[FileEntry]) -> Result<(), IndexError> {
        let now = Utc::now().timestamp();
        self.store.with_conn(|conn| {
            let tx = conn.unchecked_transaction()?;
            tx.execute("DELETE FROM nodes", [])?;
            tx.execute("DELETE FROM dirs", [])?;
            {
                let mut stmt = tx.prepare(apply::UPSERT_NODE_SQL)?;
                for entry in entries {
                    apply::insert_entry(&mut stmt, entry, false)?;
                }
            }
            let mut dirs = std::collections::BTreeSet::new();
            dirs.insert(String::new());
            for entry in entries {
                dirs.insert(entry.parent_id.trim().to_owned());
                if entry.kind == EntryKind::Folder {
                    dirs.insert(entry.id.clone());
                }
            }
            {
                let mut stmt = tx.prepare(
                    "INSERT INTO dirs(id, complete, dirty, fetched_at) VALUES(?1, 1, 0, ?2) \
                     ON CONFLICT(id) DO UPDATE SET \
                     complete = 1, dirty = 0, fetched_at = excluded.fetched_at",
                )?;
                for dir in dirs {
                    stmt.execute(params![dir, now])?;
                }
            }
            store::meta_set(&tx, "last_full_build_at", &now.to_string())?;
            // 整库重建本身就是一次完整同步：探针时间与用量和一起刷新，
            // 下次探针不必立刻怀疑；suspect 冷却时间保留（防止系统性对不上
            // 时每次探针都全盘重建）。
            store::meta_set(&tx, "last_probe_at", &now.to_string())?;
            let local_bytes: u64 = entries
                .iter()
                .filter(|entry| entry.kind == EntryKind::File)
                .map(|entry| entry.size)
                .sum();
            store::meta_set(&tx, "last_usage", &local_bytes.to_string())?;
            tx.execute("DELETE FROM meta WHERE key = 'probe_unavailable'", [])?;
            // 全量建库只含在库条目（`parent_id=*` 会排除回收站），旧回收站
            // 快照已经对不上，必须让回收站读路径回网络重建。
            tx.execute("DELETE FROM meta WHERE key = 'trash_fetched_at'", [])?;
            tx.commit()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{file, folder};
    use super::*;

    #[test]
    fn a_fresh_index_needs_one_full_build() {
        let index = TreeIndex::open_in_memory("u1").expect("打开内存库");
        assert_eq!(index.owner(), "u1");
        assert!(
            index.needs_full_build().expect("读建库标记"),
            "新库必须触发一次全量建库"
        );
        index
            .replace_all(&[file("a", "a.mkv", "")])
            .expect("整库替换");
        assert!(
            !index.needs_full_build().expect("读建库标记"),
            "建库成功后不再重复触发"
        );
        let root = index
            .listing_if_fresh("", 0, i64::MAX)
            .expect("读根目录")
            .expect("根目录应已完成");
        assert_eq!(root[0].id, "a");
    }

    #[test]
    fn full_build_drops_the_stale_trash_snapshot() {
        let index = TreeIndex::open_in_memory("u1").expect("打开内存库");
        index
            .store
            .with_conn(|conn| store::meta_set(conn, "trash_fetched_at", "500"))
            .expect("写回收站快照时间");
        index
            .replace_all(&[file("a", "a.mkv", "")])
            .expect("整库替换");
        let fetched = index
            .store
            .with_conn(|conn| store::meta_get(conn, "trash_fetched_at"))
            .expect("读回收站快照时间");
        assert!(
            fetched.is_none(),
            "全量建库后旧回收站快照已经对不上，必须丢弃"
        );
    }

    #[test]
    fn target_inside_sources_walks_the_index_parent_chain() {
        let index = TreeIndex::open_in_memory("u1").expect("打开内存库");
        index
            .store
            .with_conn(|conn| {
                apply::put_listing(conn, "", &[folder("a", "A", "")], true, 100)?;
                apply::put_listing(conn, "a", &[folder("b", "B", "a")], true, 100)?;
                apply::put_listing(conn, "b", &[folder("c", "C", "b")], true, 100)
            })
            .expect("写入");

        let sources = vec!["a".to_owned()];
        assert_eq!(
            index.target_inside_sources("a", &sources).expect("读链"),
            Some(true)
        );
        assert_eq!(
            index.target_inside_sources("c", &sources).expect("读链"),
            Some(true),
            "深层的子孙也要挡住"
        );
        assert_eq!(
            index.target_inside_sources("", &sources).expect("根目录"),
            Some(false),
            "根目录永远不是任何文件夹的子孙"
        );
        assert_eq!(
            index
                .target_inside_sources("missing", &sources)
                .expect("缺链"),
            None,
            "缺链时判断不了，交给服务端报错"
        );
        assert_eq!(
            index
                .target_inside_sources("b", &[String::new()])
                .expect("根目录作为源"),
            Some(false)
        );
    }

    #[test]
    fn last_full_build_time_is_readable_after_the_build() {
        let index = TreeIndex::open_in_memory("u1").expect("打开内存库");
        assert_eq!(
            index.last_full_build_at().expect("读建库时间"),
            None,
            "没建过库时没有时间"
        );
        index
            .replace_all(&[file("a", "a.mkv", "")])
            .expect("整库替换");
        assert!(
            index.last_full_build_at().expect("读建库时间").is_some(),
            "建库后状态行才能说明索引有多新"
        );
    }
}

/// 测试用的条目构造器；只在 `cfg(test)` 下存在。
#[cfg(test)]
pub(crate) mod test_support {
    use kichi_core::{EntryKind, FileEntry};

    pub(crate) fn file(id: &str, name: &str, parent: &str) -> FileEntry {
        entry(id, name, parent, EntryKind::File)
    }

    pub(crate) fn folder(id: &str, name: &str, parent: &str) -> FileEntry {
        entry(id, name, parent, EntryKind::Folder)
    }

    fn entry(id: &str, name: &str, parent: &str, kind: EntryKind) -> FileEntry {
        FileEntry {
            id: id.to_owned(),
            name: name.to_owned(),
            kind,
            size: 0,
            created_at: None,
            modified_at: None,
            parent_id: parent.to_owned(),
            md5: None,
            audit_flagged: false,
            starred: false,
            media: false,
            video: false,
            thumbnail_url: String::new(),
        }
    }
}
