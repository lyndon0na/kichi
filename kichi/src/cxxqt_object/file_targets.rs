//! 移动 / 复制的目标文件夹选择器。
//!
//! 只负责「列出某个目录下的子文件夹」，与当前目录列表（`files.rs`）完全隔离：
//! 不复用 `files` / `entries` / `listing` / `list_epoch`，避免打开选择器时
//! 把用户正在看的目录冲掉。

use std::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QMap, QMapPair_QString_QVariant, QString, QVariant};
use kichi_core::{EntryKind, FileEntry};

use super::actions::run_in_background;
use super::qobject;
use super::QVariantList;
use crate::session::tree_index;
use crate::session::Session;

type QVariantMap = QMap<QMapPair_QString_QVariant>;

impl qobject::Kichi {
    /// 当前目录栈（根 → 当前）：`[{ id, name }]`。
    ///
    /// 选择器从「当前目录的上一级」开始：当前目录本身是源目录，不能作为目标。
    pub fn current_path(&self) -> QVariantList {
        let mut list = QVariantList::default();
        for crumb in &self.crumbs {
            let mut map = QVariantMap::default();
            map.insert_clone(
                &QString::from("id"),
                &QVariant::from(&QString::from(crumb.id.as_str())),
            );
            map.insert_clone(
                &QString::from("name"),
                &QVariant::from(&QString::from(crumb.name.as_str())),
            );
            list.append(QVariant::from(&map));
        }
        list
    }

    /// 读取选择器某一层的子文件夹。空 id = 根目录。
    ///
    /// 索引就绪时优先读本地：完整且未标脏的快照直接渲染，不再联网（不看
    /// 30 秒 TTL，目录树是低变更数据）；脏 / 未补全时先贴已知行，再后台
    /// 补全。`force` 跳过本地快照强制联网，供界面「刷新」使用。
    /// 索引不可用 / 该目录从没见过 → 回退网络。
    pub fn load_move_targets(mut self: Pin<&mut Self>, parent_id: QString, force: bool) {
        let parent_id = parent_id.to_string();
        let parent = {
            let trimmed = parent_id.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_owned())
        };
        let dir_key = parent.clone().unwrap_or_default();
        let session = self.backend.session.clone();
        // 建库事务在后台跑着时不读索引：主线程等锁会卡住整个窗口
        let use_index = session.local_index_ready() && !self.index_building;

        let epoch = {
            let mut rust = self.as_mut().rust_mut();
            rust.move_targets_epoch = rust.move_targets_epoch.wrapping_add(1);
            rust.move_targets_loading = true;
            rust.move_targets_error = QString::default();
            rust.move_targets_epoch
        };
        let mut pre_rendered = false;

        if use_index && !force {
            match index_targets(&session, &dir_key) {
                Ok(Some(view)) => {
                    let stable = view.stable;
                    let folders = folder_from(&folder_entries(view.folders));
                    pre_rendered = true;
                    {
                        let mut rust = self.as_mut().rust_mut();
                        rust.move_targets = folders;
                        rust.move_targets_loading = !stable;
                        rust.move_targets_dir = dir_key.clone();
                        rust.move_targets_revision = rust.move_targets_revision.wrapping_add(1);
                    }
                    if stable {
                        self.as_mut().notify();
                        return;
                    }
                }
                // 索引就绪但该目录从没出现过：走下面的网络路径
                Ok(None) => {}
                Err(err) => {
                    session.record_tree_index_error(format!("本地索引读取失败：{err}"));
                }
            }
        }
        self.as_mut().notify();

