//! 文件浏览：目录栈、列表与面包屑。
//!
//! 目录栈只存在 Rust 侧：QML 传文件夹 id，不自己拼路径。
//! 排序（文件夹优先）属于展示策略，也在这里做，session 层保留官方顺序。

use std::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QMap, QMapPair_QString_QVariant, QString, QStringList, QVariant};
use kichi_core::{EntryKind, FileEntry};

use super::actions::{format_size, run_in_background};
use super::qobject;
use super::QVariantList;

/// 根目录在面包屑里的名字。
pub(crate) const ROOT_NAME: &str = "全部文件";

type QVariantMap = QMap<QMapPair_QString_QVariant>;

/// 面包屑的一项：目录 id（根为空串）+ 显示名。
#[derive(Debug, Clone)]
pub(crate) struct Crumb {
    pub(crate) id: String,
    pub(crate) name: String,
}

impl Crumb {
    pub(crate) fn root() -> Self {
        Self {
            id: String::new(),
            name: ROOT_NAME.to_owned(),
        }
    }
}

impl qobject::Kichi {
    /// 重新读取当前目录（登录完成、刷新按钮、进出目录都走这里）。
    pub fn refresh_files(mut self: Pin<&mut Self>) {
        if self.listing {
            return;
        }
        let Some(current) = self.crumbs.last().cloned() else {
            return;
        };
        let parent_id = (!current.id.is_empty()).then(|| current.id.clone());
        let session = self.backend.session.clone();
        let epoch = {
            let mut rust = self.as_mut().rust_mut();
            rust.listing = true;
            rust.list_epoch = rust.list_epoch.wrapping_add(1);
            rust.list_epoch
        };
        self.as_mut()
            .set_message(&format!("正在读取「{}」……", current.name), true);

        run_in_background(
            move || session.list_dir(parent_id.as_deref()),
            move |mut qichi, result| {
                // 退出登录 / 重新登录后旧请求作废，避免把上个会话的列表贴上来
                if qichi.as_ref().rust().list_epoch != epoch {
                    return;
                }
                qichi.as_mut().rust_mut().listing = false;
                match result {
                    Ok(entries) => {
                        // 读到了：这一跳算成功，不再需要回滚快照
                        qichi.as_mut().rust_mut().crumb_backup = None;
                        qichi.as_mut().show_entries(entries);
                    }
                    Err(err) => {
                        // 读失败：把面包屑退回跳转前，别让路径指着一个没读到的目录
                        qichi.as_mut().restore_crumbs_on_failure();
                        qichi.as_mut().on_failure(err);
                    }
                }
            },
        );
    }

    /// 进入文件夹（id 必须来自当前列表，避免 QML 侧拼错）。
    pub fn open_dir(mut self: Pin<&mut Self>, id: QString) {
        if self.listing {
            return;
        }
        let id = id.to_string();
        let found = self
            .entries
            .iter()
            .find(|entry| entry.kind == EntryKind::Folder && entry.id == id)
            .map(|entry| entry.name.clone());
        let Some(name) = found else {
            self.as_mut()
                .set_message("该文件夹已不在当前列表中，请刷新后重试", false);
            return;
        };
        {
            let mut rust = self.as_mut().rust_mut();
            rust.crumb_backup = Some(rust.crumbs.clone());
            rust.crumbs.push(Crumb { id, name });
        }
        self.as_mut().sync_breadcrumb();
        self.as_mut().refresh_files();
    }

    /// 回到上一级。
    pub fn open_parent(mut self: Pin<&mut Self>) {
        if self.listing {
            return;
        }
        let moved = {
            let mut rust = self.as_mut().rust_mut();
            if rust.crumbs.len() > 1 {
                rust.crumb_backup = Some(rust.crumbs.clone());
                rust.crumbs.pop();
                true
            } else {
                false
            }
        };
        if !moved {
            return;
        }
        self.as_mut().sync_breadcrumb();
        self.as_mut().refresh_files();
    }

