//! 把 mpv 的 Wayland 连接接到一个进程内代理上，截获它提交的视频帧。
//!
//! 做法参考 Mutsumi（MIT）：mpv 以为自己在跟合成器说话，实际对端是本模块。
//! 代理合成 `xdg_wm_base` / `wp_viewporter` / `wp_fractional_scale_manager_v1` /
//! `wl_shm`，转发 `wl_compositor` / `wl_subcompositor` / `zwp_linux_dmabuf_v1`，
//! 在 `wl_surface.commit` 时把 buffer 记成 [`Frame`] 交给 C++
//! `WaylandVideoSurface` 直接提交给合成器。
//!
//! 输出格式不再过滤：合成器广告的 dmabuf format / modifier 原样转给 mpv，
//! `wl_shm` 仍作为软件兜底。

mod dmabuf;
mod shm;
mod surface;
mod xdg;

use std::cell::RefCell;
use std::collections::HashMap;
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use wl_proxy::baseline::Baseline;
use wl_proxy::client::ClientHandler;
use wl_proxy::global_mapper::GlobalMapper;
use wl_proxy::object::{Object, ObjectCoreApi, ObjectRcUtils};
use wl_proxy::protocols::drm::wl_drm::WlDrm;
use wl_proxy::protocols::fractional_scale_v1::wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1;
use wl_proxy::protocols::fractional_scale_v1::wp_fractional_scale_v1::WpFractionalScaleV1;
use wl_proxy::protocols::linux_dmabuf_v1::zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1;
use wl_proxy::protocols::viewporter::wp_viewporter::WpViewporter;
use wl_proxy::protocols::wayland::wl_callback::WlCallback;
use wl_proxy::protocols::wayland::wl_compositor::WlCompositor;
use wl_proxy::protocols::wayland::wl_display::{WlDisplay, WlDisplayHandler};
use wl_proxy::protocols::wayland::wl_registry::{WlRegistry, WlRegistryHandler};
use wl_proxy::protocols::wayland::wl_shm::{WlShm, WlShmFormat};
use wl_proxy::protocols::wayland::wl_subcompositor::WlSubcompositor;
use wl_proxy::protocols::wayland::wl_surface::WlSurface;
use wl_proxy::protocols::xdg_shell::xdg_wm_base::XdgWmBase;
use wl_proxy::protocols::ObjectInterface;
use wl_proxy::state::State;

use super::{Session, Viewport};
use dmabuf::{BufferInfo as DmabufBufferInfo, DmabufHandler};
use shm::BufferInfo as ShmBufferInfo;
use surface::{
    CompositorHandler, FractionalScaleManagerHandler, SubcompositorHandler, ViewporterHandler,
};
use xdg::{ToplevelEntry, WmBaseHandler};

/// 代理线程被唤醒的最长间隔：只有释放 / 尺寸更新这类小事件，
/// 8 ms 对观看没有可感知影响，也不值得为此引入 epoll 词表外的 fd。
const POLL_MS: i32 = 8;

pub(super) enum ProxyEvent {
    ReleaseBuffer(u64),
    FrameDone { batch_id: u64, time_ms: u32 },
}

/// 一帧视频。dmabuf 与 wl_shm 共用这一个结构。
pub(crate) struct Frame {
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// DRM fourcc；wl_shm 帧同样用 ARGB8888 / XRGB8888 的 fourcc
    pub(crate) fourcc: u32,
    pub(crate) modifier: u64,
    pub(crate) planes: Vec<Plane>,
    /// wl_shm 帧的内存副本（已把行距压成 `width * 4`，alpha 强制 255）
    pub(crate) pixels: Option<Vec<u8>>,
    /// `Some` 时这一帧还占着 mpv 的 buffer，Drop 时要 `wl_buffer.release`
    buffer_id: Option<u64>,
    /// 这一帧提交时附带的 frame callback；显示后要回 `done`
    callback_batch: Option<u64>,
    event_tx: Sender<ProxyEvent>,
    displayed: bool,
}

pub(crate) struct Plane {
    pub(crate) fd: OwnedFd,
    pub(crate) offset: u32,
    pub(crate) stride: u32,
}

impl Frame {
    pub(super) fn shm(
        width: u32,
        height: u32,
        fourcc: u32,
        pixels: Vec<u8>,
        event_tx: Sender<ProxyEvent>,
    ) -> Self {
        Self {
            width,
            height,
            fourcc,
            modifier: 0,
            planes: Vec::new(),
            pixels: Some(pixels),
            buffer_id: None,
            callback_batch: None,
            event_tx,
            displayed: false,
        }
    }

