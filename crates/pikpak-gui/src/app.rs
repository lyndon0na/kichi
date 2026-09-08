use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{self, Align, Align2, Color32, CornerRadius, FontId, Frame, Key, Layout, Margin, Pos2, Rect, RichText, Stroke, vec2, UiBuilder};
use pikpak_core::session;
use pikpak_core::types::{task_file_id, task_id, task_name, task_size, File, Quota, Task};

use crate::format::{self, phase_label};
use crate::icons::{self, Glyph};
use crate::msg::{Cmd, Msg};
use crate::settings;
use crate::theme::{self, mix, Theme};
use crate::worker;

#[derive(PartialEq, Clone, Copy)]
enum Page {
    Files,
    Tasks,
    Downloads,
    Settings,
}

#[derive(PartialEq, Eq, Clone, Copy, PartialOrd, Ord, Hash)]
enum SortBy {
    Name,
    Size,
    Modified,
}

struct Crumb {
    id: Option<String>,
    label: String,
}

#[derive(Clone)]
enum RowAction {
    OpenFolder(String, String),
    OpenFile,
    DownloadFile(String, String),
    CopyName(String),
    Rename(String, String),
    Trash(String),
}

/// 行点击产生的选择请求。
enum RowSel {
    /// 普通单击: 只选中该项(替换原选择)。
    Replace(String),
    /// Ctrl+单击: 在选中/未选中之间切换。
    Toggle(String),
}

/// 本地下载任务的 UI 状态。
#[derive(Clone, PartialEq)]
enum DlStatus {
    Queued,
    Running,
    Done,
    Cancelled,
    Failed(String),
}

#[derive(Clone)]
struct DlJob {
    name: String,
    dir: PathBuf,
    total: u64,
    done: u64,
    status: DlStatus,
    /// 估算速率(bytes/s)。
    speed: u64,
    last_done: u64,
    last_at: Option<Instant>,
}

impl DlJob {
    fn queued(name: String, dir: PathBuf) -> Self {
        DlJob {
            name,
            dir,
            total: 0,
            done: 0,
            status: DlStatus::Queued,
            speed: 0,
            last_done: 0,
            last_at: None,
        }
    }
}

/// 下载窗口内的行级操作。
enum DlOp {
    Cancel,
    OpenDir,
    Remove,
}

pub struct App {
    tx: Sender<Cmd>,
    rx: Receiver<Msg>,

    // 认证
    auth_checking: bool,
    username: String,
    login_username: String,
    login_password: String,
    auth_error: Option<String>,
    dark: bool,

    page: Page,

    // 文件浏览
    stack: Vec<Crumb>,
    req_id: u64,
    selected: HashSet<String>,
    sort_by: SortBy,
    sort_desc: bool,
    filter: String,
    files: Vec<File>,
    dir_next: Option<String>,
    dir_loading: bool,

    // 离线
    offline_url: String,
    offline_name: String,
    offline_to_current: bool,
    buckets: BTreeMap<String, Vec<Task>>,
    tasks_loading: bool,

    quota: Option<Quota>,

    // 对话框
    mkdir_open: bool,
    mkdir_name: String,
    rename_id: Option<String>,
    rename_name: String,
    trash_confirm: Option<Vec<(String, String)>>, // [(id, name)]

    /// 已移入回收站、等待服务端列表同步的 id(本地先行隐藏)。
    hidden: HashSet<String>,

    // 本地下载
    download_dir: String,
    jobs: BTreeMap<u64, DlJob>,

    toast: Option<(Color32, String, Instant)>,
}

// ---------- 文件类型 -> 图标 / 颜色 ----------

