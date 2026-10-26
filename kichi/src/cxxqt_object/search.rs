//! 云端搜索：先在当前目录本地筛（0 请求），「全盘搜索」才遍历网盘。
//!
//! 分工：
//! - `session::search`：逐页扫描、名字匹配、可中断、三态结束；
//! - 本文件：QML 入口、后台线程、`pump()` 回投、epoch 作废、属性刷新。
//!
//! 几个刻意的取舍：
//! - **每条命中都保留在 `search_index` 里**（用户明确：不设固定条数上限），
//!   `search_results` 只放前 [`SEARCH_LIST_LIMIT`] 条给界面滚——几千行 QVariant
//!   只会让 ListView 卡住，计数与状态行用的是全部命中。
//! - 后台线程只回投「进度 + 本页命中」，不碰 Qt 对象；页码退避用官方给的
//!   `next_page_token`，因此不需要缓存页数据。
//! - 停止 / 退出搜索 / 登出都让 `search_epoch` +1：在途页的结果直接丢弃。

use std::pin::Pin;
use std::sync::Arc;

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QMap, QMapPair_QString_QVariant, QString, QVariant};
use kichi_core::FileEntry;

use super::actions::enqueue_ui;
use super::files::{entry_map, sort_entries, ROOT_NAME};
use super::qobject;
use super::QVariantList;
use crate::session::download::Cancel;
use crate::session::{Failure, ScanBatch, ScanEnd, SearchHit};

type QVariantMap = QMap<QMapPair_QString_QVariant>;

/// 界面上最多显示多少条结果（内存里仍是全部命中）。
const SEARCH_LIST_LIMIT: usize = 500;

impl qobject::Kichi {
    /// 第一段：只筛当前目录**已加载**的条目（0 请求）。
    pub fn search_local(mut self: Pin<&mut Self>, query: QString) {
        let query = query.to_string();
        let trimmed = query.trim().to_owned();
        if trimmed.is_empty() {
            // 清空关键词 = 回到当前目录（Esc 的第一段就是这么来的）
            self.as_mut().exit_search();
            return;
        }
        {
            let mut rust = self.as_mut().rust_mut();
            rust.search_active = true;
            rust.search_query = QString::from(query.as_str());
            // 本地筛选会把「结果」那一栏顶掉：在途的全盘扫描整轮作废
            rust.search_epoch = rust.search_epoch.wrapping_add(1);
            if let Some(cancel) = rust.search_cancel.take() {
                cancel.cancel();
            }
            rust.search_settled = true;
            rust.search_scanned = 0;
            rust.search_results = QVariantList::default();
            rust.search_displayed = 0;
            rust.search_hits = 0;
            rust.search_index.clear();
        }
        let folder = self
            .crumbs
            .last()
            .map(|crumb| crumb.name.clone())
            .unwrap_or_else(|| ROOT_NAME.to_owned());
        let matched: Vec<FileEntry> = self
            .entries
            .iter()
            .filter(|entry| kichi_core::search::matches(&entry.name, &trimmed))
            .cloned()
            .collect();
        let count = matched.len();
        let current = self.entries.len();
        let status = if count == 0 {
            format!(
                "「{folder}」里没有名称包含「{trimmed}」的条目（只筛了本目录已加载的 {current} 项）"
            )
        } else {
            format!(
                "「{folder}」里命中 {count} 项（只筛了本目录已加载的 {current} 项，没有请求网盘）"
            )
        };
        self.as_mut().apply_local_filter(matched, &status);
    }

