//! 内嵌播放的色彩目标。
//!
//! Wayland presenter 启动时查询合成器给视频 surface 的 preferred image
//! description，缓存成 [`ColorInfo`]；再按 mpv `wayland_common.c` 的规则
//! 归一化成 [`ColorTarget`]：
//!
//! - compositor 报的是非 HDR transfer、但带 HDR headroom（max != reference）
//!   时，输出改走 PQ；
//! - 所有亮度按 203 nits（libplacebo 的 SDR 白）重新缩放，`target-peak`
//!   就是这个缩放后的峰值；
//! - `reference_luminance` 固定为 203，surface image description 也按这个
//!   参考白构造。
//!
//! mpv 的 `target-prim` / `target-trc` / `target-peak` 都用归一化后的
//! [`ColorTarget`]，行为与外部 mpv 的 `target-colorspace-hint` 一致。

/// 合成器 preferred image description 的原始值。
///
/// `max_luminance` / `min_luminance_10000` / `reference_luminance` 来自
/// `wp_image_description_info_v1.luminances`；`target_max_luminance` 来自
/// `target_luminance`，是 display 真正的 HDR 峰值。
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ColorInfo {
    pub primaries: i32,
    pub transfer: i32,
    pub min_luminance_10000: u32,
    pub max_luminance: u32,
    pub reference_luminance: u32,
    /// `target_luminance` 的 max；0 表示 compositor 没有发送。
    pub target_max_luminance: u32,
    pub max_cll: u32,
    pub max_fall: u32,
}

/// 归一化后真正交给 mpv / surface description 的目标。
///
/// `max_luminance` 是 mpv `target-peak`（已按 203 nit 参考白缩放），不是
/// surface description 的 `luminances.max`：PQ 描述的 surface 亮度固定为
/// `0..10000`，SDR 描述的 `luminances.max` 也是归一化后的值。
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

/// libplacebo 的 `PL_COLOR_SDR_WHITE`，也是 PQ 描述的参考白。
const SDR_WHITE_NITS: f64 = 203.0;

extern "C" {
    fn kichi_wayland_preferred_target(out: *mut ColorTarget) -> bool;
}

/// 合成器首选色彩目标；没有 Wayland presenter / 查询失败时 `None`。
pub(crate) fn preferred_target() -> Option<ColorTarget> {
    let mut target = ColorTarget::default();
    // SAFETY: `target` 是有效可写的 C 结构；C++ 侧只读缓存并拷贝。
    let ok = unsafe { kichi_wayland_preferred_target(&mut target) };
    (ok && target.is_valid()).then_some(target)
}

/// 按 mpv `wayland_common.c: info_done()` 的规则把 compositor preferred
/// 转成输出 target。
///
/// compositor 的亮度是相对 reference 的：`max / reference` 才是 headroom。
/// mpv / libplacebo 的 SDR 白固定为 203 nits，所以这里把整个亮度范围按
/// `203 / reference` 缩放；非 HDR transfer 又有 headroom 时，PQ 是唯一能
/// 同时表达“SDR 白 203 + 峰值 > 203”的曲线。
pub(crate) fn normalize(info: ColorInfo) -> Option<ColorTarget> {
    if primaries_name(info.primaries).is_none() || transfer_name(info.transfer).is_none() {
        return None;
    }

    let reference = f64::from(info.reference_luminance);
    let min = f64::from(info.min_luminance_10000) / 10_000.0;
    if reference <= min || reference <= 0.0 {
        return None;
    }

    // target_luminance 才是 display 的 HDR 峰值；缺失时退回 max_cll / luminances.
    let target_max = f64::from(if info.target_max_luminance > 0 {
        info.target_max_luminance
    } else if info.max_cll > 0 {
        info.max_cll
    } else {
        info.max_luminance
    });
    if target_max <= 0.0 {
        return None;
    }

    // 与 mpv 完全一致：a = c = luminances.min，b = 203 / (reference - a)。
    let a = min;
    let c = min;
    let b = (SDR_WHITE_NITS - c) / (reference - a);
    if !b.is_finite() || b <= 0.0 {
        return None;
    }
    let peak = ((target_max - a) * b + c).round();
    if !peak.is_finite() || peak < 1.0 {
        return None;
    }
    let peak = peak.min(10_000.0) as u32;

    let hdr = matches!(info.transfer, TRANSFER_PQ | TRANSFER_HLG);
    let has_headroom = ((target_max / reference) - 1.0).abs() > 1e-4;
    let transfer = if hdr || !has_headroom {
        info.transfer
    } else {
        TRANSFER_PQ
    };

    Some(ColorTarget {
        primaries: info.primaries,
        transfer,
        // PQ 描述的 surface 亮度固定为 0..10000；其他曲线用相对亮度。
        min_luminance_10000: if transfer == TRANSFER_PQ {
            0
        } else {
            info.min_luminance_10000
        },
        max_luminance: peak,
        reference_luminance: SDR_WHITE_NITS as u32,
    })
}

impl ColorTarget {
    /// 能否映射成 mpv 的 `target-*` 选项。
    pub(crate) fn is_valid(&self) -> bool {
        primaries_name(self.primaries).is_some()
            && transfer_name(self.transfer).is_some()
            && self.max_luminance > 0
    }
}

