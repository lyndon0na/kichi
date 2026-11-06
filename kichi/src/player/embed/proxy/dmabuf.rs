//! dmabuf 协商与 buffer 登记。
//!
//! 格式白名单只有「ARGB8888 / XRGB8888 + LINEAR」：单平面、8-bit、
//! Qt 侧可以直接 `eglCreateImageKHR` 成普通 `GL_TEXTURE_2D`，不需要
//! external-OES 采样器。mpv 的 gpu-next 会把解码结果转成这两种再输出。

use std::cell::RefCell;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::rc::Rc;

use wl_proxy::object::ObjectCoreApi;
use wl_proxy::protocols::drm::wl_drm::{WlDrm, WlDrmHandler};
use wl_proxy::protocols::linux_dmabuf_v1::zwp_linux_buffer_params_v1::{
    ZwpLinuxBufferParamsV1, ZwpLinuxBufferParamsV1Flags, ZwpLinuxBufferParamsV1Handler,
};
use wl_proxy::protocols::linux_dmabuf_v1::zwp_linux_dmabuf_feedback_v1::{
    ZwpLinuxDmabufFeedbackV1, ZwpLinuxDmabufFeedbackV1Handler, ZwpLinuxDmabufFeedbackV1TrancheFlags,
};
use wl_proxy::protocols::linux_dmabuf_v1::zwp_linux_dmabuf_v1::{
    ZwpLinuxDmabufV1, ZwpLinuxDmabufV1Handler,
};
use wl_proxy::protocols::wayland::wl_buffer::{WlBuffer, WlBufferHandler};
use wl_proxy::protocols::wayland::wl_surface::WlSurface;

use super::{Frame, Plane, ProxyEvent, SharedState};

pub(crate) const FOURCC_ARGB8888: u32 = 0x3432_5241; // 'AR24'
pub(crate) const FOURCC_XRGB8888: u32 = 0x3432_5258; // 'XR24'
const DRM_FORMAT_MOD_LINEAR: u64 = 0;

fn allowed(format: u32, modifier: u64) -> bool {
    modifier == DRM_FORMAT_MOD_LINEAR && matches!(format, FOURCC_ARGB8888 | FOURCC_XRGB8888)
}

struct StoredPlane {
    fd: OwnedFd,
    offset: u32,
    stride: u32,
}

pub(crate) struct BufferInfo {
    pub(crate) buffer: Rc<WlBuffer>,
    planes: Vec<StoredPlane>,
    width: u32,
    height: u32,
    fourcc: u32,
    modifier: u64,
}

