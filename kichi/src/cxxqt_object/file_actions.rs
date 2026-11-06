//! 文件的写操作：改名 / 新建文件夹 / 移动 / 复制 / 移入回收站 / 彻底删除。
//!
//! 从 `files.rs` 拆出来：那里只管浏览（目录栈、列表、排序），
//! 这里是会改云端数据的入口，出错方式和浏览完全不同。
//!
//! 每个入口只占用自己操作的条目 id：不同条目的写请求可以同时在途，
//! 同一 id 在完成前拒绝第二个写操作。后台执行 → 主线程提交或回滚
//! 各自的乐观快照 → 发带目标 id 的 `fileActionDone`。
//! 普通成功不再常驻状态栏；移入回收站例外，保留「可恢复」这一额外后果。

use std::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QString, QStringList};

use super::actions::run_in_background;
use super::file_action_lock::FileActionDomain;
use super::file_optimistic::{is_unconfirmed_create, pending_folder_id};
use super::qobject;
use crate::session::tree_index::Change;
use crate::session::Failure;

impl qobject::Kichi {
    /// 改名。
    ///
    /// 名字在 Rust 侧再校验一次（QML 的输入框不是唯一入口），
    /// 本地先改名并按当前排序重排；失败恢复操作前快照和搜索命中。
    pub fn rename_entry(mut self: Pin<&mut Self>, id: QString, name: QString) -> bool {
        let id = id.to_string();
        let name = name.to_string();
        let ids = vec![id.trim().to_owned()];
        let Some(action_id) = self
            .as_mut()
            .begin_file_action(&ids, FileActionDomain::Files)
        else {
            self.as_mut()
                .set_message("相关条目正在处理中，请稍后再改名");
            return false;
        };
        if self.as_ref().rust().listing {
            self.as_mut().fail_file_action(
                action_id,
                "rename",
                Failure::Other("文件列表正在刷新，请稍后再重命名".into()),
            );
            return false;
        }
        if id.trim().is_empty() {
            self.as_mut().fail_file_action(
                action_id,
                "rename",
                Failure::Other("没有选中条目".into()),
            );
            return false;
        }
        if is_unconfirmed_create(self.as_ref().rust(), &id) {
            self.as_mut().fail_file_action(
                action_id,
                "rename",
                Failure::Other("文件夹正在同步真实 id，请稍后再改名".into()),
            );
            return false;
        }
        if !self.as_mut().optimistic_rename(action_id, &id, &name) {
            self.as_mut().fail_file_action(
                action_id,
                "rename",
                Failure::Other("条目已不在当前列表中".into()),
            );
            return false;
        }
        let session = self.backend.session.clone();
        let for_message = name.trim().to_owned();
        self.as_mut()
            .set_file_action_message(action_id, &format!("正在改名为「{for_message}」……"));
        run_in_background(
            move || {
                session.rename_entry(&id, &name)?;
                // 网络成功后才写索引；索引失败只记录，不改变这次操作的结果。
                session.write_index(|index| {
                    index.apply_change(Change::Rename {
                        id: &id,
                        name: &name,
                    })
                });
                Ok(())
            },
            move |mut qichi, result| match result {
                Ok(()) => {
                    qichi.as_mut().commit_file_mutation(action_id);
                    qichi.as_mut().end_file_action(
                        action_id,
                        "rename",
                        &format!("已改名为「{for_message}」"),
                    );
                }
                Err(err) => {
                    qichi.as_mut().rollback_file_mutation(action_id);
                    qichi.as_mut().fail_file_action(action_id, "rename", err);
                }
            },
        );
        true
    }

