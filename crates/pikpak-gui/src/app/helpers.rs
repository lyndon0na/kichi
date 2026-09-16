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
/// `subs` 为同集外挂字幕的本地路径, 会作为 `--sub-file` 挂载。
pub(crate) fn play_with_mpv(
    name: &str,
    url: &str,
    headers: &[(String, String)],
    subs: &[PathBuf],
) -> std::io::Result<()> {
    let mut cmd = std::process::Command::new("mpv");
    cmd.arg("--force-window=yes");
    // 直链直接交给 ffmpeg 播放即可, 关闭 ytdl 钩子(否则会对直链跑 youtube-dl)。
    cmd.arg("--ytdl=no");
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
    // 附多个字幕时优先选中文字幕, 找不到再退回默认(通常英文)。
    cmd.arg("--slang=zh-Hans,zh-CN,zh-Hant,zh-TW,chs,cht,sc,tc,chi,zh,en");
    for sub in subs {
        cmd.arg(format!("--sub-file={}", sub.display()));
    }
    cmd.arg("--").arg(url);
    cmd.spawn().map(|_| ())
}

/// 可用 mpv 播放的音/视频扩展名(小写, 单点维护)。
const MEDIA_EXTS: &[&str] = &[
    "mp4", "mkv", "avi", "mov", "wmv", "flv", "webm", "ts", "rmvb", "m4v", "m2ts", "mpg", "mpeg",
    "mp3", "flac", "wav", "aac", "ogg", "m4a", "opus", "ape",
];

/// 常见字幕扩展名(小写, 单点维护)。
const SUBTITLE_EXTS: &[&str] = &["ass", "ssa", "srt", "sub", "vtt", "sbv", "sup"];

/// 取小写扩展名(最后一段), 无扩展名时为空串。
fn ext_lower(name: &str) -> String {
    name.rsplit_once('.')
        .map(|(_, e): (&str, &str)| e.to_lowercase())
        .unwrap_or_default()
}

/// 判断文件名是否为可用 mpv 播放的音/视频。
pub(crate) fn is_media_file(name: &str) -> bool {
    MEDIA_EXTS.contains(&ext_lower(name).as_str())
}

/// 判断文件名是否为常见字幕格式。
pub(crate) fn is_subtitle_file(name: &str) -> bool {
    SUBTITLE_EXTS.contains(&ext_lower(name).as_str())
}

/// 判断字幕是否与某视频同集: 去掉扩展名后与视频名相同(忽略大小写), 或以视频名为前缀
/// 且其后紧跟非字母数字分隔符(如 `.sc.ass` / `_chs.srt` / `.zh-CN.ass`)。
pub(crate) fn subtitle_of(video: &str, sub: &str) -> bool {
    let (Some((vstem, _)), Some((sstem, _))) =
        (video.rsplit_once('.'), sub.rsplit_once('.'))
    else {
        return false;
    };
    let vstem = vstem.to_lowercase();
    let sstem = sstem.to_lowercase();
    if sstem == vstem {
        return true;
    }
    sstem
        .strip_prefix(&vstem)
        .is_some_and(|rest| rest.chars().next().is_some_and(|c| !c.is_alphanumeric()))
}

/// 运行系统文件对话框命令。
///
/// 返回 `Some(paths)` 表示命令可用并已执行(用户取消时为 `Some(空)`);
/// 返回 `None` 表示该命令不存在, 应尝试下一个后端。
fn run_dialog(cmd: &str, args: &[std::ffi::OsString]) -> Option<Vec<PathBuf>> {
    let out = match std::process::Command::new(cmd).args(args).output() {
        Ok(o) => o,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            tracing::warn!("调用 {cmd} 失败: {e}");
            return Some(Vec::new());
        }
    };
    if !out.status.success() {
        // 用户取消。
        return Some(Vec::new());
    }
    let text = String::from_utf8_lossy(&out.stdout);
    Some(
        text.lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .collect(),
    )
}

