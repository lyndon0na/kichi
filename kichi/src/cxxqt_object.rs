//! 桥接对象：Rust 侧暴露给 QML 的登录状态与控制入口。
//!
//! cxx-qt 0.10 踩坑记录（均已实测复现，写进交付报告）：
//! 1. 命名空间要写在桥接宏上 `#[cxx_qt::bridge(namespace = "...")]`。
//!    写在 `type` 上的 `#[namespace]` 会让 `impl cxx_qt::Initialize` 报
//!    `Kichi: Initialize is not satisfied`——这是本次卡最久的根因。
//! 2. 桥接里不要出现 `qsignal` 或 `impl cxx_qt::Threading`（同样会丢构造器；
//!    另一条佐证是 `Initialize` 的 impl 块里除 `#[namespace]` 外不许带属性）。
//! 3. 属性变化靠 QML 的 `onStatusMessageChanged` 等逐属性绑定驱动；
//!    后台结果走主线程队列（QML 定时器调用 `pump` 抽水），见 `actions.rs`。
//! 4. invokable 声明端与实现端签名要一致（`Pin<&mut Self>` 对 `Pin<&mut Self>`）。

#[cxx_qt::bridge(namespace = "kichi")]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(QString, auth_state, cxx_name = "authState", READ)]
        #[qproperty(QString, status_message, cxx_name = "statusMessage", READ)]
        #[qproperty(bool, busy, READ)]
        #[qproperty(bool, captcha_required, cxx_name = "captchaRequired", READ)]
        type Kichi = super::KichiRust;

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

    impl cxx_qt::Initialize for Kichi {}
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
