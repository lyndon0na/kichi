//! 读索引：子目录、父链路径、搜索与星标。
//!
//! 查询结果转换成 [`FileEntry`] 供界面列表直接使用。schema 不存缩略图地址
//! 与审核状态（ADR 0024），因此这两项在这里是空 / false；媒体与视频标记
//! 按扩展名推导，保证播放入口不丢。

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use kichi_core::{normalize_query, EntryKind, FileEntry, TrashEntry};
use rusqlite::{Connection, OptionalExtension, Row};

/// 父链最多回溯多少层（防止环形父目录把线程拖死）。
pub(super) const MAX_PATH_DEPTH: usize = 64;

const NODE_COLUMNS: &str = "id, parent_id, name, kind, size, created_at, modified_at, md5, starred";

/// 一个目录下的在库条目（不含回收站）。
pub(super) fn list_dir(conn: &Connection, dir_id: &str) -> rusqlite::Result<Vec<FileEntry>> {
    let sql = format!(
        "SELECT {NODE_COLUMNS} FROM nodes \
         WHERE parent_id = ?1 AND trashed = 0 \
         ORDER BY kind DESC, name COLLATE NOCASE"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([dir_id], row_to_entry)?;
    rows.collect()
}

/// 目录快照仍新鲜时返回条目；否则 `None` 让调用方走网络。
pub(super) fn listing_if_fresh(
    conn: &Connection,
    dir_id: &str,
    now: i64,
    max_age_secs: i64,
) -> rusqlite::Result<Option<Vec<FileEntry>>> {
    let state = conn
        .query_row(
            "SELECT complete, dirty, fetched_at FROM dirs WHERE id = ?1",
            [dir_id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                ))
            },
        )
        .optional()?;
    let Some((complete, dirty, fetched_at)) = state else {
        return Ok(None);
    };
    if complete != 1 || dirty != 0 {
        return Ok(None);
    }
    let Some(fetched_at) = fetched_at else {
        return Ok(None);
    };
    // 时钟回拨时 now - fetched_at 为负，按「刚刷新」处理
    if now.saturating_sub(fetched_at) > max_age_secs {
        return Ok(None);
    }
    list_dir(conn, dir_id).map(Some)
}

/// 不管新鲜度，返回该目录索引里现有的在库条目。
///
/// 过期 / 脏 / 未补全的目录也能先拿旧列表渲染，避免首次进入时闪骨架。
/// 目录行不存在或已知条目为空时返回 `None`，调用方仍显示加载态。
pub(super) fn listing_any(
    conn: &Connection,
    dir_id: &str,
) -> rusqlite::Result<Option<Vec<FileEntry>>> {
    let known = conn
        .query_row("SELECT 1 FROM dirs WHERE id = ?1", [dir_id], |_| Ok(()))
        .optional()?
        .is_some();
    if !known {
        return Ok(None);
    }
    let entries = list_dir(conn, dir_id)?;
    Ok((!entries.is_empty()).then_some(entries))
}

/// 移动 / 复制选择器的一层目录：已知子文件夹 + 快照状态。
///
/// `known` 表示这个目录在索引里出现过（哪怕一行子文件夹都没有）；
/// `fresh` 才表示可以完全信任、0 网络直接渲染。
pub(crate) struct MoveTargets {
    pub(crate) folders: Vec<FileEntry>,
    pub(crate) known: bool,
    pub(crate) fresh: bool,
}

/// 只读子文件夹（`kind = 1`，排除回收站），并给出目录快照状态。
pub(super) fn move_targets(
    conn: &Connection,
    dir_id: &str,
    now: i64,
    max_age_secs: i64,
) -> rusqlite::Result<MoveTargets> {
    let state = conn
        .query_row(
            "SELECT complete, dirty, fetched_at FROM dirs WHERE id = ?1",
            [dir_id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                ))
            },
        )
        .optional()?;
    let sql = format!(
        "SELECT {NODE_COLUMNS} FROM nodes \
         WHERE parent_id = ?1 AND kind = 1 AND trashed = 0 \
         ORDER BY name COLLATE NOCASE, id"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([dir_id], row_to_entry)?;
    let folders = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    let fresh = state.is_some_and(|(complete, dirty, fetched_at)| {
        complete == 1
            && dirty == 0
            && fetched_at.is_some_and(|at| now.saturating_sub(at) <= max_age_secs)
    });
    Ok(MoveTargets {
        folders,
        known: state.is_some(),
        fresh,
    })
}

