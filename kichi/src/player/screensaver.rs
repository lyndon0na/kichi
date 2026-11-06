//! 内嵌播放期间的屏幕抑制。
//!
//! 内嵌 mpv 的 Wayland surface 只活在内嵌代理里，合成器不知道它，
//! mpv 自带的 `stop-screensaver`（Wayland idle-inhibit）建立不起来。
//! 这里由 kichi 直接向 `org.freedesktop.ScreenSaver`（KWin）申请抑制：
//! 播放中持有 cookie，暂停 / 停止时归还，与外部 mpv 的 `stop-screensaver=yes` 一致。

use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use ashpd::zbus::{block_on, Connection};

const SERVICE: &str = "org.freedesktop.ScreenSaver";
const PATH: &str = "/ScreenSaver";
const INTERFACE: &str = "org.freedesktop.ScreenSaver";
const APP_NAME: &str = "kichi";
const REASON: &str = "正在播放视频";
/// 收尾时等待 `UnInhibit` 完成的时限；普通的状态切换都不阻塞 mpv 线程。
const RELEASE_TIMEOUT: Duration = Duration::from_millis(500);

enum Command {
    SetActive(bool),
    Shutdown(Sender<()>),
}

/// 播放会话持有的屏幕抑制；`Drop` 时归还。
pub(crate) struct ScreenInhibitor {
    commands: Option<Sender<Command>>,
}

impl ScreenInhibitor {
    pub(crate) fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        match std::thread::Builder::new()
            .name("kichi-screensaver".into())
            .spawn(move || run(rx))
        {
            Ok(_) => Self { commands: Some(tx) },
            Err(err) => {
                eprintln!("[kichi] 屏幕抑制线程启动失败：{err}");
                Self { commands: None }
            }
        }
    }

    /// 播放中为真、暂停 / 停止为假；重复设置同一状态是空操作。
    pub(crate) fn set_active(&self, active: bool) {
        if let Some(tx) = &self.commands {
            let _ = tx.send(Command::SetActive(active));
        }
    }
}

impl Drop for ScreenInhibitor {
    fn drop(&mut self) {
        let Some(tx) = self.commands.take() else {
            return;
        };
        let (ack_tx, ack_rx) = mpsc::channel();
        if tx.send(Command::Shutdown(ack_tx)).is_ok() {
            let _ = ack_rx.recv_timeout(RELEASE_TIMEOUT);
        }
    }
}

fn run(commands: Receiver<Command>) {
    let connection = match block_on(Connection::session()) {
        Ok(connection) => connection,
        Err(err) => {
            eprintln!("[kichi] 屏幕抑制不可用（连接不到会话总线）：{err}");
            return;
        }
    };
    let mut cookie: Option<u32> = None;
    let mut warned = false;
    loop {
        match commands.recv() {
            Ok(Command::SetActive(true)) => inhibit(&connection, &mut cookie, &mut warned),
            Ok(Command::SetActive(false)) => release(&connection, &mut cookie),
            Ok(Command::Shutdown(ack)) => {
                release(&connection, &mut cookie);
                let _ = ack.send(());
                return;
            }
            Err(_) => {
                release(&connection, &mut cookie);
                return;
            }
        }
    }
}

fn inhibit(connection: &Connection, cookie: &mut Option<u32>, warned: &mut bool) {
    if cookie.is_some() {
        return;
    }
    let result = block_on(connection.call_method(
        Some(SERVICE),
        PATH,
        Some(INTERFACE),
        "Inhibit",
        &(APP_NAME, REASON),
    ))
    .and_then(|reply| reply.body().deserialize::<u32>());
    match result {
        Ok(value) => *cookie = Some(value),
        Err(err) => {
            if !*warned {
                eprintln!("[kichi] 屏幕抑制失败：{err}");
                *warned = true;
            }
        }
    }
}

fn release(connection: &Connection, cookie: &mut Option<u32>) {
    let Some(value) = cookie.take() else {
        return;
    };
    let result = block_on(connection.call_method(
        Some(SERVICE),
        PATH,
        Some(INTERFACE),
        "UnInhibit",
        &(value,),
    ));
    if let Err(err) = result {
        eprintln!("[kichi] 屏幕抑制解除失败：{err}");
    }
}
