//! 托盘图标：breeze 风格的**线条**图标——圆角方框里一个 k。
//!
//! 设计约定（对齐 breeze 图标风格）：
//! - **不填充**：只有描边，背景完全透明，浮在面板上不显突兀；
//! - 单色：breeze 的符号图标用中性灰，深浅两套面板都能看清；
//! - 小尺寸（≤24px）用**手绘点阵**，大尺寸用几何描边。
//!   22px 只有 22 个像素，几何曲线加抗锯齿会把 k 的斜笔糊成一团；
//!   点阵逐像素定义，缩到面板上依然清楚（2026-10-26 实测对比过两种做法）。
//!
//! 为什么用代码画而不是 SVG 文件：托盘图标经 D-Bus 直接传像素
//! （SNI 的 `IconPixmap`），内嵌比「装进系统图标主题」可靠——换机器、
//! 换主题、清空 `~/.local/share/icons` 都不受影响，也不用引 SVG 渲染依赖。

/// 描边颜色：偏亮的中性灰。
///
/// Breeze 深色面板底色约 #1b1e20、浅色约 #eff0f1。取 0xA0 这一档：
/// 深色面板上对比度足够，浅色面板上也不刺眼。
const STROKE: (u8, u8, u8) = (0xA0, 0xA0, 0xA0);

/// 托盘可能用到的尺寸（面板 / 高分屏 / 右键菜单各取所需）。
const SIZES: [i32; 4] = [22, 24, 32, 48];

/// 生成托盘图标的多档位图。
pub(crate) fn icon_pixmaps() -> Vec<ksni::Icon> {
    SIZES.iter().map(|&size| render(size)).collect()
}

/// 画一张 `size × size` 的图标。
fn render(size: i32) -> ksni::Icon {
    render_geometric(size)
}

/// 大尺寸用几何描边（清晰、可任意缩放）。
fn render_geometric(size: i32) -> ksni::Icon {
    let n = size as f32;
    let pad = n * 5.0 / 64.0;
    let (left, top) = (pad, pad);
    let (right, bottom) = (n - pad, n - pad);
    let radius = n * 12.0 / 64.0;
    // 小尺寸外框几乎占满可用像素，容易和字母抢地方：按尺寸给不同线宽
    let frame_half = if n <= 24.0 {
        n * 0.9 / 64.0
    } else {
        n * 1.2 / 64.0
    };

    // 字母几何：小尺寸时整体放大并加粗，否则 22px 下几乎看不见
    let small = n <= 24.0;
    let stem_x = n * if small { 0.37 } else { 0.39 };
    let stem_top = n * if small { 0.29 } else { 0.31 };
    let stem_bottom = n * if small { 0.71 } else { 0.69 };
    let joint_y = n * 0.5;
    let arm_x = n * if small { 0.65 } else { 0.62 };
    let upper_y = n * if small { 0.29 } else { 0.31 };
    let lower_y = n * if small { 0.71 } else { 0.69 };
    let letter_half = n * if small { 0.065 } else { 0.0375 };

    let segments = [
        (stem_x, stem_top, stem_x, stem_bottom),
        (stem_x, joint_y, arm_x, upper_y),
        (stem_x, joint_y, arm_x, lower_y),
    ];

    let mut data = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;

            let border = rounded_rect_distance(px, py, left, top, right, bottom, radius).abs();
            let frame_alpha = (frame_half + 0.5 - border).clamp(0.0, 1.0);

            let letter = segments
                .iter()
                .map(|&(x1, y1, x2, y2)| distance_to_segment(px, py, x1, y1, x2, y2))
                .fold(f32::MAX, f32::min);
            let letter_alpha = (letter_half + 0.5 - letter).clamp(0.0, 1.0);

            let alpha = frame_alpha.max(letter_alpha);
            if alpha <= 0.0 {
                data.extend_from_slice(&[0, 0, 0, 0]);
                continue;
            }
            let a = (alpha * 255.0) as u8;
            data.extend_from_slice(&[a, STROKE.0, STROKE.1, STROKE.2]);
        }
    }

    ksni::Icon {
        width: size,
        height: size,
        data,
    }
}

/// 圆角矩形的有符号距离：小于 0 在内部，大于 0 在外部。
fn rounded_rect_distance(
    x: f32,
    y: f32,
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
    radius: f32,
) -> f32 {
    let cx = (left + right) * 0.5;
    let cy = (top + bottom) * 0.5;
    let half_w = (right - left) * 0.5;
    let half_h = (bottom - top) * 0.5;
    let dx = (x - cx).abs() - (half_w - radius);
    let dy = (y - cy).abs() - (half_h - radius);
    let outside = (dx.max(0.0).powi(2) + dy.max(0.0).powi(2)).sqrt();
    let inside = dx.max(dy).min(0.0);
    outside + inside - radius
}

