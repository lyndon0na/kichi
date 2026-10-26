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
                    Ok(entries) => qichi.as_mut().show_entries(entries),
                    Err(err) => qichi.as_mut().on_failure(err),
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
        }
        self.as_mut().sync_breadcrumb();
    }

    /// 把列表与状态写回属性（只在主线程调用）。
    fn show_entries(mut self: Pin<&mut Self>, mut entries: Vec<FileEntry>) {
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
    pub(crate) fn re_sort_entries(mut self: Pin<&mut Self>) {
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

    /// 目录栈 → 面包屑属性（显示名，顺序从根到当前）。
    fn sync_breadcrumb(mut self: Pin<&mut Self>) {
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

    /// 改名。
    ///
    /// 名字在 Rust 侧再校验一次（QML 的输入框不是唯一入口），
    /// 成功后整目录刷新——只改本地缓存的话，同名冲突、排序位置都会失真。
    pub fn rename_entry(mut self: Pin<&mut Self>, id: QString, name: QString) {
        if !self.as_mut().begin_file_action() {
            return;
        }
        let id = id.to_string();
        let name = name.to_string();
        if id.trim().is_empty() {
            self.as_mut().end_file_action("rename", "没有选中条目");
            return;
        }
        let session = self.backend.session.clone();
        let for_message = name.trim().to_owned();
        self.as_mut()
            .set_message(&format!("正在改名为「{for_message}」……"), true);
        run_in_background(
            move || session.rename_entry(&id, &name),
            move |mut qichi, result| match result {
                Ok(()) => {
                    qichi
                        .as_mut()
                        .end_file_action("rename", &format!("已改名为「{for_message}」"));
                    qichi.as_mut().refresh_files();
                }
                Err(err) => qichi.as_mut().fail_file_action("rename", err),
            },
        );
    }

    /// 在当前目录下新建文件夹。
    pub fn create_folder(mut self: Pin<&mut Self>, name: QString) {
        if !self.as_mut().begin_file_action() {
            return;
        }
        let name = name.to_string();
        let parent_id = self
            .crumbs
            .last()
            .map(|crumb| crumb.id.clone())
            .filter(|id| !id.is_empty());
        let session = self.backend.session.clone();
        let for_message = name.trim().to_owned();
        self.as_mut().set_message("正在新建文件夹……", true);
        run_in_background(
            move || session.create_folder(parent_id.as_deref(), &name),
            move |mut qichi, result| match result {
                Ok(()) => {
                    qichi
                        .as_mut()
                        .end_file_action("folder", &format!("已新建文件夹「{for_message}」"));
                    qichi.as_mut().refresh_files();
                }
                Err(err) => qichi.as_mut().fail_file_action("folder", err),
            },
        );
    }

    /// 移入回收站。只删一个条目：界面上的删除入口都是单条的，
    /// 批量留给以后做多选时再加。
    pub fn trash_entry(mut self: Pin<&mut Self>, id: QString) {
        if !self.as_mut().begin_file_action() {
            return;
        }
        let id = id.to_string();
        let name = self.entry_name(&id).unwrap_or_else(|| "该条目".to_owned());
        if id.trim().is_empty() {
            self.as_mut().end_file_action("trash", "没有选中条目");
            return;
        }
        let session = self.backend.session.clone();
        let ids = vec![id];
        self.as_mut().set_message("正在移入回收站……", true);
        run_in_background(
            move || {
                let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
                session.trash_entries(&refs)
            },
            move |mut qichi, result| match result {
                Ok(()) => {
                    qichi.as_mut().end_file_action(
                        "trash",
                        &format!("「{name}」已移入回收站（可在官方客户端恢复）"),
                    );
                    qichi.as_mut().refresh_files();
                }
                Err(err) => qichi.as_mut().fail_file_action("trash", err),
            },
        );
    }

    /// 抢占「文件操作进行中」标志；已在操作中返回 false（防止连点重复提交）。
    fn begin_file_action(mut self: Pin<&mut Self>) -> bool {
        if self.as_ref().rust().file_action_running {
            return false;
        }
        self.as_mut().rust_mut().file_action_running = true;
        true
    }

    /// 结束一次文件操作。
    fn end_file_action(mut self: Pin<&mut Self>, action: &str, message: &str) {
        self.as_mut().rust_mut().file_action_running = false;
        self.as_mut().set_message(message, false);
        self.as_mut()
            .file_action_done(QString::from(action), true, QString::from(message));
    }

    /// 文件操作失败：清旗标 + 走统一的错误出口（分类提示不吞）。
    fn fail_file_action(mut self: Pin<&mut Self>, action: &str, failure: crate::session::Failure) {
        self.as_mut().rust_mut().file_action_running = false;
        let text = failure.to_string();
        self.as_mut().on_failure(failure);
        self.as_mut()
            .file_action_done(QString::from(action), false, QString::from(text.as_str()));
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
fn list_from(entries: &[FileEntry]) -> QVariantList {
    let mut files = QVariantList::default();
    for entry in entries {
        files.append(QVariant::from(&entry_map(entry)));
    }
    files
}

/// 一条列表项 → QML 用的小 map（只带界面要显示的字段）。
fn entry_map(entry: &FileEntry) -> QVariantMap {
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