    /// 第二段：遍历整个网盘。本地没筛到也可以直接点它。
    pub fn search_all(mut self: Pin<&mut Self>) {
        let query = self.search_query.to_string().trim().to_owned();
        if query.is_empty() {
            self.as_mut()
                .set_message("先输入关键词，再点「全盘搜索」", false);
            return;
        }
        if !self.search_settled {
            // 已经在扫了：防连点，重复起线程只会让两轮互抢
            return;
        }
        let cancel = Arc::new(Cancel::default());
        let epoch = {
            let mut rust = self.as_mut().rust_mut();
            rust.search_active = true;
            if let Some(previous) = rust.search_cancel.take() {
                previous.cancel();
            }
            rust.search_epoch = rust.search_epoch.wrapping_add(1);
            rust.search_cancel = Some(Arc::clone(&cancel));
            rust.search_settled = false;
            rust.search_scanned = 0;
            rust.search_results = QVariantList::default();
            rust.search_displayed = 0;
            rust.search_hits = 0;
            rust.search_index.clear();
            rust.search_truncated = false;
            rust.search_status = QString::from("正在扫描网盘……");
            rust.search_epoch
        };
        self.as_mut().notify();

        let session = self.backend.session.clone();
        let post_cancel = Arc::clone(&cancel);
        let spawned = std::thread::Builder::new()
            .name("kichi-search".into())
            .spawn(move || {
                let mut on_batch = |batch: ScanBatch| {
                    post_batch(epoch, batch, Arc::clone(&post_cancel));
                };
                let outcome = session.search_all(&query, &cancel, &mut on_batch);
                let cancelled = cancel.is_cancelled();
                post_finished(epoch, outcome, cancelled);
            });
        if let Err(err) = spawned {
            eprintln!("[kichi] 无法创建搜索线程：{err}");
            self.as_mut().finish_scan(ScanEnd::Stopped);
            self.as_mut()
                .set_message("无法创建搜索线程，全盘搜索没有开始", false);
        }
    }

    /// 停止扫描：按 `Cancel`，已扫出的结果保留。
    pub fn stop_search(mut self: Pin<&mut Self>) {
        if self.search_settled {
            return;
        }
        {
            let mut rust = self.as_mut().rust_mut();
            if let Some(cancel) = rust.search_cancel.take() {
                cancel.cancel();
            }
            // 线程下一轮检查到取消就退出；界面立刻宣告「已停止」，
            // 已扫出的结果原地保留
            rust.search_epoch = rust.search_epoch.wrapping_add(1);
        }
        self.as_mut().finish_scan(ScanEnd::Stopped);
    }

    /// 退出搜索视图，回到当前目录。
    pub fn exit_search(mut self: Pin<&mut Self>) {
        {
            let mut rust = self.as_mut().rust_mut();
            rust.search_epoch = rust.search_epoch.wrapping_add(1);
            if let Some(cancel) = rust.search_cancel.take() {
                cancel.cancel();
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
        }
        // 当前目录的条目一直留在 Rust 侧：换个列表重画即可，不用重新拉网
        let entries = self.entries.clone();
        self.as_mut().show_entries(entries);
    }

    /// 进了搜索结果里的文件夹：走正常目录导航，面包屑是真实层级。
    ///
    /// 搜索里点文件夹 = 一次「跳转」，用完就退出搜索视图——否则打开的目录
    /// 与被筛过的旧列表对不上，用户会以为进了错的文件夹。
    /// （决策：计划只要求文件夹可进入；文件走行尾的播放/下载按钮。）
    pub fn open_search_hit(mut self: Pin<&mut Self>, id: QString) {
        let id = id.to_string();
        let Some(hit) = self.search_index.get(&id).cloned() else {
            self.as_mut().set_message("该结果已过期，请重新搜索", false);
            return;
        };
        if !hit.is_folder() {
            self.as_mut().set_message(
                "结果里的文件请点行尾的播放 / 下载按钮；要进它所在的文件夹，搜索那个文件夹名",
                false,
            );
            return;
        }
        if self.listing {
            return;
        }
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
            rust.crumbs.push(super::files::Crumb {
                id: hit.id().to_owned(),
                name: hit.name().to_owned(),
            });
        }
        self.as_mut().sync_breadcrumb_visible();
        self.as_mut().refresh_files();
    }

