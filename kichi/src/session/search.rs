//! 搜索的会话面：本地索引全盘搜索 + 云端逐页扫描（回退）。
//!
//! 索引就绪时，每次有效输入都走 [`Session::search_index_local`]（0 请求）；
//! 索引未就绪才退回 `search_all` 遍历网盘，或由界面只筛当前目录。
//!
//! 云端扫描每页回调一次进度（调用方决定怎么显示），`Cancel` 随时可中断，
//! 结束时给出三态之一。线程、回投、界面状态在
//! `kichi/src/cxxqt_object/search.rs`。
//!
//! 全部函数阻塞，调用方负责丢到后台线程。

use std::collections::HashMap;
use std::sync::Arc;

use kichi_core::search::{matches, FileHit};
use kichi_core::{MatchTier, SearchMatcher};

use super::download::Cancel;
use super::tree_index;
use super::{Failure, Session};

/// 本地搜索最多为多少条命中解析路径（界面也只显示这么多行）。
pub const SEARCH_PATH_LIMIT: usize = 500;

/// 扫描结束时为什么停下。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanEnd {
    /// 官方不再给 `next_page_token`：全盘扫完
    Finished,
    /// 用户按了停止（已扫出的结果保留）
    Stopped,
    /// 官方分页异常（token 原地打转），结果可能不全
    BrokenPaging,
}

impl ScanEnd {
    /// 界面用的短标记（QML 据此决定状态文案）。
    pub fn as_str(self) -> &'static str {
        match self {
            ScanEnd::Finished => "finished",
            ScanEnd::Stopped => "stopped",
            ScanEnd::BrokenPaging => "broken",
        }
    }
}

/// 一批扫描进度（一页一投）。
#[derive(Debug, Clone)]
pub struct ScanBatch {
    /// 目前为止扫过的条目总数（不是页数）
    pub scanned: usize,
    /// 本页的命中（已按关键词过滤）
    pub hits: Vec<SearchHit>,
}

/// 一条搜索命中：条目 + 所在文件夹 + 扫到的顺序。
#[derive(Debug, Clone)]
pub struct SearchHit {
    /// 完整的列表条目（界面按目录列表那一套字段显示）
    pub entry: kichi_core::FileEntry,
    /// 所在文件夹 id：点进去直接到真实层级；空串 = 根目录
    pub folder_id: String,
    /// 所在文件夹的真实路径（尽力还原；拼不出时为 `None`）
    pub folder_path: Option<String>,
    /// 扫到的顺序：结果「先到先显示」，同批结果按它排
    pub order: usize,
    /// 条目当前在回收站：界面要标「回收站」，且不能当在库文件夹导航
    pub in_trash: bool,
}

/// 一次本地索引搜索的结果。
#[derive(Debug, Clone)]
pub struct IndexSearchResult {
    /// 全部命中（已按匹配档位 + 名字排好），不只是界面显示的前 500 条
    pub hits: Vec<SearchHit>,
    /// 其中回收站条目数（状态行用）
    pub trash_hits: usize,
    /// 最近一次全量建库时间（unix 秒）；状态行说明「索引更新于」用
    pub built_at: Option<i64>,
}

impl SearchHit {
    pub fn id(&self) -> &str {
        &self.entry.id
    }

    pub fn name(&self) -> &str {
        &self.entry.name
    }

    pub fn is_folder(&self) -> bool {
        self.entry.kind == kichi_core::EntryKind::Folder
    }
}