/// 合成器首选描述变了才需要重设 mpv target / 视频 surface。
///
/// `previous` 是已经应用到 mpv 的目标；无效候选一律忽略，避免把已验证的
/// HDR target 换成未知值。
pub(crate) fn target_update(
    previous: Option<ColorTarget>,
    next: ColorTarget,
) -> Option<ColorTarget> {
    if !next.is_valid() || previous == Some(next) {
        None
    } else {
        Some(next)
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

    fn kwin_hdr() -> ColorInfo {
        ColorInfo {
            primaries: PRIMARIES_BT2020,
            transfer: TRANSFER_GAMMA22,
            min_luminance_10000: 0,
            max_luminance: 760,
            reference_luminance: 400,
            target_max_luminance: 760,
            max_cll: 760,
            max_fall: 0,
        }
    }

    fn kwin_sdr() -> ColorInfo {
        ColorInfo {
            primaries: PRIMARIES_BT709,
            transfer: TRANSFER_SRGB,
            min_luminance_10000: 2000,
            max_luminance: 80,
            reference_luminance: 80,
            target_max_luminance: 80,
            max_cll: 0,
            max_fall: 0,
        }
    }

    fn hdr() -> ColorTarget {
        normalize(kwin_hdr()).expect("KWin HDR preferred 应可归一化")
    }

    fn sdr() -> ColorTarget {
        normalize(kwin_sdr()).expect("KWin SDR preferred 应可归一化")
    }

    #[test]
    fn normalizes_kwin_hdr_to_pq() {
        let target = normalize(kwin_hdr()).expect("KWin HDR preferred");
        assert_eq!(target.primaries, PRIMARIES_BT2020);
        assert_eq!(target.transfer, TRANSFER_PQ);
        assert_eq!(target.min_luminance_10000, 0);
        // 760 * 203 / 400 ≈ 386
        assert_eq!(target.max_luminance, 386);
        assert_eq!(target.reference_luminance, 203);
    }

    #[test]
    fn normalizes_sdr_to_reference_white() {
        let target = normalize(kwin_sdr()).expect("KWin SDR preferred");
        assert_eq!(target.primaries, PRIMARIES_BT709);
        assert_eq!(target.transfer, TRANSFER_SRGB);
        assert_eq!(target.max_luminance, 203);
        assert_eq!(target.reference_luminance, 203);
    }

    #[test]
    fn keeps_real_hdr_transfer() {
        let target = normalize(ColorInfo {
            primaries: PRIMARIES_BT2020,
            transfer: TRANSFER_PQ,
            min_luminance_10000: 0,
            max_luminance: 10_000,
            reference_luminance: 203,
            target_max_luminance: 760,
            max_cll: 760,
            max_fall: 0,
        })
        .expect("PQ preferred");
        assert_eq!(target.transfer, TRANSFER_PQ);
        assert_eq!(target.max_luminance, 760);
        assert_eq!(target.reference_luminance, 203);
    }

    #[test]
    fn keeps_hlg_without_forcing_pq() {
        let target = normalize(ColorInfo {
            transfer: TRANSFER_HLG,
            ..kwin_hdr()
        })
        .expect("HLG preferred");
        assert_eq!(target.transfer, TRANSFER_HLG);
        assert_eq!(target.max_luminance, 386);
    }

    #[test]
    fn falls_back_to_cll_when_target_luminance_is_missing() {
        let target = normalize(ColorInfo {
            target_max_luminance: 0,
            max_cll: 500,
            ..kwin_hdr()
        })
        .expect("缺 target_luminance 时用 maxCLL");
        assert_eq!(target.transfer, TRANSFER_PQ);
        assert_eq!(target.max_luminance, 254); // 500 * 203 / 400 ≈ 254
    }

    #[test]
    fn rejects_incomplete_targets() {
        assert!(hdr().is_valid());
        assert!(sdr().is_valid());
        assert!(!ColorTarget::default().is_valid());
        assert!(!ColorTarget {
            max_luminance: 0,
            ..hdr()
        }
        .is_valid());
        assert!(!ColorTarget {
            primaries: 99,
            ..hdr()
        }
        .is_valid());
    }

    #[test]
    fn only_reports_meaningful_target_changes() {
        // 第一次拿到首选要应用；同一份描述重复上报则不要重复设 mpv。
        assert_eq!(target_update(None, hdr()), Some(hdr()));
        assert_eq!(target_update(Some(hdr()), hdr()), None);
        // 换屏 / HDR 开关：HDR ↔ SDR 必须更新。
        assert_eq!(target_update(Some(hdr()), sdr()), Some(sdr()));
        assert_eq!(target_update(Some(sdr()), hdr()), Some(hdr()));
        // 未知候选不能覆盖当前已知目标。
        assert_eq!(target_update(Some(hdr()), ColorTarget::default()), None);
        assert_eq!(
            target_update(
                Some(hdr()),
                ColorTarget {
                    max_luminance: 0,
                    ..sdr()
                }
            ),
            None
        );
    }
}
