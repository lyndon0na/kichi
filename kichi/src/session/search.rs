//! 云端搜索的会话面：逐页扫描整个网盘 + 名字匹配 + 真实路径还原。
//!
//! **两段式**（用户决策）：
//! - 当前目录里已加载的条目由界面直接筛（0 请求），不进本模块；
//! - 只有用户按「全盘搜索」才来这里遍历网盘。
//!
//! 本模块只做「一次扫描」：每页回调一次进度（调用方决定怎么显示），
//! `Cancel` 随时可中断，结束时给出三态之一。线程、回投、界面状态在
//! `kichi/src/cxxqt_object/search.rs`。
//!
//! 全部函数阻塞，调用方负责丢到后台线程。

use std::collections::HashMap;
use std::sync::Arc;

use kichi_core::search::{matches, FileHit};

use super::download::Cancel;
use super::{Failure, Session};

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

/// 一条搜索命中：条目 + 所在文件夹（`None` = 根目录）+ 扫到的顺序。
#[derive(Debug, Clone)]
pub struct SearchHit {
    pub id: String,
    pub name: String,
    pub folder_id: String,
    pub folder_path: Option<String>,
    pub is_folder: bool,
    /// 扫到的顺序：结果「先到先显示」，界面用 id 认条目，用 order 排序
    pub order: usize,
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
            for hit in entries {
                if !matches(&hit.entry.name, query) {
                    continue;
                }
                let (folder_id, folder_path) = folder_of(&client, &mut path, &hit);
                hits.push(SearchHit {
                    id: hit.entry.id,
                    name: hit.entry.name,
                    folder_id,
                    folder_path,
                    is_folder: hit.entry.kind == kichi_core::EntryKind::Folder,
                    order: scanned - 1,
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

/// 取一个文件夹自己的名字与父目录 id（`list(id)` 找自己）。
///
/// ⚠ 实测（2026-10-26 侦察）：`list(id)` 返回的是**子条目**，不含文件夹自身，
/// 所以这里取不到时只能放弃路径（界面显示文件夹 id 兜底），不臆造名字。
fn folder_meta(client: &kichi_core::DriveClient, folder_id: &str) -> Option<(String, String)> {
    let (entries, _) = client.list(Some(folder_id), None).ok()?;
    entries
        .into_iter()
        .find(|entry| entry.id == folder_id)
        .map(|entry| (entry.name, String::new()))
}

/// 共享扫描句柄（桥接层用它跨线程发取消）。
pub type SharedCancel = Arc<Cancel>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_end_names_are_stable_for_qml() {
        // QML 按字符串分支，改名要同步改 SearchResults.qml
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