/// 用 rfd(portal) 兜底弹目录框(阻塞)。
fn pick_folder_rfd(initial: &std::path::Path) -> Option<PathBuf> {
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

/// 目录选择(阻塞), 供同步调用与后台线程使用。返回空表示取消。
fn pick_dir_blocking(initial: &std::path::Path) -> Vec<PathBuf> {
    let dir = initial.to_string_lossy().to_string();
    if let Some(v) = run_dialog(
        "kdialog",
        &[
            std::ffi::OsString::from("--getexistingdirectory"),
            dir.clone().into(),
        ],
    ) {
        return v;
    }
    if let Some(v) = run_dialog(
        "zenity",
        &[
            std::ffi::OsString::from("--file-selection"),
            std::ffi::OsString::from("--directory"),
            format!("--filename={dir}/").into(),
        ],
    ) {
        return v;
    }
    pick_folder_rfd(initial).into_iter().collect()
}

/// 弹出一个原生目录选择框(阻塞式)。
pub(crate) fn pick_folder(initial: &std::path::Path) -> Option<PathBuf> {
    pick_dir_blocking(initial).into_iter().next()
}

/// 异步弹出目录选择框(0 或 1 个路径)。
pub(crate) fn pick_dir_async(
    initial: &std::path::Path,
) -> std::sync::mpsc::Receiver<Vec<PathBuf>> {
    let (tx, rx) = std::sync::mpsc::channel();
    let initial = initial.to_path_buf();
    std::thread::spawn(move || {
        let _ = tx.send(pick_dir_blocking(&initial));
    });
    rx
}

/// 多选文件(阻塞), 供后台线程调用。
fn pick_files_blocking(initial: &std::path::Path) -> Vec<PathBuf> {
    let dir = initial.to_string_lossy().to_string();
    if let Some(v) = run_dialog(
        "kdialog",
        &[
            std::ffi::OsString::from("--getopenfilename"),
            std::ffi::OsString::from("--multiple"),
            std::ffi::OsString::from("--separate-output"),
            dir.clone().into(),
        ],
    ) {
        tracing::debug!("文件选择: kdialog 选中 {} 个", v.len());
        return v;
    }
    if let Some(v) = run_dialog(
        "zenity",
        &[
            std::ffi::OsString::from("--file-selection"),
            std::ffi::OsString::from("--multiple"),
            std::ffi::OsString::from("--separator=\n"),
            format!("--filename={dir}/").into(),
        ],
    ) {
        tracing::debug!("文件选择: zenity 选中 {} 个", v.len());
        return v;
    }
    tracing::debug!("文件选择: 回退 rfd(portal)");
    // rfd(portal) 兜底。
    let initial = initial.to_path_buf();
    let handles = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .ok()?;
        rt.block_on(async move {
            rfd::AsyncFileDialog::new()
                .set_directory(&initial)
                .pick_files()
                .await
        })
    })
    .join()
    .ok()
    .flatten();
    handles
        .map(|hs| hs.into_iter().map(|h| h.path().to_path_buf()).collect())
        .unwrap_or_default()
}

/// 异步弹出原生多选文件框。
///
/// 对话框在独立线程执行, 选完的结果通过返回的 receiver 送达;
/// 不阻塞 UI 线程。
pub(crate) fn pick_files_async(
    initial: &std::path::Path,
) -> std::sync::mpsc::Receiver<Vec<PathBuf>> {
    let (tx, rx) = std::sync::mpsc::channel();
    let initial = initial.to_path_buf();
    std::thread::spawn(move || {
        let files = pick_files_blocking(&initial);
        let _ = tx.send(files);
    });
    rx
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
    use super::{is_media_file, is_subtitle_file, subtitle_of};

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

    #[test]
    fn detects_subtitles() {
        assert!(is_subtitle_file("x.ass"));
        assert!(is_subtitle_file("x.SRT"));
        assert!(is_subtitle_file("x.zh-CN.vtt"));
        assert!(!is_subtitle_file("x.mkv"));
        assert!(!is_subtitle_file("x.txt"));
    }

    #[test]
    fn matches_same_episode_subtitles() {
        let video = "[VCB-Studio] K-ON!! [01][Ma10p_1080p][x265_flac_2aac].mkv";
        assert!(subtitle_of(video, "[VCB-Studio] K-ON!! [01][Ma10p_1080p][x265_flac_2aac].sc.ass"));
        assert!(subtitle_of(video, "[VCB-Studio] K-ON!! [01][Ma10p_1080p][x265_flac_2aac].ass"));
        assert!(subtitle_of("ep01.mkv", "ep01.chs.srt"));
        assert!(subtitle_of("ep01.mkv", "ep01-CN.ass"));
        // 忽略大小写, 且多字节文件名不 panic。
        assert!(subtitle_of("Show.EP01.mkv", "show.ep01.ass"));
        assert!(subtitle_of("动画.01.mkv", "动画.01.chs.ass"));
    }

    #[test]
    fn rejects_other_episode_or_unrelated() {
        assert!(!subtitle_of("ep01.mkv", "ep010.ass"));
        assert!(!subtitle_of("ep01.mkv", "ep02.ass"));
        assert!(!subtitle_of("ep01.mkv", "extra.ass"));
        assert!(!subtitle_of("ep01.mkv", "noext"));
    }
}
