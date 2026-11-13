//! 搜索桥接的共用部分：扫描兼建库的回投、结果行构造，以及本地索引查询。
//!
//! 分工：
//! - `session::search`：本地索引搜索与云端逐页扫描；
//! - `search_local.rs`：输入阶段的当前目录筛选、全盘索引查询与单飞；
//! - 本文件：云端扫描入口、`pump()` 回投、epoch 作废、属性刷新。
//!
//! 几个刻意的取舍：
//! - **每条命中都保留在 `search_index` 里**（用户明确：不设固定条数上限），
//!   `search_results` 只放前 [`SEARCH_LIST_LIMIT`] 条给界面滚——几千行 QVariant
//!   只会让 ListView 卡住，计数与状态行用的是全部命中。
//! - 后台线程只回投「进度 + 本页命中」，不碰 Qt 对象；建库收集全部条目，
//!   搜索只回投命中，扫完再整体提交索引。
//! - 退出搜索 / 关键词变化让 `search_epoch` +1，在途的本地查询结果直接丢弃；
//!   建库线程不因退出搜索而停止。

use std::pin::Pin;
use std::sync::Arc;

use chrono::Utc;
use cxx_qt::CxxQtType;
use cxx_qt_lib::{QMap, QMapPair_QString_QVariant, QString, QVariant};
use kichi_core::{EntryKind, FileEntry, MatchTier, SearchFilter, SearchMatcher};

use super::actions_index::ScanTap;
use super::files::{entry_map, sort_entries, ROOT_NAME};
use super::qobject;
use super::QVariantList;
use crate::session::{ScanProgress, SearchHit};

type QVariantMap = QMap<QMapPair_QString_QVariant>;

/// 界面上最多显示多少条结果（内存里仍是全部命中）。
pub(super) const SEARCH_LIST_LIMIT: usize = 500;

impl qobject::Kichi {
    /// 搜索整个网盘：索引就绪时直接查本地库（0 请求）；未就绪时启动一次
    /// 「扫描兼建库」，边扫边回投结果，扫完原子提交索引。
    ///
    /// 索引关闭时这里会先打开开关（QML 已让用户确认过），不另起一次建库。
    pub fn search_all(mut self: Pin<&mut Self>) {
        let query = self.search_query.to_string().trim().to_owned();
        if query.is_empty() {
            self.as_mut()
                .set_message("先输入关键词，再按回车或点搜索按钮");
            return;
        }
        if crate::cxxqt_object::search_local::local_index_ready(&self.backend.session) {
            self.as_mut().start_local_index_search(query);
            return;
        }

        if !self.backend.session.local_index_enabled() {
            self.as_mut().enable_local_index_for_search();
        }
        if self.index_building {
            // 已经有建库在跑：把关键词挂上去，不另起一轮扫描。
            self.as_mut().attach_search_to_scan(query);
            return;
        }

        let session = self.backend.session.clone();
        let index = match session.ensure_tree_index_open() {
            Ok(index) => index,
            Err(err) => {
                self.as_mut()
                    .set_message(&format!("本地索引打开失败：{err}"));
                return;
            }
        };
        let tap = Arc::new(std::sync::Mutex::new(ScanTap::inactive()));
        {
            let mut tap = tap.lock().unwrap_or_else(|err| err.into_inner());
            tap.activate(query.clone(), self.search_filter.clone());
        }
        {
            let mut rust = self.as_mut().rust_mut();
            rust.index_scan_tap = Some(Arc::clone(&tap));
            rust.search_active = true;
            rust.search_epoch = rust.search_epoch.wrapping_add(1);
            if let Some(previous) = rust.search_cancel.take() {
                previous.cancel();
            }
            rust.search_settled = false;
            rust.search_scanned = 0;
            rust.search_results = QVariantList::default();
            rust.search_displayed = 0;
            rust.search_hits = 0;
            rust.search_index.clear();
            rust.search_truncated = false;
            rust.search_status = QString::from("正在扫描网盘并建立索引……");
            rust.search_source = QString::from("network");
            rust.search_local_running = false;
            rust.search_pending = None;
            rust.status_message = QString::default();
        }
        self.as_mut().notify();
        self.as_mut().start_tree_index_build(session, index);
    }