fn file_visual(f: &File) -> (Glyph, Color32) {
    let light_gray = Color32::from_rgb(120, 126, 140);
    if f.is_folder() {
        return (Glyph::Folder, Color32::from_rgb(232, 178, 84));
    }
    let ext = f
        .name
        .rsplit_once('.')
        .map(|(_, e)| e.to_lowercase())
        .unwrap_or_default();
    let videos = [
        "mp4", "mkv", "avi", "mov", "wmv", "flv", "webm", "ts", "rmvb", "m4v",
    ];
    let audio = ["mp3", "flac", "wav", "aac", "ogg", "m4a", "opus", "ape"];
    let images = [
        "jpg", "jpeg", "png", "gif", "bmp", "webp", "heic", "svg", "tiff",
    ];
    let docs = [
        "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "txt", "md", "epub", "csv",
    ];
    let arch = ["zip", "rar", "7z", "tar", "gz", "bz2", "xz", "iso"];
    if videos.contains(&ext.as_str()) {
        (Glyph::Video, Color32::from_rgb(196, 130, 220))
    } else if audio.contains(&ext.as_str()) {
        (Glyph::Audio, Color32::from_rgb(104, 196, 136))
    } else if images.contains(&ext.as_str()) {
        (Glyph::Image, Color32::from_rgb(96, 184, 200))
    } else if docs.contains(&ext.as_str()) {
        (Glyph::Doc, Color32::from_rgb(214, 178, 96))
    } else if arch.contains(&ext.as_str()) {
        (Glyph::Archive, Color32::from_rgb(208, 142, 110))
    } else {
        (Glyph::File, light_gray)
    }
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        install_fonts(&cc.egui_ctx);
        let worker::Worker { tx, rx } = worker::spawn();
        let saved = settings::load();

        let mut app = App {
            tx,
            rx,
            auth_checking: false,
            username: String::new(),
            login_username: saved.username.clone(),
            login_password: String::new(),
            auth_error: None,
            dark: saved.dark,
            page: Page::Files,
            stack: vec![Crumb {
                id: None,
                label: "我的云盘".into(),
            }],
            req_id: 0,
            selected: HashSet::new(),
            sort_by: SortBy::Name,
            sort_desc: false,
            filter: String::new(),
            files: Vec::new(),
            dir_next: None,
            dir_loading: false,
            offline_url: String::new(),
            offline_name: String::new(),
            offline_to_current: false,
            buckets: BTreeMap::new(),
            tasks_loading: false,
            quota: None,
            mkdir_open: false,
            mkdir_name: String::new(),
            rename_id: None,
            rename_name: String::new(),
            trash_confirm: None,
            hidden: HashSet::new(),
            download_dir: saved.download_dir.clone(),
            jobs: BTreeMap::new(),
            toast: None,
        };

        match session::load_session() {
            Ok(Some(s)) => {
                app.auth_checking = true;
                let _ = app.tx.send(Cmd::Resume {
                    device_id: s.device_id,
                    access_token: s.access_token,
                    refresh_token: s.refresh_token,
                    user_id: s.user_id,
                    username: s.username,
                });
            }
            Ok(None) => {}
            Err(e) => {
                app.toast = Some((
                    Color32::from_rgb(200, 90, 60),
                    e.to_string(),
                    Instant::now(),
                ))
            }
        }
        app
    }

    fn theme(&self) -> Theme {
        Theme::new(self.dark)
    }

    fn send(&self, cmd: Cmd) {
        let _ = self.tx.send(cmd);
    }

    fn drain(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::LoginOk { username } => {
                    self.username = username;
                    self.auth_checking = false;
                    self.auth_error = None;
                    self.buckets.clear();
                    self.quota = None;
                    self.reset_browse();
                    self.send(Cmd::RefreshQuota);
                    self.send(Cmd::RefreshTasks);
                    self.persist_settings();
                }
                Msg::SessionInvalid { reason } => {
                    self.auth_checking = false;
                    self.username.clear();
                    self.auth_error = Some(reason);
                    self.jobs.clear();
                }
                Msg::LoggedOut => {
                    self.username.clear();
                    self.auth_checking = false;
                    self.auth_error = None;
                    self.quota = None;
                    self.buckets.clear();
                    self.files.clear();
                    self.selected.clear();
                    self.hidden.clear();
                    self.jobs.clear();
                    self.reset_stack();
                }
                Msg::Files {
                    parent,
                    req_id,
                    append,
                    list,
                } => {
                    if self.req_id != req_id || parent != self.current_parent() {
                        continue;
                    }
                    self.dir_loading = false;
                    if append {
                        self.files.extend(list.files);
                    } else {
                        self.files = list.files;
                        self.selected.clear();
                    }
                    self.dir_next = list.next_page_token;
                    // 服务端已不再返回这些 id 时, 才解除本地隐藏
                    if !self.hidden.is_empty() {
                        let present: HashSet<String> =
                            self.files.iter().map(|f| f.id.clone()).collect();
                        self.hidden.retain(|id| present.contains(id));
                    }
                }
                Msg::FolderCreated => {
                    self.toast_ok("新建文件夹成功");
                    self.refresh_dir();
                }
                Msg::Renamed => {
                    self.rename_id = None;
                    self.toast_ok("重命名成功");
                    self.refresh_dir();
                }
                Msg::Trashed => {
                    self.trash_confirm = None;
                    self.selected.clear();
                    self.refresh_dir();
                }
                Msg::OfflineCreated => {
                    self.toast_ok("已提交离线下载");
                    self.offline_url.clear();
                    self.offline_name.clear();
                    self.send(Cmd::RefreshTasks);
                }
                Msg::OfflineRetried => self.send(Cmd::RefreshTasks),
                Msg::OfflineDeleted => self.send(Cmd::RefreshTasks),
                Msg::Quota(quota) => self.quota = quota,
                Msg::TasksAll { buckets } => {
                    self.buckets = buckets;
                    self.tasks_loading = false;
                }
                Msg::DlProgress {
                    req_id,
                    total,
                    done,
                } => {
                    if let Some(j) = self.jobs.get_mut(&req_id) {
                        if j.status == DlStatus::Queued {
                            j.status = DlStatus::Running;
                        }
                        if total > 0 {
                            j.total = total;
                        }
                        let now = Instant::now();
                        if let Some(at) = j.last_at {
                            let dt = now.duration_since(at).as_secs_f64();
                            if dt > 0.0 && done >= j.last_done {
                                let inst = ((done - j.last_done) as f64 / dt) as u64;
                                j.speed = if j.speed == 0 {
                                    inst
                                } else {
                                    ((j.speed as f64) * 0.6 + inst as f64 * 0.4) as u64
                                };
                            }
                        }
                        j.last_at = Some(now);
                        j.last_done = done;
                        if done > j.done {
                            j.done = done;
                        }
                    }
                }
                Msg::DlFinished { req_id, bytes } => {
                    if let Some(j) = self.jobs.get_mut(&req_id) {
                        j.status = DlStatus::Done;
                        j.done = bytes;
                        if bytes > 0 && j.total == 0 {
                            j.total = bytes;
                        }
                    }
                }
                Msg::DlCancelled { req_id } => {
                    if let Some(j) = self.jobs.get_mut(&req_id) {
                        j.status = DlStatus::Cancelled;
                    }
                }
                Msg::DlFailed { req_id, what } => {
                    if let Some(j) = self.jobs.get_mut(&req_id) {
                        j.status = DlStatus::Failed(what);
                    }
                }
                Msg::Error { what } => self.toast_err(&what),
            }
        }
    }

    fn toast(&mut self, msg: &str, color: Color32) {
        self.toast = Some((color, msg.to_string(), Instant::now()));
    }
    fn toast_ok(&mut self, msg: &str) {
        self.toast(msg, self.theme().ok);
    }
    fn toast_warn(&mut self, msg: &str) {
        self.toast(msg, self.theme().warn);
    }
    fn toast_err(&mut self, msg: &str) {
        self.toast(msg, self.theme().danger);
    }

    fn persist_settings(&self) {
        let name = if !self.username.is_empty() {
            self.username.clone()
        } else {
            self.login_username.trim().to_string()
        };
        settings::save(&settings::Settings {
            username: name,
            dark: self.dark,
            download_dir: self.download_dir.clone(),
        });
    }

    fn toggle_theme(&mut self) {
        self.dark = !self.dark;
        self.persist_settings();
    }

    fn current_parent(&self) -> Option<String> {
        self.stack.last().and_then(|c| c.id.clone())
    }

    fn reset_stack(&mut self) {
        self.stack = vec![Crumb {
            id: None,
            label: "我的云盘".into(),
        }];
        self.dir_next = None;
        self.dir_loading = true;
    }

    fn reset_browse(&mut self) {
        self.reset_stack();
        self.req_id += 1;
        let req = self.req_id;
        self.send(Cmd::ListFiles {
            parent: None,
            token: None,
            append: false,
            req_id: req,
        });
    }

    fn refresh_dir(&mut self) {
        self.req_id += 1;
        let req = self.req_id;
        let parent = self.current_parent();
        self.files.clear();
        self.dir_next = None;
        self.dir_loading = true;
        self.selected.clear();
        self.send(Cmd::ListFiles {
            parent,
            token: None,
            append: false,
            req_id: req,
        });
    }

    fn load_more(&mut self) {
        if self.dir_next.is_none() {
            return;
        }
        self.req_id += 1;
        let req = self.req_id;
        let parent = self.current_parent();
        let token = self.dir_next.clone();
        self.send(Cmd::ListFiles {
            parent,
            token,
            append: true,
            req_id: req,
        });
    }

    fn goto_folder(&mut self, id: &str, name: &str) {
        self.stack.push(Crumb {
            id: Some(id.to_string()),
            label: name.to_string(),
        });
        self.selected.clear();
        self.refresh_dir();
    }

    /// 当前目录内过滤后的可见文件(文件夹在前, 组内按当前排序)。
    fn visible_rows(&self) -> (Vec<File>, Vec<File>) {
        let kw = self.filter.trim().to_lowercase();
        let mut folders: Vec<&File> = Vec::new();
        let mut plain: Vec<&File> = Vec::new();
        for f in &self.files {
            if self.hidden.contains(&f.id) {
                continue;
            }
            let hit = kw.is_empty() || f.name.to_lowercase().contains(&kw);
            if !hit {
                continue;
            }
            if f.is_folder() {
                folders.push(f);
            } else {
                plain.push(f);
            }
        }
        let cmp = |a: &File, b: &File| -> std::cmp::Ordering {
            let o = match self.sort_by {
                SortBy::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                SortBy::Size => a.size.cmp(&b.size),
                SortBy::Modified => a
                    .modified_time
                    .as_deref()
                    .or(a.created_time.as_deref())
                    .cmp(&b.modified_time.as_deref().or(b.created_time.as_deref())),
            };
            if self.sort_desc {
                o.reverse()
            } else {
                o
            }
        };
        folders.sort_by(|a, b| cmp(a, b));
        plain.sort_by(|a, b| cmp(a, b));
        (
            folders.into_iter().cloned().collect(),
            plain.into_iter().cloned().collect(),
        )
    }

    fn selected_names(&self) -> Vec<(String, String)> {
        self.files
            .iter()
            .filter(|f| self.selected.contains(&f.id))
            .map(|f| (f.id.clone(), f.name.clone()))
            .collect()
    }

    /// 选中项里可下载的文件(id, name), 文件夹除外。
    fn selected_plain_files(&self) -> Vec<(String, String)> {
        self.files
            .iter()
            .filter(|f| self.selected.contains(&f.id) && !f.is_folder())
            .map(|f| (f.id.clone(), f.name.clone()))
            .collect()
    }

    fn selected_has_folder(&self) -> bool {
        self.files
            .iter()
            .any(|f| self.selected.contains(&f.id) && f.is_folder())
    }

    fn alloc_req_id(&mut self) -> u64 {
        self.req_id += 1;
        self.req_id
    }

    fn has_active_downloads(&self) -> bool {
        self.jobs
            .values()
            .any(|j| j.status == DlStatus::Queued || j.status == DlStatus::Running)
    }

    /// 弹原生目录选择框选保存位置(取消返回 None)。
    fn choose_download_dir(&mut self) -> Option<PathBuf> {
        let initial = {
            let p = PathBuf::from(&self.download_dir);
            if !self.download_dir.is_empty() && p.is_dir() {
                p
            } else {
                dirs::download_dir()
                    .or_else(dirs::home_dir)
                    .unwrap_or_default()
            }
        };
        let picked = pick_folder(&initial)?;
        if let Some(s) = picked.to_str() {
            self.download_dir = s.to_string();
        }
        self.persist_settings();
        Some(picked)
    }

    /// 逐个提交下载任务(共享同一个已选目录)。
    fn enqueue_downloads(&mut self, items: Vec<(String, String)>, dir: PathBuf) {
        if items.is_empty() {
            return;
        }
        self.page = Page::Downloads;
        for (id, name) in items {
            let req_id = self.alloc_req_id();
            self.jobs
                .insert(req_id, DlJob::queued(name.clone(), dir.clone()));
            self.send(Cmd::StartDownload {
                req_id,
                file_id: id,
                name,
                dest_dir: dir.clone(),
            });
        }
    }

    /// 弹目录选择框并下载单个文件。用于右键菜单/离线任务页。
    fn download_single(&mut self, id: String, name: String) {
        let Some(dir) = self.choose_download_dir() else {
            return;
        };
        self.enqueue_downloads(vec![(id, name)], dir);
    }
}

// ================= 主循环 =================

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain();

        let th = self.theme();
        theme::configure(ctx, &th);

        if self.auth_checking {
            ctx.request_repaint_after(Duration::from_millis(120));
        } else if self.has_active_downloads() {
            ctx.request_repaint_after(Duration::from_millis(80));
        } else {
            ctx.request_repaint_after(Duration::from_millis(600));
        }

        if self.username.is_empty() {
            self.login_ui(ctx, &th);
            return;
        }

        self.app_shell(ctx, &th);
        self.dialogs(ctx, &th);
        self.draw_toast(ctx);
    }
}

// ---------------- 应用骨架: 左侧边栏 + 主内容 ----------------

impl App {
    fn app_shell(&mut self, ctx: &egui::Context, th: &Theme) {
        egui::SidePanel::left("app_sidebar")
            .resizable(true)
            .default_width(228.0)
            .width_range(196.0..=320.0)
            .frame(
                Frame::new()
                    .fill(th.panel)
                    .stroke(Stroke::new(1.0, th.border))
                    .inner_margin(Margin::symmetric(10, 12)),
            )
            .show(ctx, |ui| {
                self.sidebar(ui, th);
            });

        match self.page {
            Page::Files => self.files_page(ctx, th),
            Page::Tasks => self.tasks_page(ctx, th),
            Page::Downloads => self.downloads_page(ctx, th),
            Page::Settings => self.settings_page(ctx, th),
        }
    }