/// 本地未删除文件的大小和（canary 探针与配额用量对账用）。
pub(super) fn local_file_bytes(conn: &Connection) -> rusqlite::Result<u64> {
    conn.query_row(
        "SELECT COALESCE(SUM(size), 0) FROM nodes WHERE trashed = 0 AND kind = 0",
        [],
        |row| row.get::<_, i64>(0),
    )
    .map(|value| value.max(0) as u64)
}

/// 索引里已知的一条条目 + 是否在回收站。
pub(crate) struct KnownNode {
    pub(crate) entry: FileEntry,
    pub(crate) trashed: bool,
}

pub(super) fn nodes_by_ids(conn: &Connection, ids: &[String]) -> rusqlite::Result<Vec<KnownNode>> {
    let sql = format!("SELECT {NODE_COLUMNS}, trashed FROM nodes WHERE id = ?1");
    let mut stmt = conn.prepare(&sql)?;
    let mut known = Vec::with_capacity(ids.len());
    for id in ids {
        let found = stmt
            .query_row([id], |row| {
                Ok(KnownNode {
                    entry: row_to_entry(row)?,
                    trashed: row.get::<_, i64>(9)? != 0,
                })
            })
            .optional()?;
        if let Some(node) = found {
            known.push(node);
        }
    }
    Ok(known)
}

/// canary 探针需要的本地状态（一次短事务读出，网络请求不持锁）。
pub(crate) struct ProbeSnapshot {
    pub(crate) local_bytes: u64,
    pub(crate) known: Vec<KnownNode>,
    pub(crate) last_probe_at: Option<i64>,
    pub(crate) last_full_build_at: Option<i64>,
    pub(crate) unavailable: bool,
    pub(crate) last_suspect_rebuild_at: Option<i64>,
}

pub(super) fn probe_snapshot(
    conn: &Connection,
    recent_ids: &[String],
) -> rusqlite::Result<ProbeSnapshot> {
    Ok(ProbeSnapshot {
        local_bytes: local_file_bytes(conn)?,
        known: nodes_by_ids(conn, recent_ids)?,
        last_probe_at: meta_i64(conn, "last_probe_at")?,
        last_full_build_at: meta_i64(conn, "last_full_build_at")?,
        unavailable: super::store::meta_get(conn, "probe_unavailable")?.is_some(),
        last_suspect_rebuild_at: meta_i64(conn, "last_suspect_rebuild_at")?,
    })
}

fn meta_i64(conn: &Connection, key: &str) -> rusqlite::Result<Option<i64>> {
    Ok(super::store::meta_get(conn, key)?.and_then(|value| value.parse::<i64>().ok()))
}

/// 回收站快照仍新鲜时返回条目；否则 `None` 让调用方走网络。
///
/// 只返回回收站里的「根」条目：本地移入文件夹时会把子孙一起置为
/// `trashed = 1`，但回收站列表只应展示用户移入的那一项。父行不在索引里时
/// 仍按根条目展示，避免索引不完整时把真正的回收站条目漏掉。
pub(super) fn trash_listing_if_fresh(
    conn: &Connection,
    now: i64,
    max_age_secs: i64,
) -> rusqlite::Result<Option<Vec<TrashEntry>>> {
    let fetched_at = super::store::meta_get(conn, "trash_fetched_at")?
        .and_then(|value| value.parse::<i64>().ok());
    let Some(fetched_at) = fetched_at else {
        return Ok(None);
    };
    if now.saturating_sub(fetched_at) > max_age_secs {
        return Ok(None);
    }
    let sql = format!(
        "SELECT {NODE_COLUMNS}, deleted_at FROM nodes n \
         WHERE n.trashed = 1 \
           AND NOT EXISTS (SELECT 1 FROM nodes p WHERE p.id = n.parent_id AND p.trashed = 1) \
         ORDER BY deleted_at DESC, name COLLATE NOCASE"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], row_to_trash_entry)?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map(Some)
}

