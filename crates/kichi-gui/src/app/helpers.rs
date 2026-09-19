use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{self, vec2, Color32, FontId, Painter, Pos2, Rect, Stroke};

use crate::icons::Glyph;
use crate::theme::{mix, Theme};

/// 统一的单行输入框样式: 更舒适的内边距 / 最小高度, 与卡片圆角一致。
pub(crate) fn input(text: &mut String) -> egui::TextEdit<'_> {
    egui::TextEdit::singleline(text)
        .margin(egui::Margin::symmetric(12, 9))
        .min_size(vec2(0.0, 38.0))
        .font(FontId::proportional(14.0))
}

/// 「用系统默认程序打开」的结果(后台探针在 xdg-open 退出后回传)。
pub(crate) enum OpenOutcome {
    /// 已交给系统(退出码 0, 或 1s 内未退出, 视为已启动)。
    Launched,
    /// 系统没有可打开该目标的关联程序。
    NoHandler,
    /// 其它失败(附 xdg-open 的错误输出 / 退出码)。
    Failed(String),
}

/// xdg-open 的观察窗口: 超时仍未退出即视为已启动。
/// 个别 handler 会阻塞到程序关闭, 不能一直等。
const OPEN_PROBE_TIMEOUT: Duration = Duration::from_millis(1000);

/// xdg-open(及其 KDE/GTK 后端)在「没有可用的关联程序」时的常见措辞。
fn looks_like_no_handler(err: &str) -> bool {
    let e = err.to_ascii_lowercase();
    [
        "no method available",
        "no application",
        "no default application",
        "no handler",
        "not supported",
        "unhandled",
    ]
    .iter()
    .any(|pat| e.contains(pat))
}

/// 用系统默认程序打开文件 / 目录 / URL。
///
/// `xdg-open` 只负责转发: 没有关联程序时它会失败退出, 但 `spawn` 察觉不到,
/// 界面会「假成功」。这里在后台线程等它退出(最多 [`OPEN_PROBE_TIMEOUT`]),
/// 结果经返回的通道回传, 由 UI 给出诚实的成功 / 失败提示。
pub(crate) fn open_async(
    target: impl Into<std::ffi::OsString>,
) -> std::sync::mpsc::Receiver<OpenOutcome> {
    let (tx, rx) = std::sync::mpsc::channel();
    let target = target.into();
    std::thread::spawn(move || {
        let mut child = match std::process::Command::new("xdg-open")
            .arg(&target)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                let _ = tx.send(OpenOutcome::Failed(e.to_string()));
                return;
            }
        };
        let deadline = Instant::now() + OPEN_PROBE_TIMEOUT;
        let status = loop {
            match child.try_wait() {
                Ok(Some(s)) => break Some(s),
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(40));
                }
                Ok(None) => break None,
                Err(e) => {
                    let _ = tx.send(OpenOutcome::Failed(e.to_string()));
                    return;
                }
            }
        };
        let outcome = match status {
            // 超时仍在运行: 已交给系统, 不打断它。
            None => OpenOutcome::Launched,
            Some(s) if s.success() => OpenOutcome::Launched,
            Some(s) => {
                let mut err = String::new();
                if let Some(pipe) = child.stderr.take() {
                    use std::io::Read;
                    let _ = pipe.take(4096).read_to_string(&mut err);
                }
                let err = err.trim();
                if looks_like_no_handler(err) {
                    OpenOutcome::NoHandler
                } else if err.is_empty() {
                    OpenOutcome::Failed(format!("xdg-open 退出码 {:?}", s.code()))
                } else {
                    OpenOutcome::Failed(err.lines().next().unwrap_or(err).to_string())
                }
            }
        };
        let _ = tx.send(outcome);
    });
    rx
}

