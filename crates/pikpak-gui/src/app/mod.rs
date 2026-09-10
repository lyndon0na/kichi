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
use crate::settings::{self, DownloadRecord, DownloadRecordStatus};
use crate::theme::{self, Theme};
use crate::worker;

use self::helpers::install_fonts;
use self::types::{ColDrag, Crumb, DlJob, DlStatus, MoveDialog, MoveMode, Page, SortBy, ViewMode};

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

    // 视图模式
    pub(crate) view_mode: ViewMode,

    // Shift+Click 范围选择锚点
    pub(crate) last_clicked_id: Option<String>,
    pub(crate) last_clicked_dl: Option<u64>,

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

    // 移动/复制目标目录选择弹窗
    pub(crate) move_dialog: Option<MoveDialog>,

    /// 已移入回收站、等待服务端列表同步的 id(本地先行隐藏)。
    pub(crate) hidden: HashSet<String>,

    // 退出确认
    pub(crate) logout_confirm: bool,

    // 本地下载
    pub(crate) download_dir: String,
    pub(crate) jobs: BTreeMap<u64, DlJob>,
    pub(crate) selected_dl: HashSet<u64>,

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
            req_id: 0, // Will be updated after loading history
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
            move_dialog: None,
            hidden: HashSet::new(),
            logout_confirm: false,
            download_dir: saved.download_dir.clone(),
            jobs: {
                let mut jobs = BTreeMap::new();
                let history = settings::load_download_history();
                let mut next_id = 0u64;
                for record in history {
                    next_id += 1;
                    let status = match record.status {
                        DownloadRecordStatus::Done => DlStatus::Done,
                        DownloadRecordStatus::Cancelled => DlStatus::Cancelled,
                        DownloadRecordStatus::Failed(what) => DlStatus::Failed(what),
                    };
                    jobs.insert(
                        next_id,
                        DlJob {
                            name: record.name,
                            dir: record.dir,
                            total: record.total,
                            done: record.done,
                            status,
                            speed: 0,
                            last_done: 0,
                            last_at: None,
                        },
                    );
                }
                jobs
            },
            selected_dl: HashSet::new(),
            col_size_w: 100.0,
            col_time_w: 160.0,
            col_dragging: None,
            view_mode: ViewMode::List,
            last_clicked_id: None,
            last_clicked_dl: None,
            toast: None,
        };

        // 恢复 req_id 为历史记录中的最大值，避免 ID 冲突
        app.req_id = app.jobs.keys().max().copied().unwrap_or(0);

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
                    self.move_dialog = None;
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
                    self.move_dialog = None;
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
                Msg::FoldersList {
                    req_id,
                    parent,
                    append,
                    list,
                } => {
                    let Some(dlg) = self.move_dialog.as_mut() else {
                        continue;
                    };
                    if dlg.req_id != req_id
                        || parent != dlg.stack.last().and_then(|c| c.id.clone())
                    {
                        continue;
                    }
                    dlg.loading = false;
                    let folders: Vec<File> =
                        list.files.into_iter().filter(|f| f.is_folder()).collect();
                    if append {
                        dlg.folders.extend(folders);
                    } else {
                        dlg.folders = folders;
                    }
                    dlg.next = list.next_page_token;
                }
                Msg::Moved { ids } => {
                    self.toast_ok("移动成功");
                    // batchMove 返回后服务端列表未必立即同步, 先把被移走的项从本地
                    // 隐藏(复用 hidden, 服务端列表不再含该 id 后自动解除), 避免
                    // 刷新前文件仍显示在原目录。
                    for id in &ids {
                        self.selected.remove(id);
                        self.hidden.insert(id.clone());
                    }
                    self.files.retain(|f| !ids.contains(&f.id));
                    self.refresh_dir();
                }
                Msg::Copied => {
                    self.toast_ok("复制成功");
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
                        // 保存下载记录到磁盘
                        settings::append_download_record(DownloadRecord {
                            name: j.name.clone(),
                            dir: j.dir.clone(),
                            total: j.total,
                            done: j.done,
                            status: DownloadRecordStatus::Done,
                            timestamp: Self::chrono_now(),
                        });
                    }
                }
                Msg::DlCancelled { req_id } => {
                    if let Some(j) = self.jobs.get_mut(&req_id) {
                        j.status = DlStatus::Cancelled;
                        // 保存下载记录到磁盘
                        settings::append_download_record(DownloadRecord {
                            name: j.name.clone(),
                            dir: j.dir.clone(),
                            total: j.total,
                            done: j.done,
                            status: DownloadRecordStatus::Cancelled,
                            timestamp: Self::chrono_now(),
                        });
                    }
                }
                Msg::DlFailed { req_id, what } => {
                    if let Some(j) = self.jobs.get_mut(&req_id) {
                        j.status = DlStatus::Failed(what.clone());
                        // 保存下载记录到磁盘
                        settings::append_download_record(DownloadRecord {
                            name: j.name.clone(),
                            dir: j.dir.clone(),
                            total: j.total,
                            done: j.done,
                            status: DownloadRecordStatus::Failed(what),
                            timestamp: Self::chrono_now(),
                        });
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

    /// 获取当前时间的 ISO 8601 格式字符串。
    fn chrono_now() -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        // 使用简单的 Unix 时间戳作为唯一标识
        format!("{}", secs)
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

    /// 把当前选中项以 move/copy 弹窗选择目标目录。空选择时给出提示。
    pub(crate) fn request_move_copy(&mut self, mode: MoveMode) {
        let ids: Vec<String> = self.selected.iter().cloned().collect();
        self.open_move_dialog(mode, ids);
    }

    /// 打开「移动/复制到…」目标选择弹窗。ids 为空时只提示、不弹窗。
    pub(crate) fn open_move_dialog(&mut self, mode: MoveMode, ids: Vec<String>) {
        if ids.is_empty() {
            self.toast_warn("请先选择要操作的文件");
            return;
        }
        let blocked: HashSet<String> = self
            .files
            .iter()
            .filter(|f| ids.contains(&f.id) && f.is_folder())
            .map(|f| f.id.clone())
            .collect();
        let src_parent = self.current_parent();
        let stack = self.stack.clone();
        self.move_dialog = Some(MoveDialog {
            mode,
            ids,
            src_parent,
            blocked,
            stack,
            folders: Vec::new(),
            next: None,
            loading: false,
            req_id: 0,
        });
        self.pick_list(false);
    }

    /// 请求目标目录选择器重新列出当前浏览目录的子文件夹。
    fn pick_list(&mut self, append: bool) {
        let cmd = {
            let Some(dlg) = self.move_dialog.as_mut() else {
                return;
            };
            dlg.req_id += 1;
            let req_id = dlg.req_id;
            let parent = dlg.stack.last().and_then(|c| c.id.clone());
            let token = if append { dlg.next.clone() } else { None };
            dlg.loading = true;
            if !append {
                dlg.folders.clear();
                dlg.next = None;
            }
            Some(Cmd::ListFolders {
                parent,
                token,
                append,
                req_id,
            })
        };
        if let Some(cmd) = cmd {
            self.send(cmd);
        }
    }

    pub(crate) fn pick_refresh(&mut self) {
        self.pick_list(false);
    }

    pub(crate) fn pick_load_more(&mut self) {
        let has_more = {
            let Some(dlg) = &self.move_dialog else {
                return;
            };
            dlg.next.is_some()
        };
        if has_more {
            self.pick_list(true);
        }
    }

    pub(crate) fn pick_open(&mut self, id: String, name: String) {
        {
            let Some(dlg) = self.move_dialog.as_mut() else {
                return;
            };
            // 被选中的源文件夹不可进入, 避免把目录移动/复制进自己。
            if dlg.blocked.contains(&id) {
                return;
            }
            dlg.stack.push(Crumb {
                id: Some(id),
                label: name,
            });
        }
        self.pick_list(false);
    }

    pub(crate) fn pick_up(&mut self) {
        {
            let Some(dlg) = self.move_dialog.as_mut() else {
                return;
            };
            if dlg.stack.len() > 1 {
                dlg.stack.pop();
            }
        }
        self.pick_list(false);
    }

    pub(crate) fn pick_jump(&mut self, idx: usize) {
        {
            let Some(dlg) = self.move_dialog.as_mut() else {
                return;
            };
            if idx + 1 < dlg.stack.len() {
                dlg.stack.truncate(idx + 1);
            }
        }
        self.pick_list(false);
    }

    /// 当前浏览目录是否是源目录(此时移动/复制没有意义)。
    pub(crate) fn pick_at_source(&self) -> bool {
        let Some(dlg) = &self.move_dialog else {
            return false;
        };
        dlg.stack.last().and_then(|c| c.id.clone()) == dlg.src_parent
    }

    /// 把选中项执行到当前浏览目录。
    pub(crate) fn pick_confirm(&mut self) {
        let Some(dlg) = self.move_dialog.take() else {
            return;
        };
        let dest = dlg.stack.last().and_then(|c| c.id.clone());
        match dlg.mode {
            MoveMode::Move => self.send(Cmd::MoveTo {
                ids: dlg.ids,
                dest,
            }),
            MoveMode::Copy => self.send(Cmd::CopyTo {
                ids: dlg.ids,
                dest,
            }),
        }
    }

    pub(crate) fn close_move_dialog(&mut self) {
        self.move_dialog = None;
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

    /// 全选下载任务。
    pub(crate) fn select_all_dl(&mut self) {
        self.selected_dl = self.jobs.keys().cloned().collect();
    }

    /// 取消全选下载任务。
    pub(crate) fn deselect_all_dl(&mut self) {
        self.selected_dl.clear();
    }

    /// 反选下载任务。
    pub(crate) fn invert_selection_dl(&mut self) {
        for key in self.jobs.keys() {
            if self.selected_dl.contains(key) {
                self.selected_dl.remove(key);
            } else {
                self.selected_dl.insert(*key);
            }
        }
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
        for (id, name) in &items {
            let req_id = self.alloc_req_id();
            self.jobs
                .insert(req_id, DlJob::queued(name.clone(), dir.clone()));
            self.send(Cmd::StartDownload {
                req_id,
                file_id: id.clone(),
                name: name.clone(),
                dest_dir: dir.clone(),
            });
        }
        self.toast_ok(&format!("已加入下载队列 ({} 个文件)", items.len()));
    }

    /// 下载单个文件。若已有默认下载目录则直接下载, 否则弹目录选择框。
    pub(crate) fn download_single(&mut self, id: String, name: String) {
        let dir = if !self.download_dir.is_empty()
            && std::path::Path::new(&self.download_dir).is_dir()
        {
            std::path::PathBuf::from(&self.download_dir)
        } else {
            let Some(d) = self.choose_download_dir() else {
                return;
            };
            d
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