impl Session {
    /// 全盘扫描（逐页；阻塞；可中断）。
    ///
    /// `query` 按名字字面包含匹配；`on_batch` 每扫完一页调一次，
    /// 调用方在这里把结果搬到界面上（不要在里面做慢操作：它跑在扫描线程上）。
    pub fn search_all(
        &self,
        query: &str,
        cancel: &Cancel,
        on_batch: &mut dyn FnMut(ScanBatch),
    ) -> Result<ScanEnd, Failure> {
        let query = query.trim();
        if query.is_empty() {
            return Err(Failure::Other("搜索关键词不能为空".into()));
        }
        let client = self.drive()?;
        let mut path = PathCache::default();
        let mut scanned = 0usize;
        let mut page_token: Option<String> = None;

        loop {
            if cancel.is_cancelled() {
                return Ok(ScanEnd::Stopped);
            }
            let (entries, next) = client
                .search_page(query, page_token.as_deref())
                .map_err(Failure::from)?;
            scanned += entries.len();

            let mut hits = Vec::new();
            // 页内每条都要有自己的顺序号：同页共用一个值会让排序落到 HashMap 的
            // 随机遍历上，同一页的结果顺序每次都不太一样（known-issues §4.1）。
            let page_start = scanned - entries.len();
            for (index, hit) in entries.into_iter().enumerate() {
                if !matches(&hit.entry.name, query) {
                    continue;
                }
                let (folder_id, folder_path) = folder_of(&client, &mut path, &hit);
                hits.push(SearchHit {
                    entry: hit.entry,
                    folder_id,
                    folder_path,
                    order: page_start + index,
                    in_trash: false,
                });
            }
            on_batch(ScanBatch { scanned, hits });

            let Some(next) = next else {
                return Ok(ScanEnd::Finished);
            };
            // 同一个 token 再取一次还是同一页：官方分页卡住了，别无限循环
            if page_token.as_deref() == Some(next.as_str()) {
                return Ok(ScanEnd::BrokenPaging);
            }
            page_token = Some(next);
        }
    }
    /// 本地索引全盘搜索（阻塞；调用方放后台线程）。
    ///
    /// 匹配包含回收站行；排序规则全部来自 [`SearchMatcher`]：完全同名 >
    /// 前缀 > 词首 > 包含，同档按名字。只给界面显示的前
    /// [`SEARCH_PATH_LIMIT`] 条解析路径，且路径全部来自索引，不回网络。
    pub fn search_index_local(&self, query: &str) -> Result<IndexSearchResult, Failure> {
        let query = query.trim();
        if query.is_empty() {
            return Err(Failure::Other("搜索关键词不能为空".into()));
        }
        let index = self
            .tree_index()
            .ok_or_else(|| Failure::Other("本地索引尚未就绪".into()))?;
        if index.is_closed() {
            return Err(Failure::Other("本地索引尚未就绪".into()));
        }
        if index.needs_full_build().map_err(index_failure)? {
            return Err(Failure::Other("本地索引尚未就绪".into()));
        }
        let matcher =
            SearchMatcher::new(query).ok_or_else(|| Failure::Other("搜索关键词不能为空".into()))?;

        let mut rows: Vec<(Option<(MatchTier, String)>, tree_index::IndexSearchHit)> = index
            .search_with_trash(query)
            .map_err(index_failure)?
            .into_iter()
            .map(|row| (matcher.sort_key(&row.entry.name), row))
            .filter(|(key, _)| key.is_some())
            .collect();
        rows.sort_by(|(left, a), (right, b)| {
            left.cmp(right).then_with(|| a.entry.id.cmp(&b.entry.id))
        });

        let folder_ids: Vec<String> = rows
            .iter()
            .take(SEARCH_PATH_LIMIT)
            .map(|(_, row)| row.entry.parent_id.trim().to_owned())
            .filter(|id| !id.is_empty())
            .collect();
        let paths = index.folder_paths(&folder_ids).map_err(index_failure)?;
        let built_at = index.last_full_build_at().map_err(index_failure)?;

        let mut trash_hits = 0usize;
        let hits = rows
            .into_iter()
            .enumerate()
            .map(|(order, (_, row))| {
                let folder_id = row.entry.parent_id.trim().to_owned();
                if row.trashed {
                    trash_hits += 1;
                }
                SearchHit {
                    folder_path: if order < SEARCH_PATH_LIMIT {
                        paths.get(&folder_id).cloned()
                    } else {
                        None
                    },
                    entry: row.entry,
                    folder_id,
                    order,
                    in_trash: row.trashed,
                }
            })
            .collect();

        Ok(IndexSearchResult {
            hits,
            trash_hits,
            built_at,
        })
    }
}

