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
use crate::session::Failure;

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
        // 系统外观变化（深色 / 强调色）也回到主线程应用。
        if let Some(appearance) = crate::appearance::take_pending() {
            crate::appearance::apply(appearance);
        }
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

// 容量 / 配额文案与会话状态名的实现在 [`super::actions_display`]，
// 这里重导出，调用点保持 `actions::quota_display` 不变。
pub(crate) use super::actions_display::{quota_display, state_name};
