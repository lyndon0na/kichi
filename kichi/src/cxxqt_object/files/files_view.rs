//! 文件列表 → QML 字段：排序、面包屑同步与条目映射。

use std::pin::Pin;
use std::time::Instant;

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QMap, QMapPair_QString_QVariant, QString, QStringList, QVariant};
use kichi_core::{EntryKind, FileEntry};

use crate::session::thumbnail_cache;

use super::actions::format_size;
use super::file_optimistic::merge_unresolved_create;
use super::qobject;
use super::QVariantList;

type QVariantMap = QMap<QMapPair_QString_QVariant>;

impl qobject::Kichi {
    pub(crate) fn restore_crumbs_on_failure(mut self: Pin<&mut Self>) {
        let restored = {
            let mut rust = self.as_mut().rust_mut();
            rust.crumb_backup.take()
        };
        let Some(crumbs) = restored else {
            return;
        };
        {
            let mut rust = self.as_mut().rust_mut();
            rust.crumbs = crumbs;
        }
        self.as_mut().sync_breadcrumb();
    }

    /// 把目录列表写回属性（只在主线程调用）。
    pub(crate) fn show_entries(mut self: Pin<&mut Self>, entries: Vec<FileEntry>) {
        let dir_id = self
            .crumbs
            .last()
            .map(|crumb| crumb.id.clone())
            .unwrap_or_default();
        let (entries, files, changed, created_ids) = {
            let mut rust = self.as_mut().rust_mut();
            let (mut entries, created_ids) = merge_unresolved_create(&mut rust, entries);
            sort_entries(&mut entries, rust.sort_field, rust.sort_ascending);
            let changed = rust.entries != entries;
            let files = changed.then(|| list_from_with_pending(&entries, &rust.pending_entry_ids));
            (entries, files, changed, created_ids)
        };
        let visible = {
            let mut rust = self.as_mut().rust_mut();
            let visible = rust.listing || rust.files_manual_refreshing;
            if changed {
                rust.files = files.unwrap_or_default();
                // 界面据此做一次性的滚动恢复（刷新 / 返回上级都靠这个节拍）
                rust.files_revision = rust.files_revision.wrapping_add(1);
            }
            rust.files_cache.store(dir_id, &entries, Instant::now());
            rust.entries = entries;
            rust.files_request_running = false;
            rust.listing = false;
            rust.files_manual_refreshing = false;
            rust.files_stale = false;
            visible
        };
        if changed || visible {
            self.as_mut().notify();
        }
        for id in created_ids {
            self.as_mut().folder_created(QString::from(id.as_str()));
        }
    }

    /// 恢复某个目录的旧快照；即使内容相同也推进 revision，让返回目录时恢复滚动位置。
    pub(crate) fn show_cached_entries(mut self: Pin<&mut Self>, mut entries: Vec<FileEntry>) {
        let files = {
            let this = self.as_ref();
            let rust = this.rust();
            sort_entries(&mut entries, rust.sort_field, rust.sort_ascending);
            list_from_with_pending(&entries, &rust.pending_entry_ids)
        };
        {
            let mut rust = self.as_mut().rust_mut();
            rust.entries = entries;
            rust.files = files;
            rust.files_revision = rust.files_revision.wrapping_add(1);
            rust.files_request_running = false;
            rust.listing = false;
            rust.files_manual_refreshing = false;
        }
        self.as_mut().notify();
    }

    /// 改了排序：对已缓存的条目本地重排，不重新拉网（QML 的 `files` 随之刷新）。
    ///
    /// 搜索视图里没有 `files` 那一栏可排：改按同一规则重排搜索结果。
    pub(crate) fn re_sort_entries(mut self: Pin<&mut Self>) {
        if self.search_active {
            self.as_mut().re_sort_search_results();
            return;
        }
        if self.entries.is_empty() {
            return;
        }
        let mut entries = self.entries.clone();
        sort_entries(&mut entries, self.sort_field, self.sort_ascending);
        let files = list_from_with_pending(&entries, &self.pending_entry_ids);
        let dir_id = self.current_dir_id.to_string();
        {
            let mut rust = self.as_mut().rust_mut();
            rust.files = files;
            rust.files_cache.replace_entries(dir_id, &entries);
            rust.entries = entries;
        }
        self.as_mut().notify();
    }

    /// 改名 / 搜索里进文件夹后要把面包屑刷出去（`sync_breadcrumb` 是同模块私有的）。
    pub(crate) fn sync_breadcrumb_visible(mut self: Pin<&mut Self>) {
        self.as_mut().sync_breadcrumb();
    }

    /// 目录栈 → 面包屑属性（显示名，顺序从根到当前）。
    pub(crate) fn sync_breadcrumb(mut self: Pin<&mut Self>) {
        let (crumb_names, current_id) = {
            let this = self.as_ref();
            let names = this
                .rust()
                .crumbs
                .iter()
                .map(|crumb| crumb.name.clone())
                .collect::<Vec<_>>();
            let id = this
                .rust()
                .crumbs
                .last()
                .map(|crumb| crumb.id.clone())
                .unwrap_or_default();
            (names, id)
        };
        let mut names = QStringList::default();
        for name in crumb_names {
            names.append(QString::from(name.as_str()));
        }
        {
            let mut rust = self.as_mut().rust_mut();
            rust.breadcrumb = names;
            rust.current_dir_id = QString::from(current_id);
        }
        self.as_mut().notify();
    }
}

