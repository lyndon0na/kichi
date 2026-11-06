//! 桥接对象的控制入口（Q_INVOKABLE）与后台任务调度。
//!
//! cxx-qt 0.10 写法要点见 `../cxxqt_object.rs` 顶部注释。
//! 声明端 `Pin<&mut Self>` 与实现端一致时，用 `mut self: Pin<&mut Self>`。

use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::{Mutex, OnceLock};

use cxx_qt::CxxQtType;
use cxx_qt_lib::QString;

use super::qobject;
use crate::session::tree_index::sync::ProbeOutcome;
use crate::session::{Failure, State};

/// 主线程执行的回调（在 `pump` 里跑）。
type UiTask = Box<dyn FnOnce(Pin<&mut qobject::Kichi>) + Send>;

impl qobject::Kichi {
    /// 登录。人机验证分支下，`captcha_token` 是用户从验证页面拿到的令牌。
    pub fn login(
        mut self: Pin<&mut Self>,
        username: QString,
        password: QString,
        captcha_token: QString,
    ) {
        if self.login_running {
            return;
        }
        let user = username.to_string().trim().to_owned();
        let password = password.to_string();
        if user.is_empty() || password.is_empty() {
            self.as_mut().set_message("请输入账号和密码");
            return;
        }
        let captcha = captcha_token.to_string();
        let captcha = (!captcha.trim().is_empty()).then(|| captcha.trim().to_owned());

        let session = self.backend.session.clone();
        self.as_mut().rust_mut().login_running = true;
        self.as_mut().set_message("正在登录……");
        run_in_background(
            move || session.sign_in(&user, &password, captcha.as_deref()),
            |mut qichi, result| match result {
                Ok(()) => qichi.as_mut().on_logged_in(),
                Err(err) => {
                    qichi.as_mut().rust_mut().login_running = false;
                    qichi.as_mut().on_failure(err);
                }
            },
        );
    }

    /// 打开人机验证页面（地址只存在于 Rust 侧）。
    pub fn open_captcha_page(&self) {
        if self.captcha_url.is_empty() {
            return;
        }
        super::open_in_browser(&self.captcha_url);
    }

    /// 启动恢复后的补全（QML 界面就绪时调用一次）。
    pub fn warm_up(mut self: Pin<&mut Self>) {
        let restored = self.auth_state.to_string() == "signedIn" && self.session_restoring;
        if !self.auto_resume_checked {
            self.as_mut().rust_mut().auto_resume_checked = true;
            if restored {
                self.as_mut().auto_resume_restored();
            } else {
                // 启动时没有恢复登录态（无令牌 / 已过期）：手动登录不该误续旧任务。
                super::transfers::clear_auto_resume();
                super::downloads::flush_history();
                self.as_mut().sync_transfers();
            }
        }
        if restored {
            self.as_mut().rust_mut().session_restoring = false;
            // 恢复成功由已登录界面本身表达，底栏不常驻启动提示。
            self.as_mut().set_message("");
            self.as_mut().refresh_identity_and_quota();
            // 先打开索引，首屏才能在网络返回前用本地旧行占位
            self.as_mut().ensure_tree_index_build();
            self.as_mut().refresh_files_background();
            self.as_mut().start_task_polling();
        }
    }

    pub fn sign_out(mut self: Pin<&mut Self>) {
        match self.backend.session.sign_out() {
            Ok(()) => {
                {
                    let mut rust = self.as_mut().rust_mut();
                    rust.auth_state = QString::from("signedOut");
                    rust.captcha_required = false;
                    rust.captcha_url.clear();
                    rust.status_message = QString::default();
                    rust.login_running = false;
                    rust.session_restoring = false;
                    rust.account = QString::default();
                    rust.display_name = QString::default();
                    rust.avatar_path = QString::default();
                    rust.quota_used_text = QString::default();
                    rust.quota_limit_text = QString::default();
                    rust.quota_loading = false;
                    rust.quota_ratio = -1.0;
                    rust.quota_vip = false;
                    rust.quota_transfer_cloud = QString::default();
                    rust.quota_transfer_download = QString::default();
                    rust.quota_transfer_upload = QString::default();
                    rust.quota_transfer_reset = QString::default();
                    rust.quota_transfer_cloud_ratio = -1.0;
                    rust.quota_transfer_download_ratio = -1.0;
                    rust.quota_transfer_upload_ratio = -1.0;
                    if let Some(cancel) = rust.index_build_cancel.take() {
                        cancel.cancel();
                    }
                    rust.index_building = false;
                    rust.index_build_epoch = rust.index_build_epoch.wrapping_add(1);
                    if let Some(cancel) = rust.index_probe_cancel.take() {
                        cancel.cancel();
                    }
                    rust.index_probing = false;
                    rust.index_probe_epoch = rust.index_probe_epoch.wrapping_add(1);
                    rust.index_error_reported = false;
                }
                self.as_mut().reset_files();
                self.as_mut().reset_thumbnails();
                self.as_mut().reset_trash();
                self.as_mut().reset_play();
                self.as_mut().reset_uploads();
                self.as_mut().reset_downloads();
                self.as_mut().reset_search();
                self.as_mut().reset_shares();
                self.as_mut().reset_library();
                self.as_mut().reset_magnet();
                self.as_mut().stop_task_polling();
            }
            Err(err) => self.as_mut().set_message(&format!("退出登录失败：{err}")),
        }
        // 退出后要立刻切回登录页
        self.as_mut().notify();
    }