/// 用系统默认浏览器打开一个 URL(交给 xdg-open, 同样适用于分享链接)。
pub(crate) fn open_url(url: &str) -> std::io::Result<()> {
    std::process::Command::new("xdg-open")
        .arg(url)
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

/// 判断字幕是否与某视频同集: 去掉扩展名后与视频名相同(忽略大小写), 或以视频名为前缀
/// 且其后紧跟非字母数字分隔符(如 `.sc.ass` / `_chs.srt` / `.zh-CN.ass`)。
pub(crate) fn subtitle_of(video: &str, sub: &str) -> bool {
    let (Some((vstem, _)), Some((sstem, _))) = (video.rsplit_once('.'), sub.rsplit_once('.'))
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
pub(crate) fn pick_dir_async(initial: &std::path::Path) -> std::sync::mpsc::Receiver<Vec<PathBuf>> {
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

/// 复选框三态(供传输任务 / 离线任务等列表共用)。
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum CheckState {
    Unchecked,
    Checked,
    Partial,
}

/// 在给定矩形内绘制现代化复选框(不处理点击)。
pub(crate) fn paint_checkbox(
    painter: &Painter,
    th: &Theme,
    rect: Rect,
    state: CheckState,
    hovered: bool,
) {
    let size = rect.width();
    let (fill, border) = match state {
        CheckState::Checked | CheckState::Partial => (th.accent, th.accent),
        CheckState::Unchecked => {
            if hovered {
                (th.card, mix(th.border, th.text_weak, 0.65))
            } else {
                (th.card, mix(th.border, th.text_faint, 0.45))
            }
        }
    };
    painter.rect_filled(rect, th.cr(4), fill);
    painter.rect_stroke(
        rect,
        th.cr(4),
        Stroke::new(1.0, border),
        egui::StrokeKind::Inside,
    );

    match state {
        CheckState::Checked => {
            let p1 = Pos2::new(rect.min.x + size * 0.26, rect.center().y + size * 0.02);
            let p2 = Pos2::new(rect.min.x + size * 0.43, rect.max.y - size * 0.28);
            let p3 = Pos2::new(rect.max.x - size * 0.24, rect.min.y + size * 0.30);
            painter.add(egui::Shape::line(
                vec![p1, p2, p3],
                Stroke::new(1.8, th.on_accent),
            ));
        }
        CheckState::Partial => {
            let y = rect.center().y;
            painter.line_segment(
                [
                    Pos2::new(rect.min.x + size * 0.28, y),
                    Pos2::new(rect.max.x - size * 0.28, y),
                ],
                Stroke::new(1.8, th.on_accent),
            );
        }
        CheckState::Unchecked => {}
    }
}

/// 绘制列表卡片底盘: 底色(依 hover / 选中) + 描边 + 选中时左侧 accent 条。
pub(crate) fn card_shell(painter: &Painter, th: &Theme, rect: Rect, hovered: bool, selected: bool) {
    let bg = if selected {
        mix(th.card, th.accent, if th.dark { 0.22 } else { 0.12 })
    } else if hovered {
        mix(th.card, th.text, if th.dark { 0.05 } else { 0.03 })
    } else {
        th.card
    };
    painter.rect_filled(rect, th.cr(12), bg);
    painter.rect_stroke(
        rect,
        th.cr(12),
        Stroke::new(1.0, th.border),
        egui::StrokeKind::Inside,
    );
    if selected {
        painter.rect_filled(
            Rect::from_min_max(
                Pos2::new(rect.min.x + 3.0, rect.min.y + 12.0),
                Pos2::new(rect.min.x + 5.0, rect.max.y - 12.0),
            ),
            th.cr(2),
            th.accent,
        );
    }
}

/// 绘制一个图标动作按钮(hover 底 + tooltip + 危险色), 返回是否被点击。
#[allow(clippy::too_many_arguments)]
pub(crate) fn icon_action(
    ui: &mut egui::Ui,
    painter: &Painter,
    th: &Theme,
    rect: Rect,
    id: egui::Id,
    glyph: Glyph,
    tip: &str,
    danger: bool,
) -> bool {
    let resp = ui.interact(rect, id, egui::Sense::click());
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        painter.rect_filled(rect, th.cr(6), th.hover);
    }
    let color = if danger { th.danger } else { th.text_weak };
    crate::icons::paint(painter, rect.shrink(6.0), glyph, color);
    resp.on_hover_text(tip).clicked()
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
    use super::{looks_like_no_handler, subtitle_of};

    #[test]
    fn detects_no_handler_wording() {
        assert!(looks_like_no_handler(
            "xdg-open: no method available for opening '/tmp/a.zzz'"
        ));
        assert!(looks_like_no_handler(
            "No application is registered as handling this file"
        ));
        assert!(looks_like_no_handler(
            "no default application for text/x-zzz"
        ));
        // 文件不存在等其它失败不能被误判成「无关联程序」。
        assert!(!looks_like_no_handler(
            "xdg-open: file '/tmp/a' does not exist"
        ));
        assert!(!looks_like_no_handler(""));
    }

    #[test]
    fn matches_same_episode_subtitles() {
        let video = "[VCB-Studio] K-ON!! [01][Ma10p_1080p][x265_flac_2aac].mkv";
        assert!(subtitle_of(
            video,
            "[VCB-Studio] K-ON!! [01][Ma10p_1080p][x265_flac_2aac].sc.ass"
        ));
        assert!(subtitle_of(
            video,
            "[VCB-Studio] K-ON!! [01][Ma10p_1080p][x265_flac_2aac].ass"
        ));
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