    /// 单个导航项; 返回是否被点击。可选显示角标。
    fn nav_item(
        &mut self,
        ui: &mut egui::Ui,
        th: &Theme,
        glyph: Glyph,
        label: &str,
        selected: bool,
        badge: Option<usize>,
    ) -> bool {
        let height = 36.0;
        let (rect, resp) =
            ui.allocate_exact_size(vec2(ui.available_width(), height), egui::Sense::click());

        let bg = if selected {
            mix(th.panel, th.accent, if th.dark { 0.24 } else { 0.13 })
        } else if resp.hovered() {
            mix(th.panel, th.text, if th.dark { 0.08 } else { 0.06 })
        } else {
            Color32::TRANSPARENT
        };
        let painter = ui.painter().clone();
        painter.rect_filled(rect, CornerRadius::same(9), bg);
        if selected {
            // 左侧强调竖条
            painter.rect_filled(
                Rect::from_min_max(
                    Pos2::new(rect.min.x + 4.0, rect.min.y + (height - 18.0) / 2.0),
                    Pos2::new(rect.min.x + 6.0, rect.min.y + (height + 18.0) / 2.0),
                ),
                CornerRadius::same(1),
                th.accent,
            );
        }
        // 图标
        let icon_rect =
            Rect::from_center_size(Pos2::new(rect.min.x + 20.0, rect.center().y), vec2(18.0, 18.0));
        let icon_color = if selected { th.accent } else { th.text_weak };
        icons::paint(&painter, icon_rect, glyph, icon_color);
        // 文字
        let txt_color = if selected { th.text } else { th.text_weak };
        let txt = painter.layout_no_wrap(
            label.to_string(),
            FontId::proportional(14.0),
            txt_color,
        );
        painter.galley(
            Pos2::new(rect.min.x + 42.0, rect.center().y - txt.size().y / 2.0),
            txt,
            txt_color,
        );
        // 角标
        if let Some(n) = badge {
            let s = format!("{n}");
            let g = painter.layout_no_wrap(s, FontId::proportional(11.0), Color32::WHITE);
            let w = (g.size().x + 14.0).max(17.0);
            let r = Rect::from_min_size(
                Pos2::new(rect.right() - w - 8.0, rect.center().y - 9.0),
                vec2(w, 18.0),
            );
            painter.rect_filled(r, CornerRadius::same(9), mix(th.panel, th.accent, 0.9));
            painter.galley(Pos2::new(r.center().x - g.size().x / 2.0, r.min.y + 3.0), g, Color32::WHITE);
        }
        resp.clicked()
    }

    fn sidebar(&mut self, ui: &mut egui::Ui, th: &Theme) {
        // ---------- 顶部品牌 ----------
        let row_h = 40.0;
        let (brect, _) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), egui::Sense::hover());
        let bp = ui.painter().clone();
        let logo = Rect::from_center_size(Pos2::new(brect.min.x + 18.0, brect.center().y), vec2(30.0, 30.0));
        bp.rect_filled(logo, CornerRadius::same(9), th.accent);
        let ltxt = bp.layout_no_wrap(
            "P".into(),
            FontId::proportional(17.0),
            th.on_accent,
        );
        bp.galley(
            Pos2::new(logo.center().x - ltxt.size().x / 2.0, logo.center().y - ltxt.size().y / 2.0),
            ltxt,
            th.on_accent,
        );
        let brand = bp.layout_no_wrap("PikPak".into(), FontId::proportional(17.0), th.text);
        bp.galley(
            Pos2::new(brect.min.x + 42.0, brect.min.y + 3.0),
            brand,
            th.text,
        );
        let sub = bp.layout_no_wrap(
            "Linux 客户端".into(),
            FontId::proportional(11.0),
            th.text_faint,
        );
        bp.galley(Pos2::new(brect.min.x + 42.0, brect.min.y + 24.0), sub, th.text_faint);

        // 主题切换(放在品牌行右侧)
        let theme_btn = Rect::from_min_size(
            Pos2::new(brect.right() - 34.0, brect.center().y - 17.0),
            vec2(28.0, 28.0),
        );
        let tresp = ui.interact(theme_btn, ui.id().with("theme_toggle"), egui::Sense::click());
        let tb = if tresp.hovered() {
            mix(th.panel, th.text, if th.dark { 0.12 } else { 0.1 })
        } else {
            Color32::TRANSPARENT
        };
        bp.rect_filled(theme_btn, CornerRadius::same(7), tb);
        let sun = !self.dark;
        draw_sun_moon(&bp, theme_btn, if sun { Color32::from_rgb(226, 162, 54) } else { Color32::from_rgb(140, 150, 190) });
        if tresp.clicked() {
            self.toggle_theme();
        }

        ui.add_space(18.0);

        // ---------- 主导航 ----------
        let running = [
            "PHASE_TYPE_PENDING",
            "PHASE_TYPE_RUNNING",
            "PHASE_TYPE_ERROR",
        ]
        .iter()
        .map(|p| self.buckets.get(*p).map(|v| v.len()).unwrap_or(0))
        .sum::<usize>();

        let mut nav: Option<Page> = None;
        if self.nav_item(
            ui,
            th,
            Glyph::Folder,
            "网盘文件",
            self.page == Page::Files,
            None,
        ) {
            nav = Some(Page::Files);
        }
        if self.nav_item(
            ui,
            th,
            Glyph::Transfer,
            "离线下载",
            self.page == Page::Tasks,
            (running > 0).then_some(running),
        ) {
            nav = Some(Page::Tasks);
        }
        if self.nav_item(
            ui,
            th,
            Glyph::Gear,
            "设置",
            self.page == Page::Settings,
            None,
        ) {
            nav = Some(Page::Settings);
        }
        if let Some(p) = nav {
            self.page = p;
        }

        // 本地下载(常驻入口, 有进行中任务时显示角标)
        {
            ui.add_space(6.0);
            let active = self
                .jobs
                .values()
                .filter(|j| matches!(j.status, DlStatus::Queued | DlStatus::Running))
                .count();
            let done = self.jobs.len().saturating_sub(active);
            let clicked = self.nav_item(
                ui,
                th,
                Glyph::Download,
                "本地下载",
                self.page == Page::Downloads,
                (active > 0).then_some(active),
            );
            if clicked {
                self.page = Page::Downloads;
            }
            // 全部结束时给个小提示文字
            if !self.jobs.is_empty() && active == 0 && done > 0 {
                ui.add_space(2.0);
                ui.label(
                    RichText::new("全部任务已完成, 点击可查看")
                        .color(th.text_faint)
                        .size(11.0),
                );
            }
        }

        // ---------- 底部: 存储配额 + 账户 ----------
        let quota = self.quota.clone();
        let username = self.username.clone();
        ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
            ui.add_space(8.0);
            self.account_card(ui, th, &username);
            ui.add_space(8.0);
            self.quota_card(ui, th, quota.as_ref());
        });
    }

    /// 底部账户行(头像 + 用户名 + 退出登录)。
    fn account_card(&mut self, ui: &mut egui::Ui, th: &Theme, username: &str) -> egui::Response {
        let h = 40.0;
        let (rect, resp) =
            ui.allocate_exact_size(vec2(ui.available_width(), h), egui::Sense::click());
        let painter = ui.painter().clone();
        painter.rect_filled(rect, CornerRadius::same(10), Color32::TRANSPARENT);
        // 头像(用名字首字或 P)
        let initial = username
            .chars()
            .next()
            .map(|c| c.to_uppercase().collect::<String>())
            .unwrap_or_else(|| "P".into());
        let av = Rect::from_center_size(Pos2::new(rect.min.x + 19.0, rect.center().y), vec2(30.0, 30.0));
        painter.rect_filled(av, CornerRadius::same(15), mix(th.panel, th.accent, 0.28));
        let g = painter.layout_no_wrap(initial, FontId::proportional(14.0), th.accent);
        painter.galley(Pos2::new(av.center().x - g.size().x / 2.0, av.center().y - g.size().y / 2.0), g, th.accent);
        // 用户名(截断)
        let name_rect = Rect::from_min_max(Pos2::new(rect.min.x + 42.0, rect.min.y + 4.0), Pos2::new(rect.right() - 44.0, rect.max.y - 4.0));
        painter.rect_filled(Rect::from_min_max(Pos2::new(rect.min.x + 42.0, rect.min.y + 5.0), Pos2::new(rect.right() - 40.0, rect.max.y - 5.0)), CornerRadius::same(8), Color32::TRANSPARENT);
        let uname = truncate_text(&painter, username, name_rect.width(), FontId::proportional(13.5), th.text);
        painter.galley(Pos2::new(name_rect.min.x, rect.center().y - uname.size().y / 2.0), uname, th.text);
        // 退出按钮
        let out_rect = Rect::from_center_size(Pos2::new(rect.right() - 17.0, rect.center().y), vec2(24.0, 24.0));
        let ores = ui.interact(out_rect, ui.id().with("logout_btn"), egui::Sense::click());
        if ores.hovered() {
            painter.rect_filled(out_rect, CornerRadius::same(12), mix(th.panel, th.danger, 0.18));
        }
        icons::paint(&painter, out_rect.shrink(3.0), Glyph::Logout, if ores.hovered() { th.danger } else { th.text_weak });
        let clicked = ores.clicked();
        ores.on_hover_text("退出登录");
        if clicked {
            self.send(Cmd::Logout);
        }
        resp
    }

    fn quota_card(&mut self, ui: &mut egui::Ui, th: &Theme, quota: Option<&Quota>) -> egui::Response {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 74.0), egui::Sense::hover());
        let card = rect;
        let painter = ui.painter().clone();
        let (r, q) = match quota {
            Some(q) if q.limit > 0 => (q.usage as f32 / q.limit as f32, Some(q)),
            _ => (0.0, None),
        };
        let frac = r.clamp(0.0, 1.0);
        // 卡片底
        let bg = mix(th.bg, th.text, if th.dark { 0.05 } else { 0.03 });
        painter.rect_filled(card, CornerRadius::same(12), bg);
        painter.rect_stroke(card, CornerRadius::same(12), Stroke::new(1.0, th.border), egui::StrokeKind::Inside);
        let inner = card.shrink(14.0);
        // 标题行
        let title = painter.layout_no_wrap("存储空间".into(), FontId::proportional(12.0), th.text_weak);
        painter.galley(Pos2::new(inner.min.x, inner.min.y + 2.0), title, th.text_weak);
        if quota.is_some() {
            let pct = format!("{}%", (frac * 100.0).round() as u32);
            let gp = painter.layout_no_wrap(pct, FontId::proportional(11.5), th.text_faint);
            painter.galley(Pos2::new(inner.right() - gp.size().x, inner.min.y + 2.0), gp, th.text_faint);
        }
        // 进度条
        let bar = Rect::from_min_max(
            Pos2::new(inner.min.x, inner.min.y + 22.0),
            Pos2::new(inner.right(), inner.min.y + 26.0),
        );
        painter.rect_filled(bar, CornerRadius::same(2), mix(th.bg, th.text, if th.dark { 0.12 } else { 0.1 }));
        if frac > 0.0 {
            let fw = (bar.width() * frac).max(3.0);
            painter.rect_filled(
                Rect::from_min_size(bar.min, vec2(fw, bar.height())),
                CornerRadius::same(2),
                th.accent,
            );
        }
        // 用量文案
        let usage_txt = match q {
            Some(q) => format!("已用 {} / 共 {}", format::fmt_bytes(q.usage), format::fmt_bytes(q.limit)),
            None => "正在获取…".into(),
        };
        let g = painter.layout_no_wrap(usage_txt, FontId::proportional(11.0), th.text_weak);
        painter.galley(Pos2::new(inner.min.x, inner.min.y + 34.0), g, th.text_weak);
        ui.add_space(0.0);
        ui.interact(card, ui.id().with("quota_card"), egui::Sense::hover())
    }
}

