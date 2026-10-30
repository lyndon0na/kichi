//! 文件的写操作：改名 / 新建文件夹 / 移动 / 复制 / 移入回收站。
//!
//! 从 `files.rs` 拆出来：那里只管浏览（目录栈、列表、排序），
//! 这里是会改云端数据的入口，出错方式和浏览完全不同。
//!
//! 所有入口共享一套收尾：抢占 `file_action_running` 旗标 →
//! 后台执行 → 主线程发 `fileActionDone` → 成功就整目录刷新。
//! 普通成功不再常驻状态栏；移入回收站例外，保留「可恢复」这一额外后果。

use std::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QString, QStringList};

use super::actions::run_in_background;
use super::qobject;
use crate::session::Failure;

impl qobject::Kichi {
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
            .set_message(&format!("正在改名为「{for_message}」……"));
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
        self.as_mut().set_message("正在新建文件夹……");
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

    /// 移入回收站。支持多选批量；失败原因留在确认框里。
    pub fn trash_entries(mut self: Pin<&mut Self>, ids: QStringList) {
        if !self.as_mut().begin_file_action() {
            return;
        }
        let ids = clean_ids(ids);
        if ids.is_empty() {
            self.as_mut()
                .fail_file_action("trash", Failure::Other("没有选中条目".into()));
            return;
        }
        let subject = item_subject(&ids, self.as_ref().rust().entry_name(&ids[0]));
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
                    qichi.as_mut().end_file_action(
                        "trash",
                        &format!("{subject}已移入回收站（可在回收站页恢复）"),
                    );
                    qichi.as_mut().mark_trash_stale();
                    qichi.as_mut().refresh_files();
                }
                Err(err) => qichi.as_mut().fail_file_action("trash", err),
            },
        );
    }

    /// 把条目移动到目标文件夹。空目标 id = 根目录。
    pub fn move_entries(
        mut self: Pin<&mut Self>,
        ids: QStringList,
        target_id: QString,
        target_name: QString,
    ) {
        self.as_mut()
            .transfer_entries("move", ids, target_id, target_name);
    }

    /// 把条目复制到目标文件夹。空目标 id = 根目录。
    pub fn copy_entries(
        mut self: Pin<&mut Self>,
        ids: QStringList,
        target_id: QString,
        target_name: QString,
    ) {
        self.as_mut()
            .transfer_entries("copy", ids, target_id, target_name);
    }

    /// 移动 / 复制共用的收尾。移动成功后刷新源目录；复制不动源目录，不刷新
    /// （避免把列表滚动位置无谓地冲回顶部）。
    fn transfer_entries(
        mut self: Pin<&mut Self>,
        action: &'static str,
        ids: QStringList,
        target_id: QString,
        target_name: QString,
    ) {
        if !self.as_mut().begin_file_action() {
            return;
        }
        let ids = clean_ids(ids);
        if ids.is_empty() {
            self.as_mut()
                .fail_file_action(action, Failure::Other("没有选中条目".into()));
            return;
        }
        let verb = if action == "move" { "移动" } else { "复制" };
        let target = target_id.to_string().trim().to_owned();
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
            return;
        }
        // 目标目录不可能等于一个文件的 id；命中就说明选中的文件夹被移到了自己里面
        if ids.iter().any(|id| id == &target) {
            self.as_mut().fail_file_action(
                action,
                Failure::Other(format!("不能把文件夹{verb}到它自己里面")),
            );
            return;
        }

        let subject = item_subject(&ids, self.as_ref().rust().entry_name(&ids[0]));
        let session = self.backend.session.clone();
        let source_dir = current;
        let target_for_thread = target.clone();
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
                    qichi.as_mut().end_file_action(
                        action,
                        &format!("已{verb}{subject}到「{target_label_for_message}」"),
                    );
                    if action == "move" {
                        // 用户已经进别的目录就别把他拽回加载态；回到源目录时会重新拉取
                        let still_there = qichi
                            .as_ref()
                            .rust()
                            .crumbs
                            .last()
                            .map(|crumb| crumb.id.clone())
                            .unwrap_or_default()
                            == source_dir;
                        if still_there {
                            qichi.as_mut().refresh_files();
                        }
                    }
                }
                Err(err) => qichi.as_mut().fail_file_action(action, err),
            },
        );
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
        self.as_mut().rust_mut().file_action_running = false;
        if action == "trash" {
            self.as_mut().set_message(message);
        } else {
            self.as_mut().clear_status_message();
        }
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
