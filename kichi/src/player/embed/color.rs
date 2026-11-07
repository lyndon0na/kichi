//! 内嵌播放的色彩目标。
//!
//! Wayland presenter 启动时查询合成器给视频 surface 的 preferred image
//! description，缓存成 [`ColorTarget`]；mpv 的 `target-prim` / `target-trc` /
//! `target-peak` 跟着它走。SDR / HDR 都由 mpv 自己 tone-map 到这个 target，
//! 行为与外部 mpv 的 `target-colorspace-hint` 一致。

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ColorTarget {
    pub primaries: i32,
    pub transfer: i32,
    pub min_luminance_10000: u32,
    pub max_luminance: u32,
    pub reference_luminance: u32,
}

// 与 kichi/cpp/embed_ffi.h 的 KICHI_PRIMARIES_* / KICHI_TRANSFER_* 保持一致。
const PRIMARIES_BT709: i32 = 1;
const PRIMARIES_BT2020: i32 = 2;
const PRIMARIES_DISPLAY_P3: i32 = 3;

const TRANSFER_GAMMA22: i32 = 1;
const TRANSFER_SRGB: i32 = 2;
const TRANSFER_PQ: i32 = 3;
const TRANSFER_HLG: i32 = 4;

extern "C" {
    fn kichi_wayland_preferred_target(out: *mut ColorTarget) -> bool;
}

/// 合成器首选色彩目标；没有 Wayland presenter / 查询失败时 `None`。
pub(crate) fn preferred_target() -> Option<ColorTarget> {
    let mut target = ColorTarget::default();
    // SAFETY: `target` 是有效可写的 C 结构；C++ 侧只读缓存并拷贝。
    let ok = unsafe { kichi_wayland_preferred_target(&mut target) };
    if !ok || target.primaries == 0 || target.transfer == 0 {
        None
    } else {
        Some(target)
    }
}

/// mpv `target-prim` 取值。
pub(crate) fn primaries_name(value: i32) -> Option<&'static str> {
    match value {
        PRIMARIES_BT709 => Some("bt.709"),
        PRIMARIES_BT2020 => Some("bt.2020"),
        PRIMARIES_DISPLAY_P3 => Some("display-p3"),
        _ => None,
    }
}

/// mpv `target-trc` 取值。
pub(crate) fn transfer_name(value: i32) -> Option<&'static str> {
    match value {
        TRANSFER_GAMMA22 => Some("gamma2.2"),
        TRANSFER_SRGB => Some("srgb"),
        TRANSFER_PQ => Some("pq"),
        TRANSFER_HLG => Some("hlg"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_preferred_names() {
        assert_eq!(primaries_name(PRIMARIES_BT709), Some("bt.709"));
        assert_eq!(primaries_name(PRIMARIES_BT2020), Some("bt.2020"));
        assert_eq!(primaries_name(PRIMARIES_DISPLAY_P3), Some("display-p3"));
        assert_eq!(primaries_name(0), None);
        assert_eq!(transfer_name(TRANSFER_GAMMA22), Some("gamma2.2"));
        assert_eq!(transfer_name(TRANSFER_SRGB), Some("srgb"));
        assert_eq!(transfer_name(TRANSFER_PQ), Some("pq"));
        assert_eq!(transfer_name(TRANSFER_HLG), Some("hlg"));
        assert_eq!(transfer_name(0), None);
    }
}
