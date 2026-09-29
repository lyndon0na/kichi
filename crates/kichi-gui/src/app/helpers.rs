use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{self, vec2, Color32, FontId, Painter, Pos2, Rect, Stroke};

use crate::icons::{self, Glyph};
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

/// 是否运行在 Flatpak 沙箱内: 沙箱里没有 mpv 这类宿主程序, 需要经 flatpak-spawn 借用。
pub(crate) fn in_flatpak() -> bool {
    std::env::var_os("FLATPAK_ID").is_some() || std::path::Path::new("/.flatpak-info").exists()
}

/// 构造调用宿主程序的命令: Flatpak 沙箱内走 `flatpak-spawn --host <程序>`,
/// 沙箱外直接执行, 两种形态下后续参数完全一致。
fn host_command(program: &str, flatpak: bool) -> std::process::Command {
    if flatpak {
        let mut cmd = std::process::Command::new("flatpak-spawn");
        cmd.arg("--host").arg(program);
        cmd
    } else {
        std::process::Command::new(program)
    }
}

/// mpv 就绪探针的结果(后台观察线程回传)。
pub(crate) enum PlayerOutcome {
    /// 窗口 / 视频输出(VO)已配置完成, mpv 窗口即将(或已经)显示。
    Ready,
    /// 在窗口就绪前 mpv 已退出(多为流打不开 / 加载失败)。
    Exited,
    /// IPC 不可用(连接失败 / 无该属性), 无法确认就绪 —— 按旧行为兜底。
    Unknown,
}

/// mpv 就绪探针句柄: 结果经后台线程回传(见 [`play_with_mpv`])。
pub(crate) struct PlayerWatch {
    pub(crate) rx: std::sync::mpsc::Receiver<PlayerOutcome>,
}

/// mpv IPC socket 的唯一序号(同一进程内多次播放不重名)。
static MPV_SOCKET_SEQ: AtomicU64 = AtomicU64::new(0);

/// mpv IPC socket 路径: `~/.cache/kichi/mpv-<pid>-<n>.sock`。
///
/// 放 home 下而非 `XDG_RUNTIME_DIR`: Flatpak 沙箱里运行时, socket 要能被
/// `flatpak-spawn --host` 起的宿主 mpv 与沙箱内的应用同时看见(home 共享)。
fn mpv_socket_path() -> PathBuf {
    let seq = MPV_SOCKET_SEQ.fetch_add(1, Ordering::Relaxed);
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("kichi")
        .join(format!("mpv-{}-{seq}.sock", std::process::id()))
}

/// 去掉一行 IPC 输出里的空白, 兼容 JSON 书写差异。
fn squash(line: &str) -> String {
    line.chars().filter(|c| !c.is_whitespace()).collect()
}

/// 是否「`vo-configured` 变为 true」的属性变更 —— mpv 窗口 / VO 就绪的信号。
///
/// `--force-window` 的窗口只在初始化完成后创建(网络流加载慢 / 失败时窗口迟迟
/// 不出现甚至不出现), 该属性从 false 翻到 true 的时机与窗口创建同步(本机
/// mpv 0.41 实测: 本地图片 0.31s 翻 true; 坏链接全程 false、一直不开窗)。
fn vo_configured_true(line: &str) -> bool {
    let l = squash(line);
    l.contains("\"name\":\"vo-configured\"") && l.contains("\"data\":true")
}

/// 是否「观察注册失败」的应答(老版本 mpv 没有 `vo-configured` 属性)。
fn observe_rejected(line: &str) -> bool {
    let l = squash(line);
    l.contains("\"request_id\":1")
        && l.contains("\"error\":")
        && !l.contains("\"error\":\"success\"")
}

/// 观察 mpv 的 `vo-configured`, 判断播放窗口是否真的就绪。
fn watch_mpv(sock: &std::path::Path) -> PlayerOutcome {
    // socket 由 mpv 启动时建立(实测 <0.1s); 连不上(给足 5s 重试)视为不可用。
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut conn = None;
    while Instant::now() < deadline {
        match UnixStream::connect(sock) {
            Ok(s) => {
                conn = Some(s);
                break;
            }
            Err(_) => std::thread::sleep(Duration::from_millis(20)),
        }
    }
    let Some(mut conn) = conn else {
        return PlayerOutcome::Unknown;
    };
    let req = "{\"command\":[\"observe_property\",1,\"vo-configured\"],\"request_id\":1}\n";
    if conn.write_all(req.as_bytes()).is_err() {
        return PlayerOutcome::Unknown;
    }
    for line in BufReader::new(conn).lines() {
        let Ok(line) = line else { break };
        if vo_configured_true(&line) {
            return PlayerOutcome::Ready;
        }
        if observe_rejected(&line) {
            return PlayerOutcome::Unknown;
        }
    }
    // 读到 EOF: mpv 在窗口就绪前退出了。
    PlayerOutcome::Exited
}