impl BufferInfo {
    pub(crate) fn to_frame(
        &self,
        buffer_id: u64,
        event_tx: std::sync::mpsc::Sender<ProxyEvent>,
    ) -> Option<Frame> {
        let planes = self
            .planes
            .iter()
            .map(|plane| {
                plane.fd.try_clone().ok().map(|fd| Plane {
                    fd,
                    offset: plane.offset,
                    stride: plane.stride,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Frame {
            width: self.width,
            height: self.height,
            fourcc: self.fourcc,
            modifier: self.modifier,
            planes,
            pixels: None,
            buffer_id: Some(buffer_id),
            callback_batch: None,
            event_tx,
            displayed: false,
        })
    }
}

pub(crate) struct DmabufHandler {
    pub(crate) state: Rc<RefCell<SharedState>>,
}

impl ZwpLinuxDmabufV1Handler for DmabufHandler {
    fn handle_format(&mut self, slf: &Rc<ZwpLinuxDmabufV1>, format: u32) {
        if allowed(format, DRM_FORMAT_MOD_LINEAR) {
            slf.send_format(format);
        }
    }

    fn handle_modifier(
        &mut self,
        slf: &Rc<ZwpLinuxDmabufV1>,
        format: u32,
        modifier_hi: u32,
        modifier_lo: u32,
    ) {
        let modifier = ((modifier_hi as u64) << 32) | modifier_lo as u64;
        if allowed(format, modifier) {
            slf.send_modifier(format, modifier_hi, modifier_lo);
        }
    }

    fn handle_create_params(
        &mut self,
        _slf: &Rc<ZwpLinuxDmabufV1>,
        params_id: &Rc<ZwpLinuxBufferParamsV1>,
    ) {
        params_id.set_forward_to_server(false);
        params_id.set_handler(BufferParamsHandler {
            state: Rc::clone(&self.state),
            planes: Vec::new(),
            modifier: None,
        });
    }

    fn handle_get_default_feedback(
        &mut self,
        slf: &Rc<ZwpLinuxDmabufV1>,
        id: &Rc<ZwpLinuxDmabufFeedbackV1>,
    ) {
        id.set_handler(FeedbackHandler::new());
        slf.send_get_default_feedback(id);
    }

    fn handle_get_surface_feedback(
        &mut self,
        slf: &Rc<ZwpLinuxDmabufV1>,
        id: &Rc<ZwpLinuxDmabufFeedbackV1>,
        _surface: &Rc<WlSurface>,
    ) {
        // 只有一个输出面，surface feedback 与 default feedback 等价。
        id.set_handler(FeedbackHandler::new());
        slf.send_get_default_feedback(id);
    }
}

/// 重写 compositor 的格式表：只留白名单项，并把 tranche 里的索引重新映射。
struct FeedbackHandler {
    index_map: Vec<Option<u16>>,
    pending_device: Option<Vec<u8>>,
    pending_flags: Option<ZwpLinuxDmabufFeedbackV1TrancheFlags>,
    pending_formats: Vec<u16>,
}

impl FeedbackHandler {
    fn new() -> Self {
        Self {
            index_map: Vec::new(),
            pending_device: None,
            pending_flags: None,
            pending_formats: Vec::new(),
        }
    }
}

impl ZwpLinuxDmabufFeedbackV1Handler for FeedbackHandler {
    fn handle_format_table(
        &mut self,
        slf: &Rc<ZwpLinuxDmabufFeedbackV1>,
        fd: &Rc<OwnedFd>,
        size: u32,
    ) {
        let entries = size as usize / 16;
        if entries == 0 {
            slf.send_format_table(fd, size);
            return;
        }
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                size as usize,
                libc::PROT_READ,
                libc::MAP_PRIVATE,
                fd.as_raw_fd(),
                0,
            )
        };
        if ptr == libc::MAP_FAILED {
            slf.send_format_table(fd, size);
            return;
        }
        let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, size as usize) };
        let mut table = Vec::with_capacity(size as usize);
        self.index_map = vec![None; entries];
        let mut new_index: u16 = 0;
        for index in 0..entries {
            let base = index * 16;
            let Ok(format_bytes) = bytes[base..base + 4].try_into() else {
                continue;
            };
            let Ok(modifier_bytes) = bytes[base + 8..base + 16].try_into() else {
                continue;
            };
            let format = u32::from_ne_bytes(format_bytes);
            let modifier = u64::from_ne_bytes(modifier_bytes);
            if allowed(format, modifier) {
                self.index_map[index] = Some(new_index);
                new_index = new_index.saturating_add(1);
                table.extend_from_slice(&bytes[base..base + 16]);
            }
        }
        unsafe { libc::munmap(ptr, size as usize) };

        if table.is_empty() {
            // 一个白名单格式都没有：不发这张表，mpv 会认为 dmabuf 不可用。
            return;
        }
        let memfd = unsafe { libc::memfd_create(c"kichi-dmabuf".as_ptr(), 0) };
        if memfd < 0 {
            return;
        }
        let written =
            unsafe { libc::write(memfd, table.as_ptr() as *const libc::c_void, table.len()) };
        if written < 0 {
            unsafe { libc::close(memfd) };
            return;
        }
        let new_fd = Rc::new(unsafe { OwnedFd::from_raw_fd(memfd) });
        slf.send_format_table(&new_fd, table.len() as u32);
    }

    fn handle_main_device(&mut self, slf: &Rc<ZwpLinuxDmabufFeedbackV1>, device: &[u8]) {
        slf.send_main_device(device);
    }

    fn handle_tranche_target_device(&mut self, _slf: &Rc<ZwpLinuxDmabufFeedbackV1>, device: &[u8]) {
        self.pending_device = Some(device.to_vec());
        self.pending_flags = None;
        self.pending_formats.clear();
    }

    fn handle_tranche_flags(
        &mut self,
        _slf: &Rc<ZwpLinuxDmabufFeedbackV1>,
        flags: ZwpLinuxDmabufFeedbackV1TrancheFlags,
    ) {
        self.pending_flags = Some(flags);
    }

    fn handle_tranche_formats(&mut self, _slf: &Rc<ZwpLinuxDmabufFeedbackV1>, indices: &[u8]) {
        for chunk in indices.chunks_exact(2) {
            let old = u16::from_ne_bytes([chunk[0], chunk[1]]);
            if let Some(Some(new)) = self.index_map.get(old as usize) {
                self.pending_formats.push(*new);
            }
        }
    }

    fn handle_tranche_done(&mut self, slf: &Rc<ZwpLinuxDmabufFeedbackV1>) {
        if self.pending_formats.is_empty() {
            self.pending_device = None;
            self.pending_flags = None;
            return;
        }
        if let Some(device) = self.pending_device.take() {
            slf.send_tranche_target_device(&device);
        }
        slf.send_tranche_flags(self.pending_flags.take().unwrap_or_default());
        let bytes: Vec<u8> = self
            .pending_formats
            .drain(..)
            .flat_map(|index| index.to_ne_bytes())
            .collect();
        slf.send_tranche_formats(&bytes);
        slf.send_tranche_done();
    }

    fn handle_done(&mut self, slf: &Rc<ZwpLinuxDmabufFeedbackV1>) {
        slf.send_done();
    }
}

