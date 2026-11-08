//! 播放加载态与收尾：取消、错误提示、播放会话登记与登出清理。

use std::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QString, QStringList};

use super::qobject;
use super::QVariantList;
use crate::session::subtitles::SubtitlePlan;

impl qobject::Kichi {
    /// 提示条上的「取消」：中止取链、关掉还没出窗口的 mpv，静默收尾。
    pub fn cancel_play_loading(mut self: Pin<&mut Self>) {
        // 取消这次播放时，后台列目录 / 片尾预取也不再需要。
        self.as_mut().cancel_queue_work();
        self.as_mut().restore_queue_for_live_session();
        self.as_mut().cancel_pending_play();
        {
            let mut rust = self.as_mut().rust_mut();
            // 片尾自动切集被取消：旧画面已经结束，空窗口一起收起。
            if rust.play_queue_state.hide_on_failure {
                rust.embed_active = false;
                rust.embed_title = QString::default();
                rust.embed_pending_epoch = None;
            }
            rust.play_loading = false;
            rust.play_loading_id = QString::default();
            rust.play_loading_name = QString::default();
            rust.play_error = QString::default();
            rust.play_notice = QString::default();
            // 不回写 status_message：底栏内容不归播放管
        }
        self.as_mut().finish_play();
    }

    /// 作废「还在加载」的那次播放（取链 + 等窗口），不动状态属性。
    ///
    /// 取链线程靠 `Cancel` 提前返回；等窗口的 mpv 靠监控线程收到取消后 `quit`。
    pub(crate) fn cancel_pending_play(mut self: Pin<&mut Self>) {
        let mut rust = self.as_mut().rust_mut();
        if let Some(cancel) = rust.play_cancel.take() {
            cancel.cancel();
        }
        rust.play_epoch = rust.play_epoch.wrapping_add(1);
    }

    /// 提示条上的「关闭」（错误态）：只清掉错误，不动别的。
    pub fn dismiss_play_error(mut self: Pin<&mut Self>) {
        {
            let mut rust = self.as_mut().rust_mut();
            rust.play_error = QString::default();
        }
        self.as_mut().notify();
    }

    /// 提示条自己收起回退提示时调用：清掉文案，下次回退才能再弹一次。
    pub fn dismiss_play_notice(mut self: Pin<&mut Self>) {
        {
            let mut rust = self.as_mut().rust_mut();
            rust.play_notice = QString::default();
        }
        self.as_mut().notify();
    }

    /// 只收起加载态（错误 / 取消的分支共用），不动底栏。
    pub(crate) fn hide_play_loading(mut self: Pin<&mut Self>) {
        {
            let mut rust = self.as_mut().rust_mut();
            rust.play_loading = false;
            rust.play_loading_id = QString::default();
            rust.play_loading_name = QString::default();
            rust.play_cancel = None;
        }
        self.as_mut().notify();
    }

    /// 加载失败：收起加载态，错误进提示条，不碰底栏。
    pub(crate) fn fail_play_loading(mut self: Pin<&mut Self>, text: &str) {
        self.as_mut().restore_queue_for_live_session();
        {
            let mut rust = self.as_mut().rust_mut();
            // 片尾自动切集失败时，旧画面已经结束：把空窗口收起来。
            if rust.play_queue_state.hide_on_failure {
                rust.embed_active = false;
                rust.embed_title = QString::default();
                rust.embed_pending_epoch = None;
            }
            rust.play_loading = false;
            rust.play_loading_id = QString::default();
            rust.play_loading_name = QString::default();
            rust.play_cancel = None;
            rust.play_notice = QString::default();
            rust.play_error = QString::from(text);
        }
        self.as_mut().finish_play();
    }

    /// 用户关掉菜单：丢掉文件与状态，什么都不请求。
    ///
    /// 选中某一项后菜单会自动关闭，那次关闭**不能**取消刚发起的请求
    /// （`play_pending` 为真时直接忽略）。加载中的播放由提示条自己的
    /// [`cancel_play_loading`](Self::cancel_play_loading) 取消，这里不碰它。
    pub fn cancel_play(mut self: Pin<&mut Self>) {
        if self.play_pending {
            return;
        }
        if self.play_menu.is_empty() && self.play_title.is_empty() {
            return;
        }
        self.as_mut().clear_play_menu();
    }

    /// 本轮播放结束（成功或其他分支都算）：清掉目标，允许下一次播放。
    pub(crate) fn finish_play(mut self: Pin<&mut Self>) {
        {
            let mut rust = self.as_mut().rust_mut();
            rust.play_pending = false;
            rust.play_title = QString::default();
            rust.play_id.clear();
            rust.play_menu = QVariantList::default();
            rust.play_subtitle_plan = SubtitlePlan::None;
            rust.play_queue_state.advancing = false;
            rust.play_queue_state.hide_on_failure = false;
            rust.queue_advancing = false;
        }
        self.as_mut().notify();
    }

    /// 更新某次播放的活跃状态，并重算 QML 用的去重 id 列表。
    ///
    /// 同一文件可以被打开不止一次，所以 QML 列表按 id 去重、内部表按请求代数
    /// 分开记；只有最后一条同 id 会话退出时才不再显示「播放中」。
    pub(crate) fn update_active_play_session(
        mut self: Pin<&mut Self>,
        epoch: u64,
        id: &str,
        active: bool,
    ) -> bool {
        let mut rust = self.as_mut().rust_mut();
        let changed = if active {
            rust.active_play_sessions
                .insert(epoch, id.to_owned())
                .is_none_or(|previous| previous != id)
        } else {
            rust.active_play_sessions.remove(&epoch).is_some()
        };
        if !changed {
            return false;
        }
        let unique = rust
            .active_play_sessions
            .values()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        let mut ids = QStringList::default();
        for file_id in unique {
            ids.append(QString::from(file_id.as_str()));
        }
        rust.playing_ids = ids;
        true
    }

    /// 收起菜单并丢掉状态。
    fn clear_play_menu(mut self: Pin<&mut Self>) {
        {
            let mut rust = self.as_mut().rust_mut();
            rust.play_menu = QVariantList::default();
            rust.play_title = QString::default();
            rust.play_id.clear();
            rust.play_pending = false;
            rust.play_subtitle_plan = SubtitlePlan::None;
        }
        self.as_mut().notify();
    }

    /// 退出登录时清干净（连同在途请求作废）。外部播放器不动；
    /// 内嵌画面在窗口里，登出后必须一起收起。
    pub(crate) fn reset_play(mut self: Pin<&mut Self>) {
        crate::player::embed::stop();
        {
            let mut rust = self.as_mut().rust_mut();
            if let Some(cancel) = rust.play_cancel.take() {
                cancel.cancel();
            }
            let reset_epoch = rust.play_epoch.wrapping_add(1);
            rust.play_epoch = reset_epoch;
            rust.play_progress.reset(reset_epoch);
            rust.play_menu = QVariantList::default();
            rust.play_title = QString::default();
            rust.play_id.clear();
            rust.play_pending = false;
            rust.play_loading = false;
            rust.play_loading_id = QString::default();
            rust.play_loading_name = QString::default();
            rust.active_play_sessions.clear();
            rust.playing_ids = QStringList::default();
            rust.play_error = QString::default();
            rust.play_notice = QString::default();
            rust.play_subtitle_plan = SubtitlePlan::None;
            rust.embed_active = false;
            rust.embed_title = QString::default();
            rust.embed_pending_epoch = None;
            rust.embed_live_epoch = None;
        }
        self.as_mut().reset_play_queue();
    }
}
