//! 写索引：目录列表落库、局部变更、回收站快照与目录未完成标记。
//!
//! `complete = true` 表示「这个目录的当前全部条目都在这一批里」：先删掉该
//! 目录下的在库行（`trashed = 0`）再 upsert，**保留回收站行**（它们仍带
//! 原始 `parent_id`，回收站读路径要用）。
//!
//! `complete = false` 只 upsert，不删任何行，也不能把已有的完整快照降级
//! 或延长它的新鲜期。
//!
//! 回收站的写操作按子树处理：移入 / 还原 / 彻底删除都要连带索引里已存在的
//! 子孙行，否则恢复后旧列表会缺项。子树的递归用 `parent_id` 上的 CTE，
//! `UNION` 同时去重并挡住环。

use chrono::{DateTime, Utc};
use kichi_core::{normalize_name, EntryKind, FileEntry, TrashEntry};
use rusqlite::{params, Connection, Statement};

/// 节点 upsert。全量建库与目录列表共用同一份字段映射。
///
/// 正常（非回收站）写入把 `deleted_at` 清空：条目若从回收站回到在库列表，
/// 旧的删除时间不能继续留着。
pub(super) const UPSERT_NODE_SQL: &str = "
INSERT INTO nodes (
  id, parent_id, name, norm_name, kind, size, created_at, modified_at,
  md5, starred, trashed, duration, width, height
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, NULL, NULL, NULL)
ON CONFLICT(id) DO UPDATE SET
  parent_id = excluded.parent_id,
  name = excluded.name,
  norm_name = excluded.norm_name,
  kind = excluded.kind,
  size = excluded.size,
  created_at = excluded.created_at,
  modified_at = excluded.modified_at,
  md5 = excluded.md5,
  starred = excluded.starred,
  trashed = excluded.trashed,
  deleted_at = NULL,
  duration = COALESCE(excluded.duration, nodes.duration),
  width = COALESCE(excluded.width, nodes.width),
  height = COALESCE(excluded.height, nodes.height)
";

/// 回收站快照 upsert：父目录与删除时间来自 [`TrashEntry`] 自身的字段，
/// 不能用 `entry.parent_id`（官方对回收站条目的 location 与原始父目录可能不同）。
const UPSERT_TRASH_NODE_SQL: &str = "
INSERT INTO nodes (
  id, parent_id, name, norm_name, kind, size, created_at, modified_at,
  md5, starred, trashed, deleted_at, duration, width, height
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 1, ?11, NULL, NULL, NULL)
ON CONFLICT(id) DO UPDATE SET
  parent_id = excluded.parent_id,
  name = excluded.name,
  norm_name = excluded.norm_name,
  kind = excluded.kind,
  size = excluded.size,
  created_at = excluded.created_at,
  modified_at = excluded.modified_at,
  md5 = excluded.md5,
  starred = excluded.starred,
  trashed = 1,
  deleted_at = excluded.deleted_at,
  duration = COALESCE(excluded.duration, nodes.duration),
  width = COALESCE(excluded.width, nodes.width),
  height = COALESCE(excluded.height, nodes.height)
";

/// 子树 CTE 的三种收尾：移入、还原、彻底删除。
const TRASH_SUBTREE_SQL: &str = concat!(
    "WITH RECURSIVE sub(id) AS (",
    " SELECT ?1 UNION",
    " SELECT n.id FROM nodes n JOIN sub ON n.parent_id = sub.id",
    ") UPDATE nodes SET trashed = 1, deleted_at = ?2 WHERE id IN (SELECT id FROM sub)"
);

const RESTORE_SUBTREE_SQL: &str = concat!(
    "WITH RECURSIVE sub(id) AS (",
    " SELECT ?1 UNION",
    " SELECT n.id FROM nodes n JOIN sub ON n.parent_id = sub.id",
    ") UPDATE nodes SET trashed = 0, deleted_at = NULL WHERE id IN (SELECT id FROM sub)"
);

