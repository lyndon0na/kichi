//! 合成的 `xdg_wm_base`：mpv 以为自己在开一个普通窗口。
//!
//! 窗口不存在，`xdg_toplevel.configure(0, 0)` 只是把 mpv 从「等 configure」
//! 里放出来；等它第一次 attach buffer 后，再按 QQuickItem 的实际逻辑尺寸补一次
//! 真实 configure，mpv 就会按这个尺寸出帧。

use std::cell::RefCell;
use std::rc::Rc;

use wl_proxy::object::ObjectCoreApi;
use wl_proxy::protocols::wayland::wl_surface::WlSurface;
use wl_proxy::protocols::xdg_shell::xdg_surface::{XdgSurface, XdgSurfaceHandler};
use wl_proxy::protocols::xdg_shell::xdg_toplevel::{XdgToplevel, XdgToplevelHandler};
use wl_proxy::protocols::xdg_shell::xdg_wm_base::{XdgWmBase, XdgWmBaseHandler};

use super::SharedState;

pub(crate) struct ToplevelEntry {
    pub(crate) xdg_surface: Rc<XdgSurface>,
    pub(crate) toplevel: Rc<XdgToplevel>,
    pub(crate) surface_id: u64,
    pub(crate) has_committed_buffer: bool,
}

pub(crate) struct WmBaseHandler {
    pub(crate) state: Rc<RefCell<SharedState>>,
}

impl XdgWmBaseHandler for WmBaseHandler {
    fn handle_destroy(&mut self, slf: &Rc<XdgWmBase>) {
        slf.delete_id();
    }

    fn handle_get_xdg_surface(
        &mut self,
        _slf: &Rc<XdgWmBase>,
        id: &Rc<XdgSurface>,
        surface: &Rc<WlSurface>,
    ) {
        id.set_forward_to_server(false);
        id.set_handler(XdgSurfaceHandlerImpl {
            state: Rc::clone(&self.state),
            surface_id: surface.unique_id(),
        });
    }

    fn handle_pong(&mut self, _slf: &Rc<XdgWmBase>, _serial: u32) {}
}

struct XdgSurfaceHandlerImpl {
    state: Rc<RefCell<SharedState>>,
    surface_id: u64,
}

impl XdgSurfaceHandler for XdgSurfaceHandlerImpl {
    fn handle_destroy(&mut self, slf: &Rc<XdgSurface>) {
        let surface_id = slf.unique_id();
        self.state
            .borrow_mut()
            .toplevels
            .retain(|entry| entry.xdg_surface.unique_id() != surface_id);
        slf.delete_id();
    }

    fn handle_get_toplevel(&mut self, slf: &Rc<XdgSurface>, id: &Rc<XdgToplevel>) {
        id.set_forward_to_server(false);
        id.set_handler(XdgToplevelHandlerImpl {
            state: Rc::clone(&self.state),
        });

        let mut state = self.state.borrow_mut();
        let serial = state.configure_serial;
        state.configure_serial = serial.wrapping_add(1);

        id.send_configure_bounds(0, 0);
        id.send_configure(0, 0, &[]);
        slf.send_configure(serial);

        state.toplevels.push(ToplevelEntry {
            xdg_surface: Rc::clone(slf),
            toplevel: Rc::clone(id),
            surface_id: self.surface_id,
            has_committed_buffer: false,
        });
    }

    fn handle_ack_configure(&mut self, _slf: &Rc<XdgSurface>, _serial: u32) {}
}

struct XdgToplevelHandlerImpl {
    state: Rc<RefCell<SharedState>>,
}

impl XdgToplevelHandler for XdgToplevelHandlerImpl {
    fn handle_destroy(&mut self, slf: &Rc<XdgToplevel>) {
        let toplevel_id = slf.unique_id();
        self.state
            .borrow_mut()
            .toplevels
            .retain(|entry| entry.toplevel.unique_id() != toplevel_id);
        slf.delete_id();
    }
}
