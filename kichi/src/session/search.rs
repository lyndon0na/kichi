//! 搜索的会话面：本地索引查询 + 云端逐页扫描兼建库。
//!
//! 输入阶段只筛当前目录已加载条目（和没有索引时一致）；回车 / 搜索按钮才
//! 全盘搜索：索引就绪走 [`Session::search_index_local`]（0 请求），
//! 未就绪则走 [`Session::scan_and_build`]：同一次遍历边回投命中边建立索引。
//!
//! 云端扫描每页回调一次进度（调用方决定怎么显示）；只有登出 / 关闭索引
//! 会中断。线程、回投、界面状态在
//! `kichi/src/cxxqt_object/search.rs`。
//!
//! 全部函数阻塞，调用方负责丢到后台线程。

use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use kichi_core::search::{matches, FileHit};
use kichi_core::{FileEntry, MatchTier, SearchFilter, SearchMatcher};

use super::download::Cancel;
use super::tree_index;
use super::tree_index::sync::MAX_FULL_BUILD_PAGES;
use super::{Failure, Session};
use crate::settings::SortField;

/// 本地搜索最多为多少条命中解析路径（界面也只显示这么多行）。
pub const SEARCH_PATH_LIMIT: usize = 500;

/// 扫描结束时为什么停下。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanEnd {
    /// 官方不再给 `next_page_token`：全盘扫完
    Finished,
    /// 登出 / 关闭索引 / 应用退出：旧库保留，不提交半成品
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

/// 后台扫描读取到的当前搜索请求。
///
/// 扫描线程每页读一次；`revision` 变了就说明关键词 / 筛选 / 启停变了，
/// 需要拿已收集的整份数据重新匹配，而不是只匹配新的一页。
#[derive(Debug, Clone)]
pub struct ScanTapSnapshot {
    pub active: bool,
    pub query: String,
    pub filter: SearchFilter,
    pub revision: u64,
}

/// 扫描的一页进度：本页新增命中，或关键词 / 筛选变化后的整份命中。
#[derive(Debug, Clone)]
pub struct ScanProgress {
    pub scanned: usize,
    pub hits: Vec<SearchHit>,
    /// true = 用这批命中替换界面里的旧结果（关键词 / 筛选变了）
    pub replace: bool,
    /// 生成这批结果时对应的 [`ScanTapSnapshot::revision`]
    pub revision: u64,
}

