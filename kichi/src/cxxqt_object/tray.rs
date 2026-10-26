//! 系统托盘：走 D-Bus StatusNotifierItem 协议（KDE Plasma 原生支持）。
//!
//! 为什么不用 Qt 的托盘：
//! ```text
//! 2026-10-26 实测，Qt.labs.platform 的 SystemTrayIcon 在 KDE 平台主题下
//! 会走 KDEPlasmaPlatformTheme6::init() → KStatusNotifierItem → new QMenu
//! → QWidgetPrivate::init → qFatal → abort()。
//! 那是 QtWidgets 的路径，而本应用只有 QGuiApplication，启动即核心转储。
//! 更糟的是对象在 QML 创建时就实例化，visible:false 也挡不住。
//! ```
//! 因此托盘改用 `ksni`：纯 Rust + D-Bus，与 Qt 的 GUI 栈完全无关，
//! 就算托盘失败也只是「没有托盘图标」，绝不会拖垮整个应用。
//!
//! 线程模型：ksni 在自己的后台线程里跑事件循环；菜单点击与图标激活
//! 通过全局队列投递给主线程，由 QML 定时器调用 `poll()` 消费——
//! 和网络结果回投用的是同一套模式（见 `actions.rs`）。

use std::sync::Mutex;
use std::sync::OnceLock;

/// 托盘事件：主线程据此操作窗口。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayEvent {
    /// 显示并聚焦主窗口
    Show,
    /// 退出应用
    Quit,
}

/// 托盘图标（`ksni` 的 trait 实现）。
struct TrayIcon;

impl ksni::Tray for TrayIcon {
    fn id(&self) -> String {
        "kichi".to_owned()
    }

    fn title(&self) -> String {
        "kichi".to_owned()
    }

    /// 图标走内嵌位图，不依赖系统图标主题（换机器/换主题都不受影响）。
    ///
    /// 同时给出 `icon_name` 作为兜底：某些实现只认名字。
    fn icon_name(&self) -> String {
        "kichi".to_owned()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        super::tray_icon::icon_pixmaps()
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        push(TrayEvent::Show);
    }

    /// 部分桌面（含 Plasma）优先用这个做默认激活
    fn secondary_activate(&mut self, _x: i32, _y: i32) {
        push(TrayEvent::Show);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::StandardItem;
        vec![
            StandardItem {
                label: "显示 kichi".into(),
                activate: Box::new(|_| {
                    push(TrayEvent::Show);
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "退出".into(),
                activate: Box::new(|_| {
                    push(TrayEvent::Quit);
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

fn events() -> &'static Mutex<Vec<TrayEvent>> {
    static EVENTS: OnceLock<Mutex<Vec<TrayEvent>>> = OnceLock::new();
    EVENTS.get_or_init(|| Mutex::new(Vec::new()))
}

/// 托盘线程 → 主线程投递（可从任意线程调用）。
fn push(event: TrayEvent) {
    match events().lock() {
        Ok(mut queue) => queue.push(event),
        // 队列锁中毒：丢掉这条事件也不影响主功能，绝不能让托盘拖垮应用
        Err(poisoned) => poisoned.into_inner().push(event),
    }
}

/// 托盘句柄（启动成功后才有）。
type Handle = ksni::blocking::Handle<TrayIcon>;

fn handle_slot() -> &'static Mutex<Option<Handle>> {
    static HANDLE: OnceLock<Mutex<Option<Handle>>> = OnceLock::new();
    HANDLE.get_or_init(|| Mutex::new(None))
}

/// 托盘是否可用（启动失败时界面会禁用「关闭到托盘」）。
pub fn is_available() -> bool {
    match handle_slot().lock() {
        Ok(guard) => guard.is_some(),
        Err(poisoned) => poisoned.into_inner().is_some(),
    }
}

/// 启动托盘（幂等）。失败只报告，不 panic、不影响主窗口。
///
/// 返回是否启动成功。D-Bus 不可用、没有 StatusNotifierWatcher 等情况下
/// 会失败——那是环境问题，不是 bug，界面上把「关闭到托盘」禁用即可。
pub fn start() -> bool {
    use ksni::blocking::TrayMethods;
    if is_available() {
        return true;
    }
    match TrayIcon.spawn() {
        Ok(handle) => {
            match handle_slot().lock() {
                Ok(mut slot) => *slot = Some(handle),
                Err(poisoned) => *poisoned.into_inner() = Some(handle),
            }
            true
        }
        Err(err) => {
            eprintln!("[kichi] 托盘不可用（关闭到托盘将禁用）：{err}");
            false
        }
    }
}

/// 注销托盘（退出应用前、或用户关掉「关闭到托盘」时调用）。
///
/// ⚠ 必须在进程真正退出**之前**调用：D-Bus 上 `org.kde.StatusNotifierItem-<pid>-N`
/// 这个 well-known name 不会随进程消失自动从面板移除，KDE 会留下一个
/// 点不动的僵尸图标（2026-10-26 用户实测踩到）。这里显式 shutdown。
pub fn stop() {
    let taken = match handle_slot().lock() {
        Ok(mut slot) => slot.take(),
        Err(poisoned) => poisoned.into_inner().take(),
    };
    if let Some(handle) = taken {
        // 要**等**注销完成再返回：不等的话进程紧接着退出，
        // D-Bus 名字还在，KDE 面板会留下点不动的僵尸图标。
        // 这一步很快（就是一次 D-Bus 调用），放在退出前不值得做异步。
        handle.shutdown().wait();
    }
}

/// 取出待处理事件（主线程调用）。
pub fn poll() -> Vec<TrayEvent> {
    match events().lock() {
        Ok(mut queue) => std::mem::take(&mut *queue),
        Err(poisoned) => std::mem::take(&mut *poisoned.into_inner()),
    }
}