struct BufferParamsHandler {
    state: Rc<RefCell<SharedState>>,
    planes: Vec<Option<StoredPlane>>,
    modifier: Option<u64>,
}

impl BufferParamsHandler {
    fn register(&mut self, buffer: &Rc<WlBuffer>, width: i32, height: i32, fourcc: u32) -> bool {
        let Some(planes) = std::mem::take(&mut self.planes)
            .into_iter()
            .collect::<Option<Vec<_>>>()
        else {
            eprintln!("[kichi] 内嵌播放：dmabuf 缺平面");
            return false;
        };
        if planes.is_empty() || width <= 0 || height <= 0 {
            eprintln!("[kichi] 内嵌播放：dmabuf 尺寸或平面数无效：{width}x{height}");
            return false;
        }
        buffer.set_forward_to_server(false);
        buffer.set_handler(BufferHandler {
            shared: Rc::clone(&self.state),
        });
        self.state.borrow_mut().buffer_info.insert(
            buffer.unique_id(),
            BufferInfo {
                buffer: Rc::clone(buffer),
                planes,
                width: width as u32,
                height: height as u32,
                fourcc,
                modifier: self.modifier.take().unwrap_or(DRM_FORMAT_MOD_LINEAR),
            },
        );
        true
    }
}

impl ZwpLinuxBufferParamsV1Handler for BufferParamsHandler {
    fn handle_destroy(&mut self, slf: &Rc<ZwpLinuxBufferParamsV1>) {
        slf.delete_id();
    }

    fn handle_add(
        &mut self,
        _slf: &Rc<ZwpLinuxBufferParamsV1>,
        fd: &Rc<OwnedFd>,
        plane_idx: u32,
        offset: u32,
        stride: u32,
        modifier_hi: u32,
        modifier_lo: u32,
    ) {
        let plane_idx = plane_idx as usize;
        if plane_idx >= 4 {
            eprintln!("[kichi] 内嵌播放：dmabuf 平面序号越界：{plane_idx}");
            return;
        }
        if self.planes.len() <= plane_idx {
            self.planes.resize_with(plane_idx + 1, || None);
        }
        if self.planes[plane_idx].is_some() {
            eprintln!("[kichi] 内嵌播放：dmabuf 平面 {plane_idx} 重复设置");
            return;
        }
        let modifier = ((modifier_hi as u64) << 32) | modifier_lo as u64;
        if self.modifier.is_some_and(|current| current != modifier) {
            eprintln!("[kichi] 内嵌播放：dmabuf 各平面 modifier 不一致");
            return;
        }
        let Ok(dup_fd) = fd.try_clone() else {
            eprintln!("[kichi] 内嵌播放：复制 dmabuf fd 失败");
            return;
        };
        self.modifier = Some(modifier);
        self.planes[plane_idx] = Some(StoredPlane {
            fd: dup_fd,
            offset,
            stride,
        });
    }

