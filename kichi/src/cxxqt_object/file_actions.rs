//! 文件的写操作：改名 / 新建文件夹 / 移入回收站。
//!
//! 从 `files.rs` 拆出来：那里只管浏览（目录栈、列表、排序），
//! 这里是会改云端数据的入口，出错方式和浏览完全不同。
//!
//! 三个入口共享一套收尾：抢占 `file_action_running` 旗标 →
//! 后台执行 → 主线程写状态栏并发 `fileActionDone` → 成功就整目录刷新。

use std::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt_lib::QString;

use super::actions::run_in_background;
use super::qobject;

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
        let ids = [id];
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
