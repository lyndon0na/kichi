//! 文件的写操作：改名 / 新建文件夹 / 移动 / 复制 / 移入回收站。
//!
//! 从 `files.rs` 拆出来：那里只管浏览（目录栈、列表、排序），
//! 这里是会改云端数据的入口，出错方式和浏览完全不同。
//!
//! 所有入口共享一套收尾：抢占 `file_action_running` 旗标 →
//! 后台执行 → 主线程提交或回滚乐观快照 → 发 `fileActionDone`。
//! 普通成功不再常驻状态栏；移入回收站例外，保留「可恢复」这一额外后果。

use std::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QString, QStringList};

use super::actions::run_in_background;
use super::file_optimistic::is_unconfirmed_create;
use super::qobject;
use crate::session::Failure;

#[derive(Clone, Copy, PartialEq, Eq)]
enum StarOrigin {
    Files,
    Library,
}

impl qobject::Kichi {
    /// 改名。
    ///
    /// 名字在 Rust 侧再校验一次（QML 的输入框不是唯一入口），
    /// 本地先改名并按当前排序重排；失败恢复操作前快照和搜索命中。
    pub fn rename_entry(mut self: Pin<&mut Self>, id: QString, name: QString) -> bool {
        if !self.as_mut().begin_file_action() {
            return false;
        }
        if self.as_ref().rust().listing {
            self.as_mut().fail_file_action(
                "rename",
                Failure::Other("文件列表正在刷新，请稍后再重命名".into()),
            );
            return false;
        }
        let id = id.to_string();
        let name = name.to_string();
        if id.trim().is_empty() {
            self.as_mut()
                .fail_file_action("rename", Failure::Other("没有选中条目".into()));
            return false;
        }
        if is_unconfirmed_create(self.as_ref().rust(), &id) {
            self.as_mut().fail_file_action(
                "rename",
                Failure::Other("文件夹正在同步真实 id，请稍后再改名".into()),
            );
            return false;
        }
        if !self.as_mut().optimistic_rename(&id, &name) {
            self.as_mut()
                .fail_file_action("rename", Failure::Other("条目已不在当前列表中".into()));
            return false;
        }
        let session = self.backend.session.clone();
        let for_message = name.trim().to_owned();
        self.as_mut()
            .set_message(&format!("正在改名为「{for_message}」……"));
        run_in_background(
            move || session.rename_entry(&id, &name),
            move |mut qichi, result| match result {
                Ok(()) => {
                    qichi.as_mut().commit_file_mutation();
                    qichi
                        .as_mut()
                        .end_file_action("rename", &format!("已改名为「{for_message}」"));
                }
                Err(err) => {
                    qichi.as_mut().rollback_file_mutation();
                    qichi.as_mut().fail_file_action("rename", err);
                }
            },
        );
        true
    }