        let result_dir = dir_key.clone();
        run_in_background(
            move || {
                let entries = session.list_dir(parent.as_deref())?;
                // `list_dir` 会翻完所有页，这一批就是该目录的完整列表：
                // 按完整快照写入，目录才可能变成可直接渲染的稳定层。
                if use_index {
                    if let Some(index) = session.tree_index() {
                        if let Err(err) = index.put_listing(&dir_key, &entries, true) {
                            if !index.is_closed() {
                                session.record_tree_index_error(format!("本地索引写入失败：{err}"));
                            }
                        }
                    }
                }
                Ok(entries)
            },
            move |mut qichi, result| {
                // 快速进出目录时旧请求作废，别把上一层的文件夹贴到当前层
                if qichi.as_ref().rust().move_targets_epoch != epoch {
                    return;
                }
                match result {
                    Ok(entries) => {
                        let folders = folder_from(&folder_entries(entries));
                        let mut rust = qichi.as_mut().rust_mut();
                        rust.move_targets_loading = false;
                        rust.move_targets = folders;
                        rust.move_targets_dir = result_dir.clone();
                        rust.move_targets_error = QString::default();
                        rust.move_targets_revision = rust.move_targets_revision.wrapping_add(1);
                    }
                    Err(err) => {
                        {
                            let mut rust = qichi.as_mut().rust_mut();
                            rust.move_targets_loading = false;
                            // 同一目录刷新失败时保留已有行；进入新目录失败则清空，
                            // 免得旧目录的文件夹被当成当前层
                            if !pre_rendered && rust.move_targets_dir != result_dir {
                                rust.move_targets = QVariantList::default();
                            }
                            rust.move_targets_error = QString::from(err.to_string().as_str());
                            rust.move_targets_revision = rust.move_targets_revision.wrapping_add(1);
                        }
                        // 认证 / 验证码 / 限速仍走统一出口，不能只留在选择器里
                        qichi.as_mut().on_failure(err);
                    }
                }
                qichi.as_mut().notify();
            },
        );
    }

    /// 关闭选择器：丢弃在途结果，清掉列表。
    pub fn cancel_move_targets(mut self: Pin<&mut Self>) {
        {
            let mut rust = self.as_mut().rust_mut();
            rust.move_targets_epoch = rust.move_targets_epoch.wrapping_add(1);
            rust.move_targets_loading = false;
            rust.move_targets = QVariantList::default();
            rust.move_targets_dir = String::new();
            rust.move_targets_error = QString::default();
            rust.move_targets_revision = rust.move_targets_revision.wrapping_add(1);
        }
        self.as_mut().notify();
    }
}

/// 索引里这一层的子文件夹；目录从未出现过时返回 `None`，调用方回退网络。
fn index_targets(
    session: &Session,
    dir_id: &str,
) -> Result<Option<tree_index::MoveTargets>, tree_index::IndexError> {
    let Some(index) = session.tree_index() else {
        return Ok(None);
    };
    if index.is_closed() {
        return Ok(None);
    }
    let view = index.move_targets(dir_id)?;
    Ok((view.known || !view.folders.is_empty()).then_some(view))
}

/// 只留文件夹，按名字排序（大小写不敏感，同名回落到原串，保证稳定）。
fn folder_entries(mut entries: Vec<FileEntry>) -> Vec<FileEntry> {
    entries.retain(|entry| entry.kind == EntryKind::Folder);
    entries.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.name.cmp(&b.name))
    });
    entries
}

fn folder_from(entries: &[FileEntry]) -> QVariantList {
    let mut list = QVariantList::default();
    for entry in entries {
        let mut map = QVariantMap::default();
        map.insert_clone(
            &QString::from("id"),
            &QVariant::from(&QString::from(entry.id.as_str())),
        );
        map.insert_clone(
            &QString::from("name"),
            &QVariant::from(&QString::from(entry.name.as_str())),
        );
        list.append(QVariant::from(&map));
    }
    list
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn entry(id: &str, name: &str, kind: EntryKind) -> FileEntry {
        FileEntry {
            id: id.to_owned(),
            name: name.to_owned(),
            kind,
            size: 0,
            created_at: None,
            modified_at: Some(Utc::now()),
            parent_id: String::new(),
            md5: None,
            audit_flagged: false,
            starred: false,
            media: false,
            video: false,
            thumbnail_url: String::new(),
        }
    }

    #[test]
    fn only_folders_are_kept() {
        let entries = vec![
            entry("1", "b", EntryKind::Folder),
            entry("2", "a.mkv", EntryKind::File),
            entry("3", "a", EntryKind::Folder),
        ];
        let folders = folder_entries(entries);
        assert_eq!(
            folders.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
            vec!["3", "1"],
            "文件要滤掉，文件夹按名字排序"
        );
    }

    #[test]
    fn sort_is_case_insensitive_and_stable() {
        let entries = vec![
            entry("1", "Beta", EntryKind::Folder),
            entry("2", "alpha", EntryKind::Folder),
            entry("3", "ALPHA", EntryKind::Folder),
        ];
        let folders = folder_entries(entries);
        let names: Vec<&str> = folders.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["ALPHA", "alpha", "Beta"]);
    }
}