    /// 把当前搜索挂到正在跑的全盘扫描上。
    ///
    /// 只改共享条件，不碰建库线程；后台下一轮会拿已收集的整份数据重新匹配。
    pub(crate) fn attach_search_to_scan(mut self: Pin<&mut Self>, query: String) {
        let filter = self.search_filter.clone();
        let Some(tap) = self.index_scan_tap.as_ref().cloned() else {
            return;
        };
        {
            let mut tap = tap.lock().unwrap_or_else(|err| err.into_inner());
            tap.activate(query.clone(), filter);
        }
        let changed = self.search_query.to_string().trim() != query;
        {
            let mut rust = self.as_mut().rust_mut();
            // 作废可能还在回投的本地索引查询，别让它盖掉扫描结果。
            rust.search_epoch = rust.search_epoch.wrapping_add(1);
            if let Some(previous) = rust.search_cancel.take() {
                previous.cancel();
            }
            rust.search_active = true;
            rust.search_query = QString::from(query.as_str());
            rust.search_source = QString::from("network");
            rust.search_settled = false;
            rust.search_scanned = rust.index_build_scanned;
            if changed {
                rust.search_index.clear();
                rust.search_results = QVariantList::default();
                rust.search_hits = 0;
                rust.search_displayed = 0;
                rust.search_truncated = false;
            }
            rust.search_status = QString::from("正在扫描网盘并建立索引……");
            rust.search_local_running = false;
            rust.search_pending = None;
        }
        self.as_mut().notify();
    }

    /// 扫描线程回投的一页：按当前关键词 / 筛选铺到结果列表。
    pub(crate) fn apply_scan_progress(
        mut self: Pin<&mut Self>,
        epoch: u64,
        progress: ScanProgress,
    ) {
        if self.index_build_epoch != epoch || !self.search_active {
            return;
        }
        let current_revision = self
            .index_scan_tap
            .as_ref()
            .and_then(|tap| tap.lock().ok().map(|tap| tap.revision));
        if current_revision != Some(progress.revision) {
            // 关键词又变了：等下一轮的整份重匹配，别把旧关键词的结果塞进来。
            return;
        }
        {
            let mut rust = self.as_mut().rust_mut();
            rust.search_scanned = i32::try_from(progress.scanned).unwrap_or(i32::MAX);
            if progress.replace {
                rust.search_index.clear();
            }
            for hit in progress.hits {
                if rust.search_index.contains_key(hit.id()) {
                    continue;
                }
                rust.search_index.insert(hit.id().to_owned(), hit);
            }
            let query = rust.search_query.to_string();
            let filter = rust.search_filter.clone();
            let now = Utc::now().timestamp();
            let ordered = ordered_hits(
                &rust.search_index,
                &query,
                &filter,
                rust.sort_field,
                rust.sort_ascending,
                now,
            );
            let mut list = QVariantList::default();
            for hit in ordered.iter().take(SEARCH_LIST_LIMIT) {
                list.append(QVariant::from(&hit_map(hit)));
            }
            rust.search_results = list;
            rust.search_displayed = ordered.len().min(SEARCH_LIST_LIMIT);
            rust.search_hits = i32::try_from(ordered.len()).unwrap_or(i32::MAX);
            rust.search_truncated = ordered.len() > SEARCH_LIST_LIMIT;
            rust.search_source = QString::from("network");
            rust.search_settled = false;
            rust.search_status = QString::from(
                format!(
                    "正在扫描网盘并建立索引……已扫 {} 条，命中 {} 条",
                    rust.search_scanned, rust.search_hits
                )
                .as_str(),
            );
        }
        self.as_mut().notify();
    }

    /// 建库收尾时把搜索状态钉住；已出的结果保留，不提交半成品。
    pub(crate) fn settle_search(mut self: Pin<&mut Self>, status: String) {
        if !self.search_active {
            return;
        }
        {
            let mut rust = self.as_mut().rust_mut();
            rust.search_settled = true;
            rust.search_cancel = None;
            rust.search_status = QString::from(status.as_str());
        }
        self.as_mut().notify();
    }