const REMOVE_SUBTREE_SQL: &str = concat!(
    "WITH RECURSIVE sub(id) AS (",
    " SELECT ?1 UNION",
    " SELECT n.id FROM nodes n JOIN sub ON n.parent_id = sub.id",
    ") DELETE FROM nodes WHERE id IN (SELECT id FROM sub)"
);

/// 一条目录列表写入索引。
///
/// `dir_id` 为空串表示根目录；`now` 是 unix 秒。
pub(super) fn put_listing(
    conn: &Connection,
    dir_id: &str,
    entries: &[FileEntry],
    complete: bool,
    now: i64,
) -> rusqlite::Result<()> {
    let tx = conn.unchecked_transaction()?;
    if complete {
        // 回收站行保留：它们仍属于这个父目录，但不在当前列表里
        tx.execute(
            "DELETE FROM nodes WHERE parent_id = ?1 AND trashed = 0",
            [dir_id],
        )?;
    }
    {
        let mut stmt = tx.prepare(UPSERT_NODE_SQL)?;
        for entry in entries {
            insert_entry(&mut stmt, entry, false)?;
        }
    }
    if complete {
        tx.execute(
            "INSERT INTO dirs(id, complete, dirty, fetched_at) VALUES(?1, 1, 0, ?2) \
             ON CONFLICT(id) DO UPDATE SET \
             complete = 1, dirty = 0, fetched_at = excluded.fetched_at",
            params![dir_id, now],
        )?;
    } else {
        // 已有完整快照时只更新 fetched_at 会让旧行显得「刚刷新过」：
        // 不完整的一批数据不能延长完整快照的新鲜期。
        tx.execute(
            "INSERT INTO dirs(id, complete, dirty, fetched_at) VALUES(?1, 0, 0, ?2) \
             ON CONFLICT(id) DO UPDATE SET fetched_at = excluded.fetched_at \
             WHERE dirs.complete = 0",
            params![dir_id, now],
        )?;
    }
    tx.commit()
}

/// 把条目绑定进已准备好的 upsert 语句。
pub(super) fn insert_entry(
    stmt: &mut Statement<'_>,
    entry: &FileEntry,
    trashed: bool,
) -> rusqlite::Result<()> {
    let kind = match entry.kind {
        EntryKind::Folder => 1_i64,
        EntryKind::File => 0_i64,
    };
    stmt.execute(params![
        entry.id,
        entry.parent_id,
        entry.name,
        normalize_name(&entry.name),
        kind,
        entry.size as i64,
        unix(entry.created_at),
        unix(entry.modified_at),
        entry.md5,
        if entry.starred { 1_i64 } else { 0_i64 },
        if trashed { 1_i64 } else { 0_i64 },
    ])?;
    Ok(())
}

/// 一次 canary 探针要落库的内容。
pub(super) struct ProbeRecord<'a> {
    /// 探针页面里发现的新条目 / 变更（按 id upsert，不删旧行）
    pub entries: &'a [FileEntry],
    /// 探针完成时间（unix 秒）
    pub now: i64,
    /// 配额用量，状态与排障参考
    pub usage: u64,
    /// 把「探针不可用」记为已提示一次（下次可信时清掉）
    pub unavailable: bool,
    /// 本次判定为 suspect 且确实要重建：记下重建时间做冷却
    pub suspect_rebuild: bool,
}

/// 探针结果落库：补丁 + 时间戳 + 不可用 / 重建冷却标记，一个事务。
pub(super) fn record_probe(conn: &Connection, record: &ProbeRecord<'_>) -> rusqlite::Result<()> {
    let tx = conn.unchecked_transaction()?;
    {
        let mut stmt = tx.prepare(UPSERT_NODE_SQL)?;
        for entry in record.entries {
            insert_entry(&mut stmt, entry, false)?;
        }
    }
    super::store::meta_set(&tx, "last_probe_at", &record.now.to_string())?;
    super::store::meta_set(&tx, "last_usage", &record.usage.to_string())?;
    if record.unavailable {
        super::store::meta_set(&tx, "probe_unavailable", "1")?;
    } else {
        tx.execute("DELETE FROM meta WHERE key = 'probe_unavailable'", [])?;
    }
    if record.suspect_rebuild {
        super::store::meta_set(&tx, "last_suspect_rebuild_at", &record.now.to_string())?;
    }
    tx.commit()
}