    /// 主线程抽水：执行后台线程投递过来的结果（QML 定时器驱动）。
    pub fn pump(mut self: Pin<&mut Self>) {
        let mut handled = false;
        // 托盘事件（显示/退出）也在主线程消费
        for event in super::tray::poll() {
            self.as_mut().handle_tray_event(event);
            handled = true;
        }
        // 再次启动 kichi：第二个进程通过实例 socket 请求唤回窗口，
        // 与托盘「显示」走同一条路径（QML 的 trayShowRequested 监听）。
        for _ in 0..crate::instance::take_activations() {
            self.as_mut()
                .handle_tray_event(super::tray::TrayEvent::Show);
        }
        loop {
            let task = {
                let mut queue = tasks().lock().unwrap_or_else(|e| e.into_inner());
                queue.pop_front()
            };
            let Some(task) = task else { break };
            task(self.as_mut());
            handled = true;
        }
        // 任务执行期间可能改了多个字段，这里统一通知一次；
        // 没有任务时不发信号，避免定时器空转刷爆绑定。
        if handled {
            self.as_mut().notify();
        }
        // 卡住时下载线程没有进度回调，秒数只能由主线程按 1Hz 自己走。
        // 只改 transferLive，不碰列表模型。
        self.as_mut().maybe_heartbeat_live();
        // 后台结构变化先置脏，到窗口后在这里补一次尾随刷新。
        if super::transfers_sync::due() {
            self.as_mut().sync_transfers();
        }
    }
}

impl qobject::Kichi {
    /// 退出前注销托盘（QML 在 Qt.quit() 之前调用）。
    ///
    /// 真退出与托盘点「退出」两条路径都会调用它（QML 的 `onClosing` 与
    /// `quitRequested`），所以传输历史的强制写盘也挂在这里：
    /// 进程直接结束前把记录表落成最后一次（ADR 0014）。
    pub fn shutdown_tray(&self) {
        super::transfers::mark_auto_resume();
        if self.settings.auto_resume_uploads {
            super::transfers::mark_auto_resume_uploads();
        }
        super::upload_queue::reset();
        super::transfers::interrupt_active_for(super::transfers::TransferDirection::Upload);
        super::transfers::interrupt_paused_for(super::transfers::TransferDirection::Upload);
        super::downloads::flush_history();
        super::tray::stop();
    }

    /// 托盘菜单/图标点击 → 主线程动作。
    ///
    /// 「显示」只置一个标志，真正的 show/raise 由 QML 监听 `trayShowRequested`
    /// 执行：窗口操作必须在 QML 侧做，Rust 侧拿不到 ApplicationWindow。
    fn handle_tray_event(mut self: Pin<&mut Self>, event: super::tray::TrayEvent) {
        match event {
            super::tray::TrayEvent::Show => {
                self.as_mut().rust_mut().tray_show_requested += 1;
                self.as_mut().notify();
            }
            super::tray::TrayEvent::Quit => {
                self.as_mut().rust_mut().quit_requested = true;
                self.as_mut().notify();
            }
        }
    }

    /// 通知 QML：所有属性都挂在这个信号上。
    pub(crate) fn notify(mut self: Pin<&mut Self>) {
        self.as_mut().changed();
    }

    /// 统一更新状态类属性。
    pub(crate) fn set_message(mut self: Pin<&mut Self>, text: &str) {
        self.as_mut().rust_mut().status_message = QString::from(text);
        self.as_mut().notify();
    }

