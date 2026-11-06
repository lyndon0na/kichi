//! 合成的 `wl_shm`：软件输出兜底。
//!
//! 只在没有可用 dmabuf 时才会走到这里（例如不支持线性 dmabuf 的驱动）。
//! 帧在 commit 时整块复制到 Rust 内存，再把 wl_buffer 立刻还给 mpv；
//! Qt 侧只需要一段 RGBA 像素，不需要 fd 生命周期管理。

use std::cell::{Cell, RefCell};
use std::fs::File;
use std::os::fd::OwnedFd;
use std::os::unix::fs::FileExt;
use std::rc::Rc;

use wl_proxy::object::{Object, ObjectCoreApi};
use wl_proxy::protocols::wayland::wl_buffer::{WlBuffer, WlBufferHandler};
use wl_proxy::protocols::wayland::wl_shm::{WlShm, WlShmError, WlShmFormat, WlShmHandler};
use wl_proxy::protocols::wayland::wl_shm_pool::{WlShmPool, WlShmPoolError, WlShmPoolHandler};

use super::dmabuf::{FOURCC_ARGB8888, FOURCC_XRGB8888};
use super::SharedState;

struct ShmPoolState {
    file: File,
    size: Cell<u64>,
}

pub(crate) struct BufferInfo {
    pub(crate) buffer: Rc<WlBuffer>,
    pool: Rc<ShmPoolState>,
    offset: u64,
    width: u32,
    height: u32,
    stride: usize,
    fourcc: u32,
}

impl BufferInfo {
    /// 复制成紧密排列的 RGBA，alpha 统一置 255（视频一帧没有半透明）。
    pub(crate) fn snapshot(&self) -> Option<(u32, u32, u32, Vec<u8>)> {
        let width = self.width as usize;
        let height = self.height as usize;
        let row_bytes = width.checked_mul(4)?;
        let mut data = vec![0u8; row_bytes.checked_mul(height)?];
        for row in 0..height {
            let source = self
                .offset
                .checked_add((row as u64).checked_mul(self.stride as u64)?)?;
            let start = row * row_bytes;
            self.pool
                .file
                .read_exact_at(&mut data[start..start + row_bytes], source)
                .ok()?;
            for pixel in data[start..start + row_bytes].chunks_exact_mut(4) {
                pixel[3] = 0xFF;
            }
        }
        Some((self.width, self.height, self.fourcc, data))
    }
}

fn buffer_end(offset: u64, width: usize, height: usize, stride: usize) -> Option<u64> {
    if width == 0 || height == 0 {
        return None;
    }
    let row_bytes = width.checked_mul(4)?;
    if stride < row_bytes {
        return None;
    }
    let byte_len = height
        .checked_sub(1)?
        .checked_mul(stride)?
        .checked_add(row_bytes)?;
    offset.checked_add(byte_len as u64)
}

pub(crate) struct ShmHandler {
    pub(crate) state: Rc<RefCell<SharedState>>,
}

impl WlShmHandler for ShmHandler {
    fn handle_create_pool(
        &mut self,
        slf: &Rc<WlShm>,
        id: &Rc<WlShmPool>,
        fd: &Rc<OwnedFd>,
        size: i32,
    ) {
        let Some(size) = u64::try_from(size).ok().filter(|size| *size > 0) else {
            post_shm_error(slf, WlShmError::INVALID_STRIDE, "wl_shm 池大小无效");
            return;
        };
        let Ok(owned_fd) = fd.try_clone() else {
            post_shm_error(slf, WlShmError::INVALID_FD, "wl_shm 池 fd 无效");
            return;
        };
        let file = File::from(owned_fd);
        if file.metadata().is_ok_and(|metadata| metadata.len() < size) {
            post_shm_error(slf, WlShmError::INVALID_FD, "wl_shm 后备文件太小");
            return;
        }
        id.set_forward_to_server(false);
        id.set_handler(ShmPoolHandler {
            state: Rc::clone(&self.state),
            pool: Rc::new(ShmPoolState {
                file,
                size: Cell::new(size),
            }),
        });
    }
}

struct ShmPoolHandler {
    state: Rc<RefCell<SharedState>>,
    pool: Rc<ShmPoolState>,
}

impl WlShmPoolHandler for ShmPoolHandler {
    fn handle_create_buffer(
        &mut self,
        slf: &Rc<WlShmPool>,
        id: &Rc<WlBuffer>,
        offset: i32,
        width: i32,
        height: i32,
        stride: i32,
        format: WlShmFormat,
    ) {
        let fourcc = if format == WlShmFormat::ARGB8888 {
            FOURCC_ARGB8888
        } else if format == WlShmFormat::XRGB8888 {
            FOURCC_XRGB8888
        } else {
            post_pool_error(slf, WlShmPoolError::INVALID_FORMAT, "不支持的 wl_shm 格式");
            return;
        };
        let (Ok(offset), Ok(width), Ok(height), Ok(stride)) = (
            u64::try_from(offset),
            u32::try_from(width),
            u32::try_from(height),
            usize::try_from(stride),
        ) else {
            post_pool_error(slf, WlShmPoolError::INVALID_STRIDE, "wl_shm 尺寸无效");
            return;
        };
        let Some(end) = buffer_end(offset, width as usize, height as usize, stride) else {
            post_pool_error(slf, WlShmPoolError::INVALID_STRIDE, "wl_shm 缓冲范围无效");
            return;
        };
        if end > self.pool.size.get() {
            post_pool_error(slf, WlShmPoolError::INVALID_STRIDE, "wl_shm 缓冲超出池大小");
            return;
        }

        id.set_forward_to_server(false);
        id.set_handler(ShmBufferHandler {
            state: Rc::clone(&self.state),
        });
        self.state.borrow_mut().shm_buffer_info.insert(
            id.unique_id(),
            BufferInfo {
                buffer: Rc::clone(id),
                pool: Rc::clone(&self.pool),
                offset,
                width,
                height,
                stride,
                fourcc,
            },
        );
    }

    fn handle_destroy(&mut self, slf: &Rc<WlShmPool>) {
        slf.delete_id();
    }

    fn handle_resize(&mut self, slf: &Rc<WlShmPool>, size: i32) {
        let Some(size) = u64::try_from(size)
            .ok()
            .filter(|size| *size > self.pool.size.get())
        else {
            post_pool_error(slf, WlShmPoolError::INVALID_STRIDE, "wl_shm 池扩容值无效");
            return;
        };
        if self
            .pool
            .file
            .metadata()
            .is_ok_and(|metadata| metadata.len() >= size)
        {
            self.pool.size.set(size);
        } else {
            post_pool_error(slf, WlShmPoolError::INVALID_STRIDE, "扩容后的后备文件太小");
        }
    }
}

struct ShmBufferHandler {
    state: Rc<RefCell<SharedState>>,
}

impl WlBufferHandler for ShmBufferHandler {
    fn handle_destroy(&mut self, slf: &Rc<WlBuffer>) {
        self.state
            .borrow_mut()
            .shm_buffer_info
            .remove(&slf.unique_id());
        slf.delete_id();
    }
}

fn post_shm_error(slf: &Rc<WlShm>, error: WlShmError, message: &str) {
    if let Some(client) = slf.client() {
        let object: Rc<dyn Object> = slf.clone();
        client.display().send_error(object, error.0, message);
    }
}

fn post_pool_error(slf: &Rc<WlShmPool>, error: WlShmPoolError, message: &str) {
    if let Some(client) = slf.client() {
        let object: Rc<dyn Object> = slf.clone();
        client.display().send_error(object, error.0, message);
    }
}
