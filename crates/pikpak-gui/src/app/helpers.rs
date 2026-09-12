use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui::{self, Color32, FontId, vec2};

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

/// 用系统默认程序打开一个本地文件(尽量交给系统查看器)。
pub(crate) fn open_path(path: &std::path::Path) -> std::io::Result<()> {
    std::process::Command::new("xdg-open")
        .arg(path)
        .spawn()
        .map(|_| ())
}

/// 用 mpv 流式播放直链, 并携带签名直链所需的请求头。
pub(crate) fn play_with_mpv(
    name: &str,
    url: &str,
    headers: &[(String, String)],
) -> std::io::Result<()> {
    let mut cmd = std::process::Command::new("mpv");
    cmd.arg("--force-window=yes");
    cmd.arg(format!("--title={name}"));
    let mut fields: Vec<String> = Vec::new();
    for (k, v) in headers {
        if k.eq_ignore_ascii_case("user-agent") {
            // mpv 有独立的 UA 选项, 用 http-header-fields 会重复追加。
            cmd.arg(format!("--user-agent={v}"));
        } else {
            fields.push(format!("{k}: {v}"));
        }
    }
    if !fields.is_empty() {
        cmd.arg(format!("--http-header-fields={}", fields.join(",")));
    }
    cmd.arg("--").arg(url);
    cmd.spawn().map(|_| ())
}

/// 判断文件名是否为可用 mpv 播放的音/视频。
pub(crate) fn is_media_file(name: &str) -> bool {
    let ext = name
        .rsplit_once('.')
        .map(|(_, e): (&str, &str)| e.to_lowercase())
        .unwrap_or_default();
    matches!(
        ext.as_str(),
        "mp4" | "mkv" | "avi" | "mov" | "wmv" | "flv" | "webm" | "ts" | "rmvb" | "m4v" | "mp3"
            | "flac" | "wav" | "aac" | "ogg" | "m4a" | "opus" | "ape"
    )
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

#[cfg(test)]
mod tests {
    use super::is_media_file;

    #[test]
    fn detects_audio_and_video() {
        assert!(is_media_file("movie.MKV"));
        assert!(is_media_file("song.flac"));
        assert!(is_media_file("clip.mp4"));
    }

    #[test]
    fn rejects_non_media() {
        assert!(!is_media_file("photo.jpg"));
        assert!(!is_media_file("report.pdf"));
        assert!(!is_media_file("archive.zip"));
        assert!(!is_media_file("noext"));
    }
}