// ---------------- 登录 ----------------

impl App {
    fn login_ui(&mut self, ctx: &egui::Context, th: &Theme) {
        const FW: f32 = 320.0;
        egui::CentralPanel::default()
            .frame(Frame::new().fill(th.bg))
            .show(ctx, |ui| {
                let h = ui.available_height();
                let pad = ((h - 470.0) / 2.0).max(26.0);
                ui.add_space(pad);
                ui.vertical_centered(|ui| {
                    // Logo
                    let (r, _) = ui.allocate_exact_size(vec2(62.0, 62.0), egui::Sense::hover());
                    let lp = ui.painter().clone();
                    let rr = r;
                    lp.rect_filled(rr, CornerRadius::same(16), th.accent);
                    let g = lp.layout_no_wrap("P".into(), FontId::proportional(32.0), th.on_accent);
                    lp.galley(
                        Pos2::new(rr.center().x - g.size().x / 2.0, rr.center().y - g.size().y / 2.0 - 1.0),
                        g,
                        th.on_accent,
                    );

                    ui.add_space(16.0);
                    ui.label(RichText::new("登录 PikPak").size(22.0).strong().color(th.text));
                    ui.add_space(2.0);
                    ui.label(RichText::new("继续使用你的云端文件").size(12.5).color(th.text_weak));

                    ui.add_space(18.0);
                    if self.auth_checking {
                        ui.spinner();
                        ui.add_space(6.0);
                        ui.label(RichText::new("正在恢复登录状态 / 登录中…").color(th.text_weak));
                        return;
                    }

                    // 表单卡片
                    egui::Frame::new()
                        .fill(th.card)
                        .stroke(Stroke::new(1.0, th.border))
                        .corner_radius(CornerRadius::same(14))
                        .inner_margin(Margin::same(20))
                        .show(ui, |ui| {
                            ui.set_width(FW);
                            ui.label(RichText::new("账号").size(12.0).color(th.text_weak));
                            ui.add_space(4.0);
                            ui.add(
                                egui::TextEdit::singleline(&mut self.login_username)
                                    .desired_width(FW)
                                    .hint_text("邮箱 / 手机号")
                                    .margin(egui::Margin::symmetric(10, 7)),
                            );
                            ui.add_space(12.0);
                            ui.label(RichText::new("密码").size(12.0).color(th.text_weak));
                            ui.add_space(4.0);
                            ui.add(
                                egui::TextEdit::singleline(&mut self.login_password)
                                    .desired_width(FW)
                                    .password(true)
                                    .hint_text("密码")
                                    .margin(egui::Margin::symmetric(10, 7)),
                            );

                            ui.add_space(16.0);
                            let ok = !self.login_username.trim().is_empty()
                                && !self.login_password.is_empty();
                            let enter = ui.input(|i| i.key_pressed(Key::Enter));
                            let btn = egui::Button::new(
                                RichText::new("登 录").color(th.on_accent).size(15.0),
                            )
                            .fill(th.accent)
                            .stroke(Stroke::NONE)
                            .corner_radius(CornerRadius::same(10))
                            .min_size(vec2(FW, 38.0));
                            let btn_resp = ui.add_enabled(ok || enter, btn);
                            if (btn_resp.clicked() || (enter && ok)) && !self.auth_checking {
                                let username = self.login_username.trim().to_string();
                                let password = std::mem::take(&mut self.login_password);
                                self.auth_error = None;
                                self.auth_checking = true;
                                self.send(Cmd::Login { username, password });
                            }
                        });

                    if let Some(err) = &self.auth_error {
                        ui.add_space(12.0);
                        ui.label(RichText::new(err).color(th.danger).size(13.0));
                    }

                    ui.add_space(16.0);
                    ui.label(
                        RichText::new("登录遇到验证码时请稍后再试或检查网络")
                            .color(th.text_faint)
                            .size(11.5),
                    );
                });
            });
    }
}

// ---------------- 文件页 ----------------

impl App {
    fn files_page(&mut self, ctx: &egui::Context, th: &Theme) {
        // 键盘快捷键(不在输入框聚焦时生效)
        if !ctx.wants_keyboard_input() {
            ctx.input(|i| {
                if i.key_pressed(Key::F5) {
                    self.refresh_dir();
                }
                if i.key_pressed(Key::A) && i.modifiers.ctrl {
                    self.selected = self.files.iter().map(|f| f.id.clone()).collect();
                }
                if (i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace))
                    && !self.selected.is_empty()
                {
                    let sel = self.selected_names();
                    if !sel.is_empty() {
                        self.trash_confirm = Some(sel);
                    }
                }
            });
        }

        let mut up = false;
        let mut jumped: Option<usize> = None;
        let mut mkdir = false;
        let mut refresh = false;