    /// 退出搜索视图，回到当前目录。
    pub fn exit_search(mut self: Pin<&mut Self>) {
        self.as_mut().clear_status_message();
        {
            let mut rust = self.as_mut().rust_mut();
            rust.search_epoch = rust.search_epoch.wrapping_add(1);
            if let Some(cancel) = rust.search_cancel.take() {
                cancel.cancel();
            }
            // 建库任务继续跑，只是不再回投命中 / 解析路径；下次搜索再挂上去。
            if let Some(tap) = rust.index_scan_tap.as_ref() {
                if let Ok(mut tap) = tap.lock() {
                    tap.deactivate();
                }
            }
            rust.search_active = false;
            rust.search_query = QString::default();
            rust.search_results = QVariantList::default();
            rust.search_displayed = 0;
            rust.search_scanned = 0;
            rust.search_hits = 0;
            rust.search_settled = true;
            rust.search_status = QString::default();
            rust.search_index.clear();
            rust.search_truncated = false;
            rust.search_source = QString::default();
            rust.search_local_running = false;
            rust.search_pending = None;
            // 筛选是文件页的常驻视图条件：退出搜索后继续作用于目录列表。
        }
        // 当前目录的条目一直留在 Rust 侧：换个列表重画即可，不用重新拉网
        let entries = self.entries.clone();
        self.as_mut().show_entries(entries);
        // show_entries 只在条目有变化 / 有加载在途时 notify。退出搜索时目录没变，
        // 这里必须补一次：否则 QML 的 searchActive 停在 true，清空关键词回不到目录视图
        // （上面 clear_status_message 的 notify 发生在字段清空之前，救不了这次状态变化）。
        self.as_mut().notify();
    }

    /// 进了搜索结果里的文件夹：走正常目录导航，面包屑是真实层级。
    ///
    /// 搜索里点文件夹 = 一次「跳转」，用完就退出搜索视图——否则打开的目录
    /// 与被筛过的旧列表对不上，用户会以为进了错的文件夹。
    pub fn search_open_folder(mut self: Pin<&mut Self>, id: QString) {
        let id = id.to_string();
        if let Some(hit) = self.search_index.get(&id).cloned() {
            if hit.in_trash {
                self.as_mut()
                    .set_message("这条结果在回收站里，请先还原再打开");
                return;
            }
            self.as_mut().navigate_to_folder(&hit);
            return;
        }
        // 本地筛选的结果来自当前目录，从未进过 search_index；它仍是一条有效目录。
        if current_folder(&self.entries, &id).is_some() {
            self.as_mut().exit_search();
            self.as_mut().open_dir(QString::from(id.as_str()));
            return;
        }
        self.as_mut().set_message("该结果已过期，请重新搜索");
    }

    /// 进入一条结果所在的父文件夹。
    ///
    /// `id` 是结果里的「所在文件夹」id；为空表示在根目录。文件夹结果也传它自己的
    /// `folder_id`，因此这里不会误把结果文件夹本身当成目标。
    pub fn search_reveal_folder(mut self: Pin<&mut Self>, id: QString) {
        let id = id.to_string();
        if self.search_index.get(&id).is_some_and(|hit| hit.in_trash) {
            self.as_mut()
                .set_message("这个文件夹在回收站里，请先还原再进入");
            return;
        }
        // 名字从同一文件夹下的其它命中反推（路径末尾那一段），推不出就按根目录处理。
        let label = self
            .search_index
            .values()
            .find(|hit| hit.folder_id == id)
            .and_then(|hit| hit.folder_path.as_ref())
            .and_then(|path| path.rsplit('/').next())
            .map(str::to_owned)
            .unwrap_or_else(|| {
                if id.is_empty() {
                    ROOT_NAME.to_owned()
                } else {
                    "所在文件夹".to_owned()
                }
            });
        let hit = SearchHit {
            entry: FileEntry {
                id,
                name: label,
                kind: EntryKind::Folder,
                size: 0,
                created_at: None,
                modified_at: None,
                parent_id: String::new(),
                md5: None,
                audit_flagged: false,
                starred: false,
                media: false,
                video: false,
                thumbnail_url: String::new(),
            },
            folder_id: String::new(),
            folder_path: None,
            order: 0,
            in_trash: false,
        };
        self.as_mut().navigate_to_folder(&hit);
    }

