//! wl_surface / 合成器相关对象的本地实现。
//!
//! mpv 的 video output 只用一个 surface：第一次 attach 到 buffer 后，
//! 代理才把真实尺寸通过 `xdg_toplevel.configure` 告诉它。之后每次
//! `wl_surface.commit` 都把 buffer 记成 [`Frame`] 放进共享槽，由 Qt 侧取走。

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::Ordering;

use wl_proxy::object::ObjectCoreApi;
use wl_proxy::protocols::fractional_scale_v1::wp_fractional_scale_manager_v1::{
    WpFractionalScaleManagerV1, WpFractionalScaleManagerV1Handler,
};
use wl_proxy::protocols::fractional_scale_v1::wp_fractional_scale_v1::{
    WpFractionalScaleV1, WpFractionalScaleV1Handler,
};
use wl_proxy::protocols::viewporter::wp_viewport::{WpViewport, WpViewportHandler};
use wl_proxy::protocols::viewporter::wp_viewporter::{WpViewporter, WpViewporterHandler};
use wl_proxy::protocols::wayland::wl_buffer::WlBuffer;
use wl_proxy::protocols::wayland::wl_callback::WlCallback;
use wl_proxy::protocols::wayland::wl_compositor::{WlCompositor, WlCompositorHandler};
use wl_proxy::protocols::wayland::wl_subcompositor::{WlSubcompositor, WlSubcompositorHandler};
use wl_proxy::protocols::wayland::wl_subsurface::{WlSubsurface, WlSubsurfaceHandler};
use wl_proxy::protocols::wayland::wl_surface::{WlSurface, WlSurfaceHandler};

use super::{Frame, ProxyEvent, SharedState};

pub(crate) struct CompositorHandler {
    pub(crate) state: Rc<RefCell<SharedState>>,
}

impl WlCompositorHandler for CompositorHandler {
    fn handle_create_surface(&mut self, _slf: &Rc<WlCompositor>, id: &Rc<WlSurface>) {
        id.set_forward_to_server(false);
        id.set_handler(InitialSurfaceHandler(SurfaceHandler {
            shared: Rc::clone(&self.state),
            pending_buffer: None,
            pending_callbacks: Vec::new(),
        }));

        let mut state = self.state.borrow_mut();
        if id.version() >= 6 {
            let scale = state.viewport.map_or(1.0, |viewport| viewport.scale);
            id.send_preferred_buffer_scale(scale.ceil() as i32);
        }
        state.surfaces.push(Rc::clone(id));
    }
}

pub(crate) struct SubcompositorHandler;

impl WlSubcompositorHandler for SubcompositorHandler {
    fn handle_get_subsurface(
        &mut self,
        _slf: &Rc<WlSubcompositor>,
        id: &Rc<WlSubsurface>,
        _surface: &Rc<WlSurface>,
        _parent: &Rc<WlSurface>,
    ) {
        id.set_forward_to_server(false);
        id.set_handler(SubsurfaceHandler);
    }
}

struct SubsurfaceHandler;

impl WlSubsurfaceHandler for SubsurfaceHandler {
    fn handle_destroy(&mut self, slf: &Rc<WlSubsurface>) {
        slf.delete_id();
    }
}

pub(crate) struct ViewporterHandler;

impl WpViewporterHandler for ViewporterHandler {
    fn handle_destroy(&mut self, slf: &Rc<WpViewporter>) {
        slf.delete_id();
    }

    fn handle_get_viewport(
        &mut self,
        _slf: &Rc<WpViewporter>,
        id: &Rc<WpViewport>,
        _surface: &Rc<WlSurface>,
    ) {
        id.set_forward_to_server(false);
        id.set_handler(ViewportHandler);
    }
}

struct ViewportHandler;

impl WpViewportHandler for ViewportHandler {
    fn handle_destroy(&mut self, slf: &Rc<WpViewport>) {
        slf.delete_id();
    }
}

pub(crate) struct FractionalScaleManagerHandler {
    pub(crate) state: Rc<RefCell<SharedState>>,
}

impl WpFractionalScaleManagerV1Handler for FractionalScaleManagerHandler {
    fn handle_destroy(&mut self, slf: &Rc<WpFractionalScaleManagerV1>) {
        slf.delete_id();
    }

    fn handle_get_fractional_scale(
        &mut self,
        _slf: &Rc<WpFractionalScaleManagerV1>,
        id: &Rc<WpFractionalScaleV1>,
        _surface: &Rc<WlSurface>,
    ) {
        id.set_forward_to_server(false);
        let mut state = self.state.borrow_mut();
        let scale = state.viewport.map_or(1.0, |viewport| viewport.scale);
        let scale_120 = (scale * 120.0).round().max(1.0) as u32;
        id.send_preferred_scale(scale_120);
        id.set_handler(FractionalScaleHandler {
            state: Rc::clone(&self.state),
        });
        state.fractional_scales.push(Rc::clone(id));
    }
}

struct FractionalScaleHandler {
    state: Rc<RefCell<SharedState>>,
}

impl WpFractionalScaleV1Handler for FractionalScaleHandler {
    fn handle_destroy(&mut self, slf: &Rc<WpFractionalScaleV1>) {
        self.state
            .borrow_mut()
            .fractional_scales
            .retain(|scale| !Rc::ptr_eq(scale, slf));
        slf.delete_id();
    }
}

struct InitialSurfaceHandler(SurfaceHandler);

