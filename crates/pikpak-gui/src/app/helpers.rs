use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui::{self, Color32, FontId, Rect, Stroke, vec2};

/// 统一的单行输入框样式: 更舒适的内边距 / 最小高度, 与卡片圆角一致。
pub(crate) fn input(text: &mut String) -> egui::TextEdit<'_> {
    egui::TextEdit::singleline(text)
        .margin(egui::Margin::symmetric(12, 9))
        .min_size(vec2(0.0, 38.0))
        .font(FontId::proportional(14.0))
}

pub(crate) fn open_dir(dir: &std::path::Path) {
    if !dir.is_dir() {
        return;
    }
    let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
}

/// 弹出一个原生目录选择框(独立线程阻塞式调用)。
pub(crate) fn pick_folder(initial: &std::path::Path) -> Option<PathBuf> {
    let initial = initial.to_path_buf();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .ok()?;
        rt.block_on(async move {
            rfd::AsyncFileDialog::new()
                .set_directory(&initial)
                .pick_folder()
                .await
        })
        .map(|h| h.path().to_path_buf())
    })
    .join()
    .ok()
    .flatten()
}

/// 在文字长度受限时做简单裁剪(带省略号)。
pub(crate) fn truncate_text(
    painter: &egui::Painter,
    s: &str,
    max_w: f32,
    font: FontId,
    color: Color32,
) -> Arc<egui::Galley> {
    if max_w <= 0.0 {
        return painter.layout_no_wrap(String::new(), font, color);
    }
    let g = painter.layout_no_wrap(s.to_string(), font.clone(), color);
    if g.size().x <= max_w {
        return g;
    }
    let chars: Vec<char> = s.chars().collect();
    let mut lo = 0usize;
    let mut hi = chars.len();
    let ell = "\u{2026}";
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let test: String = chars[..mid].iter().collect::<String>() + ell;
        let gg = painter.layout_no_wrap(test, font.clone(), color);
        if gg.size().x <= max_w {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    let out: String = if lo == 0 {
        ell.to_string()
    } else {
        chars[..lo].iter().collect::<String>() + ell
    };
    painter.layout_no_wrap(out, font, color)
}

/// 绘制日/月切换的小图标。
pub(crate) fn draw_sun_moon(painter: &egui::Painter, rect: Rect, color: Color32) {
    let c = rect.center();
    let r = rect.width() * 0.24;
    painter.circle_filled(c, r, color);
    for i in 0..8 {
        let a = std::f32::consts::TAU * (i as f32) / 8.0;
        let d0 = rect.width() * 0.36;
        let d1 = rect.width() * 0.46;
        let p0 = c + vec2(a.cos(), a.sin()) * d0;
        let p1 = c + vec2(a.cos(), a.sin()) * d1;
        painter.line_segment([p0, p1], Stroke::new(1.6, color));
    }
}

pub(crate) fn install_fonts(ctx: &egui::Context) -> bool {
    let mut fonts = egui::FontDefinitions::default();
    const CANDIDATES: [&str; 6] = [
        "/usr/share/fonts/google-droid-sans-fonts/DroidSansFallbackFull.ttf",
        "/usr/share/fonts/wqy-zenhei-fonts/wqy-zenhei.ttc",
        "/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc",
        "/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf",
        "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
    ];
    let mut loaded = false;
    for path in CANDIDATES {
        if let Ok(bytes) = std::fs::read(path) {
            let name = "cjk".to_string();
            fonts
                .font_data
                .insert(name.clone(), Arc::new(egui::FontData::from_owned(bytes)));
            for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                fonts.families.entry(family).or_default().push(name.clone());
            }
            loaded = true;
            break;
        }
    }
    ctx.set_fonts(fonts);
    loaded
}