        // -------- 顶部: 面包屑 + 操作 --------
        egui::TopBottomPanel::top("file_head")
            .frame(Frame::new().fill(th.bg).inner_margin(Margin { left: 20, right: 20, top: 14, bottom: 10 }))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    // 上级按钮
                    let at_root = self.stack.len() <= 1;
                    let (r, rresp) = ui.allocate_exact_size(vec2(30.0, 30.0), egui::Sense::click());
                    ui.painter().rect_filled(r, CornerRadius::same(8), if rresp.hovered() && !at_root { th.hover } else { Color32::TRANSPARENT });
                    icons::paint(ui.painter(), r.shrink(6.0), Glyph::Up, if at_root { th.text_faint } else { th.text_weak });
                    if rresp.clicked() && !at_root {
                        up = true;
                    }
                    rresp.clone().on_hover_text("返回上级");
                    ui.add_space(4.0);
                    // 面包屑
                    for (i, crumb) in self.stack.iter().enumerate() {
                        if i > 0 {
                            ui.label(RichText::new("›").color(th.text_faint));
                        }
                        let last = i == self.stack.len() - 1;
                        let text = RichText::new(&crumb.label)
                            .size(15.0)
                            .color(if last { th.text } else { th.text_weak });
                        let resp = ui.add(egui::Button::new(text).frame(false));
                        if last {
                            let _ = resp;
                        }
                        if !last && resp.clicked() {
                            jumped = Some(i);
                        }
                    }

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add(
                                egui::Button::new(RichText::new("＋ 新建文件夹").color(th.accent))
                                    .fill(th.accent_soft())
                                    .stroke(Stroke::new(1.0, mix(th.accent, th.bg, 0.4)))
                                    .corner_radius(CornerRadius::same(8)),
                            )
                            .clicked()
                        {
                            mkdir = true;
                        }
                        ui.add_space(4.0);
                        let (rr, rresp) = ui.allocate_exact_size(vec2(30.0, 30.0), egui::Sense::click());
                        ui.painter().rect_filled(rr, CornerRadius::same(8), if rresp.hovered() { th.hover } else { Color32::TRANSPARENT });
                        icons::paint(ui.painter(), rr.shrink(7.0), Glyph::Refresh, th.text_weak);
                        if rresp.clicked() {
                            refresh = true;
                        }
                        rresp.on_hover_text("刷新 (F5)");
                    });
                });
            });

        if up {
            self.stack.pop();
            self.refresh_dir();
        }
        if let Some(i) = jumped {
            self.stack.truncate(i + 1);
            self.refresh_dir();
        }
        if mkdir {
            self.mkdir_open = true;
            self.mkdir_name = String::new();
        }
        if refresh {
            self.refresh_dir();
        }

        // -------- 工具栏: 筛选 / 排序 --------
        let (folders, plain) = self.visible_rows();
        let visible_total = folders.len() + plain.len();

        egui::TopBottomPanel::top("file_toolbar")
            .frame(Frame::new().fill(th.bg).inner_margin(Margin { left: 20, right: 20, top: 4, bottom: 6 }))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    // 搜索 / 筛选
                    ui.label(RichText::new("筛选").color(th.text_weak).size(12.5));
                    ui.add(
                        egui::TextEdit::singleline(&mut self.filter)
                            .desired_width(240.0)
                            .hint_text("按名称过滤当前目录")
                            .margin(egui::Margin::symmetric(8, 6)),
                    );
                    if !self.filter.is_empty()
                        && ui
                            .add(egui::Button::new(RichText::new("清除").color(th.text_weak)).frame(false))
                            .clicked()
                    {
                        self.filter.clear();
                    }

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if !self.selected.is_empty() {
                            if ui
                                .add(egui::Button::new(RichText::new("清除选择").color(th.text_weak)).frame(false))
                                .clicked()
                            {
                                self.selected.clear();
                            }
                            ui.label(RichText::new(format!("已选 {}", self.selected.len())).strong().color(th.accent));
                        }
                        ui.label(RichText::new(format!("{visible_total} 项")).color(th.text_faint).size(12.5));
                    });
                });
            });

        // -------- 底部多选操作条 --------
        let sel_meta = self.selected_names();
        let dl_candidates = self.selected_plain_files();
        let dl_has_folder = self.selected_has_folder();
        let mut want_download = false;

        if !sel_meta.is_empty() {
            egui::TopBottomPanel::bottom("file_ops")
                .frame(Frame::new().fill(th.card).inner_margin(Margin::symmetric(16, 10)))
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        let single = sel_meta.len() == 1;
                        if single {
                            ui.label(RichText::new(format!("已选择 · {}", sel_meta[0].1)).size(13.0).color(th.text));
                        } else {
                            ui.label(RichText::new(format!("已选择 {} 项", sel_meta.len())).size(13.0).color(th.text));
                        }
                        ui.add_space(10.0);
                        if single && ui.button("重命名").clicked() {
                            self.rename_id = Some(sel_meta[0].0.clone());
                            self.rename_name = sel_meta[0].1.clone();
                        }
                        if !dl_candidates.is_empty()
                            && ui
                                .add(
                                    egui::Button::new(RichText::new("下载到本地…").color(th.on_accent))
                                        .fill(th.accent)
                                        .stroke(Stroke::NONE)
                                        .corner_radius(CornerRadius::same(8)),
                                )
                                .clicked()
                        {
                            want_download = true;
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui
                                .add(
                                    egui::Button::new(RichText::new("移入回收站").color(th.danger))
                                        .stroke(Stroke::new(1.0, mix(th.danger, th.bg, 0.35)))
                                        .fill(Color32::TRANSPARENT)
                                        .corner_radius(CornerRadius::same(8)),
                                )
                                .clicked()
                            {
                                self.trash_confirm = Some(sel_meta.clone());
                            }
                        });
                    });
                });
        }

        if want_download && !dl_candidates.is_empty() {
            if let Some(dir) = self.choose_download_dir() {
                let has_folder = dl_has_folder;
                self.enqueue_downloads(dl_candidates.clone(), dir);
                if has_folder {
                    self.toast_warn("已跳过选中的文件夹(暂不支持整目录下载)");
                }
            }
        }

        // -------- 文件列表 --------
        let mut open_folder: Option<(String, String)> = None;
        let mut actions: Vec<RowAction> = Vec::new();
        let mut sel_reqs: Vec<RowSel> = Vec::new();

        egui::CentralPanel::default()
            .frame(Frame::new().fill(th.bg).inner_margin(Margin { left: 20, right: 20, top: 4, bottom: 12 }))
            .show(ctx, |ui| {
                if self.dir_loading && self.files.is_empty() {
                    ui.vertical_centered(|ui| {
                        ui.add_space(90.0);
                        ui.spinner();
                        ui.add_space(8.0);
                        ui.label(RichText::new("加载中…").color(th.text_weak));
                    });
                    return;
                }

                // 列表卡片背景
                let card_top = ui.cursor().min.y;
                let card_rect = Rect::from_min_max(
                    Pos2::new(ui.cursor().min.x, card_top),
                    Pos2::new(ui.max_rect().max.x, ui.max_rect().max.y),
                );
                let card_painter = ui.painter_at(card_rect);
                card_painter.rect_filled(card_rect, CornerRadius::same(12), th.card);
                card_painter.rect_stroke(card_rect, CornerRadius::same(12), Stroke::new(1.0, th.border), egui::StrokeKind::Inside);

                let inner = card_rect.shrink2(vec2(2.0, 8.0));
                let mut inner_ui = ui.new_child(UiBuilder::new().max_rect(inner));

                // 列表头
                inner_ui.add_space(6.0);
                self.file_list_header(&mut inner_ui, th, inner.width());
                let sep_y = inner_ui.cursor().min.y;
                ui.painter().line_segment(
                    [
                        Pos2::new(inner.min.x + 6.0, sep_y + 3.0),
                        Pos2::new(inner.max.x - 6.0, sep_y + 3.0),
                    ],
                    Stroke::new(1.0, th.border),
                );

                let list_avail_h = inner.max.y - (sep_y + 4.0);
                egui::ScrollArea::vertical()
                    .id_salt("file_list_scroll")
                    .auto_shrink([false, false])
                    .max_height(list_avail_h.max(40.0))
                    .show(&mut inner_ui, |ui| {
                    ui.set_min_height(list_avail_h.max(40.0));
                    ui.set_width(inner.width());
                    let ctrl = ui.input(|i| i.modifiers.ctrl);
                    let mut even = false;
                    for f in folders.iter().chain(plain.iter()) {
                        let is_sel = self.selected.contains(&f.id);
                        if let Some(sel) = file_row(ui, th, f, is_sel, ctrl, even, &mut actions) {
                            sel_reqs.push(sel);
                        }
                        even = !even;
                    }

                    if self.dir_next.is_some() {
                        ui.add_space(4.0);
                        ui.centered_and_justified(|ui| {
                            if ui.button("加载更多").clicked() {
                                self.load_more();
                            }
                        });
                        ui.add_space(2.0);
                    }
                    if self.files.is_empty() && !self.dir_loading {
                        ui.add_space(20.0);
                        ui.centered_and_justified(|ui| {
                            ui.label(RichText::new("此文件夹为空").color(th.text_weak));
                        });
                    } else if !self.filter.is_empty() && folders.is_empty() && plain.is_empty() {
                        ui.add_space(20.0);
                        ui.centered_and_justified(|ui| {
                            ui.label(
                                RichText::new(format!("没有匹配「{}」的文件", self.filter))
                                    .color(th.text_weak),
                            );
                        });
                    }
                });

                // 应用选择变更
                for sel in sel_reqs {
                    match sel {
                        RowSel::Replace(id) => {
                            self.selected.clear();
                            self.selected.insert(id);
                        }
                        RowSel::Toggle(id) => {
                            if self.selected.contains(&id) {
                                self.selected.remove(&id);
                            } else {
                                self.selected.insert(id);
                            }
                        }
                    }
                }

                for action in actions {
                    match action {
                        RowAction::OpenFolder(id, name) => open_folder = Some((id, name)),
                        RowAction::OpenFile => {
                            self.toast_warn("文件预览将在后续版本支持");
                        }
                        RowAction::DownloadFile(id, name) => self.download_single(id, name),
                        RowAction::CopyName(name) => {
                            let ctx2 = ctx.clone();
                            ctx2.copy_text(name);
                            self.toast_ok("已复制名称");
                        }
                        RowAction::Rename(id, name) => {
                            self.rename_id = Some(id);
                            self.rename_name = name;
                        }
                        RowAction::Trash(id) => {
                            if let Some(name) = self
                                .files
                                .iter()
                                .find(|f| f.id == id)
                                .map(|f| f.name.clone())
                            {
                                self.trash_confirm = Some(vec![(id, name)]);
                            }
                        }
                    }
                }
            });

        if let Some((id, name)) = open_folder {
            self.goto_folder(&id, &name);
        }
    }

    fn file_list_header(&mut self, ui: &mut egui::Ui, th: &Theme, w: f32) {
        let (name_x, size_left, time_left) = col_layout(w);
        let h = 30.0;
        let (rect, _) = ui.allocate_exact_size(vec2(w, h), egui::Sense::hover());
        let painter = ui.painter().clone();
        let top = rect.min.y;
        let x0 = rect.min.x;

        struct Col {
            x: f32,
            label: &'static str,
            by: SortBy,
            w: f32,
        }
        let cols = [
            Col { x: name_x, label: "名称", by: SortBy::Name, w: 220.0 },
            Col { x: size_left, label: "大小", by: SortBy::Size, w: 90.0 },
            Col { x: time_left, label: "修改时间", by: SortBy::Modified, w: 160.0 },
        ];
        for col in cols {
            let active = self.sort_by == col.by;
            let color = if active { th.accent } else { th.text_faint };
            // 整列可点击区域
            let crect = Rect::from_min_max(
                Pos2::new(x0 + col.x - 8.0, top),
                Pos2::new((x0 + col.x + col.w).min(rect.right()), top + h),
            );
            let resp = ui.interact(crect, ui.id().with(("header", col.by)), egui::Sense::click());
            if resp.hovered() {
                painter.rect_filled(crect, CornerRadius::same(6), th.hover);
            }
            // 文字直接绘制(与行内容一致的对齐与渲染路径)
            let g = painter.layout_no_wrap(
                col.label.to_string(),
                FontId::proportional(12.5),
                color,
            );
            let label_w = g.size().x;
            painter.galley(
                Pos2::new(x0 + col.x, top + (h - g.size().y) / 2.0),
                g,
                color,
            );
            // 排序方向用小三角矢量绘制(避免字体缺字形出现方块)
            if active {
                let yc = top + h / 2.0;
                let mx = x0 + col.x + label_w + 5.0;
                let s = 8.0;
                let pts: Vec<egui::Pos2> = if self.sort_desc {
                    vec![
                        Pos2::new(mx, yc - s / 2.0),
                        Pos2::new(mx + s, yc - s / 2.0),
                        Pos2::new(mx + s / 2.0, yc + s / 2.0 - 1.0),
                    ]
                } else {
                    vec![
                        Pos2::new(mx, yc + s / 2.0),
                        Pos2::new(mx + s, yc + s / 2.0),
                        Pos2::new(mx + s / 2.0, yc - s / 2.0 + 1.0),
                    ]
                };
                painter.add(egui::Shape::convex_polygon(pts, color, Stroke::NONE));
            }
            // 直接应用排序(下一帧即可生效, 不再经过帧末延迟变量)
            if resp.clicked() {
                if active {
                    self.sort_desc = !self.sort_desc;
                } else {
                    self.sort_by = col.by;
                    self.sort_desc = false;
                }
            }
            let _ = resp.on_hover_text("点击排序");
        }
    }
}

/// 布局坐标。返回 (name_x, size_left, time_left)。
fn col_layout(w: f32) -> (f32, f32, f32) {
    let icon_x = 12.0;
    let name_x = icon_x + 24.0 + 10.0;
    let time_w = 150.0;
    let right_pad = 8.0;
    let time_left = (w - time_w - right_pad).max(name_x + 60.0);
    let size_w = 84.0;
    let size_left = (time_left - size_w - 16.0).max(name_x + 60.0);
    (name_x, size_left, time_left)
}

