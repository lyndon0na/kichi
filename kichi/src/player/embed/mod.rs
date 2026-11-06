//! 进程内 libmpv：mpv 的画面直接进 kichi 窗口。
//!
//! 与 `player::session`（外部 mpv / VLC）并列，由 [`start`] 走同一套
//! [`StartEvent`] 回投；外部路径仍然保留，设置里可以切回去。
//!
//! mpv 的 Wayland 连接不指向真实合成器，而是指向 [`proxy`] 里的进程内代理；
//! 代理把 `wl_surface.commit` 上的 dmabuf / shm 帧交给我们，再由 C++ 侧的
//! `MpvVideoItem` 导入成 Qt 纹理。`WAYLAND_SOCKET`（一次性 fd）只在
//! mpv 建连期间存在：客户端真的发出第一个请求后才恢复，避免 mpv 在慢网络上
//! 等太久转而连上真实合成器、弹出第二个播放器窗口。

pub(crate) mod conf;
mod ffi;
mod mpv;
mod proxy;
#[cfg(test)]
mod tests;

use std::ffi::OsStr;
use std::os::fd::IntoRawFd;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, OnceLock};

use super::{mask_url, PlaybackExtras, StartEvent};
use crate::session::download::Cancel;

pub(crate) use proxy::Frame;