    /// 清掉搜索状态并进入 `folder`（文件夹要么来自结果清单，要么是临时构造的）。
    fn navigate_to_folder(mut self: Pin<&mut Self>, folder: &SearchHit) {
        if folder.entry.kind != EntryKind::Folder {
            self.as_mut().set_message("这不是文件夹，无法进入");
            return;
        }
        if folder.in_trash {
            self.as_mut()
                .set_message("这个文件夹在回收站里，请先还原再进入");
            return;
        }
        if self.listing {
            return;
        }
        self.as_mut().clear_status_message();
        {
            let mut rust = self.as_mut().rust_mut();
            rust.search_epoch = rust.search_epoch.wrapping_add(1);
            if let Some(cancel) = rust.search_cancel.take() {
                cancel.cancel();
            }
            rust.search_active = false;
            rust.search_results = QVariantList::default();
            rust.search_displayed = 0;
            rust.search_scanned = 0;
            rust.search_hits = 0;
            rust.search_settled = true;
            rust.search_status = QString::default();
            rust.search_index.clear();
            rust.search_truncated = false;
            rust.search_source = QString::default();
            rust.search_local_running = false;
            rust.search_pending = None;
            // 进入结果里的文件夹 = 搜索结束；筛选继续作用于新目录的列表。
            // 读失败时把面包屑退回跳转前（别指着一个没读到的目录）
            rust.crumb_backup = Some(rust.crumbs.clone());
            // 根目录：id 为空，目录栈回到只有根那一级
            if folder.entry.id.is_empty() {
                rust.crumbs = vec![super::files::Crumb::root()];
            } else {
                rust.crumbs.push(super::files::Crumb {
                    id: folder.entry.id.clone(),
                    name: folder.entry.name.clone(),
                });
            }
        }
        self.as_mut().sync_breadcrumb_visible();
        self.as_mut().enter_current_dir();
    }

    /// 本地筛选的结果直接当列表显示；搜索状态在页面状态行 / 底栏上下文表达。
    pub(super) fn apply_local_filter(
        mut self: Pin<&mut Self>,
        entries: Vec<FileEntry>,
        status: &str,
    ) {
        let folder = self
            .crumbs
            .last()
            .map(|crumb| crumb.name.clone())
            .unwrap_or_else(|| ROOT_NAME.to_owned());
        // 兜底结果就是当前目录列表的裁剪：按目录的排序规则排（文件夹仍置顶），
        // 用户改了排序也立刻影响这批命中。
        let mut entries = entries;
        sort_entries(&mut entries, self.sort_field, self.sort_ascending);
        // 本地筛出来的就是当前目录里的条目：所在文件夹直接写当前目录名
        let mut filtered = QVariantList::default();
        for entry in &entries {
            filtered.append(QVariant::from(&current_dir_map(entry, &folder)));
        }
        let count = entries.len();
        {
            let mut rust = self.as_mut().rust_mut();
            rust.search_settled = true;
            rust.search_scanned = 0;
            rust.search_hits = i32::try_from(count).unwrap_or(i32::MAX);
            rust.search_displayed = count;
            // 目录内兜底筛选不会截断；别把上一轮索引结果的「只显示前 500 条」留着
            rust.search_truncated = false;
            rust.search_results = filtered;
            rust.search_status = QString::from(status);
            rust.status_message = QString::default();
        }
        self.as_mut().notify();
    }

    /// 网络扫描结果已收集完、用户改了筛选时：不重新请求网盘（ADR 0004），
    /// 只按当前条件重筛 `search_index` 里已有命中。
    pub(super) fn refilter_network_hits(mut self: Pin<&mut Self>) {
        self.as_mut().rebuild_search_view_from_state();
        {
            let mut rust = self.as_mut().rust_mut();
            if rust.search_settled {
                rust.search_status = QString::from(
                    format!(
                        "已按筛选过滤：命中 {} 条（共扫 {} 条）",
                        rust.search_hits, rust.search_scanned
                    )
                    .as_str(),
                );
            }
        }
        self.as_mut().notify();
    }