/// 排序：文件夹永远在前，同类按设置字段比较，方向由 `ascending` 决定。
///
/// 名称比较用自然排序（大小写不敏感、数字按数值），保证 `第2集` 排在
/// `第10集` 前；大小 / 时间相同时回落到名称，保证同一目录多次刷新顺序稳定。
pub(crate) fn sort_entries(
    entries: &mut [FileEntry],
    field: crate::settings::SortField,
    ascending: bool,
) {
    use crate::settings::SortField;
    let name = |a: &FileEntry, b: &FileEntry| kichi_core::natural::natural_cmp(&a.name, &b.name);
    let folder_first = |a: &FileEntry, b: &FileEntry| {
        (a.kind == EntryKind::Folder)
            .cmp(&(b.kind == EntryKind::Folder))
            .reverse()
    };
    entries.sort_by(|a, b| {
        // 目录/文件分组与排序方向无关，先决出来
        let group = folder_first(a, b);
        if group != std::cmp::Ordering::Equal {
            return group;
        }
        let order = match field {
            SortField::Name => name(a, b),
            SortField::Size => a.size.cmp(&b.size).then_with(|| name(a, b)),
            SortField::Modified => {
                let left = a.modified_at.map(|time| time.timestamp()).unwrap_or(0);
                let right = b.modified_at.map(|time| time.timestamp()).unwrap_or(0);
                left.cmp(&right).then_with(|| name(a, b))
            }
        };
        if ascending {
            order
        } else {
            order.reverse()
        }
    });
}

pub(crate) fn list_from_with_pending(
    entries: &[FileEntry],
    pending_ids: &std::collections::HashSet<String>,
) -> QVariantList {
    let mut files = QVariantList::default();
    for entry in entries {
        let mut map = entry_map(entry);
        map.insert_clone(
            &QString::from("pending"),
            &QVariant::from(&pending_ids.contains(&entry.id)),
        );
        files.append(QVariant::from(&map));
    }
    files
}

/// 一条列表项 → QML 用的小 map（只带界面要显示的字段）。
///
/// 搜索结果的条目行也用这一份：两边字段一致，界面就不必分辨数据从哪来。
pub(crate) fn entry_map(entry: &FileEntry) -> QVariantMap {
    let text = |value: &str| QVariant::from(&QString::from(value));
    let kind = match entry.kind {
        EntryKind::Folder => "folder",
        EntryKind::File => "file",
    };
    let size = match entry.kind {
        EntryKind::Folder => "—".to_owned(),
        EntryKind::File => format_size(entry.size),
    };
    let modified = local_time_text(entry.modified_at);
    let created = local_time_text(entry.created_at);

    let mut map = QVariantMap::default();
    map.insert_clone(&QString::from("id"), &text(&entry.id));
    map.insert_clone(&QString::from("name"), &text(&entry.name));
    map.insert_clone(&QString::from("kind"), &text(kind));
    map.insert_clone(&QString::from("sizeText"), &text(&size));
    map.insert_clone(&QString::from("modifiedText"), &text(&modified));
    map.insert_clone(&QString::from("createdText"), &text(&created));
    map.insert_clone(&QString::from("parentId"), &text(entry.parent_id.as_str()));
    map.insert_clone(&QString::from("starred"), &QVariant::from(&entry.starred));
    map.insert_clone(
        &QString::from("auditFlag"),
        &QVariant::from(&entry.audit_flagged),
    );
    map.insert_clone(
        &QString::from("playable"),
        &QVariant::from(&(entry.kind == EntryKind::File && entry.media)),
    );
    map.insert_clone(
        &QString::from("video"),
        &QVariant::from(&(entry.kind == EntryKind::File && entry.video)),
    );
    // 图片按扩展名判定（列表不带 kind 过滤时官方分类字段为空）。
    // QML 只据此决定双击是否打开预览，不参与缩略图 / 播放入口。
    map.insert_clone(
        &QString::from("image"),
        &QVariant::from(&(entry.kind == EntryKind::File && kichi_core::is_image_name(&entry.name))),
    );
    // 只告诉界面「有没有可尝试的缩略图」，远程 URL 永远不离开 Rust。
    map.insert_clone(
        &QString::from("hasThumbnail"),
        &QVariant::from(&has_thumbnail(entry)),
    );
    map.insert_clone(&QString::from("pending"), &QVariant::from(&false));
    // 目录列表 / 资料库 / 手工筛出的行都不在回收站；搜索结果会覆盖这一项。
    map.insert_clone(&QString::from("inTrash"), &QVariant::from(&false));
    map
}

/// 列表与属性弹窗共用的本地时间文本；缺失时为空串（界面各自决定占位符）。
pub(crate) fn local_time_text(time: Option<chrono::DateTime<chrono::Utc>>) -> String {
    time.map(|time| {
        time.with_timezone(&chrono::Local)
            .format("%Y-%m-%d %H:%M")
            .to_string()
    })
    .unwrap_or_default()
}

/// 远程地址或本地缓存任一可用，界面就能先显示缩略图。
///
/// 快照与本地索引条目不保存签名地址（`thumbnail_url` 反序列化为空），
/// 冷启动只有本地缓存能立刻出图，不能等后台校准补回地址。
fn has_thumbnail(entry: &FileEntry) -> bool {
    if entry.kind != EntryKind::File {
        return false;
    }
    if !entry.thumbnail_url.is_empty() {
        return true;
    }
    let visual = entry.video || kichi_core::is_image_name(&entry.name);
    visual && thumbnail_cache::find(&entry.id).is_some()
}

#[cfg(test)]
#[path = "files_view_tests.rs"]
mod tests;