    pub(crate) fn set_callback_batch(&mut self, batch_id: u64) {
        self.callback_batch = Some(batch_id);
    }

    /// 把 callback 挂到还没显示的帧上；已经有 callback 或已显示时返回 `false`。
    pub(crate) fn attach_callback_batch(&mut self, batch_id: u64) -> bool {
        if self.callback_batch.is_some() || self.displayed {
            return false;
        }
        self.callback_batch = Some(batch_id);
        true
    }

    pub(crate) fn mark_displayed(&mut self) -> bool {
        if self.displayed {
            return false;
        }
        self.displayed = true;
        if let Some(batch_id) = self.callback_batch.take() {
            let _ = self.event_tx.send(ProxyEvent::FrameDone {
                batch_id,
                time_ms: 0,
            });
        }
        true
    }
}

impl Drop for Frame {
    fn drop(&mut self) {
        // 还没显示就被换掉 / 丢弃：也要回 done，否则 mpv 会一直等这一帧。
        if let Some(batch_id) = self.callback_batch.take() {
            if !self.displayed {
                let _ = self.event_tx.send(ProxyEvent::FrameDone {
                    batch_id,
                    time_ms: 0,
                });
            }
        }
        if let Some(buffer_id) = self.buffer_id.take() {
            let _ = self.event_tx.send(ProxyEvent::ReleaseBuffer(buffer_id));
        }
    }
}

pub(crate) struct SharedState {
    pub(crate) buffer_info: HashMap<u64, DmabufBufferInfo>,
    pub(crate) shm_buffer_info: HashMap<u64, ShmBufferInfo>,
    pub(crate) event_tx: Sender<ProxyEvent>,
    pub(crate) frame_callbacks: HashMap<u64, Vec<Rc<WlCallback>>>,
    pub(crate) next_callback_batch: u64,
    pub(crate) toplevels: Vec<ToplevelEntry>,
    pub(crate) viewport: Option<Viewport>,
    configure_serial: u32,
    pub(crate) fractional_scales: Vec<Rc<WpFractionalScaleV1>>,
    pub(crate) surfaces: Vec<Rc<WlSurface>>,
    /// 最新一帧；Qt 侧取走后由它负责释放
    pub(crate) frame_slot: Arc<Mutex<Option<Frame>>>,
    pub(crate) frame_dirty: Arc<AtomicBool>,
}

impl SharedState {
    pub(crate) fn configure_toplevels(&mut self) {
        let Some(viewport) = self.viewport else {
            return;
        };
        let mut serial = self.configure_serial;
        for entry in &self.toplevels {
            if !entry.has_committed_buffer {
                continue;
            }
            entry
                .toplevel
                .send_configure(viewport.width, viewport.height, &[]);
            entry.xdg_surface.send_configure(serial);
            serial = serial.wrapping_add(1);
        }
        self.configure_serial = serial;
    }

    pub(crate) fn surface_committed_buffer(&mut self, surface_id: u64) {
        let Some(entry) = self
            .toplevels
            .iter_mut()
            .find(|entry| entry.surface_id == surface_id)
        else {
            return;
        };
        if entry.has_committed_buffer {
            return;
        }
        entry.has_committed_buffer = true;
        self.configure_toplevels();
    }

    pub(crate) fn update_fractional_scales(&mut self, scale_120: u32) {
        for scale in &self.fractional_scales {
            scale.send_preferred_scale(scale_120);
        }
        let buffer_scale = (scale_120 as f64 / 120.0).ceil() as i32;
        for surface in &self.surfaces {
            if surface.version() >= 6 {
                surface.send_preferred_buffer_scale(buffer_scale);
            }
        }
    }

    pub(crate) fn take_callbacks(&mut self, callbacks: Vec<Rc<WlCallback>>) -> Option<u64> {
        if callbacks.is_empty() {
            return None;
        }
        let batch_id = self.next_callback_batch;
        self.next_callback_batch = batch_id.wrapping_add(1);
        self.frame_callbacks.insert(batch_id, callbacks);
        Some(batch_id)
    }
}

/// 启动代理线程；返回后 mpv 就可以通过 `WAYLAND_SOCKET` 连上来了。
pub(super) fn spawn(
    session: Arc<Session>,
    upstream: String,
    server: UnixStream,
) -> std::io::Result<std::thread::JoinHandle<()>> {
    std::thread::Builder::new()
        .name("kichi-wl-proxy".into())
        .spawn(move || run(session, upstream, server))
}