    /// 文件写入的乐观修改已经直接改过 `entries` / `search_index`，这里只重建
    /// 搜索结果行和计数。全盘结果与本地筛选共用同一个出口。
    pub(crate) fn rebuild_search_view_from_state(mut self: Pin<&mut Self>) {
        if !self.search_active {
            return;
        }
        let folder = self
            .crumbs
            .last()
            .map(|crumb| crumb.name.clone())
            .unwrap_or_else(|| ROOT_NAME.to_owned());
        {
            let mut rust = self.as_mut().rust_mut();
            rebuild_visible_search(&mut rust, &folder);
        }
        self.as_mut().notify();
    }

    /// 退出登录：清空搜索并作废在途扫描。
    pub(crate) fn reset_search(mut self: Pin<&mut Self>) {
        self.as_mut().exit_search();
        {
            let mut rust = self.as_mut().rust_mut();
            rust.search_status = QString::default();
            // 筛选是文件页常驻条件，但如果退出登录不清，换账号后会把上一个账号
            // 设的条件带过去。
            super::search_filter::reset_filter(&mut rust);
        }
        self.as_mut().notify();
    }

    /// 改了排序：搜索视图按新列重排结果（档位仍优先，同档内换列）。
    /// 目录列表由 `files.rs` 处理；这里只重排当前显示，不重新搜索。
    pub(crate) fn re_sort_search_results(mut self: Pin<&mut Self>) {
        if !self.search_active {
            return;
        }
        self.as_mut().rebuild_search_view_from_state();
    }
}

/// 搜索结果的显示顺序：先按当前筛选过滤，再按匹配质量
/// （完全同名 > 前缀 > 词首 > 包含）分档；**同档内按用户选的列与方向**
/// （默认名称升序），再相同才按名字 / 扫到顺序 / id 兜底。文件夹不因类型插队。
///
/// 规则本身在 `kichi_core::SearchMatcher` / `SearchFilter` 与
/// `SortField::compare_entries`（纯函数、单测）；这里只接线。
/// **过滤与排序都必须发生在调用方截断之前**，否则命中会被先截掉。
fn ordered_hits(
    index: &std::collections::HashMap<String, SearchHit>,
    query: &str,
    filter: &SearchFilter,
    sort_field: crate::settings::SortField,
    sort_ascending: bool,
    now: i64,
) -> Vec<SearchHit> {
    let mut hits: Vec<SearchHit> = index
        .values()
        .filter(|hit| filter.matches(&hit.entry, now))
        .cloned()
        .collect();
    let matcher = SearchMatcher::new(query);
    let rank = |name: &str| matcher.as_ref().and_then(|matcher| matcher.tier(name));
    hits.sort_by(|left, right| {
        // 只比档位：名字交给同档内的排序列，否则选「大小」时永不生效
        compare_rank(rank(&left.entry.name), rank(&right.entry.name))
            .then_with(|| sort_field.compare_entries(&left.entry, &right.entry, sort_ascending))
            .then_with(|| {
                left.entry
                    .name
                    .to_lowercase()
                    .cmp(&right.entry.name.to_lowercase())
            })
            .then_with(|| left.order.cmp(&right.order))
            .then_with(|| left.id().cmp(right.id()))
    });
    hits
}

/// 匹配档位先后；不匹配（`None`）排最后，与 [`SearchMatcher::compare`] 一致。
fn compare_rank(left: Option<MatchTier>, right: Option<MatchTier>) -> std::cmp::Ordering {
    match (left, right) {
        (Some(left), Some(right)) => left.cmp(&right),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    }
}

fn current_folder<'a>(entries: &'a [FileEntry], id: &str) -> Option<&'a FileEntry> {
    entries
        .iter()
        .find(|entry| entry.kind == EntryKind::Folder && entry.id == id)
}

/// 当前目录里的一条 → QML 小 map（结构与搜索结果一致，所在文件夹写当前目录名）。
fn current_dir_map(entry: &FileEntry, folder: &str) -> QVariantMap {
    let text = |value: &str| QVariant::from(&QString::from(value));
    let mut map = entry_map(entry);
    map.insert_clone(&QString::from("folderText"), &text(folder));
    map.insert_clone(&QString::from("folderId"), &text(""));
    map
}