    /// 清掉临时提醒；底栏随后自动回到当前页面 / 目录上下文。
    pub fn clear_status_message(mut self: Pin<&mut Self>) {
        self.as_mut().set_message("");
    }

    fn set_captcha(mut self: Pin<&mut Self>, url: Option<&str>) {
        {
            let mut rust = self.as_mut().rust_mut();
            rust.captcha_required = true;
            rust.captcha_url = url.unwrap_or_default().to_owned();
            rust.login_running = false;
        }
        self.as_mut().notify();
    }

    /// 登录成功后的统一收尾：更新状态、查询身份与容量。
    fn on_logged_in(mut self: Pin<&mut Self>) {
        {
            let mut rust = self.as_mut().rust_mut();
            rust.auth_state = QString::from("signedIn");
            rust.captcha_required = false;
            rust.captcha_url.clear();
            rust.status_message = QString::default();
            rust.login_running = false;
            rust.session_restoring = false;
            rust.account = QString::default();
            rust.display_name = QString::default();
            rust.avatar_path = QString::default();
            rust.quota_used_text = QString::default();
            rust.quota_limit_text = QString::default();
            rust.quota_loading = false;
            rust.quota_ratio = -1.0;
            rust.quota_vip = false;
            rust.quota_transfer_cloud = QString::default();
            rust.quota_transfer_download = QString::default();
            rust.quota_transfer_upload = QString::default();
            rust.quota_transfer_reset = QString::default();
            rust.quota_transfer_cloud_ratio = -1.0;
            rust.quota_transfer_download_ratio = -1.0;
            rust.quota_transfer_upload_ratio = -1.0;
            // 上一个账号的建库若还在跑：取消它，代数推进后它的结果也不会回来
            if let Some(cancel) = rust.index_build_cancel.take() {
                cancel.cancel();
            }
            rust.index_building = false;
            rust.index_build_epoch = rust.index_build_epoch.wrapping_add(1);
            // 上一个账号的探针结果绝不能落到新库上
            if let Some(cancel) = rust.index_probe_cancel.take() {
                cancel.cancel();
            }
            rust.index_probing = false;
            rust.index_probe_epoch = rust.index_probe_epoch.wrapping_add(1);
            rust.index_error_reported = false;
        }
        self.as_mut().notify();
        // 登录成功可能来自另一个账号（尤其是令牌失效后重新登录）。作废旧上传队列，
        // 同时推进 epoch，让仍开着的 Portal 选择器回调不能用旧目录 id 入队。
        self.as_mut().reset_uploads();
        self.as_mut().reset_magnet();
        self.as_mut().sync_identity_from_session();
        self.as_mut().refresh_identity_and_quota();
        // 先打开索引，首屏才能在网络返回前用本地旧行占位
        self.as_mut().ensure_tree_index_build();
        self.as_mut().refresh_files_background();
        self.as_mut().start_task_polling();
    }

    /// 把会话里积压的索引错误提示一次（本次登录周期不再重复）。
    pub(crate) fn report_index_error(mut self: Pin<&mut Self>) {
        if self.index_error_reported {
            return;
        }
        let Some(message) = self.backend.session.take_tree_index_error() else {
            return;
        };
        self.as_mut().rust_mut().index_error_reported = true;
        self.as_mut().set_message(&message);
    }

    /// 登录 / 启动恢复后调用：库为空或版本不认识时后台建一次全量索引；
    /// 已建好则顺手做一次 canary 探针。
    ///
    /// 建库失败只在底栏提示一次，不影响目录浏览（索引只是缓存，网络路径照常）。
    pub(crate) fn ensure_tree_index_build(mut self: Pin<&mut Self>) {
        if self.index_building {
            return;
        }
        self.as_mut().report_index_error();
        let session = self.backend.session.clone();
        let index = match session.ensure_tree_index_open() {
            Ok(index) => index,
            Err(_) => {
                self.as_mut().report_index_error();
                return;
            }
        };
        match index.needs_full_build() {
            Ok(true) => {}
            Ok(false) => {
                // 索引已建好：登录 / 启动恢复顺手对一次账（探针自带冷却与单飞）
                self.as_mut().probe_tree_index();
                return;
            }
            Err(err) => {
                session.record_tree_index_error(format!("本地索引读取失败：{err}"));
                self.as_mut().report_index_error();
                return;
            }
        }
        self.as_mut().start_tree_index_build(session, index);
    }