fn run(session: Arc<Session>, upstream: String, server: UnixStream) {
    let state = match State::builder(Baseline::ALL_OF_THEM)
        .with_server_display_name(&upstream)
        .build()
    {
        Ok(state) => state,
        Err(err) => {
            session.fail_proxy(format!("代理创建失败：{err}"));
            return;
        }
    };
    // 析构器必须活到会话结束：mpv 的显示连接断开时不应顺手毁掉整个代理。
    let _destructor = state.create_destructor();
    let server: OwnedFd = server.into();
    // `add_client` 只是把 socketpair 的这端登记成端点，并不代表 mpv 已经连上来。
    // 真正建连要等客户端发出第一个请求（registry），那时才允许恢复
    // WAYLAND_SOCKET，否则 mpv 会改连真实合成器，画面不进 kichi。
    let client = match state.add_client(&Rc::new(server)) {
        Ok(client) => client,
        Err(err) => {
            session.fail_proxy(format!("代理接入播放器失败：{err}"));
            return;
        }
    };
    client.set_handler(ClientHandlerImpl);

    let (event_tx, event_rx) = std::sync::mpsc::channel();
    let shared = Rc::new(RefCell::new(SharedState {
        buffer_info: HashMap::new(),
        shm_buffer_info: HashMap::new(),
        event_tx,
        frame_callbacks: HashMap::new(),
        next_callback_batch: 1,
        toplevels: Vec::new(),
        viewport: *session.viewport.lock().unwrap_or_else(|e| e.into_inner()),
        configure_serial: 1,
        fractional_scales: Vec::new(),
        surfaces: Vec::new(),
        frame_slot: Arc::clone(&session.frame_slot),
        frame_dirty: Arc::clone(&session.frame_dirty),
    }));
    client.display().set_handler(DisplayHandler {
        state: Rc::clone(&shared),
        session: Arc::clone(&session),
    });

    loop {
        if session.proxy_stop.load(Ordering::Acquire) || !state.is_not_destroyed() {
            break;
        }
        while let Ok(event) = event_rx.try_recv() {
            handle_event(&shared, event);
        }
        if session.viewport_dirty.swap(false, Ordering::AcqRel) {
            let viewport = *session.viewport.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(viewport) = viewport {
                update_viewport(&shared, viewport);
            }
        }
        if let Err(err) = state.dispatch_available() {
            session.fail_proxy(format!("代理分发失败：{err}"));
            break;
        }
        if let Err(err) = state.before_poll() {
            session.fail_proxy(format!("代理刷新失败：{err}"));
            break;
        }
        if let Err(err) = poll_once(state.poll_fd().as_raw_fd()) {
            session.fail_proxy(format!("代理等待失败：{err}"));
            break;
        }
    }
    session.proxy_stop.store(true, Ordering::Release);
}

fn poll_once(fd: std::os::fd::RawFd) -> std::io::Result<()> {
    let mut pfd = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    let result = unsafe { libc::poll(&mut pfd, 1, POLL_MS) };
    if result < 0 {
        let err = std::io::Error::last_os_error();
        if err.kind() != std::io::ErrorKind::Interrupted {
            return Err(err);
        }
    }
    Ok(())
}

fn handle_event(shared: &Rc<RefCell<SharedState>>, event: ProxyEvent) {
    match event {
        ProxyEvent::ReleaseBuffer(buffer_id) => {
            let shared = shared.borrow();
            if let Some(info) = shared.buffer_info.get(&buffer_id) {
                info.buffer.send_release();
            } else if let Some(info) = shared.shm_buffer_info.get(&buffer_id) {
                info.buffer.send_release();
            }
        }
        ProxyEvent::FrameDone { batch_id, time_ms } => {
            if let Some(callbacks) = shared.borrow_mut().frame_callbacks.remove(&batch_id) {
                for callback in callbacks {
                    callback.send_done(time_ms);
                    callback.delete_id();
                }
            }
        }
    }
}

fn update_viewport(shared: &Rc<RefCell<SharedState>>, viewport: Viewport) {
    let scale_120 = (viewport.scale * 120.0).round().max(1.0) as u32;
    let mut shared = shared.borrow_mut();
    shared.viewport = Some(viewport);
    shared.update_fractional_scales(scale_120);
    shared.configure_toplevels();
}

struct DisplayHandler {
    state: Rc<RefCell<SharedState>>,
    session: Arc<Session>,
}