    /// 本地筛选的结果直接当列表显示（状态栏文案由调用方给）。
    fn apply_local_filter(mut self: Pin<&mut Self>, mut entries: Vec<FileEntry>, status: &str) {
        let folder = self
            .crumbs
            .last()
            .map(|crumb| crumb.name.clone())
            .unwrap_or_else(|| ROOT_NAME.to_owned());
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
            rust.search_results = filtered;
            rust.search_status = QString::from(status);
            rust.status_message = QString::from(format!("「{folder}」：{status}").as_str());
            rust.busy = false;
        }
        self.as_mut().notify();
    }

    /// 全盘扫描的一页到达：标进度、追加命中、刷新列表（主线程）。
    pub(crate) fn apply_batch(mut self: Pin<&mut Self>, batch: ScanBatch) {
        {
            let mut rust = self.as_mut().rust_mut();
            rust.search_scanned = i32::try_from(batch.scanned).unwrap_or(i32::MAX);
            for hit in batch.hits {
                // 同一条目可能在官方分页里重复出现：按 id 去重
                if rust.search_index.contains_key(hit.id()) {
                    continue;
                }
                if rust.search_displayed < SEARCH_LIST_LIMIT {
                    let value = QVariant::from(&hit_map(&hit));
                    rust.search_results.append(value);
                    rust.search_displayed += 1;
                } else {
                    rust.search_truncated = true;
                }
                rust.search_index.insert(hit.id().to_owned(), hit);
            }
            rust.search_hits = i32::try_from(rust.search_index.len()).unwrap_or(i32::MAX);
            rust.search_status = QString::from(
                format!(
                    "正在扫描网盘……已扫 {} 条，命中 {} 条",
                    rust.search_scanned, rust.search_hits
                )
                .as_str(),
            );
        }
        self.as_mut().notify();
    }

    /// 全盘扫描结束：三态文案（扫完 / 用户停止 / 官方分页异常）。
    fn finish_scan(mut self: Pin<&mut Self>, end: ScanEnd) {
        {
            let mut rust = self.as_mut().rust_mut();
            rust.search_settled = true;
            rust.search_cancel = None;
            let scanned = rust.search_scanned;
            let hits = rust.search_hits;
            let text = match end {
                ScanEnd::Finished => {
                    format!("全盘扫描完成：共扫 {scanned} 条，命中 {hits} 条")
                }
                ScanEnd::Stopped => {
                    format!("已停止：当前结果基于已扫的前 {scanned} 条，命中 {hits} 条")
                }
                ScanEnd::BrokenPaging => format!(
                    "官方分页异常（下一页标记原地打转），扫描已停止，结果可能不全：已扫 {scanned} 条，命中 {hits} 条"
                ),
            };
            rust.search_status = QString::from(text.as_str());
        }
        self.as_mut().notify();
    }

    /// 退出登录：清空搜索并作废在途扫描。
    pub(crate) fn reset_search(mut self: Pin<&mut Self>) {
        self.as_mut().exit_search();
        self.as_mut().rust_mut().search_status = QString::default();
        self.as_mut().notify();
    }

    /// 改了排序：按同一规则重排已显示的结果（搜索期间不走 `files` 那一栏）。
    pub(crate) fn re_sort_search_results(mut self: Pin<&mut Self>) {
        let field = self.sort_field;
        let ascending = self.sort_ascending;
        let mut hits: Vec<SearchHit> = self.search_index.values().cloned().collect();
        hits.sort_by_key(|hit| hit.order);
        let mut entries: Vec<FileEntry> = hits.iter().map(|hit| hit.entry.clone()).collect();
        sort_entries(&mut entries, field, ascending);
        // 排完按 id 回查所在文件夹：条目本身不带路径
        let by_id: std::collections::HashMap<&str, &SearchHit> =
            hits.iter().map(|hit| (hit.id(), hit)).collect();
        let mut list = QVariantList::default();
        for entry in entries.iter().take(SEARCH_LIST_LIMIT) {
            let Some(hit) = by_id.get(entry.id.as_str()) else {
                continue;
            };
            list.append(QVariant::from(&hit_map(hit)));
        }
        self.as_mut().rust_mut().search_results = list;
        self.as_mut().notify();
    }
}