/// 单条内嵌播放会话。
pub(crate) struct Session {
    cancel: Arc<Cancel>,
    pub(crate) proxy_stop: AtomicBool,
    pub(crate) viewport: Mutex<Option<Viewport>>,
    pub(crate) viewport_dirty: AtomicBool,
    pub(crate) frame_slot: Arc<Mutex<Option<Frame>>>,
    pub(crate) frame_dirty: Arc<AtomicBool>,
    control: Mutex<Option<Sender<Control>>>,
    paused: AtomicBool,
    /// 当前音量 / 静音：界面和 mpv 原生键位都会改它。
    volume: AtomicI32,
    muted: AtomicBool,
    position: Mutex<Option<f64>>,
    duration: Mutex<Option<f64>>,
    /// 当前倍速（1.0 = 原速）；界面与 mpv 原生键位都会改它。
    speed: Mutex<f64>,
    /// 最近一次从 mpv 读到的轨道列表（切换 / 加载后刷新）。
    tracks: Mutex<Vec<TrackInfo>>,
    /// 代理线程的失败原因；播放失败要把它带给界面，不能只打 stderr。
    proxy_error: Mutex<Option<String>>,
    /// mpv 的 Wayland 客户端已经发来第一个请求：这时恢复 `WAYLAND_SOCKET` 才安全。
    wayland_connected: AtomicBool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Viewport {
    pub(crate) width: i32,
    pub(crate) height: i32,
    pub(crate) scale: f64,
}

/// 界面线程发给 mpv 事件线程的控制命令。
pub(crate) enum Control {
    TogglePause,
    SeekAbsolute(f64),
    SetVolume(i64),
    SetMuted(bool),
    SetSpeed(f64),
    /// 切换音轨 / 字幕轨；`id < 0` 表示关闭字幕。
    SelectTrack {
        audio: bool,
        id: i64,
    },
    /// 把界面收到的按键原样交给 mpv 的 input 层（mpv 原生键位）。
    Keypress(String),
    /// 立即套用新的字幕样式（设置页 / 播放窗口调整时实时预览）。
    SubtitleStyle(SubtitleStyle),
    Quit,
}

/// 一条可切换的轨道（音轨或字幕轨）。
#[derive(Debug, Clone, PartialEq)]
pub struct TrackInfo {
    pub id: i64,
    /// true = 音轨；false = 字幕轨
    pub audio: bool,
    pub label: String,
    /// 轨道语言标记（mpv 的 `track-list/N/lang`，可能为空）
    pub lang: String,
    pub selected: bool,
}

/// 字幕样式（对应 mpv 的 `sub-*` 选项）。
#[derive(Debug, Clone, PartialEq)]
pub struct SubtitleStyle {
    pub font_size: i64,
    /// mpv `sub-pos`：0 = 顶部，100 = 底部
    pub position: i64,
    pub color: String,
    pub outline_size: f64,
    pub outline_color: String,
}

impl SubtitleStyle {
    pub fn new(
        font_size: i32,
        position: i32,
        color: &str,
        outline_size: i32,
        outline_color: &str,
    ) -> Self {
        Self {
            font_size: i64::from(font_size).clamp(
                i64::from(crate::settings::SUB_FONT_SIZE_MIN),
                i64::from(crate::settings::SUB_FONT_SIZE_MAX),
            ),
            position: i64::from(position).clamp(0, i64::from(crate::settings::SUB_POS_MAX)),
            color: sanitize_color(color, crate::settings::DEFAULT_SUB_COLOR),
            outline_size: f64::from(outline_size)
                .clamp(0.0, f64::from(crate::settings::SUB_OUTLINE_SIZE_MAX)),
            outline_color: sanitize_color(
                outline_color,
                crate::settings::DEFAULT_SUB_OUTLINE_COLOR,
            ),
        }
    }
}

impl Default for SubtitleStyle {
    fn default() -> Self {
        Self {
            font_size: i64::from(crate::settings::DEFAULT_SUB_FONT_SIZE),
            position: i64::from(crate::settings::DEFAULT_SUB_POS),
            color: crate::settings::DEFAULT_SUB_COLOR.to_owned(),
            outline_size: f64::from(crate::settings::DEFAULT_SUB_OUTLINE_SIZE),
            outline_color: crate::settings::DEFAULT_SUB_OUTLINE_COLOR.to_owned(),
        }
    }
}

fn sanitize_color(value: &str, fallback: &str) -> String {
    if crate::settings::valid_color(value) {
        value.to_owned()
    } else {
        fallback.to_owned()
    }
}

/// 新会话的初始状态（全部来自设置，跨视频 / 跨启动记住）。
#[derive(Debug, Clone, PartialEq)]
pub struct SessionSettings {
    pub volume: i32,
    pub muted: bool,
    pub speed: f64,
    pub subtitles: SubtitleStyle,
    /// mpv `alang` 的值；空串 = 不指定。
    pub audio_lang: String,
    /// mpv `slang` 的值；空串 = 不指定。
    pub sub_lang: String,
    /// 外挂字幕文件名匹配用的首选语言标记；空 = 保持中文优先。
    pub sub_name_tags: Vec<String>,
    /// kichi 自己的 `embed-mpv.conf`；`None` = 不读配置文件。
    pub conf: Option<std::path::PathBuf>,
    /// kichi 自己的 `embed-input.conf`；`None` = 不加键位文件。
    pub input_conf: Option<std::path::PathBuf>,
}

impl SessionSettings {
    pub fn new(
        volume: i32,
        muted: bool,
        speed: f64,
        subtitles: SubtitleStyle,
        audio_lang: String,
        sub_lang: String,
        sub_name_tags: Vec<String>,
    ) -> Self {
        Self {
            volume: volume.clamp(0, 100),
            muted,
            speed: sanitize_speed(speed),
            subtitles,
            audio_lang,
            sub_lang,
            sub_name_tags,
            conf: None,
            input_conf: None,
        }
    }
}

fn sanitize_speed(speed: f64) -> f64 {
    if speed.is_finite() {
        speed.clamp(0.5, 3.0)
    } else {
        1.0
    }
}

impl Session {
    fn new(cancel: Arc<Cancel>) -> Self {
        Self {
            cancel,
            proxy_stop: AtomicBool::new(false),
            viewport: Mutex::new(None),
            viewport_dirty: AtomicBool::new(false),
            frame_slot: Arc::new(Mutex::new(None)),
            frame_dirty: Arc::new(AtomicBool::new(false)),
            control: Mutex::new(None),
            paused: AtomicBool::new(false),
            volume: AtomicI32::new(100),
            muted: AtomicBool::new(false),
            position: Mutex::new(None),
            duration: Mutex::new(None),
            speed: Mutex::new(1.0),
            tracks: Mutex::new(Vec::new()),
            proxy_error: Mutex::new(None),
            wayland_connected: AtomicBool::new(false),
        }
    }

    pub(crate) fn cancel(&self) -> &Arc<Cancel> {
        &self.cancel
    }

    pub(crate) fn set_control(&self, tx: Sender<Control>) {
        *self
            .control
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(tx);
    }