    /// 在当前目录下新建文件夹。
    pub fn create_folder(mut self: Pin<&mut Self>, name: QString) -> bool {
        let Some(action_id) = self
            .as_mut()
            .begin_file_action(&[], FileActionDomain::Files)
        else {
            self.as_mut()
                .set_message("文件正在处理中，请稍后再新建文件夹");
            return false;
        };
        if self.as_ref().rust().listing {
            self.as_mut().fail_file_action(
                action_id,
                "folder",
                Failure::Other("文件列表正在刷新，请稍后再新建文件夹".into()),
            );
            return false;
        }
        let name = name.to_string();
        if name.trim().is_empty() {
            self.as_mut().fail_file_action(
                action_id,
                "folder",
                Failure::Other("文件夹名称不能为空".into()),
            );
            return false;
        }
        let parent_id = self
            .crumbs
            .last()
            .map(|crumb| crumb.id.clone())
            .filter(|id| !id.is_empty());
        let parent_for_cache = parent_id.clone().unwrap_or_default();
        let session = self.backend.session.clone();
        let for_message = name.trim().to_owned();
        let temp_id = pending_folder_id(action_id);
        if !self.as_mut().add_file_action_target(action_id, &temp_id) {
            self.as_mut().fail_file_action(
                action_id,
                "folder",
                Failure::Other("文件夹正在同步真实 id，请稍后再试".into()),
            );
            return false;
        }
        if !self
            .as_mut()
            .optimistic_create_folder(action_id, &name, &temp_id)
        {
            self.as_mut().fail_file_action(
                action_id,
                "folder",
                Failure::Other("无法在当前目录插入新文件夹".into()),
            );
            return false;
        };
        self.as_mut()
            .set_file_action_message(action_id, "正在新建文件夹……");
        self.as_mut()
            .folder_create_started(QString::from(temp_id.as_str()));
        run_in_background(
            move || session.create_folder(parent_id.as_deref(), &name),
            move |mut qichi, result| match result {
                Ok(()) => {
                    qichi.as_mut().commit_file_mutation(action_id);
                    // 新建响应没有 id，必须校准目录；若操作期间已经导航，只让父目录
                    // 在下次进入时重新读取，不强行刷新用户现在所在的别的目录。
                    qichi.as_mut().mark_file_dir_stale(&parent_for_cache);
                    qichi.as_mut().end_file_action(
                        action_id,
                        "folder",
                        &format!("已新建文件夹「{for_message}」"),
                    );
                    if qichi.as_ref().rust().current_dir_id.to_string() == parent_for_cache {
                        qichi.as_mut().refresh_files_background();
                    }
                }
                Err(err) => {
                    qichi.as_mut().rollback_file_mutation(action_id);
                    qichi.as_mut().fail_file_action(action_id, "folder", err);
                }
            },
        );
        true
    }

    /// 移入回收站。支持多选批量；请求被接受后立即更新列表，失败再回滚。
    pub fn trash_entries(mut self: Pin<&mut Self>, ids: QStringList) -> bool {
        if self.trash_clearing {
            self.as_mut()
                .set_message("回收站正在清空，请稍后再移入回收站");
            return false;
        }
        let ids = clean_ids(ids);
        let Some(action_id) = self
            .as_mut()
            .begin_file_action(&ids, FileActionDomain::Files)
        else {
            self.as_mut()
                .set_message("相关条目正在处理中，请稍后再移入回收站");
            return false;
        };
        self.as_mut().mark_file_action_adds_to_trash(action_id);
        if self.as_ref().rust().listing {
            self.as_mut().fail_file_action(
                action_id,
                "trash",
                Failure::Other("文件列表正在刷新，请稍后再移入回收站".into()),
            );
            return false;
        }
        if ids.is_empty() {
            self.as_mut().fail_file_action(
                action_id,
                "trash",
                Failure::Other("没有选中条目".into()),
            );
            return false;
        }
        if ids
            .iter()
            .any(|id| is_unconfirmed_create(self.as_ref().rust(), id))
        {
            self.as_mut().fail_file_action(
                action_id,
                "trash",
                Failure::Other("文件夹正在同步真实 id，请稍后再移入回收站".into()),
            );
            return false;
        }
        let subject = item_subject(&ids, self.as_ref().rust().entry_name(&ids[0]));
        if !self.as_mut().optimistic_remove_entries(action_id, &ids) {
            self.as_mut().fail_file_action(
                action_id,
                "trash",
                Failure::Other("选中的条目已不在当前列表中".into()),
            );
            return false;
        }
        let session = self.backend.session.clone();
        self.as_mut()
            .set_file_action_message(action_id, &format!("正在把{subject}移入回收站……"));
        run_in_background(
            move || {
                let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
                session.trash_entries(&refs)?;
                session.write_index(|index| index.apply_change(Change::Trash(&ids)));
                Ok(())
            },
            move |mut qichi, result| match result {
                Ok(()) => {
                    qichi.as_mut().commit_file_mutation(action_id);
                    qichi.as_mut().mark_trash_stale();
                    qichi.as_mut().end_file_action(
                        action_id,
                        "trash",
                        &format!("{subject}已移入回收站（可在回收站页恢复）"),
                    );
                }
                Err(err) => {
                    qichi.as_mut().rollback_file_mutation(action_id);
                    qichi.as_mut().fail_file_action(action_id, "trash", err);
                }
            },
        );
        true
    }

