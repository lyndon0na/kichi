mod dialogs;
mod downloads_page;
mod files_page;
mod helpers;
mod login;
mod settings_page;
mod sidebar;
mod tasks_page;
pub(crate) mod types;

use std::collections::{BTreeMap, HashSet};
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32};
use pikpak_core::session;
use pikpak_core::types::{File, Quota, Task};

use crate::msg::{Cmd, Msg};
use crate::settings;
use crate::theme::{self, Theme};
use crate::worker;

use self::helpers::install_fonts;
use self::types::{ColDrag, Crumb, DlJob, DlStatus, Page, SortBy};

pub struct App {
    tx: Sender<Cmd>,
    rx: Receiver<Msg>,

    // 认证
    pub(crate) auth_checking: bool,
    pub(crate) username: String,
    pub(crate) login_username: String,
    pub(crate) login_password: String,
    pub(crate) auth_error: Option<String>,
    pub(crate) dark: bool,

    pub(crate) page: Page,

    // 文件浏览
    pub(crate) stack: Vec<Crumb>,
    pub(crate) req_id: u64,
    pub(crate) selected: HashSet<String>,
    pub(crate) sort_by: SortBy,
    pub(crate) sort_desc: bool,
    pub(crate) filter: String,
    pub(crate) files: Vec<File>,
    pub(crate) dir_next: Option<String>,
    pub(crate) dir_loading: bool,

    // 列宽 (名称列 = 剩余空间)
    pub(crate) col_size_w: f32,
    pub(crate) col_time_w: f32,
    pub(crate) col_dragging: Option<ColDrag>,

    // 离线
    pub(crate) offline_url: String,
    pub(crate) offline_name: String,
    pub(crate) offline_to_current: bool,
    pub(crate) buckets: BTreeMap<String, Vec<Task>>,
    pub(crate) tasks_loading: bool,

    pub(crate) quota: Option<Quota>,

    // 对话框
    pub(crate) mkdir_open: bool,
    pub(crate) mkdir_name: String,
    pub(crate) rename_id: Option<String>,
    pub(crate) rename_name: String,
    pub(crate) trash_confirm: Option<Vec<(String, String)>>,

    /// 已移入回收站、等待服务端列表同步的 id(本地先行隐藏)。
    pub(crate) hidden: HashSet<String>,

    // 退出确认
    pub(crate) logout_confirm: bool,

    // 本地下载
    pub(crate) download_dir: String,
    pub(crate) jobs: BTreeMap<u64, DlJob>,

    pub(crate) toast: Option<(Color32, String, Instant)>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let font_loaded = install_fonts(&cc.egui_ctx);
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
            logout_confirm: false,
            download_dir: saved.download_dir.clone(),
            jobs: BTreeMap::new(),
            col_size_w: 100.0,
            col_time_w: 160.0,
            col_dragging: None,
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
        if !font_loaded && app.toast.is_none() {
            app.toast = Some((
                Color32::from_rgb(200, 160, 60),
                "未找到中文字体，中文可能显示为方块。请安装 wqy-zenhei 或 google-droid-sans-fonts".into(),
                Instant::now(),
            ));
        }
        app
    }

    pub(crate) fn theme(&self) -> Theme {
        Theme::new(self.dark)
    }

    pub(crate) fn send(&self, cmd: Cmd) {
        let _ = self.tx.send(cmd);
    }

    pub(crate) fn drain(&mut self) {
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

    pub(crate) fn toast(&mut self, msg: &str, color: Color32) {
        self.toast = Some((color, msg.to_string(), Instant::now()));
    }
    pub(crate) fn toast_ok(&mut self, msg: &str) {
        self.toast(msg, self.theme().ok);
    }
    pub(crate) fn toast_warn(&mut self, msg: &str) {
        self.toast(msg, self.theme().warn);
    }
    pub(crate) fn toast_err(&mut self, msg: &str) {
        self.toast(msg, self.theme().danger);
    }

    pub(crate) fn persist_settings(&self) {
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

    pub(crate) fn toggle_theme(&mut self) {
        self.dark = !self.dark;
        self.persist_settings();
    }

    pub(crate) fn current_parent(&self) -> Option<String> {
        self.stack.last().and_then(|c| c.id.clone())
    }

    pub(crate) fn reset_stack(&mut self) {
        self.stack = vec![Crumb {
            id: None,
            label: "我的云盘".into(),
        }];
        self.dir_next = None;
        self.dir_loading = true;
    }

    pub(crate) fn reset_browse(&mut self) {
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

    pub(crate) fn refresh_dir(&mut self) {
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

    pub(crate) fn load_more(&mut self) {
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

    pub(crate) fn goto_folder(&mut self, id: &str, name: &str) {
        self.stack.push(Crumb {
            id: Some(id.to_string()),
            label: name.to_string(),
        });
        self.selected.clear();
        self.refresh_dir();
    }

    /// 当前目录内过滤后的可见文件(文件夹在前, 组内按当前排序)。
    pub(crate) fn visible_rows(&self) -> (Vec<File>, Vec<File>) {
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

    pub(crate) fn selected_names(&self) -> Vec<(String, String)> {
        self.files
            .iter()
            .filter(|f| self.selected.contains(&f.id))
            .map(|f| (f.id.clone(), f.name.clone()))
            .collect()
    }

    /// 选中项里可下载的文件(id, name), 文件夹除外。
    pub(crate) fn selected_plain_files(&self) -> Vec<(String, String)> {
        self.files
            .iter()
            .filter(|f| self.selected.contains(&f.id) && !f.is_folder())
            .map(|f| (f.id.clone(), f.name.clone()))
            .collect()
    }

    pub(crate) fn selected_has_folder(&self) -> bool {
        self.files
            .iter()
            .any(|f| self.selected.contains(&f.id) && f.is_folder())
    }

    pub(crate) fn alloc_req_id(&mut self) -> u64 {
        self.req_id += 1;
        self.req_id
    }

    pub(crate) fn has_active_downloads(&self) -> bool {
        self.jobs
            .values()
            .any(|j| j.status == DlStatus::Queued || j.status == DlStatus::Running)
    }

    /// 弹原生目录选择框选保存位置(取消返回 None)。
    pub(crate) fn choose_download_dir(&mut self) -> Option<std::path::PathBuf> {
        let initial = {
            let p = std::path::PathBuf::from(&self.download_dir);
            if !self.download_dir.is_empty() && p.is_dir() {
                p
            } else {
                dirs::download_dir()
                    .or_else(dirs::home_dir)
                    .unwrap_or_default()
            }
        };
        let picked = helpers::pick_folder(&initial)?;
        if let Some(s) = picked.to_str() {
            self.download_dir = s.to_string();
        }
        self.persist_settings();
        Some(picked)
    }

    /// 逐个提交下载任务(共享同一个已选目录)。
    pub(crate) fn enqueue_downloads(&mut self, items: Vec<(String, String)>, dir: std::path::PathBuf) {
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
    pub(crate) fn download_single(&mut self, id: String, name: String) {
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
