mod dialogs;
mod files_page;
mod helpers;
mod login;
mod settings_page;
mod shares_page;
mod sidebar;
mod tasks_page;
mod transfers_page;
mod trash_page;
pub(crate) mod types;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32};
use kichi_core::session;
use kichi_core::types::{File, FileList, Quota, Share, Task};

use crate::kde;
use crate::msg::{Cmd, Msg};
use crate::settings::{
    self, DownloadRecord, DownloadRecordStatus, UploadRecord, UploadRecordStatus,
};
use crate::theme::{self, Theme};
use crate::worker;

use self::helpers::install_fonts;
use self::types::{
    ClipKind, Clipboard, ColDrag, Crumb, DirEntry, DlFilter, DlJob, DlStatus, Page, QualityReady,
    ShareResult, SortBy, TransferTab, UlFilter, UlJob, UlStatus, UploadPick, ViewMode,
};

/// 目录缓存新鲜期: 命中后超过该时长, 先展示旧数据再后台静默校正。
const DIR_TTL: Duration = Duration::from_secs(60);
/// 「我的分享」列表新鲜期: 进入页面时命中则不发请求。
const SHARES_TTL: Duration = Duration::from_secs(60);
/// 回收站列表新鲜期。
const TRASH_TTL: Duration = Duration::from_secs(60);
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
    /// 是否把密码保存到系统密钥环以自动登录。
    pub(crate) remember_password: bool,
    /// 手动登录成功后待写入密钥环的密码。
    pub(crate) pending_remember: Option<String>,
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

    // 本地上传
    pub(crate) ul_jobs: BTreeMap<u64, UlJob>,
    pub(crate) selected_ul: HashSet<u64>,
    pub(crate) ul_filter: UlFilter,
    pub(crate) ul_last_clicked: Option<u64>,
    /// 进行中的异步选择 (是否目录, 目标目录, 目标路径展示, 结果通道), 避免阻塞 UI 线程。
    pub(crate) upload_pick: Option<UploadPick>,

    /// 正在准备中的预览任务 (req_id, 文件名); 用于给出加载反馈。
    pub(crate) preview_pending: Option<(u64, String)>,

    /// 已解析的媒体文件清晰度缓存(file_id -> 清晰度+字幕)。
    pub(crate) quality_cache: HashMap<String, QualityReady>,
    /// 正在解析清晰度的文件 id。
    pub(crate) quality_inflight: HashSet<String>,

    // 我的分享
    pub(crate) shares: Vec<Share>,
    pub(crate) shares_next: Option<String>,
    pub(crate) shares_loading: bool,
    /// 最近一次分享列表请求的 id, 用于丢弃乱序的旧响应。
    pub(crate) shares_req: u64,
    /// 分享列表多选(share_id)。
    pub(crate) shares_selected: HashSet<String>,
    /// 最近一次成功加载分享列表的时间, 用于 SWR 新鲜度判定。
    pub(crate) shares_fetched_at: Option<Instant>,
    /// 待创建分享的选中项 (id, name); Some 表示「创建分享」设置框打开。
    pub(crate) share_dialog: Option<Vec<(String, String)>>,
    pub(crate) share_expiration_days: i64,
    pub(crate) share_need_password: bool,
    /// 分享创建成功后的结果框。
    pub(crate) share_result: Option<ShareResult>,
    /// 取消分享确认 (share_id, 标题)。
    pub(crate) share_delete_confirm: Option<Vec<(String, String)>>,

    // 转存分享
    /// 转存弹窗是否打开。
    pub(crate) save_share_open: bool,
    /// 用户输入的分享链接或分享 ID。
    pub(crate) save_share_input: String,
    /// 用户输入的提取码。
    pub(crate) save_share_pass_code: String,
    /// 是否正在解析分享链接。
    pub(crate) save_share_resolving: bool,
    /// 解析成功后的分享 ID。
    pub(crate) save_share_id: Option<String>,
    /// 解析成功后的分享标题。
    pub(crate) save_share_title: Option<String>,
    /// 解析成功后的 pass_code_token(转存时需要)。
    pub(crate) save_share_token: Option<String>,
    /// 解析出的文件列表。
    pub(crate) save_share_files: Vec<File>,
    /// 用户选中的文件 id。
    pub(crate) save_share_selected: HashSet<String>,
    /// 文件名搜索过滤。
    pub(crate) save_share_filter: String,
    /// 分享文件列表分页 token。
    pub(crate) save_share_next: Option<String>,
    /// 是否正在加载更多文件。
    pub(crate) save_share_loading_more: bool,
    /// 是否正在转存。
    pub(crate) save_share_saving: bool,
    /// 解析或转存的错误信息。
    pub(crate) save_share_error: Option<String>,
    /// 转存目标目录选择器是否打开。
    pub(crate) save_share_picker_open: bool,
    /// 转存目标目录选择器的面包屑导航。
    pub(crate) save_share_picker_stack: Vec<Crumb>,
    /// 转存目标目录选择器当前目录的子文件夹。
    pub(crate) save_share_picker_folders: Vec<File>,
    /// 转存目标目录选择器加载状态。
    pub(crate) save_share_picker_loading: bool,
    /// 转存目标目录选择器请求 ID。
    pub(crate) save_share_picker_req: u64,
    /// 用户选择的转存目标目录 (id, name); None 表示默认位置。
    pub(crate) save_share_dest: Option<(String, String)>,
    /// 自动移动失败时的目标目录信息, 用于重试。
    pub(crate) save_share_move_failed: Option<(String, String)>,

    // 回收站
    pub(crate) trash: Vec<File>,
    pub(crate) trash_next: Option<String>,
    pub(crate) trash_loading: bool,
    /// 最近一次回收站列表请求的 id, 用于丢弃乱序的旧响应。
    pub(crate) trash_req: u64,
    pub(crate) trash_selected: HashSet<String>,
    /// 最近一次成功加载回收站的时间, 用于 SWR 新鲜度判定。
    pub(crate) trash_fetched_at: Option<Instant>,
    /// 彻底删除确认 (id, name)。
    pub(crate) trash_delete_confirm: Option<Vec<(String, String)>>,
    /// 清空回收站确认。
    pub(crate) trash_empty_confirm: bool,

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
            remember_password: saved.remember_password,
            pending_remember: None,
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
                    let status = match record.status {
                        DownloadRecordStatus::Done => DlStatus::Done,
                        // 取消的任务不再保留在列表中(旧版本可能写入过取消记录)。
                        DownloadRecordStatus::Cancelled => continue,
                        DownloadRecordStatus::Failed(what) => DlStatus::Failed(what),
                    };
                    next_id += 1;
                    jobs.insert(
                        next_id,
                        DlJob {
                            file_id: record.file_id,
                            record_id: record.timestamp,
                            name: record.name,
                            dir: record.dir,
                            total: record.total,
                            done: record.done,
                            status,
                            speed: 0,
                            last_done: 0,
                            last_at: None,
                            at: (record.at != 0).then_some(record.at),
                        },
                    );
                }
                jobs
            },
            selected_dl: HashSet::new(),
            dl_filter: DlFilter::All,
            ul_jobs: {
                let mut ul = BTreeMap::new();
                let mut next_id = 0u64;
                // 历史按最新在前存储, 倒序分配 id 使 id 随时间递增。
                for record in settings::load_upload_history().into_iter().rev() {
                    let status = match record.status {
                        UploadRecordStatus::Done => UlStatus::Done,
                        UploadRecordStatus::Failed(what) => UlStatus::Failed(what),
                    };
                    next_id += 1;
                    ul.insert(
                        next_id,
                        UlJob {
                            local_path: record.local_path,
                            name: record.name,
                            parent: record.parent,
                            dest_stack: record.dest_stack,
                            total: record.total,
                            done: record.done,
                            status,
                            speed: 0,
                            last_done: 0,
                            last_at: None,
                            record_id: record.timestamp,
                            is_dir: record.is_dir,
                            files_done: 0,
                            files_total: 0,
                            current: String::new(),
                            at: (record.at != 0).then_some(record.at),
                        },
                    );
                }
                ul
            },
            selected_ul: HashSet::new(),
            ul_filter: UlFilter::All,
            ul_last_clicked: None,
            upload_pick: None,
            preview_pending: None,
            quality_cache: HashMap::new(),
            quality_inflight: HashSet::new(),
            shares: Vec::new(),
            shares_next: None,
            shares_loading: false,
            shares_req: 0,
            shares_selected: HashSet::new(),
            shares_fetched_at: None,
            share_dialog: None,
            share_expiration_days: -1,
            share_need_password: false,
            share_result: None,
            share_delete_confirm: None,
            save_share_open: false,
            save_share_input: String::new(),
            save_share_pass_code: String::new(),
            save_share_resolving: false,
            save_share_id: None,
            save_share_title: None,
            save_share_token: None,
            save_share_files: Vec::new(),
            save_share_selected: HashSet::new(),
            save_share_filter: String::new(),
            save_share_next: None,
            save_share_loading_more: false,
            save_share_saving: false,
            save_share_error: None,
            save_share_picker_open: false,
            save_share_picker_stack: vec![Crumb {
                id: None,
                label: "我的云盘".to_string(),
            }],
            save_share_picker_folders: Vec::new(),
            save_share_picker_loading: false,
            save_share_picker_req: 0,
            save_share_dest: None,
            save_share_move_failed: None,
            trash: Vec::new(),
            trash_next: None,
            trash_loading: false,
            trash_req: 0,
            trash_selected: HashSet::new(),
            trash_fetched_at: None,
            trash_delete_confirm: None,
            trash_empty_confirm: false,
            col_size_w: 100.0,
            col_time_w: 160.0,
            col_dragging: None,
            view_mode: ViewMode::List,
            last_clicked_dl: None,
            toast: None,
        };

        // 恢复 req_id 为历史记录中的最大值，避免 ID 冲突
        app.req_id = app
            .jobs
            .keys()
            .chain(app.ul_jobs.keys())
            .max()
            .copied()
            .unwrap_or(0);

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
            Ok(None) => app.auto_login_if_possible(),
            Err(e) => {
                // 会话文件损坏/无法读取: 清掉以免每次启动都报错, 再尝试密钥环自动登录。
                let _ = session::clear_session();
                app.toast = Some((
                    Color32::from_rgb(200, 90, 60),
                    e.to_string(),
                    Instant::now(),
                ));
                app.auto_login_if_possible();
            }
        }
        if !font_loaded && app.toast.is_none() {
            app.toast = Some((
                Color32::from_rgb(200, 160, 60),
                "未找到中文字体，中文可能显示为方块。请安装 wqy-zenhei 或 google-droid-sans-fonts"
                    .into(),
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
                    // 记住密码: 手动登录成功后写入密钥环; 未勾选则清除旧条目。
                    if self.remember_password {
                        if let Some(pw) = self.pending_remember.take() {
                            self.send(Cmd::RememberPassword {
                                username: self.username.clone(),
                                password: pw,
                            });
                        }
                    } else {
                        self.pending_remember = None;
                        self.send(Cmd::ForgetPassword {
                            username: self.username.clone(),
                        });
                    }
                    self.buckets.clear();
                    self.quota = None;
                    self.reset_browse();
                    self.send(Cmd::RefreshQuota);
                    self.send(Cmd::RefreshTasks);
                    self.persist_settings();
                }
                Msg::LoginFailed { what } => {
                    self.auth_checking = false;
                    self.auth_error = Some(what);
                    self.username.clear();
                    self.pending_remember = None;
                }
                Msg::AutoLoginUnavailable => {
                    self.auth_checking = false;
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
                    self.clear_share_state();
                    self.clear_trash_state();
                    // 登录态失效: 若保存过密码则尝试自动重登。
                    self.auto_login_if_possible();
                }
                Msg::LoggedOut => {
                    self.username.clear();
                    self.auth_checking = false;
                    self.auth_error = None;
                    self.remember_password = false;
                    self.pending_remember = None;
                    self.persist_settings();
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
                    self.clear_share_state();
                    self.clear_trash_state();
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
                    // 回收站内容已变, 作废缓存。
                    self.trash_fetched_at = None;
                    self.reload_dir();
                }
                Msg::TrashList {
                    req_id,
                    append,
                    list,
                } => {
                    if req_id != self.trash_req {
                        continue;
                    }
                    self.trash_loading = false;
                    self.trash_fetched_at = Some(Instant::now());
                    self.trash_next = list.next_page_token;
                    if append {
                        let known: HashSet<String> =
                            self.trash.iter().map(|f| f.id.clone()).collect();
                        for f in list.files {
                            if !known.contains(&f.id) {
                                self.trash.push(f);
                            }
                        }
                    } else {
                        self.trash = list.files;
                    }
                }
                Msg::TrashFailed { what } => {
                    self.trash_loading = false;
                    self.toast_err(&what);
                }
                Msg::TrashRestored { ids } => {
                    self.trash.retain(|f| !ids.contains(&f.id));
                    self.trash_selected.retain(|id| !ids.contains(id));
                    self.trash_delete_confirm = None;
                    self.trash_empty_confirm = false;
                    // 还原可能回到被删时的原目录, 作废目录缓存以便下次重新加载。
                    self.dir_cache.clear();
                    self.send(Cmd::RefreshQuota);
                    self.toast_ok(&format!("已还原 {} 项", ids.len()));
                }
                Msg::TrashDeleted { ids } => {
                    self.trash.retain(|f| !ids.contains(&f.id));
                    self.trash_selected.retain(|id| !ids.contains(id));
                    self.trash_delete_confirm = None;
                    self.trash_empty_confirm = false;
                    self.send(Cmd::RefreshQuota);
                    self.toast_ok(&format!("已彻底删除 {} 项", ids.len()));
                }
                Msg::TrashEmptied => {
                    self.trash.clear();
                    self.trash_next = None;
                    self.trash_selected.clear();
                    self.trash_delete_confirm = None;
                    self.trash_empty_confirm = false;
                    self.send(Cmd::RefreshQuota);
                    self.toast_ok("回收站已清空");
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
                Msg::Folders {
                    parent,
                    req_id,
                    files,
                } => {
                    // 路由到对应的目录选择器
                    if self.save_share_picker_open && req_id == self.save_share_picker_req {
                        if parent != self.save_share_picker_parent() {
                            continue;
                        }
                        self.save_share_picker_loading = false;
                        self.save_share_picker_folders = files;
                    } else if req_id == self.offline_picker_req {
                        if parent != self.offline_picker_parent() {
                            continue;
                        }
                        self.offline_picker_loading = false;
                        self.offline_picker_folders = files;
                    }
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
                        let rec_id = Self::chrono_now();
                        j.record_id = rec_id.clone();
                        j.at = Some(crate::format::now_unix());
                        settings::append_download_record(DownloadRecord {
                            file_id: j.file_id.clone(),
                            name: j.name.clone(),
                            dir: j.dir.clone(),
                            total: j.total,
                            done: j.done,
                            status: DownloadRecordStatus::Done,
                            at: j.at.unwrap_or(0),
                            timestamp: rec_id,
                        });
                    }
                }
                Msg::DlCancelled { req_id } => {
                    // 取消后从列表移除且不保留记录; 未完成的 .part 已由下载线程删除。
                    self.jobs.remove(&req_id);
                    self.selected_dl.remove(&req_id);
                    if self.last_clicked_dl == Some(req_id) {
                        self.last_clicked_dl = None;
                    }
                }
                Msg::DlFailed { req_id, what } => {
                    if let Some(j) = self.jobs.get_mut(&req_id) {
                        j.status = DlStatus::Failed(what.clone());
                        // 保存下载记录到磁盘
                        let rec_id = Self::chrono_now();
                        j.record_id = rec_id.clone();
                        j.at = Some(crate::format::now_unix());
                        settings::append_download_record(DownloadRecord {
                            file_id: j.file_id.clone(),
                            name: j.name.clone(),
                            dir: j.dir.clone(),
                            total: j.total,
                            done: j.done,
                            status: DownloadRecordStatus::Failed(what),
                            at: j.at.unwrap_or(0),
                            timestamp: rec_id,
                        });
                    }
                }
                Msg::UlProgress {
                    req_id,
                    total,
                    done,
                } => {
                    if let Some(j) = self.ul_jobs.get_mut(&req_id) {
                        if j.status == UlStatus::Queued {
                            j.status = UlStatus::Running;
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
                Msg::UlFiles {
                    req_id,
                    done,
                    total,
                    current,
                } => {
                    if let Some(j) = self.ul_jobs.get_mut(&req_id) {
                        j.files_done = done;
                        j.files_total = total;
                        j.current = current;
                        if j.status == UlStatus::Queued {
                            j.status = UlStatus::Running;
                        }
                    }
                }
                Msg::UlFinished { req_id } => {
                    let parent = self.ul_jobs.get_mut(&req_id).map(|j| {
                        j.status = UlStatus::Done;
                        if j.total > 0 {
                            j.done = j.total;
                        }
                        // 写入上传历史。
                        let rec_id = Self::chrono_now();
                        j.record_id = rec_id.clone();
                        j.at = Some(crate::format::now_unix());
                        settings::append_upload_record(UploadRecord {
                            local_path: j.local_path.clone(),
                            name: j.name.clone(),
                            parent: j.parent.clone(),
                            dest_stack: j.dest_stack.clone(),
                            total: j.total,
                            done: j.done,
                            status: UploadRecordStatus::Done,
                            is_dir: j.is_dir,
                            at: j.at.unwrap_or(0),
                            timestamp: rec_id,
                        });
                        j.parent.clone()
                    });
                    if let Some(parent) = parent {
                        // 上传完成后目标目录内容已变, 作废缓存并按需刷新。
                        self.dir_cache.remove(&parent);
                        if parent == self.current_parent() {
                            self.reload_dir();
                        }
                    }
                    self.toast_ok("上传完成");
                }
                Msg::UlCancelled { req_id } => {
                    self.ul_jobs.remove(&req_id);
                    self.selected_ul.remove(&req_id);
                    if self.ul_last_clicked == Some(req_id) {
                        self.ul_last_clicked = None;
                    }
                }
                Msg::UlFailed { req_id, what } => {
                    if let Some(j) = self.ul_jobs.get_mut(&req_id) {
                        j.status = UlStatus::Failed(what.clone());
                        // 写入上传历史。
                        let rec_id = Self::chrono_now();
                        j.record_id = rec_id.clone();
                        j.at = Some(crate::format::now_unix());
                        settings::append_upload_record(UploadRecord {
                            local_path: j.local_path.clone(),
                            name: j.name.clone(),
                            parent: j.parent.clone(),
                            dest_stack: j.dest_stack.clone(),
                            total: j.total,
                            done: j.done,
                            status: UploadRecordStatus::Failed(what.clone()),
                            is_dir: j.is_dir,
                            at: j.at.unwrap_or(0),
                            timestamp: rec_id,
                        });
                    }
                    self.toast_err(&format!("上传失败: {what}"));
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
                Msg::ShareCreated {
                    url,
                    pass_code,
                    share_text,
                    label,
                } => {
                    self.share_result = Some(ShareResult {
                        url,
                        pass_code,
                        share_text,
                        label,
                    });
                    // 新分享已产生: 作废缓存, 并在分享页时立即刷新以展示。
                    self.shares_fetched_at = None;
                    if self.page == Page::Shares {
                        self.refresh_shares();
                    }
                }
                Msg::Shares {
                    req_id,
                    append,
                    list,
                } => {
                    if req_id != self.shares_req {
                        continue;
                    }
                    self.shares_loading = false;
                    self.shares_fetched_at = Some(Instant::now());
                    self.shares_next = list.next_page_token;
                    if append {
                        let known: HashSet<String> =
                            self.shares.iter().map(|s| s.share_id.clone()).collect();
                        for s in list.shares {
                            if !known.contains(&s.share_id) {
                                self.shares.push(s);
                            }
                        }
                    } else {
                        self.shares = list.shares;
                    }
                }
                Msg::SharesFailed { what } => {
                    self.shares_loading = false;
                    self.toast_err(&what);
                }
                Msg::SharesDeleted { ids } => {
                    self.shares.retain(|s| !ids.contains(&s.share_id));
                    self.shares_selected.retain(|id| !ids.contains(id));
                    self.share_delete_confirm = None;
                    self.toast_ok(&format!("已取消 {} 个分享", ids.len()));
                }
                Msg::ShareResolved {
                    share_id,
                    title,
                    pass_code_token,
                    files,
                    next_page_token,
                } => {
                    self.save_share_resolving = false;
                    self.save_share_id = Some(share_id);
                    self.save_share_title = Some(title);
                    self.save_share_token = Some(pass_code_token);
                    self.save_share_files = files;
                    self.save_share_next = next_page_token;
                    self.save_share_selected = HashSet::new();
                }
                Msg::ShareFilesLoaded {
                    files,
                    next_page_token,
                } => {
                    self.save_share_loading_more = false;
                    self.save_share_files.extend(files);
                    self.save_share_next = next_page_token;
                }
                Msg::ShareFilesLoadFailed { what } => {
                    self.save_share_loading_more = false;
                    self.save_share_error = Some(what);
                }
                Msg::ShareResolveFailed { what } => {
                    self.save_share_resolving = false;
                    self.save_share_error = Some(what);
                }
                Msg::ShareSaved { auto_move_failed } => {
                    self.save_share_saving = false;
                    self.save_share_open = false;
                    let dest = self.save_share_dest.clone();
                    if auto_move_failed {
                        // 保存失败信息以便重试
                        self.save_share_move_failed = dest.clone();
                    }
                    self.clear_save_share_state();
                    if auto_move_failed {
                        if let Some((_, name)) = dest {
                            self.toast_warn(&format!(
                                "转存成功, 但自动移动到「{name}」失败。文件仍在「转存自分享」中"
                            ));
                        } else {
                            self.toast_warn("转存成功, 但自动移动失败, 请在「转存自分享」中查看");
                        }
                    } else if dest.is_some() {
                        self.toast_ok("转存成功, 文件已移动到目标目录");
                    } else {
                        self.toast_ok("转存成功, 文件已保存到「转存自分享」");
                    }
                }
                Msg::ShareSaveFailed { what } => {
                    self.save_share_saving = false;
                    self.save_share_error = Some(what);
                }
                Msg::ShareMoveRetried => {
                    self.save_share_move_failed = None;
                    self.toast_ok("移动成功");
                    self.reload_dir();
                }
                Msg::ShareMoveRetryFailed { what } => {
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

    /// 生成一条历史记录的唯一标识(纳秒时间戳, 字符串形式)。
    fn chrono_now() -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        format!("{nanos}")
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
            remember_password: self.remember_password,
        });
    }

    /// 若开启了「记住密码」且有账号, 用密钥环里保存的密码尝试自动登录。
    /// 密钥环不可用或没有条目时, worker 会回一条 `AutoLoginUnavailable` 以结束加载态。
    pub(crate) fn auto_login_if_possible(&mut self) {
        let username = self.login_username.trim().to_string();
        if self.remember_password && !username.is_empty() {
            self.auth_checking = true;
            self.auth_error = None;
            self.send(Cmd::AutoLogin { username });
        }
    }

    /// 定期重读 kdeglobals, 让系统换主题后应用即时更新。
    fn poll_system_theme(&mut self) {
        if self.kde_checked.elapsed() < Duration::from_millis(1500) {
            return;
        }
        self.kde_checked = Instant::now();
        let fresh = kde::load();
        let changed = match (&self.kde_colors, &fresh) {
            (Some(a), Some(b)) => {
                a.accent != b.accent || a.dark != b.dark || a.view_bg != b.view_bg
            }
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

    /// 当前目录的层级快照 (id, label), 用于上传目标记录与导航。
    pub(crate) fn current_stack_pairs(&self) -> Vec<(Option<String>, String)> {
        self.stack
            .iter()
            .map(|c| (c.id.clone(), c.label.clone()))
            .collect()
    }

    /// 当前上传筛选下可见的任务 id(按 map 顺序)。
    pub(crate) fn visible_ul_ids(&self) -> Vec<u64> {
        self.ul_jobs
            .iter()
            .filter(|(_, j)| self.ul_filter.matches(j))
            .map(|(id, _)| *id)
            .collect()
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

    /// 转存分享目录选择器当前所在目录。
    pub(crate) fn save_share_picker_parent(&self) -> Option<String> {
        self.save_share_picker_stack
            .last()
            .and_then(|c| c.id.clone())
    }

    /// 打开转存分享「保存到」网盘目录选择器, 从根目录开始。
    pub(crate) fn open_save_share_picker(&mut self) {
        self.save_share_picker_open = true;
        self.save_share_picker_stack = vec![Crumb {
            id: None,
            label: "我的云盘".into(),
        }];
        self.save_share_picker_list();
    }

    /// 请求转存分享选择器当前目录的子文件夹列表。
    pub(crate) fn save_share_picker_list(&mut self) {
        self.save_share_picker_loading = true;
        self.save_share_picker_folders.clear();
        self.save_share_picker_req += 1;
        let req_id = self.save_share_picker_req;
        self.send(Cmd::ListFolders {
            parent: self.save_share_picker_parent(),
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
        let cached = self.dir_cache.get(&parent).map(|e| {
            (
                e.files.clone(),
                e.next_token.clone(),
                e.fetched_at.elapsed(),
            )
        });
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
            self.hidden
                .retain(|id, hp| *hp != listing_parent || present.contains(id));
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
            ClipKind::Copy => self.send(Cmd::CopyTo {
                ids: clip.ids,
                dest,
            }),
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
    pub(crate) fn enqueue_downloads(
        &mut self,
        items: Vec<(String, String)>,
        dir: std::path::PathBuf,
    ) {
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
        let dir =
            if !self.download_dir.is_empty() && std::path::Path::new(&self.download_dir).is_dir() {
                std::path::PathBuf::from(&self.download_dir)
            } else {
                let Some(d) = self.choose_download_dir() else {
                    return;
                };
                d
            };
        self.enqueue_downloads(vec![(id, name)], dir);
    }

    /// 逐个提交上传任务到给定网盘目录(None = 根目录)。
    pub(crate) fn enqueue_upload(
        &mut self,
        paths: Vec<std::path::PathBuf>,
        parent: Option<String>,
        dest_stack: Vec<(Option<String>, String)>,
    ) {
        let mut n = 0usize;
        for path in paths {
            let name = path
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            if name.is_empty() {
                continue;
            }
            let req_id = self.alloc_req_id();
            self.ul_jobs.insert(
                req_id,
                UlJob::queued(path.clone(), name, parent.clone(), dest_stack.clone()),
            );
            self.send(Cmd::StartUpload {
                req_id,
                path,
                parent: parent.clone(),
            });
            n += 1;
        }
        if n > 0 {
            self.toast_ok(&format!("已加入上传队列 ({n} 个文件)"));
        }
    }

    /// 选择本地文件并上传到当前网盘目录(异步弹框, 不阻塞 UI)。
    pub(crate) fn upload_here(&mut self) {
        // 已在选择中时忽略重复点击。
        if self.upload_pick.is_some() {
            return;
        }
        let start = std::env::current_dir().unwrap_or_default();
        let parent = self.current_parent();
        let stack = self.current_stack_pairs();
        self.upload_pick = Some((false, parent, stack, helpers::pick_files_async(&start)));
    }

    /// 选择本地文件夹并递归上传到当前网盘目录(异步弹框, 不阻塞 UI)。
    pub(crate) fn upload_dir_here(&mut self) {
        if self.upload_pick.is_some() {
            return;
        }
        let start = std::env::current_dir().unwrap_or_default();
        let parent = self.current_parent();
        let stack = self.current_stack_pairs();
        self.upload_pick = Some((true, parent, stack, helpers::pick_dir_async(&start)));
    }

    /// 处理拖拽到窗口的本地文件/文件夹(上传到当前网盘目录)。
    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        if dropped.is_empty() {
            return;
        }
        let parent = self.current_parent();
        let stack = self.current_stack_pairs();
        let mut files: Vec<std::path::PathBuf> = Vec::new();
        let mut dirs: Vec<std::path::PathBuf> = Vec::new();
        for f in dropped {
            let Some(p) = f.path else { continue };
            if p.is_dir() {
                dirs.push(p);
            } else if p.is_file() {
                files.push(p);
            }
        }
        if !files.is_empty() {
            self.enqueue_upload(files, parent.clone(), stack.clone());
        }
        for d in dirs {
            self.enqueue_upload_dir(d, parent.clone(), stack.clone());
        }
    }

    /// 拖拽悬停时显示落点提示。
    fn drop_overlay(&self, ctx: &egui::Context) {
        let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());
        if !hovering {
            return;
        }
        let screen = ctx.screen_rect();
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("drop_overlay"),
        ));
        painter.rect_filled(screen, 0.0, Color32::from_black_alpha(90));
        painter.text(
            screen.center(),
            egui::Align2::CENTER_CENTER,
            "松开鼠标: 上传到当前目录",
            egui::FontId::proportional(20.0),
            Color32::WHITE,
        );
    }

    /// 每帧检查异步选择结果; 选好后按当时的目标目录入队。
    fn poll_file_picker(&mut self) {
        let Some((is_dir, parent, stack, rx)) = &self.upload_pick else {
            return;
        };
        let is_dir = *is_dir;
        match rx.try_recv() {
            Ok(paths) => {
                let parent = parent.clone();
                let stack = stack.clone();
                self.upload_pick = None;
                if paths.is_empty() {
                    return;
                }
                if is_dir {
                    for p in paths {
                        self.enqueue_upload_dir(p, parent.clone(), stack.clone());
                    }
                } else {
                    self.enqueue_upload(paths, parent, stack);
                }
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.upload_pick = None;
            }
        }
    }

    /// 提交目录递归上传任务。
    pub(crate) fn enqueue_upload_dir(
        &mut self,
        path: std::path::PathBuf,
        parent: Option<String>,
        dest_stack: Vec<(Option<String>, String)>,
    ) {
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        if name.is_empty() {
            return;
        }
        let req_id = self.alloc_req_id();
        self.ul_jobs.insert(
            req_id,
            UlJob::queued_dir(path.clone(), name, parent.clone(), dest_stack),
        );
        self.send(Cmd::StartUploadDir {
            req_id,
            path,
            parent,
        });
        self.toast_ok("已加入上传队列 (文件夹)");
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
        // 同目录下的同集字幕, 播放时一并挂载(仅视频需要)。
        let subtitles = if helpers::is_video_file(&name) {
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

    // ---------- 我的分享 ----------

    /// 清空分享相关状态(退出登录 / 会话失效时调用)。
    pub(crate) fn clear_share_state(&mut self) {
        self.shares.clear();
        self.shares_next = None;
        self.shares_loading = false;
        self.shares_req = 0;
        self.shares_selected.clear();
        self.shares_fetched_at = None;
        self.share_dialog = None;
        self.share_result = None;
        self.share_delete_confirm = None;
    }

    /// 清空转存分享弹窗状态。
    pub(crate) fn clear_save_share_state(&mut self) {
        self.save_share_input.clear();
        self.save_share_pass_code.clear();
        self.save_share_resolving = false;
        self.save_share_id = None;
        self.save_share_title = None;
        self.save_share_token = None;
        self.save_share_files.clear();
        self.save_share_selected.clear();
        self.save_share_filter.clear();
        self.save_share_next = None;
        self.save_share_loading_more = false;
        self.save_share_saving = false;
        self.save_share_error = None;
        self.save_share_picker_open = false;
        self.save_share_picker_stack = vec![Crumb {
            id: None,
            label: "我的云盘".to_string(),
        }];
        self.save_share_picker_folders.clear();
        self.save_share_picker_loading = false;
        self.save_share_dest = None;
    }

    /// 从分享链接中提取 share_id。支持完整 URL 或直接输入 ID。
    pub(crate) fn extract_share_id(input: &str) -> String {
        let input = input.trim();
        // 尝试从 URL 中提取: https://mypikpak.com/s/VO8BcRb-XXX
        if let Some(idx) = input.rfind("/s/") {
            let id = &input[idx + 3..];
            // 去掉可能的查询参数
            id.split('?').next().unwrap_or(id).to_string()
        } else {
            input.to_string()
        }
    }

    /// 进入「我的分享」页面: 缓存新鲜则零请求, 否则刷新。
    pub(crate) fn enter_shares(&mut self) {
        let fresh = self
            .shares_fetched_at
            .is_some_and(|t| t.elapsed() < SHARES_TTL);
        if !fresh {
            self.refresh_shares();
        }
    }

    /// 请求刷新「我的分享」首页列表(保留旧数据, 仅置加载态)。
    pub(crate) fn refresh_shares(&mut self) {
        self.shares_loading = true;
        self.shares_next = None;
        self.shares_selected.clear();
        self.shares_req += 1;
        let req_id = self.shares_req;
        self.send(Cmd::ListShares {
            token: None,
            append: false,
            req_id,
        });
    }

    /// 加载分享列表下一页。
    pub(crate) fn load_more_shares(&mut self) {
        let Some(token) = self.shares_next.clone() else {
            return;
        };
        self.shares_loading = true;
        self.shares_req += 1;
        let req_id = self.shares_req;
        self.send(Cmd::ListShares {
            token: Some(token),
            append: true,
            req_id,
        });
    }

    /// 打开「创建分享」设置框, targets 为 (id, name)。
    pub(crate) fn open_share_dialog(&mut self, targets: Vec<(String, String)>) {
        if targets.is_empty() {
            return;
        }
        self.share_dialog = Some(targets);
    }

    /// 从当前选中项打开「创建分享」设置框。
    pub(crate) fn share_selection(&mut self) {
        let targets = self.selected_names();
        if targets.is_empty() {
            self.toast_warn("请先选择要分享的文件");
            return;
        }
        self.open_share_dialog(targets);
    }

    /// 右键单项分享: 若该项在多选内则分享整个选中集, 否则仅分享该项。
    pub(crate) fn share_item(&mut self, id: String) {
        let targets = if self.selected.contains(&id) && self.selected.len() > 1 {
            self.selected_names()
        } else {
            self.files
                .iter()
                .filter(|f| f.id == id)
                .map(|f| (f.id.clone(), f.name.clone()))
                .collect()
        };
        self.open_share_dialog(targets);
    }

    // ---------- 回收站 ----------

    /// 清空回收站相关状态(退出登录 / 会话失效时调用)。
    pub(crate) fn clear_trash_state(&mut self) {
        self.trash.clear();
        self.trash_next = None;
        self.trash_loading = false;
        self.trash_req = 0;
        self.trash_selected.clear();
        self.trash_fetched_at = None;
        self.trash_delete_confirm = None;
        self.trash_empty_confirm = false;
    }

    /// 进入回收站页面: 缓存新鲜则零请求, 否则刷新。
    pub(crate) fn enter_trash(&mut self) {
        let fresh = self
            .trash_fetched_at
            .is_some_and(|t| t.elapsed() < TRASH_TTL);
        if !fresh {
            self.refresh_trash();
        }
    }

    /// 请求刷新回收站首页列表(保留旧数据, 仅置加载态)。
    pub(crate) fn refresh_trash(&mut self) {
        self.trash_loading = true;
        self.trash_next = None;
        self.trash_selected.clear();
        self.trash_req += 1;
        let req_id = self.trash_req;
        self.send(Cmd::ListTrash {
            token: None,
            append: false,
            req_id,
        });
    }

    /// 加载回收站下一页。
    pub(crate) fn load_more_trash(&mut self) {
        let Some(token) = self.trash_next.clone() else {
            return;
        };
        self.trash_loading = true;
        self.trash_req += 1;
        let req_id = self.trash_req;
        self.send(Cmd::ListTrash {
            token: Some(token),
            append: true,
            req_id,
        });
    }

    /// 回收站选中项 (id, name)。
    pub(crate) fn trash_selected_names(&self) -> Vec<(String, String)> {
        self.trash
            .iter()
            .filter(|f| self.trash_selected.contains(&f.id))
            .map(|f| (f.id.clone(), f.name.clone()))
            .collect()
    }
}

// ================= 主循环 =================

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain();
        self.poll_file_picker();
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
        } else if self.upload_pick.is_some() {
            // 文件选择进行中, 加快轮询以尽快取回结果。
            ctx.request_repaint_after(Duration::from_millis(100));
        } else if self.has_active_downloads()
            || self.dir_loading
            || !self.dir_inflight.is_empty()
            || self.shares_loading
            || self.trash_loading
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

        self.handle_dropped_files(ctx);
        self.app_shell(ctx, &th);
        self.dialogs(ctx, &th);
        self.draw_toast(ctx);
        self.drop_overlay(ctx);
    }
}
