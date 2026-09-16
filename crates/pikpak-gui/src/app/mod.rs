mod dialogs;
mod files_page;
mod helpers;
mod library_page;
mod login;
mod settings_page;
mod sidebar;
mod tasks_page;
mod transfers_page;
pub(crate) mod types;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32};
use pikpak_core::session;
use pikpak_core::types::{File, FileList, Quota, Task};

use crate::kde;
use crate::msg::{Cmd, Msg};
use crate::settings::{self, DownloadRecord, DownloadRecordStatus};
use crate::theme::{self, Theme};
use crate::worker;

use self::helpers::install_fonts;
use self::types::{ClipKind, Clipboard, ColDrag, Crumb, DirEntry, DlFilter, DlJob, DlStatus, Page, QualityReady, SortBy, TransferTab, ViewMode};

/// 目录缓存新鲜期: 命中后超过该时长, 先展示旧数据再后台静默校正。
const DIR_TTL: Duration = Duration::from_secs(60);
/// 目录缓存上限, 超出按 LRU 淘汰(不淘汰当前目录)。
const DIR_CACHE_CAP: usize = 64;

pub struct App {
    tx: Sender<Cmd>,
    rx: Receiver<Msg>,

    // 认证
    pub(crate) auth_checking: bool,
    pub(crate) username: String,
    pub(crate) login_username: String,
    pub(crate) login_password: String,
    pub(crate) auth_error: Option<String>,
    /// 系统(KDE)配色; 非 KDE 时为 None。
    pub(crate) kde_colors: Option<kde::KdeColors>,
    /// 上次轮询系统主题的时间。
    pub(crate) kde_checked: Instant,

    pub(crate) page: Page,
    /// 传输任务页当前分栏(上传 / 下载)。
    pub(crate) transfer_tab: TransferTab,

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
    /// 目录列表缓存: 目录 id -> 已加载内容(None = 根目录)。命中时导航不再发请求。
    pub(crate) dir_cache: HashMap<Option<String>, DirEntry>,
    /// 正在进行的首屏请求: 目录 id -> 请求 id, 用于避免重复的 revalidate。
    pub(crate) dir_inflight: HashMap<Option<String>, u64>,

    // 列宽 (名称列 = 剩余空间)
    pub(crate) col_size_w: f32,
    pub(crate) col_time_w: f32,
    pub(crate) col_dragging: Option<ColDrag>,

    // 视图模式
    pub(crate) view_mode: ViewMode,

    // Shift+Click 范围选择锚点
    pub(crate) last_clicked_dl: Option<u64>,

    // 离线
    pub(crate) offline_url: String,
    pub(crate) offline_name: String,
    /// 离线下载保存到的网盘目录 (id, 名称); None = 离线默认目录。
    pub(crate) offline_dest: Option<(String, String)>,
    /// 离线下载「保存到」网盘目录选择器状态。
    pub(crate) offline_picker_open: bool,
    pub(crate) offline_picker_stack: Vec<Crumb>,
    pub(crate) offline_picker_folders: Vec<File>,
    pub(crate) offline_picker_loading: bool,
    pub(crate) offline_picker_req: u64,
    pub(crate) buckets: BTreeMap<String, Vec<Task>>,
    pub(crate) tasks_loading: bool,
    /// 用户点击「刷新任务」后的进行中状态, 用于给出可见反馈。
    pub(crate) tasks_refreshing: bool,

    pub(crate) quota: Option<Quota>,

    // 对话框
    pub(crate) mkdir_open: bool,
    pub(crate) mkdir_name: String,
    pub(crate) rename_id: Option<String>,
    pub(crate) rename_name: String,
    pub(crate) trash_confirm: Option<Vec<(String, String)>>,

    // 复制/剪切剪贴板(在目标目录粘贴)
    pub(crate) clipboard: Option<Clipboard>,

    /// 移动/复制成功后延迟重列目录的时间点(规避服务端列表最终一致性)。
    pub(crate) relist_at: Option<Instant>,

    /// 等待服务端列表同步、本地先行隐藏的 id -> 其应隐藏的目录(回收/移出的源目录)。
    /// 用目录区分, 避免移动后的文件在目标目录里也被隐藏。
    pub(crate) hidden: HashMap<String, Option<String>>,

