//! 文件页的移动 / 复制写操作。
//!
//! 移动先在当前目录乐观移除，复制不动源列表；两者都只锁定源条目，
//! 目标目录在成功后标记为过期，下次进入时校准。

use std::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QString, QStringList};

use super::actions::run_in_background;
use super::file_action_lock::FileActionDomain;
use super::file_actions::clean_ids;
use super::file_actions::item_subject;
use super::file_optimistic::is_unconfirmed_create;
use super::qobject;
use crate::session::tree_index::Change;
use crate::session::Failure;

impl qobject::Kichi {
    /// 把条目移动到目标文件夹。空目标 id = 根目录。
    pub fn move_entries(
        mut self: Pin<&mut Self>,
        ids: QStringList,
        target_id: QString,
        target_name: QString,
    ) -> bool {
        self.as_mut()
            .transfer_entries("move", ids, target_id, target_name)
    }

    /// 把条目复制到目标文件夹。空目标 id = 根目录。
    pub fn copy_entries(
        mut self: Pin<&mut Self>,
        ids: QStringList,
        target_id: QString,
        target_name: QString,
    ) -> bool {
        self.as_mut()
            .transfer_entries("copy", ids, target_id, target_name)
    }

    /// 移动 / 复制共用的收尾。移动先在源目录移除条目；复制不动源列表。
    fn transfer_entries(
        mut self: Pin<&mut Self>,
        action: &'static str,
        ids: QStringList,
        target_id: QString,
        target_name: QString,
    ) -> bool {
        let ids = clean_ids(ids);
        let Some(action_id) = self
            .as_mut()
            .begin_file_action(&ids, FileActionDomain::Files)
        else {
            self.as_mut()
                .set_message("相关条目正在处理中，请稍后再移动或复制");
            return false;
        };
        if self.as_ref().rust().listing {
            self.as_mut().fail_file_action(
                action_id,
                action,
                Failure::Other("文件列表正在刷新，请稍后再移动或复制".into()),
            );
            return false;
        }
        if ids.is_empty() {
            self.as_mut().fail_file_action(
                action_id,
                action,
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
                action,
                Failure::Other("文件夹正在同步真实 id，请稍后再移动或复制".into()),
            );
            return false;
        }
        let verb = if action == "move" { "移动" } else { "复制" };
        let target = target_id.to_string().trim().to_owned();
        if is_unconfirmed_create(self.as_ref().rust(), &target) {
            self.as_mut().fail_file_action(
                action_id,
                action,
                Failure::Other("目标文件夹正在同步真实 id，请稍后再试".into()),
            );
            return false;
        }
        let target_label = {
            let name = target_name.to_string();
            let name = name.trim();
            if name.is_empty() {
                "全部文件".to_owned()
            } else {
                name.to_owned()
            }
        };
        let current = self
            .crumbs
            .last()
            .map(|crumb| crumb.id.clone())
            .unwrap_or_default();
        if target == current {
            self.as_mut().fail_file_action(
                action_id,
                action,
                Failure::Other("目标就是当前文件夹，请选择其它文件夹".into()),
            );
            return false;
        }
        // 目标目录不可能等于一个文件的 id；命中就说明选中的文件夹被移到了自己里面
        if ids.iter().any(|id| id == &target) {
            self.as_mut().fail_file_action(
                action_id,
                action,
                Failure::Other(format!("不能把文件夹{verb}到它自己里面")),
            );
            return false;
        }
        // 索引里有目标父链时直接挡住「移进自己的子孙」；缺链就放行，
        // 让服务端错误冒出来，不能假装成功。
        if !self.index_building {
            if let Some(index) = self.backend.session.tree_index() {
                if !index.is_closed() {
                    match index.target_inside_sources(&target, &ids) {
                        Ok(Some(true)) => {
                            self.as_mut().fail_file_action(
                                action_id,
                                action,
                                Failure::Other(format!(
                                    "不能把文件夹{verb}到它自己或它的子文件夹里"
                                )),
                            );
                            return false;
                        }
                        Ok(_) => {}
                        Err(err) => self
                            .backend
                            .session
                            .record_tree_index_error(format!("本地索引读取失败：{err}")),
                    }
                }
            }
        }

        let subject = item_subject(&ids, self.as_ref().rust().entry_name(&ids[0]));
        if action == "move" && !self.as_mut().optimistic_remove_entries(action_id, &ids) {
            self.as_mut().fail_file_action(
                action_id,
                action,
                Failure::Other("选中的条目已不在当前列表中".into()),
            );
            return false;
        }
        let session = self.backend.session.clone();
        let target_for_thread = target.clone();
        let target_for_cache = target;
        let target_label_for_message = target_label.clone();
        self.as_mut().set_file_action_message(
            action_id,
            &format!("正在{verb}{subject}到「{target_label}」……"),
        );
        run_in_background(
            move || {
                let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
                let parent = (!target_for_thread.is_empty()).then_some(target_for_thread.as_str());
                if action == "move" {
                    session.move_entries(&refs, parent)?;
                    // 只改被移动条目自身的父目录；目标与源目录都标脏。
                    session.write_index(|index| {
                        index.apply_change(Change::Move {
                            ids: &ids,
                            parent_id: &target_for_thread,
                        })
                    });
                } else {
                    // 复制不产生可确定的本地行（副本 id 要等下次列目录），
                    // 目标目录由 mark_file_dir_stale 标脏。
                    session.copy_entries(&refs, parent)?;
                }
                Ok(())
            },
            move |mut qichi, result| match result {
                Ok(()) => {
                    if action == "move" {
                        qichi.as_mut().commit_file_mutation(action_id);
                    }
                    // 目标目录里新增了条目；下次进入先显示旧快照，再立即后台校准。
                    qichi.as_mut().mark_file_dir_stale(&target_for_cache);
                    qichi.as_mut().end_file_action(
                        action_id,
                        action,
                        &format!("已{verb}{subject}到「{target_label_for_message}」"),
                    );
                }
                Err(err) => {
                    if action == "move" {
                        qichi.as_mut().rollback_file_mutation(action_id);
                    }
                    qichi.as_mut().fail_file_action(action_id, action, err);
                }
            },
        );
        true
    }
}