    /// 跳到面包屑上的某一级（点击当前目录本身无效果）。
    pub fn open_crumb(mut self: Pin<&mut Self>, index: i32) {
        if self.listing || index < 0 {
            return;
        }
        let target = index as usize;
        let len = self.crumbs.len();
        if len < 2 || target + 1 >= len {
            return;
        }
        {
            let mut rust = self.as_mut().rust_mut();
            rust.crumb_backup = Some(rust.crumbs.clone());
            rust.crumbs.truncate(target + 1);
        }
        self.as_mut().sync_breadcrumb();
        self.as_mut().refresh_files();
    }

    /// 退出登录时清空浏览状态并让在途请求作废。
    pub(crate) fn reset_files(mut self: Pin<&mut Self>) {
        {
            let mut rust = self.as_mut().rust_mut();
            rust.list_epoch = rust.list_epoch.wrapping_add(1);
            rust.listing = false;
            rust.entries.clear();
            rust.files = QVariantList::default();
            rust.crumbs = vec![Crumb::root()];
            rust.crumb_backup = None;
        }
        self.as_mut().sync_breadcrumb();
    }

    /// 目录读失败：面包屑退回跳转前，别让路径停在一个根本没读到的目录。
    fn restore_crumbs_on_failure(mut self: Pin<&mut Self>) {
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

    /// 把列表与状态写回属性（只在主线程调用）。
    ///
    /// 在本地筛选（搜索结果）里叫这个名字：筛选只是换一批条目展示，
    /// 计数与状态栏文案照旧。
    pub(crate) fn show_entries(mut self: Pin<&mut Self>, mut entries: Vec<FileEntry>) {
        sort_entries(&mut entries, self.sort_field, self.sort_ascending);
        let files = list_from(&entries);
        let count = entries.len();
        let folder = self
            .crumbs
            .last()
            .map(|crumb| crumb.name.clone())
            .unwrap_or_else(|| ROOT_NAME.to_owned());
        {
            let mut rust = self.as_mut().rust_mut();
            rust.files = files;
            rust.entries = entries;
            rust.listing = false;
            rust.busy = false;
            rust.status_message = QString::from(format!("「{folder}」共 {count} 项"));
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
        let files = list_from(&entries);
        {
            let mut rust = self.as_mut().rust_mut();
            rust.files = files;
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
        let crumb_names = {
            let this = self.as_ref();
            this.rust()
                .crumbs
                .iter()
                .map(|crumb| crumb.name.clone())
                .collect::<Vec<_>>()
        };
        let mut names = QStringList::default();
        for name in crumb_names {
            names.append(QString::from(name.as_str()));
        }
        self.as_mut().rust_mut().breadcrumb = names;
        self.as_mut().notify();
    }
}

/// 排序：文件夹永远在前，同类按设置字段比较，方向由 `ascending` 决定。
///
/// 名称比较用 lower-case（中文不受影响，英文大小写不打架）；大小相同时
/// 回落到名称，保证同一目录多次刷新顺序稳定。
pub(crate) fn sort_entries(
    entries: &mut [FileEntry],
    field: crate::settings::SortField,
    ascending: bool,
) {
    use crate::settings::SortField;
    let name = |entry: &FileEntry| entry.name.to_lowercase();
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
            SortField::Name => name(a).cmp(&name(b)),
            SortField::Size => a.size.cmp(&b.size).then_with(|| name(a).cmp(&name(b))),
            SortField::Modified => {
                let left = a.modified_at.map(|time| time.timestamp()).unwrap_or(0);
                let right = b.modified_at.map(|time| time.timestamp()).unwrap_or(0);
                left.cmp(&right).then_with(|| name(a).cmp(&name(b)))
            }
        };
        if ascending {
            order
        } else {
            order.reverse()
        }
    });
}