/// 用 mpv 流式播放直链, 并携带签名直链所需的请求头。
/// `subs` 为同集外挂字幕的本地路径, 会作为 `--sub-file` 挂载。
///
/// 返回就绪探针: mpv 窗口 / VO 配置完成(或提前退出 / IPC 不可用)时经通道回传,
/// 供 UI 把「正在唤起」的常驻提示收尾到真实结果。
pub(crate) fn play_with_mpv(
    name: &str,
    url: &str,
    headers: &[(String, String)],
    subs: &[PathBuf],
) -> std::io::Result<PlayerWatch> {
    let mut cmd = host_command("mpv", in_flatpak());
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
    // 就绪探针的 IPC socket(唯一命名; 清掉同名残留后交给 mpv 建立)。
    let sock = mpv_socket_path();
    if let Some(dir) = sock.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::remove_file(&sock);
    cmd.arg(format!("--input-ipc-server={}", sock.display()));
    cmd.arg("--").arg(url);
    cmd.spawn()?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let outcome = watch_mpv(&sock);
        let _ = std::fs::remove_file(&sock);
        let _ = tx.send(outcome);
    });
    Ok(PlayerWatch { rx })
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

/// 本地选择框的起始目录: 记忆目录仍存在则用它, 否则 home, 再退回进程 CWD。
pub(crate) fn picker_start_dir(remembered: &str) -> PathBuf {
    let p = PathBuf::from(remembered);
    if !remembered.is_empty() && p.is_dir() {
        return p;
    }
    dirs::home_dir()
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_default()
}