/// 列表行; 返回 (file_id, add) 表示选择变更(ctrl 时为切换)。
fn file_row(
    ui: &mut egui::Ui,
    th: &Theme,
    f: &File,
    is_sel: bool,
    ctrl: bool,
    even: bool,
    actions: &mut Vec<RowAction>,
) -> Option<RowSel> {
    let row_h = 40.0;
    let w = ui.available_width().max(320.0);
    let (rect, row_resp) = ui.allocate_exact_size(vec2(w, row_h), egui::Sense::click());
    let painter = ui.painter().clone();

    let hovered = row_resp.hovered();

    let bg = if is_sel {
        mix(th.card, th.accent, if th.dark { 0.22 } else { 0.12 })
    } else if hovered {
        mix(th.card, th.text, if th.dark { 0.07 } else { 0.045 })
    } else if even {
        mix(th.card, th.text, if th.dark { 0.018 } else { 0.012 })
    } else {
        Color32::TRANSPARENT
    };
    painter.rect_filled(rect.shrink2(vec2(2.0, 2.0)), CornerRadius::same(8), bg);
    if is_sel {
        painter.rect_filled(
            Rect::from_min_max(
                Pos2::new(rect.min.x + 3.0, rect.min.y + 7.0),
                Pos2::new(rect.min.x + 5.0, rect.max.y - 7.0),
            ),
            CornerRadius::same(2),
            th.accent,
        );
    }

    let yc = rect.center().y;
    // 类型图标
    let (glyph, color) = file_visual(f);
    let icon_rect = Rect::from_center_size(Pos2::new(rect.min.x + 22.0, yc), vec2(22.0, 22.0));
    painter.rect_filled(icon_rect, CornerRadius::same(6), mix(th.card, color, if th.dark { 0.16 } else { 0.10 }));
    icons::paint(&painter, icon_rect.shrink(2.5), glyph, color);

    let x0 = rect.min.x;
    let (name_col, size_col, time_col) = col_layout(w);

    // 名称
    let name_w = (size_col - 12.0 - name_col).max(24.0);
    let name_g = truncate_text(&painter, &f.name, name_w, FontId::proportional(14.0), th.text);
    painter.galley(
        Pos2::new(x0 + name_col, yc - name_g.size().y / 2.0),
        name_g,
        th.text,
    );

    // 大小 / 时间
    if !f.is_folder() {
        let g = painter.layout_no_wrap(
            format::fmt_bytes(f.size),
            FontId::proportional(12.5),
            th.text_weak,
        );
        painter.galley(
            Pos2::new(x0 + size_col, yc - g.size().y / 2.0),
            g,
            th.text_weak,
        );
    }
    let t = f
        .modified_time
        .as_deref()
        .or(f.created_time.as_deref())
        .unwrap_or("");
    let g = painter.layout_no_wrap(
        format::fmt_time(t),
        FontId::proportional(12.5),
        th.text_weak,
    );
    painter.galley(
        Pos2::new(x0 + time_col, yc - g.size().y / 2.0),
        g,
        th.text_weak,
    );

    // 右键菜单
    let f_ctx = f.clone();
    let dbl = row_resp.double_clicked();
    let rclick = row_resp.clicked();
    let _menu = row_resp.context_menu(|ui| {
        if f_ctx.is_folder() {
            if ui.button("打开").clicked() {
                actions.push(RowAction::OpenFolder(f_ctx.id.clone(), f_ctx.name.clone()));
            }
        } else {
            if ui.button("下载到本地…").clicked() {
                actions.push(RowAction::DownloadFile(f_ctx.id.clone(), f_ctx.name.clone()));
            }
            if ui.button("打开(预览暂未支持)").clicked() {
                actions.push(RowAction::OpenFile);
            }
        }
        ui.separator();
        if ui.button("重命名").clicked() {
            actions.push(RowAction::Rename(f_ctx.id.clone(), f_ctx.name.clone()));
        }
        if ui.button("复制名称").clicked() {
            actions.push(RowAction::CopyName(f_ctx.name.clone()));
        }
        ui.separator();
        if ui
            .button(RichText::new("移入回收站").color(th.danger))
            .clicked()
        {
            actions.push(RowAction::Trash(f_ctx.id.clone()));
        }
    });

    let mut sel: Option<RowSel> = None;
    if dbl {
        // 双击打开文件夹; 文件仅保持选中
        if f.is_folder() {
            actions.push(RowAction::OpenFolder(f.id.clone(), f.name.clone()));
        }
    } else if rclick {
        if ctrl {
            sel = Some(RowSel::Toggle(f.id.clone()));
        } else {
            sel = Some(RowSel::Replace(f.id.clone()));
        }
    }
    sel
}

// ---------------- 离线任务页 ----------------

impl App {
    fn tasks_page(&mut self, ctx: &egui::Context, th: &Theme) {
        let mut create = false;
        let mut do_refresh = false;
        let mut download_target: Option<(String, String)> = None;
        let mut retry: Option<String> = None;
        let mut del: Option<String> = None;
        let mut clear_all: Vec<Vec<String>> = Vec::new();

        egui::CentralPanel::default()
            .frame(Frame::new().fill(th.bg).inner_margin(Margin { left: 20, right: 20, top: 16, bottom: 12 }))
            .show(ctx, |ui| {
                // 标题
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(22.0, 22.0), egui::Sense::hover());
                    icons::paint(ui.painter(), r, Glyph::Transfer, th.accent);
                    ui.label(RichText::new("离线下载").size(19.0).strong().color(th.text));
                });
                ui.add_space(4.0);
                ui.label(RichText::new("将磁力 / 直链先转存到云端, 完成后在「网盘文件」中查看。").color(th.text_weak).size(12.5));
                ui.add_space(14.0);

                // 新建离线下载卡片
                egui::Frame::new()
                    .fill(th.card)
                    .stroke(Stroke::new(1.0, th.border))
                    .corner_radius(CornerRadius::same(14))
                    .inner_margin(Margin::same(16))
                    .show(ui, |ui| {
                        ui.label(RichText::new("新建离线下载").size(14.0).strong().color(th.text));
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("链接 / 磁力").color(th.text_weak));
                            let resp = ui.add(
                                egui::TextEdit::singleline(&mut self.offline_url)
                                    .desired_width(f32::INFINITY)
                                    .hint_text("magnet:?xt=... 或 https://..."),
                            );
                            if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                                create = true;
                            }
                        });
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("文件名").color(th.text_weak));
                            ui.add(
                                egui::TextEdit::singleline(&mut self.offline_name)
                                    .desired_width(280.0)
                                    .hint_text("可选, 留空自动识别"),
                            );
                            ui.add_space(8.0);
                            let enabled = !self.offline_url.trim().is_empty();
                            if ui
                                .add_enabled(
                                    enabled,
                                    egui::Button::new(RichText::new("提交下载").color(th.on_accent))
                                        .fill(th.accent)
                                        .stroke(Stroke::NONE)
                                        .corner_radius(CornerRadius::same(8)),
                                )
                                .clicked()
                            {
                                create = true;
                            }
                        });
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("保存到").color(th.text_weak));
                            let cur = self.current_parent();
                            let on_cur = self.offline_to_current && cur.is_some();
                            if ui
                                .selectable_label(!on_cur, "离线默认目录")
                                .clicked()
                            {
                                self.offline_to_current = false;
                            }
                            if cur.is_some() {
                                let top = self
                                    .stack
                                    .last()
                                    .map(|c| c.label.clone())
                                    .unwrap_or_default();
                                if ui
                                    .selectable_label(on_cur, format!("文件页当前目录 · {top}"))
                                    .clicked()
                                {
                                    self.offline_to_current = true;
                                }
                            }
                        });
                    });

                ui.add_space(14.0);

                // 任务列表区(超高时自身滚动)
                let scroll_h = ui.available_height().max(80.0);
                egui::ScrollArea::vertical()
                    .id_salt("tasks_scroll")
                    .auto_shrink([false, false])
                    .max_height(scroll_h)
                    .show(ui, |ui| {
                        if self.tasks_loading && self.buckets.is_empty() {
                            ui.vertical_centered(|ui| {
                                ui.add_space(30.0);
                                ui.spinner();
                                ui.add_space(6.0);
                                ui.label(RichText::new("正在获取任务…").color(th.text_weak));
                            });
                            return;
                        }

                // 任务分组列表
                egui::Frame::new()
                    .fill(th.card)
                    .stroke(Stroke::new(1.0, th.border))
                    .corner_radius(CornerRadius::same(14))
                    .inner_margin(Margin::symmetric(14, 8))
                    .show(ui, |ui| {
                        let mut any = false;
                        for phase in format::PHASE_ORDER {
                            let Some(tasks) = self.buckets.get(phase) else {
                                continue;
                            };
                            if tasks.is_empty() {
                                continue;
                            }
                            any = true;
                            let color = match phase {
                                "PHASE_TYPE_COMPLETE" => th.ok,
                                "PHASE_TYPE_ERROR" => th.danger,
                                "PHASE_TYPE_RUNNING" => Color32::from_rgb(96, 146, 235),
                                _ => th.warn,
                            };
                            let header = format!("{} ({})", phase_label(phase), tasks.len());
                            egui::CollapsingHeader::new(RichText::new(header).color(color).strong().size(13.5))
                                .default_open(phase != "PHASE_TYPE_COMPLETE")
                                .show(ui, |ui| {
                                    for t in tasks {
                                        let name = task_name(t).unwrap_or_else(|| "未知任务".into());
                                        let id = task_id(t).unwrap_or_default();
                                        let size = task_size(t).unwrap_or(0);
                                        let file_id = task_file_id(t).unwrap_or_default();
                                        ui.horizontal(|ui| {
                                            ui.add_space(2.0);
                                            ui.label(
                                                RichText::new(&name)
                                                    .size(13.5)
                                                    .color(th.text),
                                            );
                                            if size > 0 {
                                                ui.label(RichText::new(format::fmt_bytes(size)).color(th.text_faint).size(12.0));
                                            }
                                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                                if !file_id.is_empty()
                                                    && phase == "PHASE_TYPE_COMPLETE"
                                                    && ui
                                                        .add(
                                                            egui::Button::new(RichText::new("下载").color(th.accent))
                                                                .fill(th.accent_soft())
                                                                .stroke(Stroke::NONE)
                                                                .corner_radius(CornerRadius::same(7)),
                                                        )
                                                        .clicked()
                                                {
                                                    download_target = Some((file_id.clone(), name.clone()));
                                                }
                                                if !id.is_empty()
                                                    && ui.button(RichText::new("删除").color(th.text_weak)).clicked()
                                                {
                                                    del = Some(id.clone());
                                                }
                                                if phase == "PHASE_TYPE_ERROR"
                                                    && !id.is_empty()
                                                    && ui.button(RichText::new("重试").color(th.warn)).clicked()
                                                {
                                                    retry = Some(id.clone());
                                                }
                                            });
                                        });
                                        ui.add_space(2.0);
                                        ui.separator();
                                    }
                                    if phase != "PHASE_TYPE_RUNNING" {
                                        let ids: Vec<String> = tasks.iter().filter_map(task_id).collect();
                                        if !ids.is_empty() {
                                            ui.add_space(2.0);
                                            ui.horizontal(|ui| {
                                                ui.add_space(4.0);
                                                if ui.button(RichText::new("清空本组(仅移除任务记录)").color(th.text_faint).size(12.0)).clicked() {
                                                    clear_all.push(ids);
                                                }
                                            });
                                            ui.add_space(2.0);
                                        }
                                    }
                                });
                        }
                        if !any && self.buckets.values().all(|v| v.is_empty()) {
                            ui.centered_and_justified(|ui| {
                                ui.add_space(30.0);
                                ui.label(RichText::new("暂无离线任务").color(th.text_weak));
                                ui.add_space(30.0);
                            });
                        }
                    });

                // 刷新按钮
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    let (r, resp) = ui.allocate_exact_size(vec2(22.0, 22.0), egui::Sense::click());
                    icons::paint(ui.painter(), r, Glyph::Refresh, th.text_weak);
                    if ui.add(egui::Button::new(RichText::new("刷新任务").color(th.text_weak)).frame(false)).clicked() {
                        do_refresh = true;
                    }
                    let _ = resp;
                });
                    });
            });

        if create {
            let url = self.offline_url.trim().to_string();
            let name = {
                let n = self.offline_name.trim();
                if n.is_empty() {
                    None
                } else {
                    Some(n.to_string())
                }
            };
            let parent = if self.offline_to_current {
                self.current_parent()
            } else {
                None
            };
            self.send(Cmd::OfflineCreate { url, name, parent });
        }
        if do_refresh {
            self.send(Cmd::RefreshTasks);
        }
        if let Some((fid, fname)) = download_target {
            self.download_single(fid, fname);
        }
        if let Some(tid) = retry {
            let tx = self.tx.clone();
            let _ = tx.send(Cmd::OfflineRetry { task_id: tid });
        }
        if let Some(tid) = del {
            let tx = self.tx.clone();
            let _ = tx.send(Cmd::OfflineDelete {
                task_ids: vec![tid],
                delete_files: false,
            });
        }
        for ids in clear_all {
            let tx = self.tx.clone();
            let _ = tx.send(Cmd::OfflineDelete {
                task_ids: ids,
                delete_files: false,
            });
        }
    }
}