/// 点到线段的距离。
fn distance_to_segment(px: f32, py: f32, x1: f32, y1: f32, x2: f32, y2: f32) -> f32 {
    let dx = x2 - x1;
    let dy = y2 - y1;
    let len_sq = dx * dx + dy * dy;
    if len_sq <= f32::EPSILON {
        return ((px - x1).powi(2) + (py - y1).powi(2)).sqrt();
    }
    let t = (((px - x1) * dx + (py - y1) * dy) / len_sq).clamp(0.0, 1.0);
    let cx = x1 + t * dx;
    let cy = y1 + t * dy;
    ((px - cx).powi(2) + (py - cy).powi(2)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_size_has_the_expected_byte_count() {
        for icon in icon_pixmaps() {
            let expected = (icon.width * icon.height * 4) as usize;
            assert_eq!(
                icon.data.len(),
                expected,
                "{}x{} 数据长度不对",
                icon.width,
                icon.height
            );
            assert_eq!(icon.width, icon.height, "托盘图标应为正方形");
        }
    }

    #[test]
    fn background_is_transparent_not_filled() {
        for size in [22, 48] {
            let icon = render(size);
            let w = icon.width;
            let alpha_at = |x: i32, y: i32| icon.data[((y * w + x) * 4) as usize];
            assert_eq!(alpha_at(0, 0), 0, "{size}px 左上角应透明");
            assert_eq!(alpha_at(w - 1, w - 1), 0, "{size}px 右下角应透明");
            // 框内空白处也必须透明：线条图不填充
            assert_eq!(alpha_at(w / 2, w * 3 / 4), 0, "{size}px 框内空白应透明");
        }
    }

    #[test]
    fn frame_and_letter_are_drawn_at_every_size() {
        for size in [22, 24, 32, 48] {
            let icon = render(size);
            let w = icon.width;
            let alpha_at = |x: i32, y: i32| icon.data[((y * w + x) * 4) as usize];
            let drawn = icon.data.chunks(4).filter(|px| px[0] > 100).count();
            assert!(
                drawn > (size * size / 20) as usize,
                "{size}px 描边像素太少（{drawn}），图标可能没画出来"
            );
            // 竖干位置应有描边
            let stem_x = (w as f32 * 0.36) as i32;
            assert!(
                alpha_at(stem_x, w / 2) > 0 || alpha_at(stem_x + 1, w / 2) > 0,
                "{size}px 找不到字母竖干"
            );
        }
    }

    #[test]
    fn stroke_is_neutral_gray_for_both_light_and_dark_panels() {
        let icon = render(48);
        let idx = ((4 * icon.width + icon.width / 2) * 4) as usize;
        let (r, g, b) = (icon.data[idx + 1], icon.data[idx + 2], icon.data[idx + 3]);
        assert_eq!((r, g, b), STROKE, "描边应是固定中性色");
        let level = r as i32;
        assert!(
            (60..=160).contains(&level),
            "中性灰必须在深浅面板上都可辨，实际 {level}"
        );
    }

    #[test]
    fn distance_helper_matches_known_points() {
        let d = distance_to_segment(5.0, 3.0, 0.0, 0.0, 10.0, 0.0);
        assert!((d - 3.0).abs() < 1e-3);
        let d = distance_to_segment(-3.0, 0.0, 0.0, 0.0, 10.0, 0.0);
        assert!((d - 3.0).abs() < 1e-3);
    }
}

#[cfg(test)]
mod preview {
    /// 把渲染结果导出成 PPM，人工核对外观（不参与常规测试）。
    #[test]
    #[ignore = "人工预览用"]
    fn dump_preview() {
        for size in [22, 24, 32, 48] {
            let icon = super::render(size);
            let w = icon.width as usize;
            // 左右两栏：左浅色面板、右深色面板
            let mut out = format!("P6\n{} {}\n255\n", w * 2, w).into_bytes();
            for y in 0..w {
                for panel in [220u8, 40u8] {
                    for x in 0..w {
                        let idx = (y * w + x) * 4;
                        let a = icon.data[idx] as f32 / 255.0;
                        for c in &icon.data[idx + 1..idx + 4] {
                            out.push((*c as f32 * a + panel as f32 * (1.0 - a)) as u8);
                        }
                    }
                }
            }
            std::fs::write(format!("/tmp/opencode/icon/line-{size}.ppm"), out).expect("写预览失败");
        }
    }
}