    /// 起一次全量建库；调用方已确认需要建库（首次 / 探针判为 suspect）。
    fn start_tree_index_build(
        mut self: Pin<&mut Self>,
        session: std::sync::Arc<crate::session::Session>,
        index: std::sync::Arc<crate::session::tree_index::TreeIndex>,
    ) {
        if self.index_building {
            return;
        }
        let cancel = std::sync::Arc::new(crate::session::download::Cancel::default());
        let epoch = {
            let mut rust = self.as_mut().rust_mut();
            rust.index_build_epoch = rust.index_build_epoch.wrapping_add(1);
            rust.index_building = true;
            rust.index_build_cancel = Some(std::sync::Arc::clone(&cancel));
            rust.index_build_epoch
        };
        self.as_mut().set_message("正在建立本地索引……");
        let work_session = session.clone();
        run_in_background(
            move || {
                let client = work_session.drive()?;
                let mut last_reported = 0usize;
                crate::session::tree_index::sync::full_build(
                    &index,
                    &client,
                    &cancel,
                    &mut |scanned| {
                        if scanned == last_reported {
                            return;
                        }
                        last_reported = scanned;
                        // 全盘列举不给总数，进度只能报已扫描条数。
                        enqueue_ui(move |mut qichi| {
                            if qichi.as_ref().rust().index_build_epoch != epoch {
                                return;
                            }
                            qichi
                                .as_mut()
                                .set_message(&format!("正在建立本地索引（{scanned}/?）……"));
                        });
                    },
                )
            },
            move |mut qichi, result| {
                if qichi.as_ref().rust().index_build_epoch != epoch {
                    return;
                }
                {
                    let mut rust = qichi.as_mut().rust_mut();
                    rust.index_building = false;
                    rust.index_build_cancel = None;
                }
                match result {
                    Ok(crate::session::tree_index::sync::BuildOutcome::Built(count)) => {
                        qichi
                            .as_mut()
                            .set_message(&format!("本地索引已建立（{count} 条）"));
                    }
                    Ok(crate::session::tree_index::sync::BuildOutcome::Cancelled) => {
                        // 登出取消：界面已经在登出流程里，不用再留提示
                        qichi.as_mut().set_message("");
                    }
                    Err(err) => {
                        qichi
                            .as_mut()
                            .set_message(&format!("本地索引建立失败：{err}"));
                    }
                }
                qichi.as_mut().report_index_error();
            },
        );
    }

    /// 窗口重新获得焦点：触发一次 canary 探针（冷却 / 单飞都在探针入口挡）。
    pub fn on_window_activated(mut self: Pin<&mut Self>) {
        if self.auth_state.to_string() != "signedIn" {
            return;
        }
        self.as_mut().probe_tree_index();
    }

    /// canary 探针入口：60 秒冷却 + 单飞，建库期间绝不读索引。
    ///
    /// 网络 / 接口失败只记录一次尝试（让冷却照样生效），不动节点、不触发重建。
    pub(crate) fn probe_tree_index(mut self: Pin<&mut Self>) {
        if self.index_building || self.index_probing {
            return;
        }
        if self.auth_state.to_string() != "signedIn" {
            return;
        }
        let session = self.backend.session.clone();
        let index = match session.tree_index() {
            Some(index) if !index.is_closed() => index,
            _ => return,
        };
        match index.needs_full_build() {
            Ok(false) => {}
            // 没建过库 / 读失败：交给 ensure_tree_index_build，探针不抢活
            _ => return,
        }
        let now = chrono::Utc::now().timestamp();
        match index.last_probe_at() {
            Ok(Some(last))
                if now.saturating_sub(last) < crate::session::tree_index::PROBE_COOLDOWN_SECS =>
            {
                return;
            }
            Ok(_) => {}
            Err(_) => return,
        }
        let cancel = std::sync::Arc::new(crate::session::download::Cancel::default());
        let epoch = {
            let mut rust = self.as_mut().rust_mut();
            // 冷却检查期间可能正好开始建库：起线程前再确认一次
            if rust.index_building {
                return;
            }
            rust.index_probe_epoch = rust.index_probe_epoch.wrapping_add(1);
            rust.index_probing = true;
            rust.index_probe_cancel = Some(std::sync::Arc::clone(&cancel));
            rust.index_probe_epoch
        };
        let done_session = session.clone();
        run_in_background(
            move || {
                let client = session.drive()?;
                crate::session::tree_index::sync::run_canary(&index, &client, &cancel)
            },
            move |mut qichi, result| {
                if qichi.as_ref().rust().index_probe_epoch != epoch {
                    return;
                }
                {
                    let mut rust = qichi.as_mut().rust_mut();
                    rust.index_probing = false;
                    rust.index_probe_cancel = None;
                }
                match result {
                    Ok(outcome) => qichi.as_mut().apply_probe_outcome(outcome),
                    Err(err) => {
                        // 失败只记录尝试（冷却生效），节点与目录快照都不动
                        if let Ok(index) = done_session.ensure_tree_index_open() {
                            let now = chrono::Utc::now().timestamp();
                            if let Err(write) = index.note_probe_attempt(now) {
                                if !index.is_closed() {
                                    done_session.record_tree_index_error(format!(
                                        "本地索引写入失败：{write}"
                                    ));
                                }
                            }
                        }
                        // 登录态失效要切回登录页；其余失败不打断浏览
                        if matches!(err, Failure::Auth(_)) {
                            qichi.as_mut().on_failure(err);
                        }
                    }
                }
            },
        );
    }