/// 条目 → QML 模型。
pub(crate) fn list_from(entries: &[FileEntry]) -> QVariantList {
    let mut files = QVariantList::default();
    for entry in entries {
        files.append(QVariant::from(&entry_map(entry)));
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
    let modified = entry
        .modified_at
        .map(|time| {
            time.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_default();

    let mut map = QVariantMap::default();
    map.insert_clone(&QString::from("id"), &text(&entry.id));
    map.insert_clone(&QString::from("name"), &text(&entry.name));
    map.insert_clone(&QString::from("kind"), &text(kind));
    map.insert_clone(&QString::from("sizeText"), &text(&size));
    map.insert_clone(&QString::from("modifiedText"), &text(&modified));
    map.insert_clone(
        &QString::from("auditFlag"),
        &QVariant::from(&entry.audit_flagged),
    );
    map.insert_clone(
        &QString::from("playable"),
        &QVariant::from(&(entry.kind == EntryKind::File && entry.media)),
    );
    map
}

#[cfg(test)]
mod sort_tests {
    use super::*;
    use crate::settings::SortField;
    use chrono::{TimeZone, Utc};

    fn entry(name: &str, kind: EntryKind, size: u64, day: u32) -> FileEntry {
        FileEntry {
            id: name.to_owned(),
            name: name.to_owned(),
            kind,
            size,
            modified_at: Utc.with_ymd_and_hms(2026, 1, day, 0, 0, 0).single(),
            md5: None,
            audit_flagged: false,
            media: false,
        }
    }

    fn names(entries: &[FileEntry]) -> Vec<&str> {
        entries.iter().map(|entry| entry.name.as_str()).collect()
    }

    fn sample() -> Vec<FileEntry> {
        vec![
            entry("Beta.mkv", EntryKind::File, 300, 3),
            entry("alpha.mkv", EntryKind::File, 100, 1),
            entry("Zeta", EntryKind::Folder, 0, 20),
            entry("Aaa", EntryKind::Folder, 0, 10),
        ]
    }

    #[test]
    fn folders_always_come_first() {
        for field in [SortField::Name, SortField::Size, SortField::Modified] {
            let mut entries = sample();
            sort_entries(&mut entries, field, false);
            let split = entries.partition_point(|e| e.kind == EntryKind::Folder);
            assert_eq!(split, 2, "{field:?} 降序时文件夹也必须在前");
            assert!(entries[..split].iter().all(|e| e.kind == EntryKind::Folder));
        }
    }

    #[test]
    fn name_sort_is_case_insensitive() {
        let mut entries = sample();
        sort_entries(&mut entries, SortField::Name, true);
        assert_eq!(
            names(&entries),
            vec!["Aaa", "Zeta", "alpha.mkv", "Beta.mkv"]
        );
        sort_entries(&mut entries, SortField::Name, false);
        assert_eq!(
            names(&entries),
            vec!["Zeta", "Aaa", "Beta.mkv", "alpha.mkv"]
        );
    }

    #[test]
    fn size_and_time_sort_apply_to_files_only_order() {
        let mut entries = sample();
        sort_entries(&mut entries, SortField::Size, true);
        assert_eq!(
            names(&entries),
            vec!["Aaa", "Zeta", "alpha.mkv", "Beta.mkv"]
        );
        sort_entries(&mut entries, SortField::Size, false);
        assert_eq!(
            names(&entries),
            vec!["Zeta", "Aaa", "Beta.mkv", "alpha.mkv"]
        );

        let mut entries = sample();
        sort_entries(&mut entries, SortField::Modified, true);
        assert_eq!(
            names(&entries),
            vec!["Aaa", "Zeta", "alpha.mkv", "Beta.mkv"]
        );
    }

    #[test]
    fn equal_keys_fall_back_to_name_for_stable_output() {
        let mut entries = vec![
            entry("b", EntryKind::File, 5, 1),
            entry("a", EntryKind::File, 5, 1),
        ];
        sort_entries(&mut entries, SortField::Size, true);
        assert_eq!(names(&entries), vec!["a", "b"]);
    }
}