    /// 在当前目录下新建文件夹。
    pub fn create_folder(mut self: Pin<&mut Self>, name: QString) -> bool {
        if !self.as_mut().begin_file_action() {
            return false;
        }
        if self.as_ref().rust().listing {
            self.as_mut().fail_file_action(
                "folder",
                Failure::Other("文件列表正在刷新，请稍后再新建文件夹".into()),
            );
            return false;
        }
        let name = name.to_string();
        if name.trim().is_empty() {
            self.as_mut()
                .fail_file_action("folder", Failure::Other("文件夹名称不能为空".into()));
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
        if !self.as_mut().optimistic_create_folder(&name) {
            self.as_mut().fail_file_action(
                "folder",
                Failure::Other("无法在当前目录插入新文件夹".into()),
            );
            return false;
        }
        self.as_mut().set_message("正在新建文件夹……");
        run_in_background(
            move || session.create_folder(parent_id.as_deref(), &name),
            move |mut qichi, result| match result {
                Ok(()) => {
                    qichi.as_mut().commit_file_mutation();
                    // 新建响应没有 id，必须校准目录；若操作期间已经导航，只让父目录
                    // 在下次进入时重新读取，不强行刷新用户现在所在的别的目录。
                    qichi.as_mut().mark_file_dir_stale(&parent_for_cache);
                    qichi
                        .as_mut()
                        .end_file_action("folder", &format!("已新建文件夹「{for_message}」"));
                    if qichi.as_ref().rust().current_dir_id.to_string() == parent_for_cache {
                        qichi.as_mut().refresh_files_background();
                    }
                }
                Err(err) => {
                    qichi.as_mut().rollback_file_mutation();
                    qichi.as_mut().fail_file_action("folder", err);
                }
            },
        );
        true
    }

    /// 移入回收站。支持多选批量；请求被接受后立即更新列表，失败再回滚。
    pub fn trash_entries(mut self: Pin<&mut Self>, ids: QStringList) -> bool {
        if !self.as_mut().begin_file_action() {
            return false;
        }
        if self.as_ref().rust().listing {
            self.as_mut().fail_file_action(
                "trash",
                Failure::Other("文件列表正在刷新，请稍后再移入回收站".into()),
            );
            return false;
        }
        let ids = clean_ids(ids);
        if ids.is_empty() {
            self.as_mut()
                .fail_file_action("trash", Failure::Other("没有选中条目".into()));
            return false;
        }
        if ids
            .iter()
            .any(|id| is_unconfirmed_create(self.as_ref().rust(), id))
        {
            self.as_mut().fail_file_action(
                "trash",
                Failure::Other("文件夹正在同步真实 id，请稍后再移入回收站".into()),
            );
            return false;
        }
        let subject = item_subject(&ids, self.as_ref().rust().entry_name(&ids[0]));
        if !self.as_mut().optimistic_remove_entries(&ids) {
            self.as_mut()
                .fail_file_action("trash", Failure::Other("选中的条目已不在当前列表中".into()));
            return false;
        }
        let session = self.backend.session.clone();
        self.as_mut()
            .set_message(&format!("正在把{subject}移入回收站……"));
        run_in_background(
            move || {
                let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
                session.trash_entries(&refs)
            },
            move |mut qichi, result| match result {
                Ok(()) => {
                    qichi.as_mut().commit_file_mutation();
                    qichi.as_mut().mark_trash_stale();
                    qichi.as_mut().end_file_action(
                        "trash",
                        &format!("{subject}已移入回收站（可在回收站页恢复）"),
                    );
                }
                Err(err) => {
                    qichi.as_mut().rollback_file_mutation();
                    qichi.as_mut().fail_file_action("trash", err);
                }
            },
        );
        true
    }

    /// 文件页（含搜索结果）批量加星标 / 取消星标。
    pub fn set_starred(mut self: Pin<&mut Self>, ids: QStringList, starred: bool) {
        let ids = clean_ids(ids);
        let first_name = ids
            .first()
            .and_then(|id| self.as_ref().rust().entry_name(id));
        self.as_mut()
            .change_starred(ids, first_name, starred, StarOrigin::Files);
    }

    /// 资料库页单条加星标 / 取消星标。
    pub fn set_library_starred(mut self: Pin<&mut Self>, id: QString, starred: bool) {
        let id = id.to_string().trim().to_owned();
        let name = self
            .library_entries
            .iter()
            .find(|entry| entry.entry.id == id)
            .map(|entry| entry.entry.name.clone());
        self.as_mut()
            .change_starred(vec![id], name, starred, StarOrigin::Library);
    }

    /// 星标写请求共用入口：先在当前页面生效，失败再恢复。
    fn change_starred(
        mut self: Pin<&mut Self>,
        ids: Vec<String>,
        first_name: Option<String>,
        starred: bool,
        origin: StarOrigin,
    ) {
        let action = star_action(starred);
        if !self.as_mut().begin_file_action() {
            return;
        }
        let source_loading = match origin {
            StarOrigin::Files => self.as_ref().rust().listing,
            StarOrigin::Library => self.as_ref().rust().library_loading,
        };
        if source_loading {
            let message = match origin {
                StarOrigin::Files => "文件列表正在刷新，请稍后再设置星标",
                StarOrigin::Library => "资料库正在刷新，请稍后再设置星标",
            };
            self.as_mut()
                .fail_file_action(action, Failure::Other(message.into()));
            return;
        }
        let ids: Vec<String> = ids
            .into_iter()
            .map(|id| id.trim().to_owned())
            .filter(|id| !id.is_empty())
            .collect();
        if ids.is_empty() {
            self.as_mut()
                .fail_file_action(action, Failure::Other("没有选中条目".into()));
            return;
        }
        if ids
            .iter()
            .any(|id| is_unconfirmed_create(self.as_ref().rust(), id))
        {
            self.as_mut().fail_file_action(
                action,
                Failure::Other("文件夹正在同步真实 id，请稍后再设置星标".into()),
            );
            return;
        }

        let subject = item_subject(&ids, first_name);
        let progress = if starred {
            format!("正在为{subject}加星标……")
        } else {
            format!("正在取消{subject}的星标……")
        };
        let success = if starred {
            format!("已为{subject}加星标")
        } else {
            format!("已取消{subject}的星标")
        };
        let session = self.backend.session.clone();
        let request_ids = ids.clone();
        let optimistic_before = if origin == StarOrigin::Files {
            let Some(before) = self.as_mut().optimistic_starred(&ids, starred) else {
                self.as_mut()
                    .fail_file_action(action, Failure::Other("条目已不在当前列表中".into()));
                return;
            };
            Some(before)
        } else {
            None
        };
        let library_before = if origin == StarOrigin::Library {
            let Some(snapshot) = self.as_mut().optimistic_library_starred(&ids, starred) else {
                self.as_mut()
                    .fail_file_action(action, Failure::Other("条目已不在当前资料库中".into()));
                return;
            };
            Some(snapshot)
        } else {
            None
        };
        let library_kind = library_before.as_ref().map(|snapshot| snapshot.kind);
        self.as_mut().set_message(&progress);
        run_in_background(
            move || {
                let refs: Vec<&str> = request_ids.iter().map(String::as_str).collect();
                session.set_starred(&refs, starred)
            },
            move |mut qichi, result| match result {
                Ok(()) => {
                    match origin {
                        StarOrigin::Files => {
                            qichi.as_mut().mark_library_caches_stale(None);
                        }
                        StarOrigin::Library => {
                            qichi.as_mut().mark_library_caches_stale(library_kind);
                            // 文件页缓存里可能也有同一条目；下次进入时立即校准星标状态。
                            qichi.as_mut().invalidate_file_caches();
                        }
                    }
                    qichi.as_mut().end_file_action(action, &success);
                }
                Err(err) => {
                    match origin {
                        StarOrigin::Files => {
                            if let Some(snapshot) = optimistic_before {
                                qichi.as_mut().restore_starred(snapshot);
                            }
                        }
                        StarOrigin::Library => {
                            if let Some(snapshot) = library_before {
                                qichi.as_mut().rollback_library_starred(snapshot);
                            }
                        }
                    }
                    qichi.as_mut().fail_file_action(action, err);
                }
            },
        );
    }

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
        if !self.as_mut().begin_file_action() {
            return false;
        }
        if self.as_ref().rust().listing {
            self.as_mut().fail_file_action(
                action,
                Failure::Other("文件列表正在刷新，请稍后再移动或复制".into()),
            );
            return false;
        }
        let ids = clean_ids(ids);
        if ids.is_empty() {
            self.as_mut()
                .fail_file_action(action, Failure::Other("没有选中条目".into()));
            return false;
        }
        if ids
            .iter()
            .any(|id| is_unconfirmed_create(self.as_ref().rust(), id))
        {
            self.as_mut().fail_file_action(
                action,
                Failure::Other("文件夹正在同步真实 id，请稍后再移动或复制".into()),
            );
            return false;
        }
        let verb = if action == "move" { "移动" } else { "复制" };
        let target = target_id.to_string().trim().to_owned();
        if is_unconfirmed_create(self.as_ref().rust(), &target) {
            self.as_mut().fail_file_action(
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
                action,
                Failure::Other("目标就是当前文件夹，请选择其它文件夹".into()),
            );
            return false;
        }
        // 目标目录不可能等于一个文件的 id；命中就说明选中的文件夹被移到了自己里面
        if ids.iter().any(|id| id == &target) {
            self.as_mut().fail_file_action(
                action,
                Failure::Other(format!("不能把文件夹{verb}到它自己里面")),
            );
            return false;
        }

        let subject = item_subject(&ids, self.as_ref().rust().entry_name(&ids[0]));
        if action == "move" && !self.as_mut().optimistic_remove_entries(&ids) {
            self.as_mut()
                .fail_file_action(action, Failure::Other("选中的条目已不在当前列表中".into()));
            return false;
        }
        let session = self.backend.session.clone();
        let target_for_thread = target.clone();
        let target_for_cache = target;
        let target_label_for_message = target_label.clone();
        self.as_mut()
            .set_message(&format!("正在{verb}{subject}到「{target_label}」……"));
        run_in_background(
            move || {
                let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
                let parent = (!target_for_thread.is_empty()).then_some(target_for_thread.as_str());
                if action == "move" {
                    session.move_entries(&refs, parent)
                } else {
                    session.copy_entries(&refs, parent)
                }
            },
            move |mut qichi, result| match result {
                Ok(()) => {
                    if action == "move" {
                        qichi.as_mut().commit_file_mutation();
                    }
                    // 目标目录里新增了条目；下次进入先显示旧快照，再立即后台校准。
                    qichi.as_mut().mark_file_dir_stale(&target_for_cache);
                    qichi.as_mut().end_file_action(
                        action,
                        &format!("已{verb}{subject}到「{target_label_for_message}」"),
                    );
                }
                Err(err) => {
                    if action == "move" {
                        qichi.as_mut().rollback_file_mutation();
                    }
                    qichi.as_mut().fail_file_action(action, err);
                }
            },
        );
        true
    }

