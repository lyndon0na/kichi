//! 桥接对象：Rust 侧暴露给 QML 的登录状态与控制入口。
//!
//! cxx-qt 0.10 踩坑记录（均已实测复现）：
//! 1. 桥接里**不要**声明 `impl cxx_qt::Initialize for Kichi {}`。
//!    cxx-qt 只在「没有任何构造器声明」时自动生成默认构造器
//!    （内部调用 `KichiRust::default()`）；一旦写了这个标记，
//!    就要求你在桥接外真正实现 `Initialize`，否则编译报
//!    `the trait bound Kichi: Initialize is not satisfied`。
//! 2. 桥接上**不要**用 `#[cxx_qt::bridge(namespace = "...")]`：
//!    它会把 `QString` 等 C++ 类型也塞进该命名空间，生成代码报
//!    `'kichi::QString' has not been declared`。QML 类型名由
//!    `QML_NAMED_ELEMENT` 决定，不需要 C++ 命名空间。
//! 3. 属性必须带 NOTIFY 信号，否则 QML 永远不会重新求值绑定
//!    （现象：点登录毫无反应、状态文字不更新）。这里所有属性共用一个
//!    `changed` 信号，Rust 侧每次改完字段调用 `notify()` 即可。
//! 4. 桥接里不要出现 `impl cxx_qt::Threading`，线程回投会失效；
//!    后台结果走主线程队列（QML 定时器调用 `pump` 抽水），见 `actions.rs`。
//! 5. invokable 声明端与实现端签名要一致：`Pin<&mut Self>` 对
//!    `Pin<&mut Self>`，或 `&mut Self` 对 `&mut Self`。

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(QString, auth_state, cxx_name = "authState", READ, NOTIFY = changed)]
        #[qproperty(QString, status_message, cxx_name = "statusMessage", READ, NOTIFY = changed)]
        #[qproperty(bool, busy, READ, NOTIFY = changed)]
        #[qproperty(QString, account, READ, NOTIFY = changed)]
        #[qproperty(QString, quota_summary, cxx_name = "quotaSummary", READ, NOTIFY = changed)]
        #[qproperty(bool, captcha_required, cxx_name = "captchaRequired", READ, NOTIFY = changed)]
        type Kichi = super::KichiRust;

        /// 统一变更通知：所有属性都挂这一个信号
        #[qsignal]
        fn changed(self: Pin<&mut Self>);

        #[qinvokable]
        #[cxx_name = "login"]
        fn login(
            self: Pin<&mut Self>,
            username: QString,
            password: QString,
            captcha_token: QString,
        );

        #[qinvokable]
        #[cxx_name = "openCaptchaPage"]
        fn open_captcha_page(self: &Self);

        #[qinvokable]
        #[cxx_name = "signOut"]
        fn sign_out(self: Pin<&mut Self>);

        #[qinvokable]
        #[cxx_name = "warmUp"]
        fn warm_up(self: Pin<&mut Self>);

        #[qinvokable]
        #[cxx_name = "pump"]
        fn pump(self: Pin<&mut Self>);
    }
}

use cxx_qt_lib::QString;

mod actions;
mod backend;

use backend::Backend;

/// QObject `Kichi` 的 Rust 载体。
pub struct KichiRust {
    pub(crate) auth_state: QString,
    pub(crate) status_message: QString,
    pub(crate) busy: bool,
    pub(crate) captcha_required: bool,
    pub(crate) account: QString,
    pub(crate) quota_summary: QString,
    /// 人机验证地址（不做成属性：属性变化靠 QML 读状态消息）
    pub(crate) captcha_url: String,
    pub(crate) backend: Backend,
}

impl Default for KichiRust {
    fn default() -> Self {
        let mut object = Self {
            auth_state: QString::from("signedOut"),
            status_message: QString::default(),
            busy: false,
            captcha_required: false,
            account: QString::default(),
            quota_summary: QString::default(),
            captcha_url: String::new(),
            backend: Backend::new(crate::session::store_path()),
        };
        object.apply_restored();
        object
    }
}

impl KichiRust {
    /// 构造时恢复会话（只读本地存储 + 必要时刷新一次令牌）。
    fn apply_restored(&mut self) {
        use crate::session::State;
        let state = self.backend.state();
        self.auth_state = QString::from(actions::state_name(state));
        if state == State::SignedIn {
            self.busy = true;
            self.status_message = QString::from("正在恢复会话……");
            return;
        }
        let notice = self.backend.restored_notice().to_owned();
        self.status_message = QString::from(if notice.is_empty() {
            self.backend.storage()
        } else {
            notice
        });
    }
}

/// 打开系统默认浏览器（不引入额外依赖；xdg-open 是 Linux 桌面标配）。
pub(crate) fn open_in_browser(url: &str) {
    let spawned = std::process::Command::new("xdg-open")
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    if let Err(err) = spawned {
        eprintln!("[kichi] 打开浏览器失败：{err}");
    }
}