/// 搜索视图按当前领域状态重建 QML 行。
///
/// 调用方已经改完 `entries` / `search_index`；这里只同步展示计数，
/// 不改扫描进度和结束状态。只有「只筛当前目录」的兜底结果才回退到
/// `entries`；索引 / 网络结果即使 0 命中也不能拿当前目录顶替。
fn rebuild_visible_search(rust: &mut super::KichiRust, folder: &str) {
    let query = rust.search_query.to_string();
    let filter = rust.search_filter.clone();
    let now = Utc::now().timestamp();
    let mut rows = QVariantList::default();
    if rust.search_source.to_string() == "fallback" {
        let mut filtered =
            crate::session::search::filter_entries(&rust.entries, &query, &filter, now);
        // 兜底结果就是当前目录列表的裁剪：沿用目录的排序（文件夹置顶）
        sort_entries(&mut filtered, rust.sort_field, rust.sort_ascending);
        for entry in &filtered {
            rows.append(QVariant::from(&current_dir_map(entry, folder)));
        }
        rust.search_hits = i32::try_from(filtered.len()).unwrap_or(i32::MAX);
        rust.search_displayed = filtered.len();
        rust.search_truncated = false;
    } else {
        let ordered = ordered_hits(
            &rust.search_index,
            &query,
            &filter,
            rust.sort_field,
            rust.sort_ascending,
            now,
        );
        for hit in ordered.iter().take(SEARCH_LIST_LIMIT) {
            rows.append(QVariant::from(&hit_map(hit)));
        }
        rust.search_hits = i32::try_from(ordered.len()).unwrap_or(i32::MAX);
        rust.search_displayed = ordered.len().min(SEARCH_LIST_LIMIT);
        rust.search_truncated = ordered.len() > SEARCH_LIST_LIMIT;
    }
    rust.search_results = rows;
}