    /// 彻底删除（不经过回收站）。支持多选批量；请求被接受后立即更新列表，失败再回滚。
    ///
    /// 与移入回收站共用确认框：默认走回收站，用户主动勾选后才进这里。
    /// 一次仍受 core 的批量上限约束（100 项），超限按失败回滚处理。
    pub fn delete_entries(mut self: Pin<&mut Self>, ids: QStringList) -> bool {
        let ids = clean_ids(ids);
        let Some(action_id) = self
            .as_mut()
            .begin_file_action(&ids, FileActionDomain::Files)
        else {
            self.as_mut()
                .set_message("相关条目正在处理中，请稍后再删除");
            return false;
        };
        if self.as_ref().rust().listing {
            self.as_mut().fail_file_action(
                action_id,
                "delete",
                Failure::Other("文件列表正在刷新，请稍后再删除".into()),
            );
            return false;
        }
        if ids.is_empty() {
            self.as_mut().fail_file_action(
                action_id,
                "delete",
                Failure::Other("没有选中条目".into()),
            );
            return false;
        }
        if ids
            .iter()
            .any(|id| is_unconfirmed_create(self.as_ref().rust(), id))
        {
            self.as_mut().fail_file_action(
                action_id,
                "delete",
                Failure::Other("文件夹正在同步真实 id，请稍后再删除".into()),
            );
            return false;
        }
        let subject = item_subject(&ids, self.as_ref().rust().entry_name(&ids[0]));
        if !self.as_mut().optimistic_remove_entries(action_id, &ids) {
            self.as_mut().fail_file_action(
                action_id,
                "delete",
                Failure::Other("选中的条目已不在当前列表中".into()),
            );
            return false;
        }
        let session = self.backend.session.clone();
        self.as_mut()
            .set_file_action_message(action_id, &format!("正在彻底删除{subject}……"));
        run_in_background(
            move || {
                let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
                session.delete_entries_permanently(&refs)?;
                session.write_index(|index| index.apply_change(Change::Remove(&ids)));
                Ok(())
            },
            move |mut qichi, result| match result {
                Ok(()) => {
                    qichi.as_mut().commit_file_mutation(action_id);
                    qichi.as_mut().mark_trash_stale();
                    qichi.as_mut().end_file_action(
                        action_id,
                        "delete",
                        &format!("已彻底删除{subject}"),
                    );
                }
                Err(err) => {
                    qichi.as_mut().rollback_file_mutation(action_id);
                    qichi.as_mut().fail_file_action(action_id, "delete", err);
                }
            },
        );
        true
    }
}

/// QML 传来的 id 列表：去空白、去空项（去重与上限由 core 的 `normalize_ids` 负责）。
pub(super) fn clean_ids(ids: QStringList) -> Vec<String> {
    ids.iter()
        .map(|id| id.to_string().trim().to_owned())
        .filter(|id| !id.is_empty())
        .collect()
}

/// 状态栏 / 对话框里的主语：单项用名字，多项用计数（不假装知道每个名字）。
pub(super) fn item_subject(ids: &[String], first_name: Option<String>) -> String {
    if ids.len() == 1 {
        format!("「{}」", first_name.unwrap_or_else(|| "该条目".to_owned()))
    } else {
        format!("选中的 {} 项", ids.len())
    }
}