impl WlDisplayHandler for DisplayHandler {
    fn handle_get_registry(&mut self, slf: &Rc<WlDisplay>, registry: &Rc<WlRegistry>) {
        // 客户端的第一个请求：libwayland 早已读过并接管 WAYLAND_SOCKET，
        // 从这里开始恢复环境变量才安全。
        self.session.mark_wayland_connected();
        slf.send_get_registry(registry);
        let mut mapper = GlobalMapper::default();
        let xdg_wm_base = mapper.add_synthetic_global(registry, ObjectInterface::XdgWmBase, 4);
        let viewporter = mapper.add_synthetic_global(registry, ObjectInterface::WpViewporter, 1);
        let fractional_scale =
            mapper.add_synthetic_global(registry, ObjectInterface::WpFractionalScaleManagerV1, 1);
        let shm = mapper.add_synthetic_global(registry, ObjectInterface::WlShm, 1);
        registry.set_handler(RegistryHandler {
            mapper,
            state: Rc::clone(&self.state),
            xdg_wm_base,
            viewporter,
            fractional_scale,
            shm,
        });
    }
}

struct RegistryHandler {
    mapper: GlobalMapper,
    state: Rc<RefCell<SharedState>>,
    xdg_wm_base: u32,
    viewporter: u32,
    fractional_scale: u32,
    shm: u32,
}

impl WlRegistryHandler for RegistryHandler {
    fn handle_global(
        &mut self,
        slf: &Rc<WlRegistry>,
        name: u32,
        interface: ObjectInterface,
        version: u32,
    ) {
        if matches!(
            interface,
            ObjectInterface::XdgWmBase
                | ObjectInterface::WpViewporter
                | ObjectInterface::WpFractionalScaleManagerV1
                | ObjectInterface::WlShm
                // mpv 的 surface 是代理合成的；色彩描述由 kichi 的
                // Wayland presenter 直接设到真 surface 上，不能让 mpv
                // 把 image description 发到一个不存在的 server surface。
                | ObjectInterface::WpColorManagerV1
                | ObjectInterface::WpColorRepresentationManagerV1
        ) {
            self.mapper.ignore_global(name);
        } else if interface == ObjectInterface::ZwpLinuxDmabufV1 {
            // v4 起走 feedback；格式表原样转发，版本再高也没有新语义
            self.mapper
                .forward_global(slf, name, interface, version.min(4));
        } else {
            self.mapper.forward_global(slf, name, interface, version);
        }
    }

    fn handle_global_remove(&mut self, slf: &Rc<WlRegistry>, name: u32) {
        self.mapper.forward_global_remove(slf, name);
    }

    fn handle_bind(&mut self, slf: &Rc<WlRegistry>, name: u32, id: Rc<dyn Object>) {
        if name == self.xdg_wm_base {
            let wm_base = id.downcast::<XdgWmBase>();
            wm_base.set_forward_to_server(false);
            wm_base.set_handler(WmBaseHandler {
                state: Rc::clone(&self.state),
            });
        } else if name == self.viewporter {
            let viewporter = id.downcast::<WpViewporter>();
            viewporter.set_forward_to_server(false);
            viewporter.set_handler(ViewporterHandler);
        } else if name == self.fractional_scale {
            let manager = id.downcast::<WpFractionalScaleManagerV1>();
            manager.set_forward_to_server(false);
            manager.set_handler(FractionalScaleManagerHandler {
                state: Rc::clone(&self.state),
            });
        } else if name == self.shm {
            let shm = id.downcast::<WlShm>();
            shm.set_forward_to_server(false);
            shm.set_handler(shm::ShmHandler {
                state: Rc::clone(&self.state),
            });
            shm.send_format(WlShmFormat::ARGB8888);
            shm.send_format(WlShmFormat::XRGB8888);
        } else {
            let compositor = id.try_downcast::<WlCompositor>();
            let subcompositor = id.try_downcast::<WlSubcompositor>();
            let dmabuf = id.try_downcast::<ZwpLinuxDmabufV1>();
            let drm = id.try_downcast::<WlDrm>();

            self.mapper.forward_bind(slf, name, &id);

            if let Some(compositor) = compositor {
                compositor.set_handler(CompositorHandler {
                    state: Rc::clone(&self.state),
                });
            } else if let Some(subcompositor) = subcompositor {
                subcompositor.set_handler(SubcompositorHandler);
            } else if let Some(dmabuf) = dmabuf {
                dmabuf.set_handler(DmabufHandler {
                    state: Rc::clone(&self.state),
                });
            } else if let Some(drm) = drm {
                drm.set_handler(dmabuf::DrmHandler {
                    state: Rc::clone(&self.state),
                });
            }
        }
    }
}

struct ClientHandlerImpl;

impl ClientHandler for ClientHandlerImpl {
    fn disconnected(self: Box<Self>) {}
}