/// 当前目录里的一条 → QML 小 map（结构与搜索结果一致，所在文件夹写当前目录名）。
fn current_dir_map(entry: &FileEntry, folder: &str) -> QVariantMap {
    let text = |value: &str| QVariant::from(&QString::from(value));
    let mut map = entry_map(entry);
    map.insert_clone(&QString::from("folderText"), &text(folder));
    map.insert_clone(&QString::from("folderId"), &text(""));
    map
}

/// 一条命中 → QML 小 map：条目字段与目录列表完全一致，外加「所在文件夹」。
fn hit_map(hit: &SearchHit) -> QVariantMap {
    let text = |value: &str| QVariant::from(&QString::from(value));
    let mut map = entry_map(&hit.entry);
    let location = match &hit.folder_path {
        Some(path) if !path.is_empty() => path.clone(),
        // 路径拼不出来时至少说明在哪：根目录还是某个文件夹
        _ if hit.folder_id.is_empty() => ROOT_NAME.to_owned(),
        _ => "（所在文件夹）".to_owned(),
    };
    map.insert_clone(&QString::from("folderText"), &text(&location));
    map.insert_clone(&QString::from("folderId"), &text(&hit.folder_id));
    map
}

// —— 后台线程 → 主线程（不碰 Qt 对象，只入队）——

fn post_batch(epoch: u64, batch: ScanBatch, cancel: Arc<Cancel>) {
    enqueue_ui(move |mut qichi: Pin<&mut qobject::Kichi>| {
        // 停止 / 退出搜索 / 登出后旧页作废；线程自己的取消也算
        if qichi.as_ref().rust().search_epoch != epoch || cancel.is_cancelled() {
            return;
        }
        qichi.as_mut().apply_batch(batch);
    });
}

fn post_finished(epoch: u64, outcome: Result<ScanEnd, Failure>, cancelled: bool) {
    enqueue_ui(move |mut qichi: Pin<&mut qobject::Kichi>| {
        if qichi.as_ref().rust().search_epoch != epoch {
            return;
        }
        match outcome {
            // 停止之后线程才拿到结果 / 错误：界面早就是「已停止」，不再改口
            Ok(_) | Err(_) if cancelled => qichi.as_mut().finish_scan(ScanEnd::Stopped),
            Ok(end) => qichi.as_mut().finish_scan(end),
            Err(err) => {
                // 接口异常：已扫出的结果保留，如实说明为什么停
                let reason = err.to_string();
                let (scanned, hits) = {
                    let this = qichi.as_ref();
                    let rust = this.rust();
                    (rust.search_scanned, rust.search_hits)
                };
                qichi.as_mut().finish_scan(ScanEnd::Stopped);
                {
                    let mut rust = qichi.as_mut().rust_mut();
                    rust.search_status = QString::from(
                        format!(
                            "扫描中断：{reason}（已扫 {scanned} 条，命中 {hits} 条，结果可能不全）"
                        )
                        .as_str(),
                    );
                }
                qichi.as_mut().notify();
                // 登录态失效要切回登录页；其余只提示，不打断结果
                if matches!(err, Failure::Auth(_)) {
                    qichi.as_mut().on_failure(err);
                } else {
                    qichi
                        .as_mut()
                        .set_message(&format!("全盘搜索中断：{reason}"), false);
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use kichi_core::EntryKind;

    fn hit(name: &str, folder: &str, path: Option<&str>) -> SearchHit {
        SearchHit {
            entry: FileEntry {
                id: name.to_owned(),
                name: name.to_owned(),
                kind: EntryKind::File,
                size: 1024,
                modified_at: None,
                md5: None,
                audit_flagged: false,
                media: true,
            },
            folder_id: folder.to_owned(),
            folder_path: path.map(str::to_owned),
            order: 0,
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
}