// ---------------- 设置页 ----------------

impl App {
    fn settings_page(&mut self, ctx: &egui::Context, th: &Theme) {
        egui::CentralPanel::default()
            .frame(Frame::new().fill(th.bg).inner_margin(Margin { left: 20, right: 20, top: 16, bottom: 12 }))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(22.0, 22.0), egui::Sense::hover());
                    icons::paint(ui.painter(), r, Glyph::Gear, th.accent);
                    ui.label(RichText::new("设置").size(19.0).strong().color(th.text));
                });
                ui.add_space(14.0);

                // 外观
                let dark = self.dark;
                settings_card(ui, th, "外观", &mut |ui| {
                    ui.horizontal(|ui| {
                        ui.add_space(2.0);
                        ui.vertical(|ui| {
                            ui.label(RichText::new("主题").color(th.text_weak));
                            ui.label(RichText::new("选择你喜欢的明暗显示风格。").color(th.text_faint).size(12.0));
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui
                                .add(egui::Button::new(RichText::new(if dark { "切换到浅色" } else { "切换到深色" })))
                                .clicked()
                            {
                                self.toggle_theme();
                            }
                        });
                    });
                });
                ui.add_space(14.0);

                // 下载目录
                let dl = self.download_dir.clone();
                settings_card(ui, th, "下载", &mut |ui| {
                    ui.horizontal(|ui| {
                        ui.add_space(2.0);
                        ui.vertical(|ui| {
                            ui.label(RichText::new("本地下载目录").color(th.text_weak));
                            ui.label(
                                RichText::new(if dl.is_empty() { "未设置, 将使用系统下载目录".into() } else { dl.clone() })
                                    .color(th.text_faint)
                                    .size(12.0),
                            );
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.button("选择目录…").clicked() {
                                self.choose_download_dir();
                            }
                        });
                    });
                });
                ui.add_space(14.0);

                // 账户
                let name = self.username.clone();
                settings_card(ui, th, "账户", &mut |ui| {
                    ui.horizontal(|ui| {
                        ui.add_space(2.0);
                        ui.vertical(|ui| {
                            ui.label(RichText::new("当前登录").color(th.text_weak));
                            ui.label(RichText::new(&name).color(th.text).size(13.0));
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui
                                .add(
                                    egui::Button::new(RichText::new("退出登录").color(th.danger))
                                        .stroke(Stroke::new(1.0, mix(th.danger, th.bg, 0.3)))
                                        .fill(Color32::TRANSPARENT)
                                        .corner_radius(CornerRadius::same(8)),
                                )
                                .clicked()
                            {
                                self.send(Cmd::Logout);
                            }
                        });
                    });
                });
                ui.add_space(24.0);
                ui.centered_and_justified(|ui| {
                    ui.label(RichText::new("PikPak Linux · 非官方客户端 v0.1.0").color(th.text_faint).size(11.5));
                });
            });
    }
}

fn settings_card(
    ui: &mut egui::Ui,
    th: &Theme,
    title: &str,
    rows: &mut dyn FnMut(&mut egui::Ui),
) {
    egui::Frame::new()
        .fill(th.card)
        .stroke(Stroke::new(1.0, th.border))
        .corner_radius(CornerRadius::same(14))
        .inner_margin(Margin::same(18))
        .show(ui, |ui| {
            ui.label(RichText::new(title).size(14.0).strong().color(th.text));
            ui.add_space(10.0);
            rows(ui);
        });
}

// ---------------- 对话框 ----------------