/// 只记录一次探针尝试（网络失败时用）：不动节点，只让冷却生效。
pub(super) fn note_probe_attempt(conn: &Connection, now: i64) -> rusqlite::Result<()> {
    super::store::meta_set(conn, "last_probe_at", &now.to_string())
}

/// 把回收站条目绑定进已准备好的快照 upsert 语句。
fn insert_trash_entry(stmt: &mut Statement<'_>, entry: &TrashEntry) -> rusqlite::Result<()> {
    let kind = match entry.entry.kind {
        EntryKind::Folder => 1_i64,
        EntryKind::File => 0_i64,
    };
    stmt.execute(params![
        entry.entry.id,
        entry.parent_id,
        entry.entry.name,
        normalize_name(&entry.entry.name),
        kind,
        entry.entry.size as i64,
        unix(entry.entry.created_at),
        unix(entry.entry.modified_at),
        entry.entry.md5,
        if entry.entry.starred { 1_i64 } else { 0_i64 },
        unix(entry.deleted_at),
    ])?;
    Ok(())
}

/// 用服务端返回的完整回收站替换本地快照，并记录快照时间。
///
/// 先删掉所有 `trashed = 1` 行再从服务端条目重建：被还原 / 彻底删除的旧行
/// 不会残留，官方任务在别处产生的变化也能在这里收敛。
pub(super) fn replace_trash(
    conn: &Connection,
    entries: &[TrashEntry],
    now: i64,
) -> rusqlite::Result<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute("DELETE FROM nodes WHERE trashed = 1", [])?;
    {
        let mut stmt = tx.prepare(UPSERT_TRASH_NODE_SQL)?;
        for entry in entries {
            insert_trash_entry(&mut stmt, entry)?;
        }
    }
    super::store::meta_set(&tx, "trash_fetched_at", &now.to_string())?;
    tx.commit()
}