    fn handle_create(
        &mut self,
        slf: &Rc<ZwpLinuxBufferParamsV1>,
        width: i32,
        height: i32,
        fourcc: u32,
        _flags: ZwpLinuxBufferParamsV1Flags,
    ) {
        let valid = width > 0
            && height > 0
            && !self.planes.is_empty()
            && self.planes.iter().all(Option::is_some);
        if !valid {
            eprintln!("[kichi] 内嵌播放：异步 dmabuf 参数无效：{width}x{height}");
            slf.send_failed();
            return;
        }
        let buffer = slf.new_send_created();
        self.register(&buffer, width, height, fourcc);
    }

    fn handle_create_immed(
        &mut self,
        _slf: &Rc<ZwpLinuxBufferParamsV1>,
        buffer_id: &Rc<WlBuffer>,
        width: i32,
        height: i32,
        fourcc: u32,
        _flags: ZwpLinuxBufferParamsV1Flags,
    ) {
        self.register(buffer_id, width, height, fourcc);
    }
}

struct BufferHandler {
    shared: Rc<RefCell<SharedState>>,
}

impl WlBufferHandler for BufferHandler {
    fn handle_destroy(&mut self, slf: &Rc<WlBuffer>) {
        self.shared
            .borrow_mut()
            .buffer_info
            .remove(&slf.unique_id());
        slf.delete_id();
    }
}

/// wl_drm（老的 prime 路径）：接受 prime buffer，拒绝 GEM name buffer。
pub(crate) struct DrmHandler {
    pub(crate) state: Rc<RefCell<SharedState>>,
}

impl WlDrmHandler for DrmHandler {
    fn handle_create_buffer(
        &mut self,
        _slf: &Rc<WlDrm>,
        _id: &Rc<WlBuffer>,
        _name: u32,
        _width: i32,
        _height: i32,
        _stride: u32,
        _format: u32,
    ) {
        eprintln!("[kichi] 内嵌播放：不支持 wl_drm GEM buffer（需要 PRIME）");
    }

    fn handle_create_planar_buffer(
        &mut self,
        _slf: &Rc<WlDrm>,
        _id: &Rc<WlBuffer>,
        _name: u32,
        _width: i32,
        _height: i32,
        _format: u32,
        _offset0: i32,
        _stride0: i32,
        _offset1: i32,
        _stride1: i32,
        _offset2: i32,
        _stride2: i32,
    ) {
        eprintln!("[kichi] 内嵌播放：不支持 wl_drm 平面 GEM buffer（需要 PRIME）");
    }

    fn handle_create_prime_buffer(
        &mut self,
        _slf: &Rc<WlDrm>,
        id: &Rc<WlBuffer>,
        fd: &Rc<OwnedFd>,
        width: i32,
        height: i32,
        fourcc: u32,
        offset0: i32,
        stride0: i32,
        offset1: i32,
        stride1: i32,
        offset2: i32,
        stride2: i32,
    ) {
        if width <= 0
            || height <= 0
            || !matches!(fourcc, FOURCC_ARGB8888 | FOURCC_XRGB8888)
            || stride0 <= 0
        {
            eprintln!("[kichi] 内嵌播放：wl_drm PRIME 参数不支持：{width}x{height} fmt={fourcc}");
            return;
        }
        let Ok(dup_fd) = fd.try_clone() else {
            eprintln!("[kichi] 内嵌播放：复制 wl_drm PRIME fd 失败");
            return;
        };
        let mut planes = vec![StoredPlane {
            fd: dup_fd,
            offset: offset0.max(0) as u32,
            stride: stride0 as u32,
        }];
        if stride1 > 0 {
            if let Ok(fd) = fd.try_clone() {
                planes.push(StoredPlane {
                    fd,
                    offset: offset1.max(0) as u32,
                    stride: stride1 as u32,
                });
            }
        }
        if stride2 > 0 {
            if let Ok(fd) = fd.try_clone() {
                planes.push(StoredPlane {
                    fd,
                    offset: offset2.max(0) as u32,
                    stride: stride2 as u32,
                });
            }
        }
        id.set_forward_to_server(false);
        id.set_handler(BufferHandler {
            shared: Rc::clone(&self.state),
        });
        self.state.borrow_mut().buffer_info.insert(
            id.unique_id(),
            BufferInfo {
                buffer: Rc::clone(id),
                planes,
                width: width as u32,
                height: height as u32,
                fourcc,
                // wl_drm 没有显式 modifier；按线性导入
                modifier: DRM_FORMAT_MOD_LINEAR,
            },
        );
    }
}
