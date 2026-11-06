//! 给 `kichi/cpp/MpvVideoItem` 用的最小 C ABI。
//!
//! Qt 渲染线程通过这里取走 [`Frame`]（Rust 持有所有权），导入成 GL 纹理后
//! 再调用 `displayed` / `free`。除这里以外，C++ 不接触任何 Rust 数据结构。

use std::os::fd::AsRawFd;

use super::{set_viewport, take_frame, Frame};

#[no_mangle]
pub extern "C" fn kichi_embed_frame_pending() -> bool {
    super::frame_pending()
}

#[no_mangle]
pub extern "C" fn kichi_embed_frame_take() -> *mut Frame {
    take_frame()
        .map(|frame| Box::into_raw(Box::new(frame)))
        .unwrap_or(std::ptr::null_mut())
}

#[no_mangle]
pub unsafe extern "C" fn kichi_embed_frame_free(frame: *mut Frame) {
    if frame.is_null() {
        return;
    }
    drop(unsafe { Box::from_raw(frame) });
}

#[no_mangle]
pub unsafe extern "C" fn kichi_embed_frame_displayed(frame: *mut Frame) {
    if let Some(frame) = unsafe { frame.as_mut() } {
        frame.mark_displayed();
    }
}

#[no_mangle]
pub unsafe extern "C" fn kichi_embed_frame_width(frame: *const Frame) -> u32 {
    unsafe { frame.as_ref() }.map_or(0, |frame| frame.width)
}

#[no_mangle]
pub unsafe extern "C" fn kichi_embed_frame_height(frame: *const Frame) -> u32 {
    unsafe { frame.as_ref() }.map_or(0, |frame| frame.height)
}

#[no_mangle]
pub unsafe extern "C" fn kichi_embed_frame_fourcc(frame: *const Frame) -> u32 {
    unsafe { frame.as_ref() }.map_or(0, |frame| frame.fourcc)
}

#[no_mangle]
pub unsafe extern "C" fn kichi_embed_frame_modifier(frame: *const Frame) -> u64 {
    unsafe { frame.as_ref() }.map_or(0, |frame| frame.modifier)
}

#[no_mangle]
pub unsafe extern "C" fn kichi_embed_frame_is_shm(frame: *const Frame) -> bool {
    unsafe { frame.as_ref() }.is_some_and(|frame| frame.pixels.is_some())
}

#[no_mangle]
pub unsafe extern "C" fn kichi_embed_frame_pixels(frame: *const Frame) -> *const u8 {
    let Some(frame) = (unsafe { frame.as_ref() }) else {
        return std::ptr::null();
    };
    frame
        .pixels
        .as_ref()
        .map_or(std::ptr::null(), |pixels| pixels.as_ptr())
}

#[no_mangle]
pub unsafe extern "C" fn kichi_embed_frame_plane_count(frame: *const Frame) -> i32 {
    unsafe { frame.as_ref() }.map_or(0, |frame| frame.planes.len() as i32)
}

#[no_mangle]
pub unsafe extern "C" fn kichi_embed_frame_plane_fd(frame: *const Frame, index: i32) -> i32 {
    unsafe { frame.as_ref() }
        .and_then(|frame| frame.planes.get(index as usize))
        .map_or(-1, |plane| plane.fd.as_raw_fd())
}

#[no_mangle]
pub unsafe extern "C" fn kichi_embed_frame_plane_offset(frame: *const Frame, index: i32) -> u32 {
    unsafe { frame.as_ref() }
        .and_then(|frame| frame.planes.get(index as usize))
        .map_or(0, |plane| plane.offset)
}

#[no_mangle]
pub unsafe extern "C" fn kichi_embed_frame_plane_stride(frame: *const Frame, index: i32) -> u32 {
    unsafe { frame.as_ref() }
        .and_then(|frame| frame.planes.get(index as usize))
        .map_or(0, |plane| plane.stride)
}

/// QQuickItem 的尺寸或 DPR 变了：告诉代理下一帧按这个逻辑尺寸出。
#[no_mangle]
pub extern "C" fn kichi_embed_set_viewport(width: i32, height: i32, scale: f64) {
    set_viewport(width, height, scale);
}

/// 把 `frame`（像素）按比例放进 `item`（逻辑），得到居中的信箱矩形。
///
/// 返回以 item 左上角为原点的 `(x, y, width, height)`；任一尺寸无效时 `None`。
fn fit_rect(
    item_width: f64,
    item_height: f64,
    frame_width: f64,
    frame_height: f64,
) -> Option<(f64, f64, f64, f64)> {
    let valid = [item_width, item_height, frame_width, frame_height]
        .into_iter()
        .all(|value| value.is_finite() && value > 0.0);
    if !valid {
        return None;
    }
    let scale = (item_width / frame_width).min(item_height / frame_height);
    let width = frame_width * scale;
    let height = frame_height * scale;
    Some((
        (item_width - width) / 2.0,
        (item_height - height) / 2.0,
        width,
        height,
    ))
}

/// 计算信箱式显示矩形；`out` 指向 4 个 f64（x、y、宽、高），单位与 item 相同。
#[no_mangle]
pub unsafe extern "C" fn kichi_embed_fit_rect(
    item_width: f64,
    item_height: f64,
    frame_width: f64,
    frame_height: f64,
    out: *mut f64,
) -> bool {
    let Some(rect) = fit_rect(item_width, item_height, frame_width, frame_height) else {
        return false;
    };
    if out.is_null() {
        return false;
    }
    let values = [rect.0, rect.1, rect.2, rect.3];
    unsafe { std::ptr::copy_nonoverlapping(values.as_ptr(), out, values.len()) };
    true
}

#[cfg(test)]
mod tests {
    use super::fit_rect;

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 1e-9,
            "期望 {expected}，实际 {actual}"
        );
    }

    #[test]
    fn wide_frame_is_letterboxed_top_and_bottom() {
        let (x, y, width, height) = fit_rect(400.0, 300.0, 1920.0, 1080.0).expect("应能计算");
        assert_close(x, 0.0);
        assert_close(width, 400.0);
        assert_close(height, 225.0);
        assert_close(y, 37.5);
    }

    #[test]
    fn tall_frame_is_pillarboxed_left_and_right() {
        let (x, y, width, height) = fit_rect(400.0, 300.0, 1080.0, 1920.0).expect("应能计算");
        assert_close(x, 115.625);
        assert_close(y, 0.0);
        assert_close(width, 168.75);
        assert_close(height, 300.0);
    }

    #[test]
    fn matching_aspect_uses_the_whole_item() {
        let (x, y, width, height) = fit_rect(1280.0, 720.0, 1920.0, 1080.0).expect("应能计算");
        assert_close(x, 0.0);
        assert_close(y, 0.0);
        assert_close(width, 1280.0);
        assert_close(height, 720.0);
    }

    #[test]
    fn invalid_dimensions_are_rejected() {
        assert!(fit_rect(0.0, 300.0, 16.0, 9.0).is_none());
        assert!(fit_rect(400.0, 300.0, f64::NAN, 9.0).is_none());
        assert!(fit_rect(400.0, 300.0, 16.0, f64::INFINITY).is_none());
    }
}