/// 一条命中 → QML 小 map：条目字段与目录列表完全一致，外加「所在文件夹」。
pub(super) fn hit_map(hit: &SearchHit) -> QVariantMap {
    let text = |value: &str| QVariant::from(&QString::from(value));
    let mut map = entry_map(&hit.entry);
    let location = if hit.in_trash {
        match &hit.folder_path {
            Some(path) if !path.is_empty() => format!("回收站 · 原位置：{path}"),
            // 拼不出原位置就只说在回收站，绝不拿 id 冒充路径
            _ => "回收站".to_owned(),
        }
    } else {
        match &hit.folder_path {
            Some(path) if !path.is_empty() => path.clone(),
            // 路径拼不出来时至少说明在哪：根目录还是某个文件夹
            _ if hit.folder_id.is_empty() => ROOT_NAME.to_owned(),
            _ => "（所在文件夹）".to_owned(),
        }
    };
    map.insert_clone(&QString::from("folderText"), &text(&location));
    map.insert_clone(&QString::from("folderId"), &text(&hit.folder_id));
    map.insert_clone(&QString::from("inTrash"), &QVariant::from(&hit.in_trash));
    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::SortField;
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

    fn hit(name: &str, folder: &str, path: Option<&str>) -> SearchHit {
        SearchHit {
            entry: entry(name, EntryKind::File),
            folder_id: folder.to_owned(),
            folder_path: path.map(str::to_owned),
            order: 0,
            in_trash: false,
        }
    }

    fn read(map: &QVariantMap, key: &str) -> String {
        map.get(&QString::from(key))
            .and_then(|value| value.value::<QString>())
            .map(|text| text.to_string())
            .unwrap_or_default()
    }

    #[test]
    fn hit_map_carries_the_real_folder() {
        let map = hit_map(&hit(
            "汉化日记 第 3 季 01.mkv",
            "season-3",
            Some("剧集/汉化日记 第 3 季"),
        ));
        assert_eq!(read(&map, "name"), "汉化日记 第 3 季 01.mkv");
        assert_eq!(read(&map, "folderText"), "剧集/汉化日记 第 3 季");
        assert_eq!(read(&map, "folderId"), "season-3");
        assert_eq!(read(&map, "kind"), "file", "条目字段与目录列表一致");
    }

    #[test]
    fn folder_hits_are_first_class_results() {
        // 用户明确：不只是文件，文件夹也要搜得到、也要能进
        let folder = SearchHit {
            entry: entry("汉化日记 第 3 季", EntryKind::Folder),
            folder_id: "season-2".to_owned(),
            folder_path: Some("Bangumi/汉化日记/第 2 季".to_owned()),
            order: 5,
            in_trash: false,
        };
        let map = hit_map(&folder);
        assert_eq!(read(&map, "kind"), "folder", "结果要保持文件夹身份");
        assert_eq!(read(&map, "name"), "汉化日记 第 3 季");
        assert_eq!(
            read(&map, "folderId"),
            "season-2",
            "点「进入」要知道自己是谁"
        );
        assert_eq!(read(&map, "folderText"), "Bangumi/汉化日记/第 2 季");
        assert_eq!(read(&map, "sizeText"), "—", "文件夹没有大小");
    }

    #[test]
    fn results_rank_by_match_quality_then_name() {
        // 用户 2026-11-04 定的规则：匹配质量优先，同档按名字；文件夹不插队。
        let mut index = std::collections::HashMap::new();
        let names = [
            (9, "包含第"),
            (8, "其它 第"),
            (7, "第 2 集"),
            (6, "第"),
            (5, "第 1 集"),
        ];
        for (order, name) in names {
            let mut h = hit(name, "", None);
            h.order = order;
            index.insert(h.id().to_owned(), h);
        }
        // 文件夹靠行首类型标签区分，不靠位置：词首匹配就该排在包含之前
        let mut folder = SearchHit {
            entry: entry("第 3 季", EntryKind::Folder),
            folder_id: String::new(),
            folder_path: None,
            order: 9_999,
            in_trash: false,
        };
        folder.entry.id = "dir-1".to_owned();
        index.insert("dir-1".to_owned(), folder);

        let ordered = ordered_hits(
            &index,
            "第",
            &SearchFilter::default(),
            SortField::Name,
            true,
            0,
        );
        assert_eq!(
            ordered.iter().map(|h| h.name()).collect::<Vec<_>>(),
            vec!["第", "第 1 集", "第 2 集", "第 3 季", "其它 第", "包含第"],
            "完全同名 > 前缀 > 词首 > 包含；同档按名字"
        );
    }

    #[test]
    fn same_rank_uses_name_not_hash_iteration() {
        let mut index = std::collections::HashMap::new();
        for (order, name) in [(3, "b 包含"), (1, "a 包含"), (2, "A 包含")] {
            let mut h = hit(name, "", None);
            h.entry.id = format!("id-{order}");
            h.order = order;
            index.insert(h.id().to_owned(), h);
        }
        let ordered = ordered_hits(
            &index,
            "包含",
            &SearchFilter::default(),
            SortField::Name,
            true,
            0,
        );
        assert_eq!(
            ordered.iter().map(|h| h.name()).collect::<Vec<_>>(),
            vec!["a 包含", "A 包含", "b 包含"],
            "同档按名字（不区分大小写）稳定排序，不能落到 HashMap 顺序"
        );
    }

    #[test]
    fn same_rank_follows_the_chosen_sort_column() {
        // 三条都是「包含」档：换列只改同档内顺序，档位优先级不动
        let mut index = std::collections::HashMap::new();
        for (order, name, size) in [(1, "a 包含", 30), (2, "b 包含", 10), (3, "c 包含", 20)] {
            let mut item = hit(name, "", None);
            item.entry.size = size;
            item.entry.modified_at = Some(
                chrono::DateTime::from_timestamp(1_700_000_000 + size as i64, 0).expect("合法时间"),
            );
            item.order = order;
            index.insert(item.id().to_owned(), item);
        }

        let names = |field, ascending| {
            ordered_hits(
                &index,
                "包含",
                &SearchFilter::default(),
                field,
                ascending,
                0,
            )
            .iter()
            .map(|hit| hit.name().to_string())
            .collect::<Vec<_>>()
        };
        assert_eq!(
            names(SortField::Size, false),
            vec!["a 包含", "c 包含", "b 包含"],
            "按大小降序：大的在前"
        );
        assert_eq!(
            names(SortField::Modified, true),
            vec!["b 包含", "c 包含", "a 包含"],
            "按时间升序：旧的在前"
        );
        assert_eq!(
            names(SortField::Name, false),
            vec!["c 包含", "b 包含", "a 包含"],
            "按名称降序"
        );
    }

    #[test]
    fn sort_never_breaks_the_match_tier_order() {
        // 即使「包含」档的文件更大、更新，也不能排到「完全同名」前面
        let mut index = std::collections::HashMap::new();
        let mut exact = hit("电影", "", None);
        exact.entry.size = 1;
        index.insert(exact.id().to_owned(), exact);
        let mut contains = hit("电影合集", "", None);
        contains.entry.size = 9_999;
        index.insert(contains.id().to_owned(), contains);

        let ordered = ordered_hits(
            &index,
            "电影",
            &SearchFilter::default(),
            SortField::Size,
            false,
            0,
        );
        assert_eq!(
            ordered.iter().map(|h| h.name()).collect::<Vec<_>>(),
            vec!["电影", "电影合集"],
            "匹配质量优先于排序列"
        );
    }

    #[test]
    fn root_entries_say_root_and_unresolved_paths_stay_honest() {
        assert_eq!(
            read(&hit_map(&hit("a.mkv", "", None)), "folderText"),
            ROOT_NAME
        );
        let unresolved = hit_map(&hit("b.mkv", "id-unknown", None));
        assert_eq!(read(&unresolved, "folderText"), "（所在文件夹）");
        assert!(
            !read(&unresolved, "folderText").contains("id-unknown"),
            "拼不出路径时不要拿 id 冒充名字"
        );
    }

    #[test]
    fn trash_hits_are_marked_and_keep_their_original_location() {
        let mut trashed = hit(
            "汉化日记 第 3 季 01.mkv",
            "season-3",
            Some("剧集/汉化日记 第 3 季"),
        );
        trashed.in_trash = true;
        let map = hit_map(&trashed);
        assert_eq!(
            map.get(&QString::from("inTrash"))
                .and_then(|value| value.value::<bool>()),
            Some(true),
            "QML 要能一眼分辨回收站行"
        );
        assert_eq!(
            read(&map, "folderText"),
            "回收站 · 原位置：剧集/汉化日记 第 3 季"
        );

        let mut unknown = hit("b.mkv", "id-unknown", None);
        unknown.in_trash = true;
        let map = hit_map(&unknown);
        assert_eq!(read(&map, "folderText"), "回收站");
        assert!(
            !read(&map, "folderText").contains("id-unknown"),
            "拼不出原位置时不要拿 id 冒充路径"
        );
    }

    #[test]
    fn live_hits_are_explicitly_not_in_trash() {
        let map = hit_map(&hit("a.mkv", "", None));
        assert_eq!(
            map.get(&QString::from("inTrash"))
                .and_then(|value| value.value::<bool>()),
            Some(false),
            "在库条目也要有明确的 inTrash=false，QML 才能统一判断"
        );
    }

    #[test]
    fn local_filter_folders_still_count_as_current_directories() {
        let entries = vec![
            entry("folder", EntryKind::Folder),
            entry("file", EntryKind::File),
        ];
        assert!(current_folder(&entries, "folder").is_some());
        assert!(current_folder(&entries, "file").is_none());
        assert!(current_folder(&entries, "missing").is_none());
    }

    #[test]
    fn filter_runs_before_the_display_limit() {
        // 600 条不属于目标的条目排在前，3 条视频排在最后。
        // 若先截断 500 再筛选，视频会被整批丢掉。
        let mut index = std::collections::HashMap::new();
        for i in 0..600 {
            let name = format!("a{i:04}.bin");
            let mut item = hit(&name, "", None);
            item.entry.id = format!("bin-{i}");
            item.entry.name = name;
            index.insert(item.id().to_owned(), item);
        }
        for i in 0..3 {
            let name = format!("z{i}.mkv");
            let mut item = hit(&name, "", None);
            item.entry.id = format!("video-{i}");
            item.entry.name = name;
            index.insert(item.id().to_owned(), item);
        }

        let filter = SearchFilter {
            types: vec![kichi_core::TypeFilter::Video],
            ..Default::default()
        };
        let ordered = ordered_hits(&index, "a", &filter, SortField::Name, true, 0);
        assert_eq!(ordered.len(), 3, "只留下视频");
        let displayed = ordered.iter().take(SEARCH_LIST_LIMIT).count();
        assert_eq!(
            displayed, 3,
            "先过滤再截断：视频不能被 600 条不匹配项挤出前 500"
        );
    }
}