    /// 搜索路径：探针超过 5 分钟没跑过就在后台补一次，不阻塞本次搜索。
    pub(crate) fn maybe_probe_from_search(mut self: Pin<&mut Self>) {
        if self.index_building || self.index_probing {
            return;
        }
        let Some(index) = self.backend.session.tree_index() else {
            return;
        };
        if index.is_closed() {
            return;
        }
        let stale = match index.last_probe_at() {
            Ok(Some(last)) => {
                chrono::Utc::now().timestamp().saturating_sub(last)
                    >= crate::session::tree_index::PROBE_SEARCH_STALE_SECS
            }
            Ok(None) => true,
            Err(_) => false,
        };
        if stale {
            self.as_mut().probe_tree_index();
        }
    }

    /// 应用探针结论：只有 suspect 且过了冷却才启动全量重建。
    fn apply_probe_outcome(mut self: Pin<&mut Self>, outcome: ProbeOutcome) {
        match outcome {
            ProbeOutcome::Fresh { .. } | ProbeOutcome::Patched { .. } | ProbeOutcome::Cancelled => {
            }
            ProbeOutcome::Unavailable { first_time, .. } => {
                // 只提示一次，避免每次窗口获得焦点都弹同一句
                if first_time {
                    self.as_mut().set_message(
                        "网盘「最近修改」页的排序与预期不符，本地索引暂不做自动校准（目录仍会按需刷新）",
                    );
                }
            }
            ProbeOutcome::Suspect { reason, rebuild } => {
                if !rebuild {
                    return;
                }
                let message = format!(
                    "检测到网盘内容与本地索引不一致（{}），正在后台重建本地索引……",
                    reason.as_str()
                );
                self.as_mut().set_message(&message);
                let session = self.backend.session.clone();
                let index = match session.tree_index() {
                    Some(index) if !index.is_closed() => index,
                    _ => return,
                };
                self.as_mut().start_tree_index_build(session, index);
            }
        }
    }

    /// 错误统一出口：按类别更新属性，不吞错误。
    pub(crate) fn on_failure(mut self: Pin<&mut Self>, failure: Failure) {
        self.as_mut().on_failure_inner(failure, true);
    }

    /// 列表自动校准失败：仍更新底栏 / 登录态，但不发系统通知、不弹窗。
    pub(crate) fn on_refresh_failure(mut self: Pin<&mut Self>, failure: Failure) {
        self.as_mut().on_failure_inner(failure, false);
    }

    fn on_failure_inner(mut self: Pin<&mut Self>, failure: Failure, notify_limited: bool) {
        match failure {
            Failure::Captcha { url } => {
                let message = match &url {
                    Some(url) => format!(
                        "账号需要人机验证：点「打开验证页面」在浏览器完成验证，再把页面上的验证令牌粘到下方。验证地址：{url}"
                    ),
                    None => "验证令牌已过期，请重新提交登录".to_owned(),
                };
                self.as_mut().set_message(&message);
                self.as_mut().set_captcha(url.as_deref());
            }
            Failure::Auth(reason) => {
                {
                    let mut rust = self.as_mut().rust_mut();
                    rust.auth_state = QString::from("expired");
                    rust.status_message = QString::from(format!("登录状态失效：{reason}").as_str());
                }
                self.as_mut().notify();
            }
            limited @ Failure::RateLimited { .. } => {
                // 配额/限速要解释清楚，否则用户会以为客户端坏了
                let text = crate::session::rate_limit_text(&limited);
                self.as_mut().set_message(&text);
                if notify_limited {
                    self.as_mut().notify_rate_limited(&text);
                }
            }
            other => self.as_mut().set_message(&other.to_string()),
        }
    }

