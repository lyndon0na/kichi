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
        if self.busy {
            return;
        }
        let user = username.to_string().trim().to_owned();
        let password = password.to_string();
        if user.is_empty() || password.is_empty() {
            self.as_mut().set_message("请输入账号和密码", false);
            return;
        }
        let captcha = captcha_token.to_string();
        let captcha = (!captcha.trim().is_empty()).then(|| captcha.trim().to_owned());

        let session = self.backend.session.clone();
        self.as_mut().set_message("正在登录……", true);
        run_in_background(
            move || session.sign_in(&user, &password, captcha.as_deref()),
            |mut qichi, result| match result {
                Ok(()) => qichi.as_mut().on_logged_in(),
                Err(err) => qichi.as_mut().on_failure(err),
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
        if self.auth_state.to_string() == "signedIn" && self.busy {
            self.as_mut().refresh_identity_and_quota();
            self.as_mut().refresh_files();
            self.as_ref().start_task_polling();
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
                    rust.status_message = QString::from("已退出登录");
                    rust.busy = false;
                }
                self.as_mut().reset_files();
                self.as_mut().reset_play();
                self.as_mut().reset_downloads();
                self.as_mut().reset_search();
                self.as_mut().stop_task_polling();
            }
            Err(err) => self
                .as_mut()
                .set_message(&format!("退出登录失败：{err}"), false),
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
    }
}

impl qobject::Kichi {
    /// 退出前注销托盘（QML 在 Qt.quit() 之前调用）。
    ///
    /// 真退出与托盘点「退出」两条路径都会调用它（QML 的 `onClosing` 与
    /// `quitRequested`），所以下载历史的强制写盘也挂在这里：
    /// 进程直接结束前把记录表落成最后一次（ADR 0014）。
    pub fn shutdown_tray(&self) {
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
    pub(crate) fn set_message(mut self: Pin<&mut Self>, text: &str, busy: bool) {
        {
            let mut rust = self.as_mut().rust_mut();
            rust.status_message = QString::from(text);
            rust.busy = busy;
        }
        self.as_mut().notify();
    }

    fn set_captcha(mut self: Pin<&mut Self>, url: Option<&str>) {
        {
            let mut rust = self.as_mut().rust_mut();
            rust.captcha_required = true;
            rust.captcha_url = url.unwrap_or_default().to_owned();
            rust.busy = false;
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
            rust.status_message = QString::from("登录成功，正在同步账号信息……");
            rust.busy = true;
        }
        self.as_mut().notify();
        self.as_mut().refresh_identity_and_quota();
        self.as_mut().refresh_files();
        self.as_ref().start_task_polling();
    }

    /// 错误统一出口：按类别更新属性，不吞错误。
    pub(crate) fn on_failure(mut self: Pin<&mut Self>, failure: Failure) {
        match failure {
            Failure::Captcha { url } => {
                let message = match &url {
                    Some(url) => format!(
                        "账号需要人机验证：点「打开验证页面」在浏览器完成验证，再把页面上的验证令牌粘到下方。验证地址：{url}"
                    ),
                    None => "验证令牌已过期，请重新提交登录".to_owned(),
                };
                self.as_mut().set_message(&message, false);
                self.as_mut().set_captcha(url.as_deref());
            }
            Failure::Auth(reason) => {
                {
                    let mut rust = self.as_mut().rust_mut();
                    rust.auth_state = QString::from("expired");
                    rust.status_message = QString::from(format!("登录状态失效：{reason}").as_str());
                    rust.busy = false;
                }
                self.as_mut().notify();
            }
            limited @ Failure::RateLimited { .. } => {
                // 配额/限速要解释清楚，否则用户会以为客户端坏了
                let text = crate::session::rate_limit_text(&limited);
                self.as_mut().set_message(&text, false);
                self.as_mut().notify_rate_limited(&text);
            }
            other => self.as_mut().set_message(&other.to_string(), false),
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
        let used = format_size(quota.usage);
        let limit = quota
            .limit
            .map(format_size)
            .unwrap_or_else(|| "无限制".to_owned());
        // 不限量 / 上限为 0 时给 -1，QML 隐藏进度条，避免除零和「永远 0%」的假象
        let ratio = match quota.limit {
            None | Some(0) => -1.0,
            Some(limit) => (quota.usage as f64 / limit as f64).clamp(0.0, 1.0),
        };
        {
            let mut rust = self.as_mut().rust_mut();
            rust.quota_used_text = QString::from(used.as_str());
            rust.quota_limit_text = QString::from(limit.as_str());
            rust.quota_ratio = ratio;
            rust.quota_vip = quota.vip;
        }
        let vip = if quota.vip { "会员" } else { "免费账号" };
        self.as_mut().set_message(
            &format!("账号信息已同步：{vip} · 已用 {used} / {limit}"),
            false,
        );
    }

    /// 后台刷新身份与容量（身份失败不阻断容量）。
    fn refresh_identity_and_quota(self: Pin<&mut Self>) {
        let session = self.backend.session.clone();
        run_in_background(
            move || Ok((session.refresh_identity(), session.quota())),
            |mut qichi, result| {
                let (identity, quota) = match result {
                    Ok(pair) => pair,
                    Err(err) => {
                        qichi.as_mut().on_failure(err);
                        return;
                    }
                };
                match identity {
                    Ok(()) => {
                        if let Some(email) = qichi.as_mut().rust().backend.session.email() {
                            qichi.as_mut().rust_mut().account = QString::from(email.as_str());
                        }
                    }
                    // 身份查询失败只提示，不影响容量
                    Err(err) => qichi
                        .as_mut()
                        .set_message(&format!("账号信息获取失败：{err}"), false),
                }
                match quota {
                    Ok(quota) => qichi.as_mut().on_quota(quota),
                    Err(err) => qichi.as_mut().on_failure(err),
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
    let spawned = std::thread::Builder::new()
        .name("kichi-net".into())
        .spawn(move || {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work));
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

/// 会话状态 → QML 用的字符串。
pub(crate) fn state_name(state: State) -> &'static str {
    match state {
        State::SignedIn => "signedIn",
        State::Expired => "expired",
        State::SignedOut => "signedOut",
    }
}