impl App {
    fn dialogs(&mut self, ctx: &egui::Context, th: &Theme) {
        let _ = th;
        if self.mkdir_open {
            let parent = self.current_parent();
            let mut name = self.mkdir_name.clone();
            let mut confirmed = false;
            let mut close = false;
            egui::Window::new("新建文件夹")
                .collapsible(false)
                .resizable(false)
                .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.add_space(4.0);
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut name)
                            .desired_width(260.0)
                            .hint_text("文件夹名称"),
                    );
                    let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        let ok = !name.trim().is_empty();
                        if ui.add_enabled(ok, egui::Button::new("创建")).clicked() || enter {
                            confirmed = true;
                            close = true;
                        }
                        if ui.button("取消").clicked() {
                            close = true;
                        }
                    });
                });
            if confirmed {
                let name = name.trim().to_string();
                self.send(Cmd::CreateFolder { name, parent });
            }
            self.mkdir_open = !close;
        }

        if let Some(id) = self.rename_id.clone() {
            let mut name = self.rename_name.clone();
            let mut confirmed = false;
            let mut close = false;
            egui::Window::new("重命名")
                .collapsible(false)
                .resizable(false)
                .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.add_space(4.0);
                    let resp = ui.add(egui::TextEdit::singleline(&mut name).desired_width(260.0));
                    let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        let ok = !name.trim().is_empty();
                        if ui.add_enabled(ok, egui::Button::new("确定")).clicked() || enter {
                            confirmed = true;
                            close = true;
                        }
                        if ui.button("取消").clicked() {
                            close = true;
                        }
                    });
                });
            if confirmed {
                let name = name.trim().to_string();
                self.send(Cmd::Rename { id, name });
            }
            if close {
                self.rename_id = None;
            }
        }

        if let Some(items) = self.trash_confirm.clone() {
            let ids: Vec<String> = items.iter().map(|(id, _)| id.clone()).collect();
            let mut confirmed = false;
            let mut close = false;
            egui::Window::new("移入回收站")
                .collapsible(false)
                .resizable(false)
                .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    if items.len() == 1 {
                        ui.label(format!("确定将「{}」移入回收站吗?", items[0].1));
                    } else {
                        ui.label(format!("确定将这 {} 项移入回收站吗?", items.len()));
                        if let Some((_, first)) = items.first() {
                            ui.label(RichText::new(first.clone()).weak());
                        }
                        if items.len() > 1 {
                            ui.label(RichText::new("……").weak());
                        }
                    }
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add(egui::Button::new(RichText::new("移入回收站").color(Color32::WHITE)).fill(th.danger).stroke(Stroke::NONE))
                            .clicked()
                        {
                            confirmed = true;
                            close = true;
                        }
                        if ui.button("取消").clicked() {
                            close = true;
                        }
                    });
                });
            if confirmed {
                // 本地先行移除并记录, 避免等服务端刷新(有延迟)期间仍显示
                for id in &ids {
                    self.hidden.insert(id.clone());
                    self.selected.remove(id);
                }
                self.files.retain(|f| !ids.contains(&f.id));
                self.send(Cmd::Trash { ids });
            }
            if close {
                self.trash_confirm = None;
            }
        }
    }

    fn draw_toast(&mut self, ctx: &egui::Context) {
        let Some((color, msg, since)) = self.toast.clone() else {
            return;
        };
        if since.elapsed() > Duration::from_secs(6) {
            self.toast = None;
            return;
        }
        egui::Area::new(egui::Id::new("toast"))
            .anchor(Align2::RIGHT_BOTTOM, [-16.0, -16.0])
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style())
                    .corner_radius(CornerRadius::same(10))
                    .show(ui, |ui| {
                        ui.add_space(2.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("● ").color(color));
                            ui.label(RichText::new(msg).color(color));
                        });
                        ui.add_space(2.0);
                    });
            });
    }

    // ---------------- 本地下载页 ----------------

    fn downloads_page(&mut self, ctx: &egui::Context, th: &Theme) {
        let mut ops: Vec<(u64, DlOp)> = Vec::new();
        let running = self
            .jobs
            .values()
            .filter(|j| j.status == DlStatus::Queued || j.status == DlStatus::Running)
            .count();

        egui::CentralPanel::default()
            .frame(Frame::new().fill(th.bg).inner_margin(Margin { left: 20, right: 20, top: 16, bottom: 12 }))
            .show(ctx, |ui| {
                // 标题行
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(22.0, 22.0), egui::Sense::hover());
                    icons::paint(ui.painter(), r, Glyph::Download, th.accent);
                    ui.label(RichText::new("本地下载").size(19.0).strong().color(th.text));
                    let done = self.jobs.len().saturating_sub(running);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if !self.jobs.is_empty() {
                            ui.label(
                                RichText::new(format!(
                                    "进行中 {running} · 已完成 {done} · 共 {} 个任务",
                                    self.jobs.len()
                                ))
                                .color(th.text_weak)
                                .size(12.0),
                            );
                        }
                    });
                });
                ui.add_space(4.0);
                ui.label(
                    RichText::new("文件保存在本地下载目录, 可前往「设置」修改。")
                        .color(th.text_weak)
                        .size(12.5),
                );
                ui.add_space(14.0);

                if self.jobs.is_empty() {
                    ui.centered_and_justified(|ui| {
                        ui.add_space(60.0);
                        let (r, _) = ui.allocate_exact_size(vec2(64.0, 64.0), egui::Sense::hover());
                        icons::paint(
                            ui.painter(),
                            r,
                            Glyph::Download,
                            th.text_faint,
                        );
                        ui.add_space(10.0);
                        ui.label(RichText::new("暂无本地下载任务").color(th.text_weak).size(14.0));
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new("在「网盘文件」中选择文件, 右键或底部操作条下载到本地")
                                .color(th.text_faint)
                                .size(12.0),
                        );
                        ui.add_space(60.0);
                    });
                    return;
                }

                let scroll_h = ui.available_height().max(60.0);
                egui::ScrollArea::vertical()
                    .id_salt("downloads_scroll")
                    .auto_shrink([false, false])
                    .max_height(scroll_h)
                    .show(ui, |ui| {
                        let ids: Vec<u64> = self.jobs.keys().cloned().collect();
                        for rid in ids {
                            let Some(job) = self.jobs.get(&rid).cloned() else {
                                continue;
                            };
                            let mut op: Option<DlOp> = None;
                            ui.push_id(rid, |ui| {
                                egui::Frame::new()
                                    .fill(th.card)
                                    .stroke(Stroke::new(1.0, th.border))
                                    .corner_radius(CornerRadius::same(12))
                                    .inner_margin(Margin::same(12))
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            ui.vertical(|ui| {
                                                ui.label(
                                                    RichText::new(&job.name)
                                                        .strong()
                                                        .size(13.5)
                                                        .color(th.text),
                                                );
                                                let (col, txt) = status_line(&job);
                                                ui.label(RichText::new(txt).color(col).size(11.5));
                                            });
                                            ui.add_space(10.0);
                                            ui.vertical(|ui| {
                                                match &job.status {
                                                    DlStatus::Running if job.total > 0 => {
                                                        let frac = (job.done as f32 / job.total as f32)
                                                            .clamp(0.0, 1.0);
                                                        ui.add(
                                                            egui::ProgressBar::new(frac)
                                                                .desired_width(280.0)
                                                                .text(format!(
                                                                    "{} / {}",
                                                                    format::fmt_bytes(job.done as i64),
                                                                    format::fmt_bytes(job.total as i64)
                                                                )),
                                                        );
                                                    }
                                                    DlStatus::Running => {
                                                        ui.horizontal(|ui| {
                                                            ui.spinner();
                                                            if job.done > 0 {
                                                                ui.label(
                                                                    RichText::new(format!(
                                                                        "已接收 {}",
                                                                        format::fmt_bytes(job.done as i64)
                                                                    ))
                                                                    .color(th.text_weak),
                                                                );
                                                            } else {
                                                                ui.label(
                                                                    RichText::new("连接中…")
                                                                        .color(th.text_weak),
                                                                );
                                                            }
                                                        });
                                                    }
                                                    DlStatus::Done
                                                    | DlStatus::Cancelled
                                                    | DlStatus::Queued => {
                                                        if job.done > 0 {
                                                            ui.label(
                                                                RichText::new(format!(
                                                                    "已下载 {}",
                                                                    format::fmt_bytes(job.done as i64)
                                                                ))
                                                                .color(th.text_weak),
                                                            );
                                                        }
                                                    }
                                                    DlStatus::Failed(_) => {
                                                        ui.label(
                                                            RichText::new("下载未完成, 可移除后重试")
                                                                .color(th.text_weak),
                                                        );
                                                    }
                                                }
                                                if job.speed > 0 && job.status == DlStatus::Running {
                                                    ui.label(
                                                        RichText::new(format!(
                                                            "{} /s",
                                                            format::fmt_bytes(job.speed as i64)
                                                        ))
                                                        .color(th.text_weak),
                                                    );
                                                }
                                                if !job.dir.as_os_str().is_empty() {
                                                    ui.label(
                                                        RichText::new(format!(
                                                            "→ {}",
                                                            job.dir.display()
                                                        ))
                                                        .color(th.text_faint)
                                                        .size(11.0),
                                                    );
                                                }
                                            });
                                            ui.with_layout(
                                                Layout::right_to_left(Align::Center),
                                                |ui| match &job.status {
                                                    DlStatus::Queued | DlStatus::Running => {
                                                        if ui.button("取消").clicked() {
                                                            op = Some(DlOp::Cancel);
                                                        }
                                                    }
                                                    _ => {
                                                        if ui
                                                            .add(
                                                                egui::Button::new(
                                                                    RichText::new("移除")
                                                                        .color(th.text_weak),
                                                                ),
                                                            )
                                                            .clicked()
                                                        {
                                                            op = Some(DlOp::Remove);
                                                        }
                                                        if ui.button("打开目录").clicked() {
                                                            op = Some(DlOp::OpenDir);
                                                        }
                                                    }
                                                },
                                            );
                                        });
                                    });
                            });
                            if let Some(op) = op {
                                ops.push((rid, op));
                            }
                            ui.add_space(8.0);
                        }
                    });
            });

        for (rid, op) in ops {
            match op {
                DlOp::Cancel => self.send(Cmd::CancelDownload { req_id: rid }),
                DlOp::OpenDir => {
                    if let Some(job) = self.jobs.get(&rid) {
                        open_dir(&job.dir);
                    }
                }
                DlOp::Remove => {
                    self.jobs.remove(&rid);
                }
            }
        }
    }
}

/// 用系统默认文件管理器打开某个目录(xdg-open)。
fn open_dir(dir: &std::path::Path) {
    if !dir.is_dir() {
        return;
    }
    let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
}

/// 下载行状态文案。
fn status_line(job: &DlJob) -> (Color32, String) {
    match &job.status {
        DlStatus::Queued => (Color32::from_gray(150), "排队中".into()),
        DlStatus::Running => (Color32::from_rgb(60, 130, 200), "下载中".into()),
        DlStatus::Done => (
            Color32::from_rgb(70, 150, 90),
            format!("已完成 · 保存于 {}", job.dir.display()),
        ),
        DlStatus::Cancelled => (Color32::from_gray(150), "已取消(保留 .part 可续传)".into()),
        DlStatus::Failed(what) => (Color32::from_rgb(217, 70, 60), what.clone()),
    }
}

/// 弹出一个原生目录选择框(独立线程阻塞式调用)。
fn pick_folder(initial: &std::path::Path) -> Option<PathBuf> {
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
fn truncate_text(
    painter: &eframe::egui::Painter,
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
    // 从尾部收缩(保留前段)。
    let chars: Vec<char> = s.chars().collect();
    let mut lo = 0usize;
    let mut hi = chars.len();
    let ell = "…";
    // 二分找最大前缀
    while lo < hi {
        let mid = (lo + hi + 1) / 2;
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
fn draw_sun_moon(painter: &eframe::egui::Painter, rect: Rect, color: Color32) {
    let c = rect.center();
    // 太阳 + 光芒
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

fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    const CANDIDATES: [&str; 6] = [
        "/usr/share/fonts/google-droid-sans-fonts/DroidSansFallbackFull.ttf",
        "/usr/share/fonts/wqy-zenhei-fonts/wqy-zenhei.ttc",
        "/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc",
        "/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf",
        "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
    ];
    for path in CANDIDATES {
        if let Ok(bytes) = std::fs::read(path) {
            let name = "cjk".to_string();
            fonts
                .font_data
                .insert(name.clone(), Arc::new(egui::FontData::from_owned(bytes)));
            for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                fonts.families.entry(family).or_default().push(name.clone());
            }
            break;
        }
    }
    ctx.set_fonts(fonts);
}