    /// 配额/限速弹一条系统通知（通知开关关闭时跳过）。
    fn notify_rate_limited(self: Pin<&mut Self>, text: &str) {
        if !self.notify_enabled {
            return;
        }
        super::notifications::show("PikPak 配额或限速限制", text, "dialog-warning");
    }

    fn on_quota(mut self: Pin<&mut Self>, quota: kichi_core::Quota) {
        let display = quota_display(&quota);
        {
            let mut rust = self.as_mut().rust_mut();
            rust.quota_used_text = QString::from(display.used.as_str());
            rust.quota_limit_text = QString::from(display.limit.as_str());
            rust.quota_ratio = display.ratio;
            rust.quota_vip = quota.vip;
        }
        self.as_mut().notify();
    }

    /// 月度传输配额（云下载 / 下行 / 上传）。
    fn on_transfer_quota(mut self: Pin<&mut Self>, quota: kichi_core::TransferQuota) {
        let display = transfer_display(&quota, chrono::Utc::now());
        {
            let mut rust = self.as_mut().rust_mut();
            rust.quota_transfer_cloud = QString::from(display.cloud.as_str());
            rust.quota_transfer_download = QString::from(display.download.as_str());
            rust.quota_transfer_upload = QString::from(display.upload.as_str());
            rust.quota_transfer_reset = QString::from(display.reset.as_str());
            rust.quota_transfer_cloud_ratio = display.cloud_ratio;
            rust.quota_transfer_download_ratio = display.download_ratio;
            rust.quota_transfer_upload_ratio = display.upload_ratio;
        }
        self.as_mut().notify();
    }

    /// 后台刷新身份与容量（身份失败不阻断容量）。
    fn refresh_identity_and_quota(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().quota_loading = true;
        self.as_mut().notify();
        let session = self.backend.session.clone();
        run_in_background(
            move || {
                let identity = session.refresh_identity();
                let quota = session.quota();
                // 容量随会话落盘：下次启动先显示旧值，再等这次刷新。
                if let Ok(ref quota) = quota {
                    session.cache_quota(quota);
                }
                let transfer = session.transfer_quota();
                Ok((identity, quota, transfer))
            },
            |mut qichi, result| {
                let (identity, quota, transfer) = match result {
                    Ok(triple) => triple,
                    Err(err) => {
                        qichi.as_mut().rust_mut().quota_loading = false;
                        qichi.as_mut().on_failure(err);
                        return;
                    }
                };
                match identity {
                    Ok(()) => {
                        qichi.as_mut().sync_identity_from_session();
                        qichi.as_mut().refresh_avatar_in_background();
                    }
                    // 网络失败时仍先显示已持久化的昵称 / 邮箱 / 头像，不能一直停在骨架。
                    Err(err) => {
                        qichi.as_mut().sync_identity_from_session();
                        qichi
                            .as_mut()
                            .set_message(&format!("账号信息获取失败：{err}"));
                    }
                }
                qichi.as_mut().rust_mut().quota_loading = false;
                match quota {
                    Ok(quota) => qichi.as_mut().on_quota(quota),
                    Err(err) => qichi.as_mut().on_failure(err),
                }
                match transfer {
                    Ok(transfer) => qichi.as_mut().on_transfer_quota(transfer),
                    // 传输配额是设置页的补充信息，失败只报状态，不弹系统通知
                    Err(err) => qichi.as_mut().on_refresh_failure(err),
                }
            },
        );
    }

    /// 把会话里已持久化的身份同步到 QML；头像只读本地缓存。
    fn sync_identity_from_session(mut self: Pin<&mut Self>) {
        let (account, display_name, avatar) = {
            let session = &self.as_ref().backend.session;
            (
                session.email().unwrap_or_default(),
                session.display_name().unwrap_or_default(),
                session
                    .cached_avatar_path()
                    .map(|path| super::reveal::file_uri(&path)),
            )
        };
        {
            let mut rust = self.as_mut().rust_mut();
            rust.account = QString::from(account.as_str());
            rust.display_name = QString::from(display_name.as_str());
            rust.avatar_path = QString::from(avatar.unwrap_or_default().as_str());
        }
        self.as_mut().notify();
    }