/// 局部变更。
pub enum Change<'a> {
    /// 新增 / 覆盖一条在库条目。
    Put(&'a FileEntry),
    /// 移入回收站：条目与索引里已存在的子孙都置为回收站，保留原始 `parent_id`。
    Trash(&'a [String]),
    /// 从回收站还原：条目与子孙恢复在库，原始 `parent_id` 不变。
    Restore(&'a [String]),
    /// 彻底删除：删掉条目与索引里已存在的子孙。
    Remove(&'a [String]),
    /// 清空回收站：删掉所有 `trashed = 1` 的行。
    RemoveAllTrashed,
    Rename {
        id: &'a str,
        name: &'a str,
    },
    Starred {
        ids: &'a [String],
        starred: bool,
    },
    Move {
        ids: &'a [String],
        parent_id: &'a str,
    },
}

pub(super) fn apply_change(
    conn: &Connection,
    change: Change<'_>,
    now: i64,
) -> rusqlite::Result<()> {
    let tx = conn.unchecked_transaction()?;
    match change {
        Change::Put(entry) => {
            {
                let mut stmt = tx.prepare(UPSERT_NODE_SQL)?;
                insert_entry(&mut stmt, entry, false)?;
            }
            mark_dir_dirty(&tx, entry.parent_id.trim())?;
        }
        Change::Trash(ids) => {
            for id in ids {
                if let Some(parent) = parent_of_id(&tx, id)? {
                    mark_dir_dirty(&tx, &parent)?;
                }
                tx.execute(TRASH_SUBTREE_SQL, params![id, now])?;
            }
            invalidate_trash_snapshot(&tx)?;
        }
        Change::Restore(ids) => {
            for id in ids {
                if let Some(parent) = parent_of_id(&tx, id)? {
                    mark_dir_dirty(&tx, &parent)?;
                }
                tx.execute(RESTORE_SUBTREE_SQL, [id])?;
            }
            invalidate_trash_snapshot(&tx)?;
        }
        Change::Remove(ids) => {
            for id in ids {
                if let Some(parent) = parent_of_id(&tx, id)? {
                    mark_dir_dirty(&tx, &parent)?;
                }
                tx.execute(REMOVE_SUBTREE_SQL, [id])?;
            }
            invalidate_trash_snapshot(&tx)?;
        }
        Change::RemoveAllTrashed => {
            let parents = {
                let mut stmt =
                    tx.prepare("SELECT DISTINCT parent_id FROM nodes WHERE trashed = 1")?;
                let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
                rows.collect::<rusqlite::Result<Vec<_>>>()?
            };
            for parent in parents {
                mark_dir_dirty(&tx, parent.trim())?;
            }
            tx.execute("DELETE FROM nodes WHERE trashed = 1", [])?;
            invalidate_trash_snapshot(&tx)?;
        }
        Change::Rename { id, name } => {
            if let Some(parent) = parent_of_id(&tx, id)? {
                tx.execute(
                    "UPDATE nodes SET name = ?2, norm_name = ?3 WHERE id = ?1",
                    params![id, name, normalize_name(name)],
                )?;
                mark_dir_dirty(&tx, &parent)?;
            }
        }
        Change::Starred { ids, starred } => {
            let value = if starred { 1_i64 } else { 0_i64 };
            for id in ids {
                tx.execute(
                    "UPDATE nodes SET starred = ?2 WHERE id = ?1",
                    params![id, value],
                )?;
            }
        }
        Change::Move {
            ids,
            parent_id: new_parent,
        } => {
            for id in ids {
                let Some(old_parent) = parent_of_id(&tx, id)? else {
                    continue;
                };
                tx.execute(
                    "UPDATE nodes SET parent_id = ?2 WHERE id = ?1",
                    params![id, new_parent],
                )?;
                mark_dir_dirty(&tx, &old_parent)?;
                mark_dir_dirty(&tx, new_parent.trim())?;
            }
        }
    }
    tx.commit()
}

/// 网络父链回填：只补文件夹自身的名字与父目录，不动目录快照状态。
pub(super) fn backfill_trail(
    conn: &Connection,
    trail: &[(String, String)],
) -> rusqlite::Result<()> {
    let tx = conn.unchecked_transaction()?;
    for (index, (id, name)) in trail.iter().enumerate() {
        let parent = index
            .checked_sub(1)
            .and_then(|previous| trail.get(previous))
            .map(|(parent, _)| parent.as_str())
            .unwrap_or("");
        tx.execute(
            "INSERT INTO nodes(id, parent_id, name, norm_name, kind, starred, trashed) \
             VALUES(?1, ?2, ?3, ?4, 1, 0, 0) \
             ON CONFLICT(id) DO UPDATE SET \
             parent_id = excluded.parent_id, name = excluded.name, \
             norm_name = excluded.norm_name, kind = 1, trashed = 0",
            params![id, parent, name, normalize_name(name)],
        )?;
    }
    tx.commit()
}

/// 本地回收站写操作让快照不再可信：删掉快照时间戳，读路径下次回网络。
fn invalidate_trash_snapshot(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM meta WHERE key = 'trash_fetched_at'", [])?;
    Ok(())
}

fn parent_of_id(conn: &Connection, id: &str) -> rusqlite::Result<Option<String>> {
    use rusqlite::OptionalExtension;
    conn.query_row("SELECT parent_id FROM nodes WHERE id = ?1", [id], |row| {
        row.get(0)
    })
    .optional()
}

/// 目录内容已不可信：下次读路径必须重新校验；不改变 complete 标记。
pub(super) fn mark_dir_dirty(conn: &Connection, dir_id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO dirs(id, complete, dirty, fetched_at) VALUES(?1, 0, 1, NULL) \
         ON CONFLICT(id) DO UPDATE SET dirty = 1",
        [dir_id],
    )?;
    Ok(())
}

fn unix(value: Option<DateTime<Utc>>) -> Option<i64> {
    value.map(|time| time.timestamp())
}

#[cfg(test)]
#[path = "apply_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "trash_apply_tests.rs"]
mod trash_tests;