/// 根 → 目标文件夹自身的 `(id, 名称)` 列表；索引里缺链时返回 `None`。
pub(super) fn path_to(
    conn: &Connection,
    folder_id: &str,
) -> rusqlite::Result<Option<Vec<(String, String)>>> {
    let mut trail = Vec::new();
    let mut current = folder_id.trim().to_owned();
    while !current.is_empty() {
        if trail.len() >= MAX_PATH_DEPTH {
            return Ok(None);
        }
        let found = conn
            .query_row(
                "SELECT name, parent_id FROM nodes WHERE id = ?1 AND trashed = 0",
                [&current],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        let Some((name, parent)) = found else {
            return Ok(None);
        };
        trail.push((current, name));
        current = parent.trim().to_owned();
    }
    trail.reverse();
    Ok(Some(trail))
}

/// 在库条目的自身名称与父目录 id；索引里没有时返回 `None`。
pub(super) fn parent_location(
    conn: &Connection,
    entry_id: &str,
) -> rusqlite::Result<Option<(String, String)>> {
    conn.query_row(
        "SELECT name, parent_id FROM nodes WHERE id = ?1 AND trashed = 0",
        [entry_id.trim()],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
    )
    .optional()
}

/// 名字包含匹配（`instr(norm_name, ?)`）；关键词按同一套规则归一化。
pub(super) fn search(
    conn: &Connection,
    query: &str,
    limit: usize,
) -> rusqlite::Result<Vec<FileEntry>> {
    let needle = normalize_query(query);
    if needle.is_empty() || limit == 0 {
        return Ok(Vec::new());
    }
    let sql = format!(
        "SELECT {NODE_COLUMNS} FROM nodes \
         WHERE trashed = 0 AND instr(norm_name, ?1) > 0 \
         ORDER BY name COLLATE NOCASE LIMIT ?2"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params![needle, limit as i64], row_to_entry)?;
    rows.collect()
}

/// 一条本地搜索命中：条目 + 回收站状态。
///
/// `entry.parent_id` 是条目在回收站之前的原始父目录；`trashed` 为真时
/// 界面要标「回收站」，并且不能把它当成在库文件夹导航。
pub(crate) struct IndexSearchHit {
    pub(crate) entry: FileEntry,
    pub(crate) trashed: bool,
    pub(crate) deleted_at: Option<DateTime<Utc>>,
}

/// 全盘名字包含匹配，**包含回收站行**。
///
/// 匹配用归一化后的 `norm_name`（`instr` 只做粗筛，档位排序在
/// `kichi-core` 里做）；没有条数上限，调用方决定展示多少。SQL 先按
/// `name COLLATE NOCASE, id` 排好，给同档结果一个稳定兜底。
pub(super) fn search_with_trash(
    conn: &Connection,
    query: &str,
) -> rusqlite::Result<Vec<IndexSearchHit>> {
    let needle = normalize_query(query);
    if needle.is_empty() {
        return Ok(Vec::new());
    }
    let sql = format!(
        "SELECT {NODE_COLUMNS}, trashed, deleted_at FROM nodes \
         WHERE instr(norm_name, ?1) > 0 \
         ORDER BY name COLLATE NOCASE, id"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([needle], row_to_search_hit)?;
    rows.collect()
}

/// 一批文件夹 id → 「父/子/孙」路径；只读索引，不回网络。
///
/// 同一次搜索里大量命中共享父目录，所以节点元信息与已经算好的路径都缓存：
/// 祖先第一次走通后，后代直接复用前缀。找不到 / 成环 / 超深度的 id 不会
/// 出现在结果里（界面显示「（所在文件夹）」而不是拿 id 冒充）。
/// 路径解析**不过滤 trashed**：回收站条目要显示还原前的原始位置。
pub(super) fn folder_paths(
    conn: &Connection,
    folder_ids: &[String],
) -> rusqlite::Result<HashMap<String, String>> {
    let mut nodes: HashMap<String, Option<(String, String)>> = HashMap::new();
    let mut paths: HashMap<String, Option<Vec<String>>> = HashMap::new();
    let mut resolved = HashMap::new();
    for id in folder_ids {
        let id = id.trim();
        if id.is_empty() || resolved.contains_key(id) {
            continue;
        }
        if let Some(parts) = resolve_path(conn, id, &mut nodes, &mut paths)? {
            resolved.insert(id.to_owned(), parts.join("/"));
        }
    }
    Ok(resolved)
}

fn resolve_path(
    conn: &Connection,
    start: &str,
    nodes: &mut HashMap<String, Option<(String, String)>>,
    paths: &mut HashMap<String, Option<Vec<String>>>,
) -> rusqlite::Result<Option<Vec<String>>> {
    if let Some(cached) = paths.get(start) {
        return Ok(cached.clone());
    }
    let mut chain: Vec<(String, String)> = Vec::new();
    let mut visited = HashSet::new();
    let mut current = start.to_owned();
    let mut base = Vec::new();
    loop {
        if current.is_empty() {
            break;
        }
        // 超深度或父链成环：这一条链上的节点都不可信
        if !visited.insert(current.clone()) || chain.len() >= MAX_PATH_DEPTH {
            paths.insert(start.to_owned(), None);
            for (id, _) in &chain {
                paths.insert(id.clone(), None);
            }
            return Ok(None);
        }
        if let Some(cached) = paths.get(&current) {
            match cached {
                Some(prefix) => {
                    base = prefix.clone();
                    break;
                }
                None => {
                    paths.insert(start.to_owned(), None);
                    for (id, _) in &chain {
                        paths.insert(id.clone(), None);
                    }
                    return Ok(None);
                }
            }
        }
        let meta = match nodes.get(&current) {
            Some(meta) => meta.clone(),
            None => {
                let found = conn
                    .query_row(
                        "SELECT name, parent_id FROM nodes WHERE id = ?1",
                        [&current],
                        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                    )
                    .optional()?;
                nodes.insert(current.clone(), found.clone());
                found
            }
        };
        let Some((name, parent)) = meta else {
            paths.insert(start.to_owned(), None);
            for (id, _) in &chain {
                paths.insert(id.clone(), None);
            }
            return Ok(None);
        };
        chain.push((current.clone(), name));
        current = parent.trim().to_owned();
    }
    let mut parts = base;
    for (id, name) in chain.iter().rev() {
        parts.push(name.clone());
        paths.insert(id.clone(), Some(parts.clone()));
    }
    paths.insert(start.to_owned(), Some(parts.clone()));
    Ok(Some(parts))
}

pub(super) fn starred(conn: &Connection, limit: usize) -> rusqlite::Result<Vec<FileEntry>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    // 与云端 `starred_entries` 的排序一致：更新时间降序 → 名称 → id，
    // 这样索引与回退路径切换时行序不会跳变。SQLite 的 DESC 把 NULL 排最后。
    let sql = format!(
        "SELECT {NODE_COLUMNS} FROM nodes \
         WHERE trashed = 0 AND starred = 1 \
         ORDER BY modified_at DESC, name, id LIMIT ?1"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([limit as i64], row_to_entry)?;
    rows.collect()
}

fn row_to_entry(row: &Row<'_>) -> rusqlite::Result<FileEntry> {
    let kind = match row.get::<_, i64>(3)? {
        1 => EntryKind::Folder,
        _ => EntryKind::File,
    };
    let name: String = row.get(2)?;
    let is_file = kind == EntryKind::File;
    Ok(FileEntry {
        id: row.get(0)?,
        parent_id: row.get(1)?,
        name: name.clone(),
        kind,
        size: row.get::<_, i64>(4)?.max(0) as u64,
        created_at: unix_time(row.get(5)?),
        modified_at: unix_time(row.get(6)?),
        md5: row.get(7)?,
        starred: row.get::<_, i64>(8)? != 0,
        audit_flagged: false,
        media: is_file && kichi_core::is_media_name(&name),
        video: is_file && kichi_core::is_video_name(&name),
        thumbnail_url: String::new(),
    })
}

/// 搜索行的前 9 列与普通条目相同，第 10 列是回收站标记，第 11 列是删除时间。
fn row_to_search_hit(row: &Row<'_>) -> rusqlite::Result<IndexSearchHit> {
    Ok(IndexSearchHit {
        entry: row_to_entry(row)?,
        trashed: row.get::<_, i64>(9)? != 0,
        deleted_at: unix_time(row.get(10)?),
    })
}

/// 回收站行的前 9 列与普通条目相同，第 10 列是删除时间。
fn row_to_trash_entry(row: &Row<'_>) -> rusqlite::Result<TrashEntry> {
    Ok(TrashEntry {
        entry: row_to_entry(row)?,
        parent_id: row.get(1)?,
        deleted_at: unix_time(row.get(9)?),
    })
}

fn unix_time(value: Option<i64>) -> Option<DateTime<Utc>> {
    value.and_then(|seconds| DateTime::from_timestamp(seconds, 0))
}

#[cfg(test)]
#[path = "query_tests.rs"]
mod tests;