    fn send(&self, control: Control) {
        let guard = self
            .control
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(tx) = guard.as_ref() {
            let _ = tx.send(control);
        }
    }

    pub(crate) fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Release);
    }

    pub(crate) fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Acquire)
    }

    pub(crate) fn set_volume(&self, volume: i32) {
        self.volume.store(volume.clamp(0, 100), Ordering::Release);
    }

    pub(crate) fn volume(&self) -> i32 {
        self.volume.load(Ordering::Acquire)
    }

    pub(crate) fn set_muted(&self, muted: bool) {
        self.muted.store(muted, Ordering::Release);
    }

    pub(crate) fn muted(&self) -> bool {
        self.muted.load(Ordering::Acquire)
    }

    pub(crate) fn set_position(&self, position: Option<f64>) {
        *self
            .position
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = position;
    }

    pub(crate) fn set_duration(&self, duration: Option<f64>) {
        *self
            .duration
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = duration;
    }

    fn position(&self) -> Option<f64> {
        *self
            .position
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn duration(&self) -> Option<f64> {
        *self
            .duration
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn set_speed(&self, speed: f64) {
        *self
            .speed
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = sanitize_speed(speed);
    }

    fn speed(&self) -> f64 {
        *self
            .speed
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn set_tracks(&self, tracks: Vec<TrackInfo>) {
        *self
            .tracks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = tracks;
    }

    fn tracks(&self) -> Vec<TrackInfo> {
        self.tracks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// 代理线程失败：记下原因并让 mpv 线程收尾，最终由界面提示条显示。
    pub(crate) fn fail_proxy(&self, reason: String) {
        eprintln!("[kichi] 内嵌代理失败：{reason}");
        let mut slot = self
            .proxy_error
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if slot.is_none() {
            *slot = Some(reason);
        }
        self.proxy_stop.store(true, Ordering::Release);
    }

    pub(crate) fn proxy_error(&self) -> Option<String> {
        self.proxy_error
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// 代理接受了 mpv 的连接；此后 `WAYLAND_SOCKET` 可以安全恢复。
    pub(crate) fn mark_wayland_connected(&self) {
        self.wayland_connected.store(true, Ordering::Release);
    }

    pub(crate) fn wayland_connected(&self) -> bool {
        self.wayland_connected.load(Ordering::Acquire)
    }
}

fn current_slot() -> &'static Mutex<Option<Arc<Session>>> {
    static CURRENT: OnceLock<Mutex<Option<Arc<Session>>>> = OnceLock::new();
    CURRENT.get_or_init(|| Mutex::new(None))
}

fn current() -> Option<Arc<Session>> {
    current_slot()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

fn replace_current(next: Option<Arc<Session>>) {
    *current_slot()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = next;
}

fn clear_current(session: &Arc<Session>) {
    let mut slot = current_slot()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if slot
        .as_ref()
        .is_some_and(|current| Arc::ptr_eq(current, session))
    {
        *slot = None;
    }
}

fn next_session_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// 内嵌播放的前置条件：mpv 的 VO 必须能连到 Wayland 代理。
pub fn available() -> Result<(), String> {
    if std::env::var("WAYLAND_DISPLAY").is_ok_and(|value| !value.is_empty()) {
        Ok(())
    } else {
        Err("当前会话没有 WAYLAND_DISPLAY".to_owned())
    }
}

/// 启动一次内嵌播放。失败时返回可直接放进提示条的中文原因。
pub fn start(
    url: &str,
    title: &str,
    cancel: Arc<Cancel>,
    extras: PlaybackExtras,
    settings: SessionSettings,
    notify: impl Fn(StartEvent) + Send + Sync + 'static,
) -> Result<u64, String> {
    if url.trim().is_empty() {
        return Err("播放链接为空".to_owned());
    }
    available().map_err(|reason| format!("内嵌播放不可用：{reason}"))?;
    let upstream = std::env::var("WAYLAND_DISPLAY")
        .map_err(|_| "内嵌播放需要 Wayland 会话（当前没有 WAYLAND_DISPLAY）".to_owned())?;

    // 先让上一条会话收手；不等待，它自己会清掉自己的登记。
    if let Some(previous) = current() {
        previous.cancel().cancel();
        previous.proxy_stop.store(true, Ordering::Release);
    }

    let (client, server) =
        UnixStream::pair().map_err(|err| format!("内嵌播放代理创建失败：{err}"))?;
    let session = Arc::new(Session::new(Arc::clone(&cancel)));
    session.set_volume(settings.volume);
    session.set_muted(settings.muted);
    session.set_speed(settings.speed);
    let proxy_handle = proxy::spawn(Arc::clone(&session), upstream, server)
        .map_err(|err| format!("内嵌播放代理线程启动失败：{err}"))?;
    let guard = WaylandSocketGuard::arm(client);
    let (control_tx, control_rx) = std::sync::mpsc::channel();
    session.set_control(control_tx);

    let id = next_session_id();
    let current_session = Arc::clone(&session);
    replace_current(Some(Arc::clone(&session)));
    let url = url.to_owned();
    let title = title.to_owned();
    eprintln!("[kichi] 内嵌播放「{title}」→ {}", mask_url(&url));

    let started = std::thread::Builder::new()
        .name("kichi-embed-mpv".into())
        .spawn(move || {
            mpv::run(
                Arc::clone(&current_session),
                &url,
                &title,
                extras,
                settings,
                notify,
                control_rx,
                guard,
            );
            current_session.proxy_stop.store(true, Ordering::Release);
            let _ = proxy_handle.join();
            clear_current(&current_session);
        });
    if let Err(err) = started {
        session.proxy_stop.store(true, Ordering::Release);
        clear_current(&session);
        return Err(format!("内嵌播放监控线程启动失败：{err}"));
    }

    Ok(id)
}

/// 屏幕上的「关闭」：结束当前内嵌会话，收尾线程照常回投 Stopped。
pub fn stop() {
    if let Some(session) = current() {
        session.cancel().cancel();
    }
}

/// 停掉当前会话并等它收尾；返回是否在超时前结束。
///
/// 改走外部播放器时必须等：mpv 线程还握着 `WAYLAND_SOCKET` 的窗口期里
/// spawn 出去的外部播放器会继承这个环境变量，连到内嵌代理而不是合成器。
pub fn stop_and_wait(timeout: std::time::Duration) -> bool {
    let Some(session) = current() else {
        return true;
    };
    session.cancel().cancel();
    session.proxy_stop.store(true, Ordering::Release);
    let deadline = std::time::Instant::now() + timeout;
    while current().is_some() {
        if std::time::Instant::now() >= deadline {
            eprintln!("[kichi] 内嵌播放器没有在 {timeout:?} 内退出");
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    true
}

/// kichi 真退出：等内嵌 mpv 收完再走，避免留下半死的线程。
pub fn shutdown_all() {
    let Some(session) = current() else {
        return;
    };
    session.send(Control::Quit);
    session.cancel().cancel();
    session.proxy_stop.store(true, Ordering::Release);
    for _ in 0..60 {
        if current().is_none() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

#[cfg(test)]
pub fn is_active() -> bool {
    current().is_some()
}

pub fn set_viewport(width: i32, height: i32, scale: f64) {
    let Some(session) = current() else {
        return;
    };
    if width <= 0 || height <= 0 || !scale.is_finite() || scale <= 0.0 {
        return;
    }
    let viewport = Viewport {
        width,
        height,
        scale,
    };
    if *session
        .viewport
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        == Some(viewport)
    {
        return;
    }
    *session
        .viewport
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(viewport);
    session.viewport_dirty.store(true, Ordering::Release);
}

pub fn toggle_pause() {
    if let Some(session) = current() {
        session.send(Control::TogglePause);
    }
}

/// 调音量（0–100）。没有会话时静默忽略：值由调用方持久化，下次开播生效。
pub fn set_volume(percent: i32) {
    let percent = percent.clamp(0, 100);
    if let Some(session) = current() {
        // 先记下再下发：界面紧接着读 `volume()` 时能立刻看到新值。
        session.set_volume(percent);
        session.send(Control::SetVolume(i64::from(percent)));
    }
}

pub fn set_muted(muted: bool) {
    if let Some(session) = current() {
        session.set_muted(muted);
        session.send(Control::SetMuted(muted));
    }
}

/// 当前会话音量（没有会话时 100）。mpv 原生键位改的音量也从这里读。
pub fn volume() -> i32 {
    current().map_or(100, |session| session.volume())
}

/// 当前会话是否静音（没有会话时 false）。
pub fn muted() -> bool {
    current().is_some_and(|session| session.muted())
}

/// 改倍速（0.5–3.0）。没有会话时静默忽略：值由设置持久化，下次开播生效。
pub fn set_speed(speed: f64) {
    if !speed.is_finite() {
        return;
    }
    let speed = sanitize_speed(speed);
    if let Some(session) = current() {
        // 先记下再下发：界面紧接着读 `speed()` 时能立刻看到新值。
        session.set_speed(speed);
        session.send(Control::SetSpeed(speed));
    }
}

/// 当前倍速（会话不存在时按原速）。
pub fn speed() -> f64 {
    current().map_or(1.0, |session| session.speed())
}

/// 切换音轨 / 字幕轨；`id < 0` 表示关闭字幕。
pub fn select_track(audio: bool, id: i64) {
    if let Some(session) = current() {
        session.send(Control::SelectTrack { audio, id });
    }
}

/// 把界面按键交给 mpv 的 input 层。只接受短字符串，控制字符一律丢弃。
pub fn keypress(key: &str) {
    let key = key.trim();
    if key.is_empty() || key.len() > 64 || key.chars().any(char::is_control) {
        return;
    }
    if let Some(session) = current() {
        session.send(Control::Keypress(key.to_owned()));
    }
}

/// 立即套用字幕样式（实时预览；持久化由调用方负责）。
pub fn set_subtitle_style(style: SubtitleStyle) {
    if let Some(session) = current() {
        session.send(Control::SubtitleStyle(style));
    }
}

/// 当前会话的轨道列表；没有会话时为空。
pub fn tracks() -> Vec<TrackInfo> {
    current().map_or_else(Vec::new, |session| session.tracks())
}

pub fn seek_absolute(seconds: f64) {
    if !seconds.is_finite() || seconds < 0.0 {
        return;
    }
    if let Some(session) = current() {
        session.send(Control::SeekAbsolute(seconds));
    }
}

pub fn is_paused() -> bool {
    current().is_some_and(|session| session.is_paused())
}

pub fn position() -> Option<f64> {
    current().and_then(|session| session.position())
}

pub fn duration() -> Option<f64> {
    current().and_then(|session| session.duration())
}

/// 有新帧等着画（C++ 侧轮询用）。
pub fn frame_pending() -> bool {
    let Some(session) = current() else {
        return false;
    };
    if !session.frame_dirty.load(Ordering::Acquire) {
        return false;
    }
    let has_frame = session
        .frame_slot
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .is_some();
    has_frame
}

/// 取走最新一帧；调用方负责 `kichi_embed_frame_free`。
pub fn take_frame() -> Option<Frame> {
    let session = current()?;
    let frame = session
        .frame_slot
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take();
    if frame.is_some() {
        session.frame_dirty.store(false, Ordering::Release);
    }
    frame
}

/// 让 mpv 通过一次性 fd 连到代理；`vo-configured` 后恢复。
struct WaylandSocketGuard {
    value: String,
    previous: Option<std::ffi::OsString>,
}

impl WaylandSocketGuard {
    fn arm(client: UnixStream) -> Self {
        // 坑：libwayland 会直接接管这个 fd，不能在这里 close。
        let fd = client.into_raw_fd();
        let value = fd.to_string();
        let _lock = env_lock();
        let previous = std::env::var_os("WAYLAND_SOCKET");
        std::env::set_var("WAYLAND_SOCKET", &value);
        Self { value, previous }
    }
}

impl Drop for WaylandSocketGuard {
    fn drop(&mut self) {
        let _lock = env_lock();
        // 新会话可能已经换了值：只恢复自己设置的那一份。
        if std::env::var_os("WAYLAND_SOCKET").as_deref() == Some(OsStr::new(&self.value)) {
            match self.previous.take() {
                Some(previous) => std::env::set_var("WAYLAND_SOCKET", previous),
                None => std::env::remove_var("WAYLAND_SOCKET"),
            }
        }
    }
}

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    static ENV: Mutex<()> = Mutex::new(());
    ENV.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}