fn index_failure(err: tree_index::IndexError) -> Failure {
    Failure::Other(format!("本地索引读取失败：{err}"))
}

/// 一条命中的「所在文件夹」：能给出 id 就给出（点进去直接进真实层级），
/// 路径尽力而为（根目录的条目没有路径，界面显示「根目录」）。
fn folder_of(
    client: &kichi_core::DriveClient,
    path: &mut PathCache,
    hit: &FileHit,
) -> (String, Option<String>) {
    let folder_id = hit.parent_id.trim().to_owned();
    if folder_id.is_empty() {
        return (String::new(), None);
    }
    let folder_path = path.resolve(client, &folder_id);
    (folder_id, folder_path)
}

/// id → (名字, 父目录 id) 的缓存，用来把 `parent_id` 还原成真实路径。
///
/// 全盘扫描里每条命中都要拼路径，逐条回溯会重复请求同一批文件夹——
/// 所以查过的都记下来（失败也只查一次，不留假数据）。
#[derive(Default)]
struct PathCache {
    names: HashMap<String, Option<(String, String)>>,
}

impl PathCache {
    /// 文件夹 id → `父/子/孙` 形式的路径；查不到时返回 `None`。
    fn resolve(&mut self, client: &kichi_core::DriveClient, folder_id: &str) -> Option<String> {
        let mut names = Vec::new();
        let mut current = folder_id.to_owned();
        // 防环：正常层级最多几十层
        let mut guard = 0;
        while !current.is_empty() {
            guard += 1;
            if guard > MAX_PATH_DEPTH {
                names.reverse();
                return Some(names.join("/"));
            }
            let (name, parent) = self.lookup(client, &current)?;
            names.push(name);
            current = parent;
        }
        names.reverse();
        Some(names.join("/"))
    }

    fn lookup(
        &mut self,
        client: &kichi_core::DriveClient,
        folder_id: &str,
    ) -> Option<(String, String)> {
        if let Some(cached) = self.names.get(folder_id) {
            return cached.clone();
        }
        let found = folder_meta(client, folder_id);
        self.names.insert(folder_id.to_owned(), found.clone());
        found
    }
}

/// 路径最多回溯多少层（防环形结构把线程拖死）。
const MAX_PATH_DEPTH: usize = 64;

/// 取一个文件夹自己的名字与父目录 id。
///
/// `list(id)` 返回的是**子条目**、不含文件夹自身（实测），所以必须走
/// `GET /drive/v1/files/{id}`；取不到时放弃路径（界面不拿 id 冒充名字）。
fn folder_meta(client: &kichi_core::DriveClient, folder_id: &str) -> Option<(String, String)> {
    client.parent_of(folder_id).ok()
}

/// 共享扫描句柄（桥接层用它跨线程发取消）。
pub type SharedCancel = Arc<Cancel>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_end_names_are_stable_for_qml() {
        // QML 按字符串分支显示搜索状态，改名要同步改 SearchStatusBar.qml
        assert_eq!(ScanEnd::Finished.as_str(), "finished");
        assert_eq!(ScanEnd::Stopped.as_str(), "stopped");
        assert_eq!(ScanEnd::BrokenPaging.as_str(), "broken");
    }

    #[test]
    fn cancel_is_shared_and_observed() {
        let cancel: SharedCancel = Arc::new(Cancel::default());
        let clone = Arc::clone(&cancel);
        assert!(!cancel.is_cancelled());
        std::thread::spawn(move || clone.cancel())
            .join()
            .expect("线程正常结束");
        assert!(cancel.is_cancelled(), "扫描线程下一轮就能看到取消");
    }
}
