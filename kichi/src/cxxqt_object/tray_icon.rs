//! 托盘图标：渲染仓库里的 `assets/icons/hicolor/scalable/apps/kichi.svg`，
//! 直接用 SVG 原色（白盒面 + 黑线条），不做任何染色。
//!
//! 已知取舍：SNI 只能传一张图，**做不到随面板主题自适应**。原色版在深色面板上
//! 和应用图标一致；浅色面板上白色盒面会糊进背景，只剩黑色线条可辨
//! （2026-11-03 用户看过深浅面板对比图后选定这个效果）。
//!
//! 为什么用 `resvg` 而不是 Qt：托盘图标经 D-Bus 传像素（SNI 的 `IconPixmap`），
//! 渲染发生在 ksni 的后台线程里；引 Qt 的 SVG 模块会把 QtWidgets 路径拽进来
//! （见 `docs/pitfalls.md` 第 1 条），还要多装一个系统开发包。
//! `resvg` 是纯 Rust，SVG 编译期内嵌，换机器、换图标主题、清空
//! `~/.local/share/icons` 都不受影响。

/// 托盘可能用到的尺寸（面板 / 高分屏 / 右键菜单各取所需）。
const SIZES: [i32; 4] = [22, 24, 32, 48];

/// 应用图标源文件，与 AppImage 打包用的是同一份。
const SVG: &str = include_str!("../../assets/icons/hicolor/scalable/apps/kichi.svg");

/// 生成托盘图标的多档位图。
pub(crate) fn icon_pixmaps() -> Vec<ksni::Icon> {
    let Some(tree) = parse() else {
        // 内嵌资源解析失败属于代码问题，每台机器表现一致；
        // 这里不 panic，退化成「没有像素」，托盘仍能注册。
        eprintln!("[kichi] 托盘图标渲染失败，面板可能没有图标");
        return Vec::new();
    };
    SIZES
        .iter()
        .filter_map(|&size| render(&tree, size))
        .collect()
}

/// 解析内嵌 SVG。
fn parse() -> Option<resvg::usvg::Tree> {
    resvg::usvg::Tree::from_str(SVG, &resvg::usvg::Options::default()).ok()
}

/// 画一张 `size × size` 的图标。
fn render(tree: &resvg::usvg::Tree, size: i32) -> Option<ksni::Icon> {
    let svg_size = tree.size();
    let scale = size as f32 / svg_size.width().max(svg_size.height());
    let mut canvas = resvg::tiny_skia::Pixmap::new(size as u32, size as u32)?;
    resvg::render(
        tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut canvas.as_mut(),
    );

    // SNI 要 ARGB32 网络字节序（A,R,G,B），tiny_skia 给的是 RGBA
    let data: Vec<u8> = canvas
        .pixels()
        .iter()
        .flat_map(|px| [px.alpha(), px.red(), px.green(), px.blue()])
        .collect();

    Some(ksni::Icon {
        width: size,
        height: size,
        data,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_size_has_the_expected_byte_count() {
        let icons = icon_pixmaps();
        assert_eq!(icons.len(), SIZES.len(), "四档图标应全部渲染成功");
        for icon in icons {
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
    fn outside_the_box_is_transparent() {
        let tree = parse().expect("内嵌 SVG 应能解析");
        for size in [22, 48] {
            let icon = render(&tree, size).expect("渲染失败");
            let w = icon.width;
            let alpha_at = |x: i32, y: i32| icon.data[((y * w + x) * 4) as usize];
            assert_eq!(alpha_at(0, 0), 0, "{size}px 左上角应透明");
            assert_eq!(alpha_at(w - 1, w - 1), 0, "{size}px 右下角应透明");
        }
    }

    #[test]
    fn keeps_the_svg_original_colors() {
        let tree = parse().expect("内嵌 SVG 应能解析");
        for size in SIZES {
            let icon = render(&tree, size).expect("渲染失败");
            let lums: Vec<u32> = icon
                .data
                .chunks(4)
                .filter(|px| px[0] > 200)
                .map(|px| (px[1] as u32 + px[2] as u32 + px[3] as u32) / 3)
                .collect();
            let total = lums.len();
            // 白盒面是不透明像素的多数：不能被染色或抠空
            let light = lums.iter().filter(|&&l| l > 200).count();
            let darkest = lums.iter().min().copied().unwrap_or(255);
            assert!(
                total > (size * size / 8) as usize,
                "{size}px 不透明像素太少（{total}），图标几乎是空的"
            );
            assert!(
                light * 2 > total,
                "{size}px 白盒面丢了（{light}/{total}），原色没保住"
            );
            // 黑线条还在：实测 22px 最暗的不透明像素亮度约 127、48px 约 31
            // （线宽不到一个像素时被抗锯齿抬高，所以阈值按「明显暗于白面」取）
            assert!(
                darkest < 200,
                "{size}px 线条和白面糊在一起了（最暗 {darkest}/{total} 个不透明像素）"
            );
        }
    }
}

#[cfg(test)]
mod preview {
    /// 把渲染结果导出成 PPM，人工核对外观（不参与常规测试）。
    #[test]
    #[ignore = "人工预览用"]
    fn dump_preview() {
        let dir = std::env::temp_dir().join("kichi-tray-preview");
        std::fs::create_dir_all(&dir).expect("创建预览目录失败");
        for icon in super::icon_pixmaps() {
            let w = icon.width as usize;
            // 左右两栏：左浅色面板（Breeze #eff0f1）、右深色面板（#1b1e20）
            let mut out = format!("P6\n{} {}\n255\n", w * 2, w).into_bytes();
            for y in 0..w {
                for panel in [239u8, 27u8] {
                    for x in 0..w {
                        let idx = (y * w + x) * 4;
                        let a = icon.data[idx] as f32 / 255.0;
                        for c in &icon.data[idx + 1..idx + 4] {
                            out.push((*c as f32 * a + panel as f32 * (1.0 - a)) as u8);
                        }
                    }
                }
            }
            std::fs::write(dir.join(format!("svg-{}.ppm", icon.width)), out).expect("写预览失败");

            // 顺带导出真实像素（ARGB → RGBA 的 PAM），便于核对颜色与 alpha
            let mut pam = format!(
                "P7\nWIDTH {}\nHEIGHT {}\nDEPTH 4\nMAXVAL 255\nTUPLTYPE RGB_ALPHA\nENDHDR\n",
                icon.width, icon.height
            )
            .into_bytes();
            for px in icon.data.chunks(4) {
                pam.extend_from_slice(&[px[1], px[2], px[3], px[0]]);
            }
            std::fs::write(dir.join(format!("rgba-{}.pam", icon.width)), pam).expect("写预览失败");
        }
    }
}