/// 本次选择要记住的目录: 目录选择记其自身, 文件选择记首个文件的父目录。
pub(crate) fn picked_dir(paths: &[PathBuf], is_dir: bool) -> Option<PathBuf> {
    let first = paths.first()?;
    if is_dir {
        Some(first.clone())
    } else {
        first.parent().map(std::path::Path::to_path_buf)
    }
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

/// 目录选择器里的一行文件夹(矢量文件夹图标 + 名称, 超长截断)。
pub(crate) fn folder_row(ui: &mut egui::Ui, th: &Theme, name: &str) -> bool {
    let h = 30.0;
    let w = ui.available_width().max(120.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, h), egui::Sense::click());
    let painter = ui.painter().clone();
    if resp.hovered() {
        painter.rect_filled(rect, th.cr(6), th.hover);
    }
    let icon_rect = Rect::from_center_size(
        Pos2::new(rect.min.x + 16.0, rect.center().y),
        vec2(18.0, 18.0),
    );
    icons::paint(
        &painter,
        icon_rect,
        Glyph::Folder,
        Color32::from_rgb(232, 178, 84),
    );
    let g = truncate_text(
        &painter,
        name,
        rect.width() - 34.0,
        FontId::proportional(13.5),
        th.text,
    );
    painter.galley(
        Pos2::new(rect.min.x + 32.0, rect.center().y - g.size().y / 2.0),
        g,
        th.text,
    );
    resp.clicked()
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
        Stroke::new(1.0_f32, border),
        egui::StrokeKind::Inside,
    );

    match state {
        CheckState::Checked => {
            let p1 = Pos2::new(rect.min.x + size * 0.26, rect.center().y + size * 0.02);
            let p2 = Pos2::new(rect.min.x + size * 0.43, rect.max.y - size * 0.28);
            let p3 = Pos2::new(rect.max.x - size * 0.24, rect.min.y + size * 0.30);
            painter.add(egui::Shape::line(
                vec![p1, p2, p3],
                Stroke::new(1.8_f32, th.on_accent),
            ));
        }
        CheckState::Partial => {
            let y = rect.center().y;
            painter.line_segment(
                [
                    Pos2::new(rect.min.x + size * 0.28, y),
                    Pos2::new(rect.max.x - size * 0.28, y),
                ],
                Stroke::new(1.8_f32, th.on_accent),
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
        Stroke::new(1.0_f32, th.border),
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

/// 中文字体候选: 「挂载根 × 相对路径」两维展开。Flatpak 沙箱里运行时自带字体中
/// 没有中文, 宿主字体由 flatpak 挂到 /run/host/fonts, 目录结构与 /usr/share/fonts 相同。
const FONT_ROOTS: [&str; 2] = ["/usr/share/fonts", "/run/host/fonts"];
const FONT_FILES: [&str; 7] = [
    "google-droid-sans-fonts/DroidSansFallbackFull.ttf",
    "wqy-zenhei-fonts/wqy-zenhei.ttc",
    "truetype/wqy/wqy-zenhei.ttc",
    "truetype/droid/DroidSansFallbackFull.ttf",
    "noto-cjk/NotoSansCJK-Regular.ttc",
    "opentype/noto/NotoSansCJK-Regular.ttc",
    "google-noto-sans-cjk-vf-fonts/NotoSansCJK-VF.ttc",
];

/// 按候选顺序返回第一款可读的中文字体。
fn read_cjk_font_from(roots: &[&str], files: &[&str]) -> Option<Vec<u8>> {
    for root in roots {
        for rel in files {
            if let Ok(bytes) = std::fs::read(format!("{root}/{rel}")) {
                return Some(bytes);
            }
        }
    }
    None
}

pub(crate) fn install_fonts(ctx: &egui::Context) -> bool {
    let mut fonts = egui::FontDefinitions::default();
    let loaded = match read_cjk_font_from(&FONT_ROOTS, &FONT_FILES) {
        Some(bytes) => {
            let name = "cjk".to_string();
            fonts
                .font_data
                .insert(name.clone(), Arc::new(egui::FontData::from_owned(bytes)));
            for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                fonts.families.entry(family).or_default().push(name.clone());
            }
            true
        }
        None => false,
    };
    ctx.set_fonts(fonts);
    loaded
}

#[cfg(test)]
mod tests {
    use super::{
        host_command, looks_like_no_handler, observe_rejected, picked_dir, picker_start_dir,
        read_cjk_font_from, subtitle_of, vo_configured_true, watch_mpv, PlayerOutcome,
    };

    #[test]
    fn picker_start_prefers_existing_remembered_dir() {
        let dir = std::env::temp_dir().join(format!("kichi-pick-{}-hit", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // 末尾带分隔符的写法同样应被认作有效目录。
        let trailing = format!("{}/", dir.to_str().unwrap());
        for remembered in [dir.to_str().unwrap(), trailing.as_str()] {
            assert_eq!(picker_start_dir(remembered), dir);
        }
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn picker_start_falls_back_when_remembered_dir_gone() {
        let missing =
            std::env::temp_dir().join(format!("kichi-pick-{}-missing", std::process::id()));
        let _ = std::fs::remove_dir_all(&missing);
        for remembered in ["", missing.to_str().unwrap()] {
            let start = picker_start_dir(remembered);
            assert_ne!(start, missing);
            // home 可用时优先 home, 否则退回进程 CWD, 都拿不到才是空路径。
            match dirs::home_dir() {
                Some(home) => assert_eq!(start, home),
                None => assert_eq!(start, std::env::current_dir().unwrap_or_default()),
            }
        }
    }

    #[test]
    fn picked_dir_remembers_self_for_dir_and_parent_for_files() {
        let base = std::path::PathBuf::from("/tmp/kichi-pick-cases");
        assert_eq!(picked_dir(&[base.join("a.zip")], false), Some(base.clone()));
        // 多选时以第一个所选文件的位置为准。
        assert_eq!(
            picked_dir(
                &[base.join("a.zip"), std::path::PathBuf::from("/etc/b.iso")],
                false
            ),
            Some(base.clone())
        );
        assert_eq!(
            picked_dir(&[base.join("sub")], true),
            Some(base.join("sub"))
        );
        assert_eq!(picked_dir(&[], false), None);
        assert_eq!(picked_dir(&[], true), None);
    }

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

    #[test]
    fn flatpak_prefixes_host_spawn() {
        let native = host_command("mpv", false);
        assert_eq!(native.get_program(), "mpv");
        assert_eq!(native.get_args().count(), 0);

        let sandboxed = host_command("mpv", true);
        assert_eq!(sandboxed.get_program(), "flatpak-spawn");
        let args: Vec<String> = sandboxed
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args, ["--host", "mpv"]);
    }

    #[test]
    fn cjk_font_lookup_scans_roots_then_files() {
        let root = std::env::temp_dir().join(format!("kichi-font-{}", std::process::id()));
        let rel = "wqy-zenhei-fonts/wqy-zenhei.ttc";
        std::fs::create_dir_all(root.join("wqy-zenhei-fonts")).unwrap();
        std::fs::write(root.join(rel), b"font").unwrap();

        let files = ["nope/x.ttf", rel];
        // 宿主根与沙箱根共用同一份相对路径表, 命中即止。
        assert_eq!(
            read_cjk_font_from(&["/nonexistent", root.to_str().unwrap()], &files).as_deref(),
            Some(&b"font"[..])
        );
        assert_eq!(read_cjk_font_from(&["/nonexistent"], &files), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn detects_mpv_vo_configured_and_observe_failure() {
        // 样例取自 mpv 0.41 实测 IPC 输出。
        assert!(vo_configured_true(
            "{\"event\":\"property-change\",\"name\":\"vo-configured\",\"data\":true}"
        ));
        // 带空格的 JSON 同样识别。
        assert!(vo_configured_true(
            "{ \"event\": \"property-change\", \"id\": 1, \"name\": \"vo-configured\", \"data\": true }"
        ));
        assert!(!vo_configured_true(
            "{\"event\":\"property-change\",\"name\":\"vo-configured\",\"data\":false}"
        ));
        assert!(!vo_configured_true(
            "{\"event\":\"property-change\",\"id\":2,\"name\":\"osd-width\",\"data\":2304}"
        ));
        // observe 应答: success 不算失败, 属性缺失(老 mpv)才算。
        assert!(!observe_rejected(
            "{\"request_id\":1,\"error\":\"success\"}"
        ));
        assert!(observe_rejected(
            "{\"request_id\":1,\"error\":\"property not found\"}"
        ));
        assert!(!observe_rejected(
            "{\"event\":\"start-file\",\"playlist_entry_id\":1}"
        ));
    }

    #[test]
    fn mpv_probe_reports_ready_exited_and_unknown() {
        use std::io::{BufRead, BufReader, Write};
        use std::os::unix::net::UnixListener;

        let dir = std::env::temp_dir().join(format!("kichi-mpv-probe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        // 扮演 mpv: 收到观察命令后回「初始 false → 就绪 true」。
        let ready = dir.join("ready.sock");
        let _ = std::fs::remove_file(&ready);
        let listener = UnixListener::bind(&ready).unwrap();
        let server = std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut cmd = String::new();
            BufReader::new(conn.try_clone().unwrap())
                .read_line(&mut cmd)
                .unwrap();
            conn.write_all(b"{\"request_id\":1,\"error\":\"success\"}\n")
                .unwrap();
            conn.write_all(
                b"{\"event\":\"property-change\",\"name\":\"vo-configured\",\"data\":false}\n",
            )
            .unwrap();
            conn.write_all(
                b"{\"event\":\"property-change\",\"name\":\"vo-configured\",\"data\":true}\n",
            )
            .unwrap();
            cmd
        });
        assert!(matches!(watch_mpv(&ready), PlayerOutcome::Ready));
        let cmd = server.join().unwrap();
        assert!(cmd.contains("observe_property") && cmd.contains("vo-configured"));

        // 收到命令后直接断开(模拟 mpv 加载失败提前退出)。
        let exited = dir.join("exited.sock");
        let _ = std::fs::remove_file(&exited);
        let listener = UnixListener::bind(&exited).unwrap();
        let server = std::thread::spawn(move || {
            let (conn, _) = listener.accept().unwrap();
            let mut cmd = String::new();
            let _ = BufReader::new(conn).read_line(&mut cmd);
        });
        assert!(matches!(watch_mpv(&exited), PlayerOutcome::Exited));
        server.join().unwrap();

        // 老版本 mpv 没有该属性: observe 应答失败 → 无法确认。
        let rejected = dir.join("rejected.sock");
        let _ = std::fs::remove_file(&rejected);
        let listener = UnixListener::bind(&rejected).unwrap();
        let server = std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut cmd = String::new();
            BufReader::new(conn.try_clone().unwrap())
                .read_line(&mut cmd)
                .unwrap();
            conn.write_all(b"{\"request_id\":1,\"error\":\"property not found\"}\n")
                .unwrap();
        });
        assert!(matches!(watch_mpv(&rejected), PlayerOutcome::Unknown));
        server.join().unwrap();

        let _ = std::fs::remove_dir_all(&dir);
    }
}