/// 一轮「扫描兼建库」的结局。
#[derive(Debug, Clone)]
pub struct ScanBuildReport {
    pub end: ScanEnd,
    /// 只有自然扫完才会尝试提交；`Err` 是本地库写入失败
    pub built: Option<Result<usize, String>>,
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
    /// 全盘扫描并建立索引（逐页；阻塞；只由登出 / 关闭索引中断）。
    ///
    /// 一次遍历同时做两件事：把所有条目收集起来，扫完后原子替换本地索引；
    /// 同时按 [`ScanTapSnapshot`] 里的当前关键词 / 筛选匹配命中并逐页回投。
    /// `tap` 为 `None` 或 `active == false` 时只建库、不做搜索匹配，也不解析
    /// 文件夹路径（建库不需要额外请求）。
    ///
    /// `on_batch` 跑在扫描线程上，调用方只应入队，不要做慢操作。
    pub fn scan_and_build(
        &self,
        index: &tree_index::TreeIndex,
        cancel: &Cancel,
        tap: &mut dyn FnMut() -> Option<ScanTapSnapshot>,
        on_progress: &mut dyn FnMut(usize),
        on_batch: &mut dyn FnMut(ScanProgress),
    ) -> Result<ScanBuildReport, Failure> {
        let client = self.drive()?;
        let mut path = PathCache::default();
        let mut collected: Vec<FileHit> = Vec::new();
        let mut scanned = 0usize;
        let mut page_token: Option<String> = None;
        let mut pages = 0usize;
        let mut last_revision: Option<u64> = None;
        // 一轮扫描内时间窗口固定，避免翻页过程中边界抖动。
        let now = Utc::now().timestamp();
        index.begin_build_tracking();

        loop {
            if cancel.is_cancelled() || index.is_closed() {
                let _ = index.finish_build_tracking(false);
                return Ok(ScanBuildReport {
                    end: ScanEnd::Stopped,
                    built: None,
                });
            }
            pages += 1;
            if pages > MAX_FULL_BUILD_PAGES {
                let _ = index.finish_build_tracking(false);
                return Err(Failure::Other(
                    "全盘列举页数超过上限，已停止建库（旧索引保留）".to_owned(),
                ));
            }
            let (entries, next) = match client.list_all_page(page_token.as_deref()) {
                Ok(page) => page,
                Err(err) => {
                    let _ = index.finish_build_tracking(false);
                    return Err(Failure::from(err));
                }
            };
            let page_start = scanned;
            scanned += entries.len();
            collected.extend(entries.iter().cloned());
            on_progress(collected.len());

            if let Some(snapshot) = tap() {
                if snapshot.active && !snapshot.query.trim().is_empty() {
                    // 关键词 / 筛选变了：拿整份已收集数据重匹配；否则只处理新页。
                    let replace = last_revision != Some(snapshot.revision);
                    let source: &[FileHit] = if replace { &collected } else { &entries };
                    let order_base = if replace { 0 } else { page_start };
                    let hits = match_hits(
                        source,
                        &snapshot.query,
                        &snapshot.filter,
                        order_base,
                        now,
                        |hit| folder_of(&client, &mut path, hit),
                    );
                    on_batch(ScanProgress {
                        scanned,
                        hits,
                        replace,
                        revision: snapshot.revision,
                    });
                    last_revision = Some(snapshot.revision);
                } else {
                    last_revision = None;
                }
            } else {
                last_revision = None;
            }

            let Some(next) = next else {
                let entries: Vec<FileEntry> =
                    collected.iter().map(|hit| hit.entry.clone()).collect();
                let built = match index.replace_all(&entries) {
                    Ok(()) => {
                        if let Err(err) = index.finish_build_tracking(true) {
                            self.record_tree_index_error(format!(
                                "本地索引已建立，但重放建库期间的写操作失败：{err}"
                            ));
                        }
                        Ok(entries.len())
                    }
                    Err(err) => {
                        let _ = index.finish_build_tracking(false);
                        Err(format!("本地索引写入失败：{err}"))
                    }
                };
                return Ok(ScanBuildReport {
                    end: ScanEnd::Finished,
                    built: Some(built),
                });
            };
            // 同一个 token 再取一次还是同一页：官方分页卡住了，别无限循环
            if page_token.as_deref() == Some(next.as_str()) {
                let _ = index.finish_build_tracking(false);
                return Ok(ScanBuildReport {
                    end: ScanEnd::BrokenPaging,
                    built: None,
                });
            }
            page_token = Some(next);
        }
    }
    /// 本地索引全盘搜索（阻塞；调用方放后台线程）。
    ///
    /// 匹配包含回收站行；排序规则：先按 [`SearchMatcher`] 的匹配档位
    /// （完全同名 > 前缀 > 词首 > 包含），**同档内按用户选的列与方向**
    /// （[`SortField::compare_entries`]，默认名称升序），再以名字 / id 兜底。
    /// 筛选与排序都在截断之前完成（`kind` / 时间 / 大小已下推 SQL，扩展名分类
    /// 在 Rust 侧），只给界面显示的前 [`SEARCH_PATH_LIMIT`] 条解析路径，
    /// 且路径全部来自索引，不回网络。
    pub fn search_index_local(
        &self,
        query: &str,
        filter: &SearchFilter,
        sort_field: SortField,
        sort_ascending: bool,
    ) -> Result<IndexSearchResult, Failure> {
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

        let now = Utc::now().timestamp();
        let mut rows: Vec<(Option<(MatchTier, String)>, tree_index::IndexSearchHit)> = index
            .search_with_trash(query, filter, now)
            .map_err(index_failure)?
            .into_iter()
            .map(|row| (matcher.sort_key(&row.entry.name), row))
            .filter(|(key, _)| key.is_some())
            .collect();
        rows.sort_by(|(left, a), (right, b)| {
            // 只拿档位比：名字交给下面的排序列，否则选「大小」时同档内永远是名称序
            let left_tier = left.as_ref().map(|(tier, _)| *tier);
            let right_tier = right.as_ref().map(|(tier, _)| *tier);
            left_tier
                .cmp(&right_tier)
                .then_with(|| sort_field.compare_entries(&a.entry, &b.entry, sort_ascending))
                .then_with(|| {
                    a.entry
                        .name
                        .to_lowercase()
                        .cmp(&b.entry.name.to_lowercase())
                })
                .then_with(|| a.entry.id.cmp(&b.entry.id))
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

/// 索引未就绪时的当前目录兜底：名字命中 + 筛选条件一起判断（0 请求）。
///
/// 保持当前目录原有顺序；`now` 是时间窗口基准。抽成纯函数便于单测，
/// 桥接层只负责把结果铺到界面上。
pub fn filter_entries(
    entries: &[FileEntry],
    query: &str,
    filter: &SearchFilter,
    now: i64,
) -> Vec<FileEntry> {
    entries
        .iter()
        .filter(|entry| matches(&entry.name, query) && filter.matches(entry, now))
        .cloned()
        .collect()
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

/// 按当前关键词 / 筛选匹配一批条目，并补齐「所在文件夹」。
///
/// `source` 要么是刚扫到的一页，要么是整份已收集数据；`order_base` 让
/// 命中顺序在两种模式下都对应全盘扫描顺序。
fn match_hits(
    source: &[FileHit],
    query: &str,
    filter: &SearchFilter,
    order_base: usize,
    now: i64,
    mut resolve_folder: impl FnMut(&FileHit) -> (String, Option<String>),
) -> Vec<SearchHit> {
    let matcher = SearchMatcher::new(query);
    let Some(matcher) = matcher else {
        return Vec::new();
    };
    let mut hits = Vec::new();
    for (offset, hit) in source.iter().enumerate() {
        if !matcher.matches(&hit.entry.name) || !filter.matches(&hit.entry, now) {
            continue;
        }
        let (folder_id, folder_path) = resolve_folder(hit);
        hits.push(SearchHit {
            entry: hit.entry.clone(),
            folder_id,
            folder_path,
            order: order_base + offset,
            in_trash: false,
        });
    }
    hits
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
    use kichi_core::EntryKind;

    fn entry(name: &str, kind: EntryKind) -> FileEntry {
        FileEntry {
            id: name.to_owned(),
            name: name.to_owned(),
            kind,
            size: 1024,
            created_at: None,
            modified_at: None,
            parent_id: String::new(),
            md5: None,
            audit_flagged: false,
            starred: false,
            media: kind == EntryKind::File,
            video: false,
            thumbnail_url: String::new(),
        }
    }

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

    #[test]
    fn match_hits_uses_the_current_query_and_global_order() {
        let source = vec![
            FileHit {
                entry: entry("Alpha.mkv", EntryKind::File),
                parent_id: "f1".to_owned(),
            },
            FileHit {
                entry: entry("Beta.mkv", EntryKind::File),
                parent_id: "f2".to_owned(),
            },
            FileHit {
                entry: entry("alpha folder", EntryKind::Folder),
                parent_id: String::new(),
            },
        ];

        let hits = match_hits(&source, "alpha", &SearchFilter::default(), 10, 0, |hit| {
            let folder = hit.parent_id.clone();
            (folder, Some("目录".to_owned()))
        });

        assert_eq!(hits.len(), 2, "名字包含匹配，文件夹也参与");
        assert_eq!(hits[0].order, 10, "命中顺序要接在全盘扫描位置上");
        assert_eq!(hits[1].order, 12);
        assert_eq!(hits[1].entry.kind, EntryKind::Folder);
    }
}