    /// 抢占「文件操作进行中」标志；已在操作中返回 false（防止连点重复提交）。
    pub(crate) fn begin_file_action(mut self: Pin<&mut Self>) -> bool {
        if self.as_ref().rust().file_action_running {
            return false;
        }
        self.as_mut().rust_mut().file_action_running = true;
        true
    }

    /// 结束一次文件操作。
    fn end_file_action(mut self: Pin<&mut Self>, action: &str, message: &str) {
        {
            let mut rust = self.as_mut().rust_mut();
            rust.file_action_running = false;
        }
        if action == "trash" {
            self.as_mut().set_message(message);
        } else {
            self.as_mut().clear_status_message();
        }
        self.as_mut()
            .file_action_done(QString::from(action), true, QString::from(message));
        self.as_mut().resume_pending_file_refresh();
    }

    /// 文件操作失败：清旗标 + 走统一的错误出口（分类提示不吞）。
    fn fail_file_action(mut self: Pin<&mut Self>, action: &str, failure: crate::session::Failure) {
        self.as_mut().rust_mut().file_action_running = false;
        let text = failure.to_string();
        self.as_mut().on_failure(failure);
        self.as_mut()
            .file_action_done(QString::from(action), false, QString::from(text.as_str()));
        self.as_mut().resume_pending_file_refresh();
    }

    /// 操作期间用户若已请求刷新 / 导航，等写操作收尾后读取用户当前所在目录。
    fn resume_pending_file_refresh(mut self: Pin<&mut Self>) {
        self.as_mut().resume_pending_files_request();
    }
}

fn star_action(starred: bool) -> &'static str {
    if starred {
        "star"
    } else {
        "unstar"
    }
}

/// QML 传来的 id 列表：去空白、去空项（去重与上限由 core 的 `normalize_ids` 负责）。
fn clean_ids(ids: QStringList) -> Vec<String> {
    ids.iter()
        .map(|id| id.to_string().trim().to_owned())
        .filter(|id| !id.is_empty())
        .collect()
}

/// 状态栏 / 对话框里的主语：单项用名字，多项用计数（不假装知道每个名字）。
fn item_subject(ids: &[String], first_name: Option<String>) -> String {
    if ids.len() == 1 {
        format!("「{}」", first_name.unwrap_or_else(|| "该条目".to_owned()))
    } else {
        format!("选中的 {} 项", ids.len())
    }
}
