//! 合成 `wp_presentation`：把 C++ 在真实 child surface 上收到的
//! presented / discarded 转给 mpv。
//!
//! mpv 的 surface 是代理造的，合成器不知道它；如果直接把 mpv 的
//! `wp_presentation.feedback` 转发给服务端，会引用一个不存在的 surface。
//! 因此真 global 对 mpv 隐藏，代理广告一个同接口的 manager；每个 feedback
//! 分配一个 token，随帧交给 C++，只有真实事件回来才兑现。队列空 / 没有
//! 事件时宁可不发，不能用 buffer release 时间冒充 presentation 时间。

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;

use wl_proxy::object::ObjectCoreApi;
use wl_proxy::protocols::presentation_time::wp_presentation::{
    WpPresentation, WpPresentationHandler,
};
use wl_proxy::protocols::presentation_time::wp_presentation_feedback::{
    WpPresentationFeedback, WpPresentationFeedbackKind,
};
use wl_proxy::protocols::wayland::wl_surface::WlSurface;

use super::SharedState;

/// 一个等待真实 presented / discarded 的合成 feedback。
struct PendingFeedback {
    callback: Rc<WpPresentationFeedback>,
}

/// 合成 presentation 的全部状态；和代理的帧 / surface 生命周期一起走。
pub(crate) struct PresentationState {
    /// token → 合成对象
    pending: HashMap<u64, PendingFeedback>,
    /// 每个假 surface 上尚未与 commit 配对的 feedback，按请求顺序兑现
    order: HashMap<u64, VecDeque<u64>>,
    next_token: u64,
}

impl PresentationState {
    pub(crate) fn new() -> Self {
        Self {
            pending: HashMap::new(),
            order: HashMap::new(),
            next_token: 1,
        }
    }

    /// 把一个新 feedback 排到对应假 surface 的队尾；token 从 1 开始，0 保留。
    fn queue(&mut self, surface_id: u64, callback: Rc<WpPresentationFeedback>) -> u64 {
        let token = self.next_token.max(1);
        self.next_token = token.wrapping_add(1).max(1);
        self.pending.insert(token, PendingFeedback { callback });
        self.order.entry(surface_id).or_default().push_back(token);
        token
    }

    /// 取出该 surface 最早一个等待 commit 的 feedback。
    pub(crate) fn take_token(&mut self, surface_id: u64) -> Option<u64> {
        let queue = self.order.get_mut(&surface_id)?;
        let token = queue.pop_front()?;
        if queue.is_empty() {
            self.order.remove(&surface_id);
        }
        Some(token)
    }

    /// 真实 presented / discarded 到了：取走 feedback 并清掉排队引用。
    fn finish(&mut self, token: u64) -> Option<PendingFeedback> {
        let pending = self.pending.remove(&token);
        for queue in self.order.values_mut() {
            queue.retain(|queued| *queued != token);
        }
        self.order.retain(|_, queue| !queue.is_empty());
        pending
    }

    /// frame 在提交前被换掉 / 没有对应 feedback：立刻回 discarded。
    pub(crate) fn discard(&mut self, token: u64) {
        if let Some(pending) = self.finish(token) {
            pending.callback.send_discarded();
        }
    }

    /// surface 销毁：这个假 surface 上还没兑现的 feedback 全部 discarded。
    pub(crate) fn discard_surface(&mut self, surface_id: u64) {
        let Some(tokens) = self.order.remove(&surface_id) else {
            return;
        };
        for token in tokens {
            if let Some(pending) = self.pending.remove(&token) {
                pending.callback.send_discarded();
            }
        }
    }
}

pub(super) struct ManagerHandler {
    pub(super) state: Rc<RefCell<SharedState>>,
}

impl WpPresentationHandler for ManagerHandler {
    fn handle_destroy(&mut self, slf: &Rc<WpPresentation>) {
        slf.delete_id();
    }

    fn handle_feedback(
        &mut self,
        _slf: &Rc<WpPresentation>,
        surface: &Rc<WlSurface>,
        callback: &Rc<WpPresentationFeedback>,
    ) {
        // callback 是客户端本地对象，没有 server id 可转发。
        callback.set_forward_to_server(false);
        self.state
            .borrow_mut()
            .presentation
            .queue(surface.unique_id(), Rc::clone(callback));
    }
}

/// C++ `KichiPresentationEvent` 的镜像；字段顺序必须与 embed_ffi.h 一致。
#[repr(C)]
struct FfiPresentationEvent {
    token: u64,
    presented: u32,
    tv_sec_hi: u32,
    tv_sec_lo: u32,
    tv_nsec: u32,
    refresh: u32,
    seq_hi: u32,
    seq_lo: u32,
    flags: u32,
}

impl FfiPresentationEvent {
    fn empty() -> Self {
        Self {
            token: 0,
            presented: 0,
            tv_sec_hi: 0,
            tv_sec_lo: 0,
            tv_nsec: 0,
            refresh: 0,
            seq_hi: 0,
            seq_lo: 0,
            flags: 0,
        }
    }
}

extern "C" {
    /// 真实 `wp_presentation.clock_id`；C++ 还没绑定 / 不可用时返回 1
    /// （CLOCK_MONOTONIC，KWin 默认）。
    fn kichi_embed_presentation_clock() -> u32;
    /// 取一条 C++ 侧的真实 presentation 事件；没有时返回 false。
    fn kichi_embed_presentation_take(event: *mut FfiPresentationEvent) -> bool;
}

pub(super) fn clock_id() -> u32 {
    // SAFETY: 只读一个 C++ 侧原子值，无指针参数。
    let clock = unsafe { kichi_embed_presentation_clock() };
    if clock == 0 {
        1
    } else {
        clock
    }
}

/// 把 C++ 队列里的真实事件兑现给 mpv。没有匹配 feedback 的事件直接丢弃。
pub(super) fn drain(state: &Rc<RefCell<SharedState>>) {
    loop {
        let mut event = FfiPresentationEvent::empty();
        // SAFETY: event 指向本栈上的有效、字段兼容的 C 结构。
        if !unsafe { kichi_embed_presentation_take(&mut event) } {
            break;
        }
        let Some(pending) = state.borrow_mut().presentation.finish(event.token) else {
            continue;
        };
        if event.presented != 0 {
            pending.callback.send_presented(
                event.tv_sec_hi,
                event.tv_sec_lo,
                event.tv_nsec,
                event.refresh,
                event.seq_hi,
                event.seq_lo,
                WpPresentationFeedbackKind(event.flags),
            );
        } else {
            pending.callback.send_discarded();
        }
    }
}