    // 退出确认
    pub(crate) logout_confirm: bool,

    // 本地下载
    pub(crate) download_dir: String,
    pub(crate) jobs: BTreeMap<u64, DlJob>,
    pub(crate) selected_dl: HashSet<u64>,
    pub(crate) dl_filter: DlFilter,

    /// 正在准备中的预览任务 (req_id, 文件名); 用于给出加载反馈。
    pub(crate) preview_pending: Option<(u64, String)>,

    /// 已解析的媒体文件清晰度缓存(file_id -> 清晰度+字幕)。
    pub(crate) quality_cache: HashMap<String, QualityReady>,
    /// 正在解析清晰度的文件 id。
    pub(crate) quality_inflight: HashSet<String>,

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
            kde_colors: kde::load(),
            kde_checked: Instant::now(),
            page: Page::Files,
            transfer_tab: TransferTab::Download,
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
            dir_cache: HashMap::new(),
            dir_inflight: HashMap::new(),
            offline_url: String::new(),
            offline_name: String::new(),
            offline_dest: None,
            offline_picker_open: false,
            offline_picker_stack: vec![Crumb {
                id: None,
                label: "我的云盘".into(),
            }],
            offline_picker_folders: Vec::new(),
            offline_picker_loading: false,
            offline_picker_req: 0,
            buckets: BTreeMap::new(),
            tasks_loading: false,
            tasks_refreshing: false,
            quota: None,
            mkdir_open: false,
            mkdir_name: String::new(),
            rename_id: None,
            rename_name: String::new(),
            trash_confirm: None,
            clipboard: None,
            relist_at: None,
            hidden: HashMap::new(),
            logout_confirm: false,
            download_dir: saved.download_dir.clone(),
            jobs: {
                let mut jobs = BTreeMap::new();
                let history = settings::load_download_history();
                let mut next_id = 0u64;
                // 历史文件按最新在前存储; 倒序分配 id, 使 id 随时间递增(旧的 id 小)。
                for record in history.into_iter().rev() {
                    next_id += 1;
                    let status = match record.status {
                        DownloadRecordStatus::Done => DlStatus::Done,
                        DownloadRecordStatus::Cancelled => DlStatus::Cancelled,
                        DownloadRecordStatus::Failed(what) => DlStatus::Failed(what),
                    };
                    jobs.insert(
                        next_id,
                        DlJob {
                            file_id: record.file_id,
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
            dl_filter: DlFilter::All,
            preview_pending: None,
            quality_cache: HashMap::new(),
            quality_inflight: HashSet::new(),
            col_size_w: 100.0,
            col_time_w: 160.0,
            col_dragging: None,
            view_mode: ViewMode::List,
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
        match &self.kde_colors {
            Some(k) => Theme::from_kde(k),
            None => Theme::fallback(),
        }
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
                    self.selected_dl.clear();
                    self.last_clicked_dl = None;
                    self.clipboard = None;
                    self.preview_pending = None;
                    self.quality_cache.clear();
                    self.quality_inflight.clear();
                    self.dir_cache.clear();
                    self.dir_inflight.clear();
                }
                Msg::LoggedOut => {
                    self.username.clear();
                    self.auth_checking = false;
                    self.auth_error = None;
                    self.quota = None;
                    self.buckets.clear();
                    self.files.clear();
                    self.selected.clear();
                    self.dir_cache.clear();
                    self.dir_inflight.clear();
                    self.hidden.clear();
                    self.jobs.clear();
                    self.selected_dl.clear();
                    self.last_clicked_dl = None;
                    self.clipboard = None;
                    self.preview_pending = None;
                    self.quality_cache.clear();
                    self.quality_inflight.clear();
                    self.reset_stack();
                }
                Msg::Files {
                    parent,
                    req_id,
                    append,
                    list,
                } => {
                    // 响应按 parent 路由进缓存; 即使已切换到别的目录, 迟到的
                    // 响应也能正确落位, 下次进入该目录即可命中。
                    self.apply_files(parent, req_id, append, list);
                }
                Msg::FolderCreated => {
                    self.toast_ok("新建文件夹成功");
                    self.reload_dir();
                }
                Msg::Renamed => {
                    self.rename_id = None;
                    self.toast_ok("重命名成功");
                    self.reload_dir();
                }
                Msg::Trashed => {
                    self.trash_confirm = None;
                    self.selected.clear();
                    self.reload_dir();
                }
                Msg::Moved { ids, src, dest } => {
                    self.toast_ok("移动成功");
                    // batchMove 返回后服务端列表未必立即同步, 先把被移走的项从源
                    // 目录隐藏(服务端列表不再含该 id 后自动解除), 避免刷新前文件仍
                    // 显示在原目录; 隐藏按源目录区分, 目标目录里仍会正常显示。
                    for id in &ids {
                        self.selected.remove(id);
                        self.hidden.insert(id.clone(), src.clone());
                    }
                    // 同步更新源目录缓存, 并对目标目录作废缓存, 稍后重列。
                    if let Some(entry) = self.dir_cache.get_mut(&src) {
                        entry.files.retain(|f| !ids.contains(&f.id));
                    }
                    self.files.retain(|f| !ids.contains(&f.id));
                    self.dir_cache.remove(&dest);
                    self.relist_at = Some(Instant::now() + Duration::from_millis(1500));
                }
                Msg::Copied { dest } => {
                    self.toast_ok("复制成功");
                    // 复制的目标目录当前可能正在展示, 稍后重列以显示新文件。
                    self.dir_cache.remove(&dest);
                    self.relist_at = Some(Instant::now() + Duration::from_millis(1500));
                }
                Msg::OfflineCreated => {
                    self.toast_ok("已提交离线下载");
                    self.offline_url.clear();
                    self.offline_name.clear();
                    self.send(Cmd::RefreshTasks);
                }
                Msg::OfflineRetried => self.send(Cmd::RefreshTasks),
                Msg::OfflineDeleted => self.send(Cmd::RefreshTasks),
                Msg::Folders { parent, req_id, files } => {
                    if req_id != self.offline_picker_req || parent != self.offline_picker_parent() {
                        continue;
                    }
                    self.offline_picker_loading = false;
                    self.offline_picker_folders = files;
                }
                Msg::Quota(quota) => self.quota = quota,
                Msg::TasksAll { buckets } => {
                    // 合并而非整体替换: 某个分桶刷新失败时保留其旧数据。
                    for (phase, tasks) in buckets {
                        if tasks.is_empty() {
                            self.buckets.remove(&phase);
                        } else {
                            self.buckets.insert(phase, tasks);
                        }
                    }
                    self.tasks_loading = false;
                    self.tasks_refreshing = false;
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
                            file_id: j.file_id.clone(),
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
                            file_id: j.file_id.clone(),
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
                            file_id: j.file_id.clone(),
                            name: j.name.clone(),
                            dir: j.dir.clone(),
                            total: j.total,
                            done: j.done,
                            status: DownloadRecordStatus::Failed(what),
                            timestamp: Self::chrono_now(),
                        });
                    }
                }
                Msg::FilesFailed { parent, what } => {
                    // 结束该目录的加载态并释放在途登记; 若仍停留在该目录,
                    // 无缓存数据时会显示空目录提示, 而非一直转圈。
                    self.dir_inflight.remove(&parent);
                    if parent == self.current_parent() {
                        self.dir_loading = false;
                    }
                    self.toast_err(&what);
                }
                Msg::Error { what } => {
                    self.tasks_refreshing = false;
                    self.toast_err(&what);
                }
                Msg::PreviewStream {
                    req_id,
                    name,
                    url,
                    headers,
                    subs,
                } => {
                    if self
                        .preview_pending
                        .as_ref()
                        .is_some_and(|(id, _)| *id == req_id)
                    {
                        self.preview_pending = None;
                    }
                    match helpers::play_with_mpv(&name, &url, &headers, &subs) {
                        Ok(()) => self.toast_ok(&format!("正在用 mpv 播放「{name}」")),
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                            // 音视频流式播放依赖 mpv, 缺失时只提示, 不下载回退。
                            self.toast_warn("播放音视频需要 mpv, 请先安装 mpv");
                        }
                        Err(e) => self.toast_err(&format!("启动 mpv 失败: {e}")),
                    }
                }
                Msg::PreviewReady { req_id, name, path } => {
                    if self
                        .preview_pending
                        .as_ref()
                        .is_some_and(|(id, _)| *id == req_id)
                    {
                        self.preview_pending = None;
                    }
                    match helpers::open_path(&path) {
                        Ok(()) => self.toast_ok(&format!("已打开「{name}」")),
                        Err(e) => self.toast_err(&format!("打开文件失败: {e}")),
                    }
                }
                Msg::PreviewQualities {
                    file_id,
                    qualities,
                    subs,
                } => {
                    self.quality_inflight.remove(&file_id);
                    self.quality_cache.insert(
                        file_id,
                        QualityReady {
                            options: qualities,
                            subs,
                        },
                    );
                }
                Msg::QualitiesFailed { file_id, what } => {
                    self.quality_inflight.remove(&file_id);
                    self.toast_err(&what);
                }
                Msg::PreviewFailed { req_id, what } => {
                    if self
                        .preview_pending
                        .as_ref()
                        .is_some_and(|(id, _)| *id == req_id)
                    {
                        self.preview_pending = None;
                    }
                    self.toast_err(&what);
                }
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
            download_dir: self.download_dir.clone(),
        });
    }

    /// 定期重读 kdeglobals, 让系统换主题后应用即时更新。
    fn poll_system_theme(&mut self) {
        if self.kde_checked.elapsed() < Duration::from_millis(1500) {
            return;
        }
        self.kde_checked = Instant::now();
        let fresh = kde::load();
        let changed = match (&self.kde_colors, &fresh) {
            (Some(a), Some(b)) => a.accent != b.accent || a.dark != b.dark || a.view_bg != b.view_bg,
            (None, Some(_)) | (Some(_), None) => true,
            (None, None) => false,
        };
        if changed {
            self.kde_colors = fresh;
        }
    }

    pub(crate) fn current_parent(&self) -> Option<String> {
        self.stack.last().and_then(|c| c.id.clone())
    }

    /// 离线下载目录选择器当前所在目录。
    pub(crate) fn offline_picker_parent(&self) -> Option<String> {
        self.offline_picker_stack.last().and_then(|c| c.id.clone())
    }

    /// 打开离线下载「保存到」网盘目录选择器, 从根目录开始。
    pub(crate) fn open_offline_picker(&mut self) {
        self.offline_picker_open = true;
        self.offline_picker_stack = vec![Crumb {
            id: None,
            label: "我的云盘".into(),
        }];
        self.offline_picker_list();
    }

    /// 请求选择器当前目录的子文件夹列表。
    pub(crate) fn offline_picker_list(&mut self) {
        self.offline_picker_loading = true;
        self.offline_picker_folders.clear();
        self.offline_picker_req += 1;
        let req_id = self.offline_picker_req;
        self.send(Cmd::ListFolders {
            parent: self.offline_picker_parent(),
            req_id,
        });
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
        self.dir_cache.clear();
        self.dir_inflight.clear();
        self.reset_stack();
        self.selected.clear();
        self.fetch_dir(None);
    }

    /// 发送一次 ListFiles 并登记在途请求(首屏才登记, 用于去重)。
    fn send_list(&mut self, parent: Option<String>, token: Option<String>, append: bool) {
        self.req_id += 1;
        let req_id = self.req_id;
        if !append {
            self.dir_inflight.insert(parent.clone(), req_id);
        }
        self.send(Cmd::ListFiles {
            parent,
            token,
            append,
            req_id,
        });
    }

    /// 冷加载: 清空视图并显示加载态。
    fn fetch_dir(&mut self, parent: Option<String>) {
        self.files.clear();
        self.dir_next = None;
        self.dir_loading = true;
        self.send_list(parent, None, false);
    }

    /// 静默校正: 保留当前视图(继续展示旧数据), 仅后台刷新缓存。
    /// 该目录已有在途请求时跳过, 避免重复请求。
    fn revalidate_dir(&mut self, parent: Option<String>) {
        if self.dir_inflight.contains_key(&parent) {
            return;
        }
        self.send_list(parent, None, false);
    }

    /// 打开当前目录。命中且新鲜则同帧渲染、零请求; 命中但过期先展示旧数据
    /// 再后台静默校正; 未命中才冷加载。
    pub(crate) fn show_dir(&mut self) {
        self.selected.clear();
        let parent = self.current_parent();
        let cached = self
            .dir_cache
            .get(&parent)
            .map(|e| (e.files.clone(), e.next_token.clone(), e.fetched_at.elapsed()));
        match cached {
            Some((files, next, age)) => {
                self.files = files;
                self.dir_next = next;
                self.dir_loading = false;
                if let Some(entry) = self.dir_cache.get_mut(&parent) {
                    entry.last_used = Instant::now();
                }
                if age > DIR_TTL {
                    self.revalidate_dir(parent);
                }
            }
            None if self.dir_inflight.contains_key(&parent) => {
                // 已有请求在途: 保持加载态等待, 不重复发起。
                self.files.clear();
                self.dir_next = None;
                self.dir_loading = true;
            }
            None => self.fetch_dir(parent),
        }
    }

    /// 强制重新加载当前目录(F5 / 刷新按钮), 清空视图并显示加载态。
    pub(crate) fn refresh_dir(&mut self) {
        let parent = self.current_parent();
        self.dir_cache.remove(&parent);
        self.selected.clear();
        self.fetch_dir(parent);
    }

    /// 变更后就地作废缓存并静默重列当前目录: 保留现有列表(避免闪烁),
    /// 后台重新拉取以反映增删改。
    fn reload_dir(&mut self) {
        let parent = self.current_parent();
        self.dir_cache.remove(&parent);
        self.selected.clear();
        self.send_list(parent, None, false);
    }

    /// 把一次 ListFiles 响应写入缓存, 并在其属于当前目录时同步到可见列表。
    /// `entry.req` 保证乱序到达的旧响应不会覆盖新数据。
    fn apply_files(&mut self, parent: Option<String>, req_id: u64, append: bool, list: FileList) {
        let is_current = parent == self.current_parent();
        {
            let entry = self.dir_cache.entry(parent.clone()).or_default();
            if req_id < entry.req {
                return;
            }
            entry.req = req_id;
            entry.fetched_at = Instant::now();
            entry.last_used = entry.fetched_at;
            if append {
                let ids: HashSet<String> = entry.files.iter().map(|f| f.id.clone()).collect();
                for f in list.files {
                    if !ids.contains(&f.id) {
                        entry.files.push(f);
                    }
                }
            } else {
                entry.files = list.files;
            }
            entry.next_token = list.next_page_token;
        }
        if !append && self.dir_inflight.get(&parent) == Some(&req_id) {
            self.dir_inflight.remove(&parent);
        }

        if is_current {
            self.dir_loading = false;
            if let Some(entry) = self.dir_cache.get(&parent) {
                if entry.req == req_id {
                    self.files = entry.files.clone();
                    self.dir_next = entry.next_token.clone();
                }
            }
            if !append {
                self.selected.clear();
            }
        }

        // 服务端列表已更新: 解除该目录中不再出现的隐藏项(最终一致性收敛)。
        if !self.hidden.is_empty() {
            let present: HashSet<String> = if is_current {
                self.files.iter().map(|f| f.id.clone()).collect()
            } else if let Some(entry) = self.dir_cache.get(&parent) {
                entry.files.iter().map(|f| f.id.clone()).collect()
            } else {
                HashSet::new()
            };
            let listing_parent = parent.clone();
            self.hidden.retain(|id, hp| {
                *hp != listing_parent || present.contains(id)
            });
        }

        self.evict_dir_cache();
    }

    /// 缓存超限时按 LRU 淘汰, 跳过当前目录。
    fn evict_dir_cache(&mut self) {
        if self.dir_cache.len() <= DIR_CACHE_CAP {
            return;
        }
        let current = self.current_parent();
        let mut keys: Vec<(Option<String>, Instant)> = self
            .dir_cache
            .iter()
            .map(|(k, e)| (k.clone(), e.last_used))
            .collect();
        keys.sort_by_key(|(_, t)| *t);
        for (k, _) in keys {
            if self.dir_cache.len() <= DIR_CACHE_CAP {
                break;
            }
            if k == current {
                continue;
            }
            self.dir_cache.remove(&k);
        }
    }

    pub(crate) fn load_more(&mut self) {
        if self.dir_next.is_none() {
            return;
        }
        let parent = self.current_parent();
        let token = self.dir_next.clone();
        self.send_list(parent, token, true);
    }

    pub(crate) fn goto_folder(&mut self, id: &str, name: &str) {
        self.stack.push(Crumb {
            id: Some(id.to_string()),
            label: name.to_string(),
        });
        self.show_dir();
    }

    // ---------- 复制/剪切/粘贴 ----------

    /// 把当前选中项放入剪贴板。
    pub(crate) fn clip_selection(&mut self, kind: ClipKind) {
        let ids: Vec<String> = self
            .files
            .iter()
            .filter(|f| self.selected.contains(&f.id))
            .map(|f| f.id.clone())
            .collect();
        if ids.is_empty() {
            self.toast_warn("请先选择要操作的文件");
            return;
        }
        self.set_clipboard(kind, ids);
    }

    /// 右键单项: 若该项在多选中则操作整个选中集, 否则仅操作该项。
    pub(crate) fn clip_item(&mut self, kind: ClipKind, id: String) {
        let ids = if self.selected.contains(&id) && self.selected.len() > 1 {
            self.files
                .iter()
                .filter(|f| self.selected.contains(&f.id))
                .map(|f| f.id.clone())
                .collect()
        } else {
            vec![id]
        };
        self.set_clipboard(kind, ids);
    }

    fn set_clipboard(&mut self, kind: ClipKind, ids: Vec<String>) {
        let label = if ids.len() == 1 {
            self.files
                .iter()
                .find(|f| f.id == ids[0])
                .map(|f| f.name.clone())
                .unwrap_or_else(|| "文件".to_string())
        } else {
            format!("{} 项", ids.len())
        };
        let src_parent = self.current_parent();
        self.clipboard = Some(Clipboard {
            kind,
            ids,
            src_parent,
            label: label.clone(),
        });
        let act = match kind {
            ClipKind::Copy => "复制",
            ClipKind::Cut => "剪切",
        };
        self.toast_ok(&format!("已{act}「{label}」, 进入目标目录后粘贴"));
    }

    /// 粘贴到当前目录。
    pub(crate) fn paste_clipboard(&mut self) {
        let dest = self.current_parent();
        self.paste_into(dest);
    }

    /// 粘贴到指定目录(None = 根目录)。
    pub(crate) fn paste_into(&mut self, dest: Option<String>) {
        let Some(clip) = self.clipboard.clone() else {
            self.toast_warn("剪贴板为空, 请先复制或剪切");
            return;
        };
        if let Some(d) = &dest {
            if clip.ids.contains(d) {
                self.toast_warn("不能粘贴到被操作的文件夹自身");
                return;
            }
        }
        if clip.kind == ClipKind::Cut && dest == clip.src_parent {
            self.toast_warn("已在原目录, 无需粘贴");
            return;
        }
        match clip.kind {
            ClipKind::Copy => self.send(Cmd::CopyTo { ids: clip.ids, dest }),
            ClipKind::Cut => {
                self.send(Cmd::MoveTo {
                    ids: clip.ids,
                    dest,
                    src: clip.src_parent,
                });
                // 剪切只生效一次, 粘贴后清空剪贴板。
                self.clipboard = None;
            }
        }
    }

    /// 当前目录内过滤后的可见文件(文件夹在前, 组内按当前排序)。
    pub(crate) fn visible_rows(&self) -> (Vec<File>, Vec<File>) {
        let kw = self.filter.trim().to_lowercase();
        let cur = self.current_parent();
        let mut folders: Vec<&File> = Vec::new();
        let mut plain: Vec<&File> = Vec::new();
        for f in &self.files {
            if self.hidden.get(&f.id).is_some_and(|hp| *hp == cur) {
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

    /// 当前筛选下可见的任务 id, 按最新在前排序。
    pub(crate) fn visible_dl_ids(&self) -> Vec<u64> {
        let mut ids: Vec<u64> = self
            .jobs
            .iter()
            .filter(|(_, j)| self.dl_filter.matches(j))
            .map(|(id, _)| *id)
            .collect();
        ids.sort_unstable_by(|a, b| b.cmp(a));
        ids
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
                .insert(req_id, DlJob::queued(id.clone(), name.clone(), dir.clone()));
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

    /// 当前目录下与 `name` 同集的外挂字幕 (id, 文件名)。
    fn episode_subtitles(&self, name: &str) -> Vec<(String, String)> {
        self.files
            .iter()
            .filter(|f| {
                !f.is_folder()
                    && helpers::is_subtitle_file(&f.name)
                    && helpers::subtitle_of(name, &f.name)
            })
            .map(|f| (f.id.clone(), f.name.clone()))
            .collect()
    }

    /// 预览云端文件: 音/视频交给 mpv 流式播放, 其他下载后交给系统查看器。
    pub(crate) fn open_preview(&mut self, id: String, name: String) {
        let media = helpers::is_media_file(&name);
        let req_id = self.alloc_req_id();
        self.preview_pending = Some((req_id, name.clone()));
        // 同目录下的同集字幕, 播放时一并挂载(仅媒体预览需要)。
        let subtitles = if media {
            self.episode_subtitles(&name)
        } else {
            Vec::new()
        };
        let hint = if media {
            "正在解析播放地址…"
        } else {
            "正在准备预览文件…"
        };
        self.toast(hint, self.theme().accent);
        self.send(Cmd::Preview {
            req_id,
            file_id: id,
            name,
            media,
            subtitles,
        });
    }

    /// 确保某媒体文件的可用清晰度已解析(供「播放」子菜单展示)。
    pub(crate) fn fetch_qualities(&mut self, id: String, name: String) {
        if self.quality_cache.contains_key(&id) || self.quality_inflight.contains(&id) {
            return;
        }
        self.quality_inflight.insert(id.clone());
        let subtitles = self.episode_subtitles(&name);
        self.send(Cmd::PreviewQualities {
            file_id: id,
            subtitles,
        });
    }

    /// 用某个已解析出的清晰度播放(挂载同集字幕)。
    pub(crate) fn play_option(&mut self, id: String, opt: crate::msg::QualityOption) {
        let subs = self
            .quality_cache
            .get(&id)
            .map(|r| r.subs.clone())
            .unwrap_or_default();
        let name = self
            .files
            .iter()
            .find(|f| f.id == id)
            .map(|f| f.name.clone())
            .unwrap_or_else(|| opt.label.clone());
        match helpers::play_with_mpv(&name, &opt.url, &opt.headers, &subs) {
            Ok(()) => self.toast_ok(&format!("正在用 mpv 播放「{name}」({})", opt.label)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                self.toast_warn("播放音视频需要 mpv, 请先安装 mpv");
            }
            Err(e) => self.toast_err(&format!("启动 mpv 失败: {e}")),
        }
    }
}

// ================= 主循环 =================

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain();
        self.poll_system_theme();

        // 移动/复制成功后延迟重列一次目录(服务端列表存在最终一致性)。
        // 用静默校正而非强制刷新, 避免清空列表导致闪烁。
        if let Some(at) = self.relist_at {
            if Instant::now() >= at {
                self.relist_at = None;
                let parent = self.current_parent();
                self.revalidate_dir(parent);
            } else {
                ctx.request_repaint_after(Duration::from_millis(150));
            }
        }

        let th = self.theme();
        theme::configure(ctx, &th);

        if self.auth_checking {
            ctx.request_repaint_after(Duration::from_millis(120));
        } else if !self.quality_inflight.is_empty() {
            // 清晰度解析中, 加快轮询让「播放」子菜单尽快展开选项。
            ctx.request_repaint_after(Duration::from_millis(100));
        } else if self.has_active_downloads()
            || self.dir_loading
            || !self.dir_inflight.is_empty()
        {
            // 目录请求在途(含后台静默校正)时加快轮询, 让结果尽快呈现。
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