impl WlSurfaceHandler for InitialSurfaceHandler {
    fn handle_destroy(&mut self, slf: &Rc<WlSurface>) {
        self.0.handle_destroy(slf);
    }

    fn handle_attach(
        &mut self,
        slf: &Rc<WlSurface>,
        buffer: Option<&Rc<WlBuffer>>,
        x: i32,
        y: i32,
    ) {
        self.0.handle_attach(slf, buffer, x, y);
    }

    fn handle_frame(&mut self, slf: &Rc<WlSurface>, callback: &Rc<WlCallback>) {
        self.0.handle_frame(slf, callback);
    }

    fn handle_commit(&mut self, slf: &Rc<WlSurface>) {
        if !matches!(self.0.pending_buffer, Some(Some(_))) {
            self.0.handle_commit(slf);
            return;
        }
        self.0
            .shared
            .borrow_mut()
            .surface_committed_buffer(slf.unique_id());
        self.0.handle_commit(slf);
        slf.set_handler(SurfaceHandler {
            shared: Rc::clone(&self.0.shared),
            pending_buffer: self.0.pending_buffer.take(),
            pending_callbacks: std::mem::take(&mut self.0.pending_callbacks),
        });
    }
}

struct SurfaceHandler {
    shared: Rc<RefCell<SharedState>>,
    pending_buffer: Option<Option<Rc<WlBuffer>>>,
    pending_callbacks: Vec<Rc<WlCallback>>,
}

impl SurfaceHandler {
    fn commit_frame(&mut self, buffer: &Rc<WlBuffer>, batch: Option<u64>) -> Option<Frame> {
        let buffer_id = buffer.unique_id();
        let shared = self.shared.borrow();
        if let Some(info) = shared.buffer_info.get(&buffer_id) {
            let mut frame = match info.to_frame(buffer_id, shared.event_tx.clone()) {
                Some(frame) => frame,
                None => {
                    buffer.send_release();
                    return None;
                }
            };
            if let Some(batch) = batch {
                frame.set_callback_batch(batch);
            }
            return Some(frame);
        }
        if let Some(info) = shared.shm_buffer_info.get(&buffer_id) {
            let frame = info.snapshot().map(|(width, height, fourcc, pixels)| {
                let mut frame = Frame::shm(width, height, fourcc, pixels, shared.event_tx.clone());
                if let Some(batch) = batch {
                    frame.set_callback_batch(batch);
                }
                frame
            });
            // 内存已经复制，buffer 可以立刻还给 mpv；帧的 buffer_id 为空，
            // 之后从槽里换掉时不会再发一次 release。
            buffer.send_release();
            return frame;
        }
        buffer.send_release();
        None
    }
}

impl WlSurfaceHandler for SurfaceHandler {
    fn handle_destroy(&mut self, slf: &Rc<WlSurface>) {
        let surface_id = slf.unique_id();
        {
            let mut shared = self.shared.borrow_mut();
            shared
                .surfaces
                .retain(|surface| surface.unique_id() != surface_id);
            shared.presentation.discard_surface(surface_id);
        }
        slf.delete_id();
    }

    fn handle_attach(
        &mut self,
        _slf: &Rc<WlSurface>,
        buffer: Option<&Rc<WlBuffer>>,
        _x: i32,
        _y: i32,
    ) {
        self.pending_buffer = Some(buffer.map(Rc::clone));
    }

    fn handle_frame(&mut self, _slf: &Rc<WlSurface>, callback: &Rc<WlCallback>) {
        self.pending_callbacks.push(Rc::clone(callback));
    }

    fn handle_commit(&mut self, slf: &Rc<WlSurface>) {
        let callbacks = std::mem::take(&mut self.pending_callbacks);
        let pending = self.pending_buffer.take();

        // 只有 attach 了新 buffer 才产生新帧；没有新 buffer 的提交
        // 也可能带着 frame callback（mpv 偶发的 callback-only commit），
        // 这时把 callback 挂到还没显示的那一帧上，或立刻回 done。
        let batch = self.shared.borrow_mut().take_callbacks(callbacks);
        let frame = match pending {
            Some(Some(buffer)) => self.commit_frame(&buffer, batch),
            _ => None,
        };
        // mpv 在 commit 前请求的 presentation feedback：新帧带 token 交给
        // C++，等真实 presented；没有新帧的 commit 内容没有变化，回 discarded。
        let token = self
            .shared
            .borrow_mut()
            .presentation
            .take_token(slf.unique_id());

        if let Some(mut frame) = frame {
            if let Some(token) = token {
                frame.set_presentation_token(token);
            }
            store_frame(&self.shared, frame);
            return;
        }
        if let Some(token) = token {
            self.shared.borrow_mut().presentation.discard(token);
        }
        if let Some(batch) = batch {
            let attached = self
                .shared
                .borrow()
                .frame_slot
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .as_mut()
                .is_some_and(|frame| frame.attach_callback_batch(batch));
            if !attached {
                let shared = self.shared.borrow();
                let _ = shared.event_tx.send(ProxyEvent::FrameDone {
                    batch_id: batch,
                    time_ms: 0,
                });
            }
        }
    }
}

fn store_frame(shared: &Rc<RefCell<SharedState>>, frame: Frame) {
    let shared = shared.borrow();
    let mut slot = shared
        .frame_slot
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    // 换下来的旧帧在这里 Drop：向事件队列发 release（以及未显示的 done）。
    *slot = Some(frame);
    shared.frame_dirty.store(true, Ordering::Release);
}