    /// 头像下载独立于身份 / 容量请求，慢网络不会拖住昵称和容量显示。
    fn refresh_avatar_in_background(self: Pin<&mut Self>) {
        let session = self.backend.session.clone();
        let Some(user_id) = session.user_id() else {
            return;
        };
        run_in_background(
            move || session.refresh_avatar(),
            move |mut qichi, result| {
                if qichi.as_ref().backend.session.user_id().as_deref() != Some(user_id.as_str()) {
                    return;
                }
                match result {
                    Ok(Some(path)) => {
                        qichi.as_mut().rust_mut().avatar_path =
                            QString::from(super::reveal::file_uri(&path).as_str());
                        qichi.as_mut().notify();
                    }
                    Ok(None) => {
                        let changed = !qichi.as_ref().avatar_path.is_empty();
                        qichi.as_mut().rust_mut().avatar_path = QString::default();
                        if changed {
                            qichi.as_mut().notify();
                        }
                    }
                    // 头像是装饰性数据：失败就保留旧缓存 / 首字母，不打断账号区。
                    Err(err) => eprintln!("[kichi] 头像更新失败：{err}"),
                }
            },
        );
    }
}

/// 后台任务 → 结果回到主线程：`work` 在后台线程跑，`done` 在主线程更新属性。
pub(super) fn run_in_background<W, T, D>(work: W, done: D)
where
    W: FnOnce() -> Result<T, Failure> + Send + 'static,
    T: Send + 'static,
    D: FnOnce(Pin<&mut qobject::Kichi>, Result<T, Failure>) + Send + 'static,
{
    // 线程可能根本没起来。把回调放在共享槽里，成功启动时由线程取走，
    // 启动失败时由主线程取走并立即回投错误，避免界面永远停在「正在……」。
    let done_slot = std::sync::Arc::new(Mutex::new(Some(done)));
    let done_for_thread = std::sync::Arc::clone(&done_slot);
    let spawned = std::thread::Builder::new()
        .name("kichi-net".into())
        .spawn(move || {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work));
            let done = done_for_thread
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .take();
            let Some(done) = done else {
                return;
            };
            enqueue(Box::new(
                move |mut qichi: Pin<&mut qobject::Kichi>| match outcome {
                    Ok(result) => done(qichi.as_mut(), result),
                    Err(_) => done(
                        qichi.as_mut(),
                        Err(Failure::Other("后台任务异常中断".to_owned())),
                    ),
                },
            ));
        });
    if let Err(err) = spawned {
        eprintln!("[kichi] 无法创建网络线程：{err}");
        let done = done_slot
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .take();
        if let Some(done) = done {
            let reason = format!("无法创建后台线程：{err}");
            enqueue(Box::new(move |mut qichi: Pin<&mut qobject::Kichi>| {
                done(qichi.as_mut(), Err(Failure::Other(reason)));
            }));
        }
    }
}

fn tasks() -> &'static Mutex<VecDeque<UiTask>> {
    static TASKS: OnceLock<Mutex<VecDeque<UiTask>>> = OnceLock::new();
    TASKS.get_or_init(|| Mutex::new(VecDeque::new()))
}

/// 把一个「在主线程执行」的任务入队（可从任意线程调用）。
pub(super) fn enqueue(task: UiTask) {
    let mut queue = tasks().lock().unwrap_or_else(|e| e.into_inner());
    queue.push_back(task);
}

/// 同 [`enqueue`]，给其它桥接子模块用（下载等长任务）。
pub(crate) fn enqueue_ui<F>(task: F)
where
    F: FnOnce(Pin<&mut qobject::Kichi>) + Send + 'static,
{
    enqueue(Box::new(task));
}

/// 字节数转人类可读（实现在 [`crate::format::size`]，会话层也要用同一套格式）。
pub(crate) use crate::format::size as format_size;

pub(crate) struct QuotaDisplay {
    pub(crate) used: String,
    pub(crate) limit: String,
    /// < 0 = 不限量 / 未知，QML 据此隐藏容量条
    pub(crate) ratio: f64,
}

/// 容量显示：账号区用 TB 档，避免 10 TB 显示成「10240.0 GB」。
pub(crate) fn quota_display(quota: &kichi_core::Quota) -> QuotaDisplay {
    let used = crate::format::capacity(quota.usage);
    let limit = quota
        .limit
        .map(crate::format::capacity)
        .unwrap_or_else(|| "无限制".to_owned());
    // 不限量 / 上限为 0 时给 -1，QML 隐藏进度条，避免除零和「永远 0%」的假象
    let ratio = match quota.limit {
        None | Some(0) => -1.0,
        Some(limit) => (quota.usage as f64 / limit as f64).clamp(0.0, 1.0),
    };
    QuotaDisplay { used, limit, ratio }
}

pub(crate) struct TransferDisplay {
    pub(crate) cloud: String,
    pub(crate) download: String,
    pub(crate) upload: String,
    pub(crate) reset: String,
    /// 0..1；`-1` = 没有上限，界面不画进度条
    pub(crate) cloud_ratio: f64,
    pub(crate) download_ratio: f64,
    pub(crate) upload_ratio: f64,
}

/// 额度占用比例；没给上限时返回 -1，和底栏容量条同一约定。
fn allowance_ratio(allowance: kichi_core::TransferAllowance) -> f64 {
    if allowance.limit == 0 {
        return -1.0;
    }
    (allowance.used as f64 / allowance.limit as f64).clamp(0.0, 1.0)
}

/// 传输配额显示：默认按官方说明的下个自然月 1 日 00:00（SGT）重置。
pub(crate) fn transfer_display(
    quota: &kichi_core::TransferQuota,
    now: chrono::DateTime<chrono::Utc>,
) -> TransferDisplay {
    let text = |allowance: kichi_core::TransferAllowance| {
        let used = crate::format::capacity(allowance.used);
        if allowance.limit == 0 {
            // 官方没下发上限时只报已用，不编造一个 0 B 的上限
            format!("已用 {used}")
        } else {
            format!("已用 {used} / {}", crate::format::capacity(allowance.limit))
        }
    };
    TransferDisplay {
        cloud: text(quota.cloud_download),
        download: text(quota.download),
        upload: text(quota.upload),
        reset: next_reset_text(now),
        cloud_ratio: allowance_ratio(quota.cloud_download),
        download_ratio: allowance_ratio(quota.download),
        upload_ratio: allowance_ratio(quota.upload),
    }
}

/// 下个自然月的 1 日 00:00 SGT（UTC+8）。官方接口不返回重置时间。
fn next_reset_text(now: chrono::DateTime<chrono::Utc>) -> String {
    use chrono::{Datelike, FixedOffset};
    let Some(sgt) = FixedOffset::east_opt(8 * 3600) else {
        return String::new();
    };
    let now = now.with_timezone(&sgt);
    let (year, month) = if now.month() == 12 {
        (now.year() + 1, 1)
    } else {
        (now.year(), now.month() + 1)
    };
    format!("{year}/{month}/1 00:00 SGT")
}

/// 会话状态 → QML 用的字符串。
pub(crate) fn state_name(state: State) -> &'static str {
    match state {
        State::SignedIn => "signedIn",
        State::Expired => "expired",
        State::SignedOut => "signedOut",
    }
}

#[cfg(test)]
mod tests {
    use super::{next_reset_text, transfer_display};
    use chrono::{TimeZone, Utc};
    use kichi_core::{TransferAllowance, TransferQuota};

    #[test]
    fn transfer_display_formats_the_three_monthly_allowances() {
        let quota = TransferQuota {
            cloud_download: TransferAllowance {
                limit: 40 * 1024 * 1024 * 1024 * 1024,
                used: 1024 * 1024 * 1024,
            },
            download: TransferAllowance {
                limit: 4 * 1024 * 1024 * 1024 * 1024,
                used: 0,
            },
            upload: TransferAllowance {
                limit: 0,
                used: 512 * 1024 * 1024,
            },
            ..TransferQuota::default()
        };
        let now = Utc.with_ymd_and_hms(2026, 11, 6, 12, 0, 0).unwrap();
        let display = transfer_display(&quota, now);
        assert_eq!(display.cloud, "已用 1.0 GB / 40.0 TB");
        assert_eq!(display.download, "已用 0 B / 4.0 TB");
        assert_eq!(display.upload, "已用 512.0 MB", "没给上限时只报已用");
        assert_eq!(display.reset, "2026/12/1 00:00 SGT");
        assert!((display.cloud_ratio - 1.0 / 40960.0).abs() < 1e-9);
        assert_eq!(display.download_ratio, 0.0);
        assert_eq!(display.upload_ratio, -1.0, "没上限不画进度条");
    }

    #[test]
    fn reset_rolls_over_after_december_in_sgt() {
        // UTC 2026-12-31 17:00 已是 SGT 2027-01-01 01:00
        let now = Utc.with_ymd_and_hms(2026, 12, 31, 17, 0, 0).unwrap();
        assert_eq!(next_reset_text(now), "2027/2/1 00:00 SGT");
        let now = Utc.with_ymd_and_hms(2026, 11, 30, 15, 59, 0).unwrap();
        assert_eq!(next_reset_text(now), "2026/12/1 00:00 SGT");
    }
}
