mod dialogs;
mod files_page;
mod global;
mod helpers;
mod login;
mod preview;
mod search;
mod settings_page;
mod shares;
mod sidebar;
mod tasks_page;
mod thumbs;
mod transfers_model;
mod transfers_page;
mod trash;
pub(crate) mod types;

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32};
use kichi_core::session;
use kichi_core::types::{task_id, File, FileList, Quota, Task};

use crate::filetypes;
use crate::msg::{Cmd, Msg};
use crate::settings::{
    self, DownloadChildRecord, DownloadRecord, DownloadRecordStatus, UploadRecord,
    UploadRecordStatus,
};
use crate::theme::{self, Theme};
use crate::worker;

use self::global::Global;
use self::helpers::install_fonts;
use self::preview::PreviewPage;
use self::search::SearchPage;
use self::shares::SharesPage;
use self::thumbs::ThumbsPage;
use self::transfers_model::{
    aggregate_children, compute_dir_counts, dl_record_status, sample_speed,
};
use self::trash::TrashPage;
use self::types::{
    ClipKind, Clipboard, ColDrag, Crumb, DirEntry, DlFilter, DlJob, DlNode, DlRow, DlStatus,
    OfflineTab, Page, SortBy, TransferTab, UlFilter, UlJob, UlStatus, UploadPick, ViewMode,
};

/// 非媒体预览的确认阈值: 预览需先整份下载到本地缓存, 超过则先弹确认。
const PREVIEW_CONFIRM_BYTES: i64 = 64 * 1024 * 1024;

/// 目录缓存新鲜期: 命中后超过该时长, 先展示旧数据再后台静默校正。
const DIR_TTL: Duration = Duration::from_secs(60);
/// 目录缓存上限, 超出按 LRU 淘汰(不淘汰当前目录)。
const DIR_CACHE_CAP: usize = 64;

pub struct App {
    rx: Receiver<Msg>,
    /// 各域共用的全局句柄(命令通道 / 提示条 / 系统配色)。
    pub(crate) global: Global,

    // 认证
    pub(crate) auth_checking: bool,
    pub(crate) username: String,
    pub(crate) login_username: String,
    pub(crate) login_password: String,
    pub(crate) auth_error: Option<String>,
    /// 登录被要求人机验证时, 服务端给出的验证页链接(可在浏览器打开)。
    pub(crate) auth_captcha_url: Option<String>,
    /// 是否把密码保存到系统密钥环以自动登录。
    pub(crate) remember_password: bool,
    /// 手动登录成功后待写入密钥环的密码。
    pub(crate) pending_remember: Option<String>,

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

    // 全局搜索
    pub(crate) search: SearchPage,

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
    /// 离线任务每个 phase 的下一页游标; 缺失/None 表示没有更多。
    pub(crate) buckets_next: BTreeMap<String, Option<String>>,
    /// 正在「加载更多」的 phase 集合, 用于按钮禁用/转圈。
    pub(crate) tasks_loading_more: BTreeSet<String>,
    pub(crate) tasks_loading: bool,
    /// 用户点击「刷新任务」后的进行中状态, 用于给出可见反馈。
    pub(crate) tasks_refreshing: bool,
    /// 离线任务页的阶段页签; None 表示尚未按已有数据自动选定。
    pub(crate) tasks_tab: Option<OfflineTab>,
    /// 离线任务选中项(task id)。
    pub(crate) tasks_selected: HashSet<String>,
    /// 离线任务 Shift 范围选择的锚点(task id)。
    pub(crate) tasks_anchor: Option<String>,

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
    /// 上次本地选择框用过的目录(上传文件/文件夹共用, 持久化), 见 [`helpers::picker_start_dir`]。
    pub(crate) last_dir: String,
    // 传输并发/重试(设置页可调, 改动即推送 worker)。
    pub(crate) dl_concurrency: usize,
    pub(crate) ul_concurrency: usize,
    pub(crate) part_concurrency: usize,
    pub(crate) max_attempts: usize,
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

    /// 预览域: 大文件确认 / 下载进度 / 清晰度 / 「系统打开」探针。
    pub(crate) preview: PreviewPage,

    /// 缩略图域: 纹理缓存 + 在途 / 失败登记。
    pub(crate) thumbs: ThumbsPage,

    /// 磁盘缓存(预览 + 缩略图)占用; None = 未查询或查询中。
    pub(crate) cache_usage: Option<types::CacheUsage>,
    /// 是否已发出占用查询(避免每帧重复发)。
    pub(crate) cache_usage_pending: bool,
    /// 是否正在执行手动清理。
    pub(crate) cache_sweeping: bool,
    /// 网格视图卡片大小(80-160)。
    pub(crate) grid_card_size: f32,

    // 我的分享 + 转存分享
    pub(crate) shares: SharesPage,

    // 回收站
    pub(crate) trash: TrashPage,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let font_loaded = install_fonts(&cc.egui_ctx);
        let worker::Worker { tx, rx } = worker::spawn();
        let saved = settings::load();

        let mut app = App {
            rx,
            global: Global::new(tx),
            auth_checking: false,
            username: String::new(),
            login_username: saved.username.clone(),
            login_password: String::new(),
            auth_error: None,
            auth_captcha_url: None,
            remember_password: saved.remember_password,
            pending_remember: None,
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
            search: SearchPage::default(),
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
            buckets_next: BTreeMap::new(),
            tasks_loading_more: BTreeSet::new(),
            tasks_loading: false,
            tasks_refreshing: false,
            tasks_tab: None,
            tasks_selected: HashSet::new(),
            tasks_anchor: None,
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
            last_dir: saved.last_dir.clone(),
            dl_concurrency: saved.dl_concurrency,
            ul_concurrency: saved.ul_concurrency,
            part_concurrency: saved.part_concurrency,
            max_attempts: saved.max_attempts,
            jobs: {
                let mut jobs = BTreeMap::new();
                let history = settings::load_download_history();
                let mut next_id = 0u64;
                // 历史文件按最新在前存储; 倒序分配 id, 使 id 随时间递增(旧的 id 小)。
                for record in history.into_iter().rev() {
                    if record.is_folder {
                        let folder_status = match record.status {
                            DownloadRecordStatus::Done => DlStatus::Done,
                            DownloadRecordStatus::Cancelled => continue,
                            DownloadRecordStatus::Failed(what) => DlStatus::Failed(what),
                        };
                        let cloud_id = record.file_id.clone();
                        next_id += 1;
                        let folder_id = next_id;
                        let mut nodes: Vec<DlNode> = Vec::with_capacity(record.children.len());
                        for ch in &record.children {
                            if ch.is_dir {
                                nodes.push(DlNode {
                                    is_dir: true,
                                    name: ch.name.clone(),
                                    depth: ch.depth,
                                    rid: None,
                                    expanded: true,
                                    files_done: 0,
                                    files_total: 0,
                                    done: false,
                                });
                                continue;
                            }
                            let cstatus = match &ch.status {
                                DownloadRecordStatus::Done => DlStatus::Done,
                                DownloadRecordStatus::Cancelled => {
                                    DlStatus::Failed("已取消".into())
                                }
                                DownloadRecordStatus::Failed(w) => DlStatus::Failed(w.clone()),
                            };
                            let done = cstatus == DlStatus::Done;
                            next_id += 1;
                            jobs.insert(
                                next_id,
                                DlJob {
                                    file_id: ch.file_id.clone(),
                                    record_id: String::new(),
                                    name: ch.name.clone(),
                                    dir: ch.dir.clone(),
                                    total: ch.total,
                                    done: ch.done,
                                    status: cstatus,
                                    speed: 0,
                                    last_done: 0,
                                    last_at: None,
                                    at: (ch.at != 0).then_some(ch.at),
                                    folder_id: None,
                                    parent: Some(folder_id),
                                    nodes: Vec::new(),
                                    expanded: false,
                                    files_done: 0,
                                    files_total: 0,
                                },
                            );
                            nodes.push(DlNode {
                                is_dir: false,
                                name: ch.name.clone(),
                                depth: ch.depth,
                                rid: Some(next_id),
                                expanded: false,
                                files_done: 0,
                                files_total: 0,
                                done,
                            });
                        }
                        compute_dir_counts(&mut nodes);
                        let files_total = nodes.iter().filter(|n| !n.is_dir).count() as u32;
                        let files_done =
                            nodes.iter().filter(|n| !n.is_dir && n.done).count() as u32;
                        jobs.insert(
                            folder_id,
                            DlJob {
                                file_id: cloud_id.clone(),
                                record_id: record.timestamp,
                                name: record.name,
                                dir: record.dir,
                                total: record.total,
                                done: record.done,
                                status: folder_status,
                                speed: 0,
                                last_done: 0,
                                last_at: None,
                                at: (record.at != 0).then_some(record.at),
                                folder_id: Some(cloud_id),
                                parent: None,
                                nodes,
                                expanded: false,
                                files_done,
                                files_total,
                            },
                        );
                        continue;
                    }
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
                            folder_id: None,
                            parent: None,
                            nodes: Vec::new(),
                            expanded: false,
                            files_done: 0,
                            files_total: 0,
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
            preview: PreviewPage::default(),
            thumbs: ThumbsPage::default(),
            cache_usage: None,
            cache_usage_pending: false,
            cache_sweeping: false,
            grid_card_size: 104.0,
            shares: SharesPage::default(),
            trash: TrashPage::default(),
            col_size_w: 100.0,
            col_time_w: 160.0,
            col_dragging: None,
            view_mode: ViewMode::List,
            last_clicked_dl: None,
        };

        app.restore_req_id();
        app.start_session();

        if !font_loaded && app.global.toast.is_none() {
            app.global.toast = Some((
                Color32::from_rgb(200, 160, 60),
                "未找到中文字体，中文可能显示为方块。请安装 wqy-zenhei 或 google-droid-sans-fonts"
                    .into(),
                Instant::now(),
            ));
        }
        app
    }

    /// 恢复 req_id 为历史记录中的最大值, 避免与恢复出来的历史任务 ID 冲突。
    fn restore_req_id(&mut self) {
        self.req_id = self
            .jobs
            .keys()
            .chain(self.ul_jobs.keys())
            .max()
            .copied()
            .unwrap_or(0);
    }

    /// 启动时恢复会话: 有存档则续期; 无存档或存档损坏时回退到密钥环自动登录。
    fn start_session(&mut self) {
        match session::load_session() {
            Ok(Some(s)) => {
                self.auth_checking = true;
                self.send(Cmd::Resume {
                    device_id: s.device_id,
                    access_token: s.access_token,
                    refresh_token: s.refresh_token,
                    user_id: s.user_id,
                    username: s.username,
                });
            }
            Ok(None) => self.auto_login_if_possible(),
            Err(e) => {
                // 会话文件损坏/无法读取: 清掉以免每次启动都报错, 再尝试密钥环自动登录。
                let _ = session::clear_session();
                self.global.toast = Some((
                    Color32::from_rgb(200, 90, 60),
                    e.to_string(),
                    Instant::now(),
                ));
                self.auto_login_if_possible();
            }
        }
    }

    pub(crate) fn theme(&self) -> Theme {
        self.global.theme()
    }

    pub(crate) fn send(&self, cmd: Cmd) {
        self.global.send(cmd);
    }

    pub(crate) fn drain(&mut self, ctx: &egui::Context) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::LoginOk { username } => {
                    self.username = username;
                    self.auth_checking = false;
                    self.auth_error = None;
                    self.auth_captcha_url = None;
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
                    self.buckets_next.clear();
                    self.tasks_loading_more.clear();
                    self.tasks_selected.clear();
                    self.tasks_anchor = None;
                    self.quota = None;
                    self.reset_browse();
                    self.send(Cmd::RefreshQuota);
                    self.send(Cmd::RefreshTasks);
                    self.persist_settings();
                }
                Msg::LoginFailed { what, verify_url } => {
                    self.auth_checking = false;
                    self.auth_error = Some(what);
                    self.auth_captcha_url = verify_url;
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
                    self.auth_captcha_url = None;
                    self.jobs.clear();
                    self.selected_dl.clear();
                    self.last_clicked_dl = None;
                    self.clipboard = None;
                    self.preview.clear();
                    self.dir_cache.clear();
                    self.dir_inflight.clear();
                    self.shares.clear();
                    self.trash.clear();
                    // 登录态失效: 若保存过密码则尝试自动重登。
                    self.auto_login_if_possible();
                }
                Msg::LoggedOut => {
                    self.username.clear();
                    self.auth_checking = false;
                    self.auth_error = None;
                    self.auth_captcha_url = None;
                    self.remember_password = false;
                    self.pending_remember = None;
                    self.persist_settings();
                    self.quota = None;
                    self.buckets.clear();
                    self.buckets_next.clear();
                    self.tasks_loading_more.clear();
                    self.tasks_selected.clear();
                    self.tasks_anchor = None;
                    self.files.clear();
                    self.selected.clear();
                    self.dir_cache.clear();
                    self.dir_inflight.clear();
                    self.hidden.clear();
                    self.jobs.clear();
                    self.selected_dl.clear();
                    self.last_clicked_dl = None;
                    self.clipboard = None;
                    self.preview.clear();
                    self.reset_stack();
                    self.shares.clear();
                    self.trash.clear();
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
                Msg::SearchResults {
                    req_id,
                    append,
                    list,
                } => {
                    if self.search.on_results(req_id, append, list) {
                        // 新一次搜索 = 重新开始, 与换目录同理。
                        self.thumbs.reset();
                    }
                }
                Msg::SearchFailed { what } => {
                    self.search.on_failed(&mut self.global, what);
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
                    self.trash.invalidate_cache();
                    self.reload_dir();
                }
                Msg::TrashList {
                    req_id,
                    append,
                    list,
                } => self.trash.on_list(req_id, append, list),
                Msg::TrashFailed { what } => self.trash.on_failed(&mut self.global, what),
                Msg::TrashRestored { ids } => {
                    self.trash.on_restored(&mut self.global, &ids);
                    // 还原可能回到被删时的原目录, 作废目录缓存以便下次重新加载。
                    self.dir_cache.clear();
                }
                Msg::TrashDeleted { ids } => self.trash.on_deleted(&mut self.global, &ids),
                Msg::TrashEmptied => self.trash.on_emptied(&mut self.global),
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
                    if self.shares.handles_picker(req_id) {
                        self.shares.on_folders(parent, files);
                    } else if req_id == self.offline_picker_req {
                        if parent != self.offline_picker_parent() {
                            continue;
                        }
                        self.offline_picker_loading = false;
                        self.offline_picker_folders = files;
                    }
                }
                Msg::Quota(quota) => self.quota = quota,
                Msg::TasksAll {
                    buckets,
                    next_tokens,
                } => {
                    // 合并而非整体替换: 某个分桶刷新失败时保留其旧数据。
                    for (phase, tasks) in buckets {
                        if tasks.is_empty() {
                            self.buckets.remove(&phase);
                        } else {
                            self.buckets.insert(phase.clone(), tasks);
                        }
                        if let Some(tok) = next_tokens.get(&phase) {
                            self.buckets_next.insert(phase, tok.clone());
                        }
                    }
                    self.tasks_loading = false;
                    self.tasks_refreshing = false;
                }
                Msg::TasksMore {
                    phase,
                    tasks,
                    next_page_token,
                } => {
                    self.tasks_loading_more.remove(&phase);
                    // 按 task_id 去重追加, 避免与刷新回包交叠时重复。
                    let known: HashSet<String> = self
                        .buckets
                        .get(&phase)
                        .map(|v| v.iter().filter_map(task_id).collect())
                        .unwrap_or_default();
                    let entry = self.buckets.entry(phase.clone()).or_default();
                    for t in tasks {
                        match task_id(&t) {
                            Some(id) if known.contains(&id) => {}
                            _ => entry.push(t),
                        }
                    }
                    self.buckets_next.insert(phase, next_page_token);
                }
                Msg::TasksMoreFailed { phase, what } => {
                    self.tasks_loading_more.remove(&phase);
                    self.toast_err(&what);
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
                        sample_speed(&mut j.speed, &mut j.last_done, &mut j.last_at, done);
                        if done > j.done {
                            j.done = done;
                        }
                    }
                    // 子文件进度回传后刷新所属目录任务的聚合进度。
                    if let Some(p) = self.jobs.get(&req_id).and_then(|j| j.parent) {
                        self.recompute_folder(p);
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
                    match self.jobs.get(&req_id).and_then(|j| j.parent) {
                        Some(p) => self.recompute_folder(p),
                        None => self.persist_download_job(req_id, DownloadRecordStatus::Done),
                    }
                }
                Msg::DlCancelled { req_id } => {
                    // 取消后从列表移除且不保留记录; 未完成的 .part 已由下载线程删除。
                    let parent = self.jobs.get(&req_id).and_then(|j| j.parent);
                    self.jobs.remove(&req_id);
                    self.selected_dl.remove(&req_id);
                    if self.last_clicked_dl == Some(req_id) {
                        self.last_clicked_dl = None;
                    }
                    if let Some(p) = parent {
                        if let Some(pj) = self.jobs.get_mut(&p) {
                            pj.nodes.retain(|n| n.rid != Some(req_id));
                        }
                        self.recompute_folder(p);
                    }
                }
                Msg::DlFailed { req_id, what } => {
                    if let Some(j) = self.jobs.get_mut(&req_id) {
                        j.status = DlStatus::Failed(what.clone());
                    }
                    match self.jobs.get(&req_id).and_then(|j| j.parent) {
                        Some(p) => self.recompute_folder(p),
                        None => {
                            self.persist_download_job(req_id, DownloadRecordStatus::Failed(what))
                        }
                    }
                }
                Msg::FolderScanned {
                    req_id,
                    items,
                    total_bytes,
                } => {
                    if !self
                        .jobs
                        .get(&req_id)
                        .map(|j| j.is_folder())
                        .unwrap_or(false)
                    {
                        continue;
                    }
                    // 没有文件(空目录或仅空子目录): 直接完成。
                    if items.iter().all(|it| it.is_dir) {
                        let nodes: Vec<DlNode> = items
                            .iter()
                            .map(|it| DlNode {
                                is_dir: true,
                                name: it.name.clone(),
                                depth: it.depth,
                                rid: None,
                                expanded: true,
                                files_done: 0,
                                files_total: 0,
                                done: false,
                            })
                            .collect();
                        if let Some(j) = self.jobs.get_mut(&req_id) {
                            j.status = DlStatus::Done;
                            j.total = 0;
                            j.done = 0;
                            j.nodes = nodes;
                            j.files_done = 0;
                            j.files_total = 0;
                        }
                        self.persist_download_job(req_id, DownloadRecordStatus::Done);
                        self.toast_ok("空目录已创建");
                    } else {
                        let mut nodes: Vec<DlNode> = Vec::with_capacity(items.len());
                        let mut files_total = 0u32;
                        for it in items {
                            if it.is_dir {
                                nodes.push(DlNode {
                                    is_dir: true,
                                    name: it.name,
                                    depth: it.depth,
                                    rid: None,
                                    expanded: true,
                                    files_done: 0,
                                    files_total: 0,
                                    done: false,
                                });
                            } else {
                                let cid = self.enqueue_download_item(
                                    it.file_id,
                                    it.name.clone(),
                                    it.dir,
                                    Some(req_id),
                                );
                                nodes.push(DlNode {
                                    is_dir: false,
                                    name: it.name,
                                    depth: it.depth,
                                    rid: Some(cid),
                                    expanded: false,
                                    files_done: 0,
                                    files_total: 0,
                                    done: false,
                                });
                                files_total += 1;
                            }
                        }
                        compute_dir_counts(&mut nodes);
                        if let Some(j) = self.jobs.get_mut(&req_id) {
                            j.nodes = nodes;
                            j.total = total_bytes;
                            j.done = 0;
                            j.files_done = 0;
                            j.files_total = files_total;
                            j.status = DlStatus::Running;
                        }
                        self.toast_ok(&format!("已加入下载队列 ({files_total} 个文件)"));
                    }
                }
                Msg::FolderScanFailed { req_id, what } => {
                    if let Some(j) = self.jobs.get_mut(&req_id) {
                        j.status = DlStatus::Failed(what.clone());
                    }
                    self.persist_download_job(req_id, DownloadRecordStatus::Failed(what.clone()));
                    self.toast_err(&what);
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
                        sample_speed(&mut j.speed, &mut j.last_done, &mut j.last_at, done);
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
                    // 上传占用空间, 显式刷新配额(自动轮询已降频)。
                    self.send(Cmd::RefreshQuota);
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
                } => self
                    .preview
                    .on_stream(&mut self.global, req_id, name, url, headers, subs),
                Msg::PreviewProgress {
                    req_id,
                    total,
                    done,
                } => self.preview.on_progress(req_id, total, done),
                Msg::PreviewReady { req_id, name, path } => {
                    self.preview.on_ready(req_id, name, path)
                }
                Msg::PreviewQualities {
                    file_id,
                    qualities,
                    subs,
                } => self.preview.on_qualities(file_id, qualities, subs),
                Msg::QualitiesFailed { file_id, what } => {
                    self.preview
                        .on_qualities_failed(&mut self.global, file_id, what)
                }
                Msg::PreviewFailed { req_id, what } => {
                    self.preview.on_failed(&mut self.global, req_id, what)
                }
                Msg::ShareCreated {
                    url,
                    pass_code,
                    share_text,
                    label,
                } => {
                    self.shares.on_created(url, pass_code, share_text, label);
                    // 新分享已产生: 在分享页时立即刷新以展示。
                    if self.page == Page::Shares {
                        self.shares.refresh(&mut self.global);
                    }
                }
                Msg::Shares {
                    req_id,
                    append,
                    list,
                } => self.shares.on_list(req_id, append, list),
                Msg::SharesFailed { what } => self.shares.on_list_failed(&mut self.global, what),
                Msg::SharesDeleted { ids } => self.shares.on_deleted(&mut self.global, &ids),
                Msg::ShareResolved {
                    share_id,
                    title,
                    pass_code_token,
                    files,
                    next_page_token,
                } => self.shares.on_resolved(
                    share_id,
                    title,
                    pass_code_token,
                    files,
                    next_page_token,
                ),
                Msg::ShareFilesLoaded {
                    files,
                    next_page_token,
                } => self.shares.on_files_loaded(files, next_page_token),
                Msg::ShareFilesLoadFailed { what } => self.shares.on_files_failed(what),
                Msg::ShareResolveFailed { what } => self.shares.on_resolve_failed(what),
                Msg::ShareSaved { auto_move_failed } => {
                    self.shares.on_saved(&mut self.global, auto_move_failed)
                }
                Msg::ShareSaveFailed { what } => self.shares.on_save_failed(what),
                Msg::ShareMoveRetried => {
                    self.shares.on_move_retried(&mut self.global);
                    self.reload_dir();
                }
                Msg::ShareMoveRetryFailed { what } => {
                    self.shares.on_move_retry_failed(&mut self.global, what)
                }
                Msg::ThumbnailReady {
                    file_id,
                    width,
                    height,
                    pixels,
                } => self.thumbs.on_ready(ctx, file_id, width, height, pixels),
                Msg::ThumbnailFailed { file_id } => self.thumbs.on_failed(file_id),
                Msg::CacheUsage {
                    bytes,
                    entries,
                    freed,
                } => {
                    self.cache_usage = Some(types::CacheUsage { bytes, entries });
                    self.cache_usage_pending = false;
                    self.cache_sweeping = false;
                    if freed > 0 {
                        self.toast_ok(&format!(
                            "已清理缓存, 释放 {}",
                            crate::format::fmt_bytes(freed as i64)
                        ));
                    }
                }
            }
        }
    }

    pub(crate) fn toast_ok(&mut self, msg: &str) {
        self.global.toast_ok(msg);
    }
    pub(crate) fn toast_warn(&mut self, msg: &str) {
        self.global.toast_warn(msg);
    }
    pub(crate) fn toast_err(&mut self, msg: &str) {
        self.global.toast_err(msg);
    }

    /// 当前生效的离线任务页签: 未手动选择时按已有数据自动挑一个(优先「下载中」)。
    pub(crate) fn active_tasks_tab(&mut self) -> OfflineTab {
        if let Some(t) = self.tasks_tab {
            return t;
        }
        let pick = [
            OfflineTab::Running,
            OfflineTab::Pending,
            OfflineTab::Error,
            OfflineTab::Complete,
        ]
        .into_iter()
        .find(|t| self.buckets.get(t.phase()).is_some_and(|v| !v.is_empty()))
        .unwrap_or(OfflineTab::Pending);
        // 一旦某个阶段已有数据就固定下来, 避免页签自行跳变。
        if self.buckets.values().any(|v| !v.is_empty()) {
            self.tasks_tab = Some(pick);
        }
        pick
    }

    /// 加载某个 phase 的下一页离线任务。
    pub(crate) fn load_more_tasks(&mut self, phase: &str) {
        if self.tasks_loading_more.contains(phase) {
            return;
        }
        if self
            .buckets_next
            .get(phase)
            .and_then(|t| t.as_ref())
            .is_none()
        {
            return;
        }
        self.tasks_loading_more.insert(phase.to_string());
        self.send(Cmd::LoadMoreTasks {
            phase: phase.to_string(),
        });
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
            last_dir: self.last_dir.clone(),
            remember_password: self.remember_password,
            dl_concurrency: self.dl_concurrency,
            ul_concurrency: self.ul_concurrency,
            part_concurrency: self.part_concurrency,
            max_attempts: self.max_attempts,
        });
    }

    /// 若开启了「记住密码」且有账号, 用密钥环里保存的密码尝试自动登录。
    /// 密钥环不可用或没有条目时, worker 会回一条 `AutoLoginUnavailable` 以结束加载态。
    pub(crate) fn auto_login_if_possible(&mut self) {
        let username = self.login_username.trim().to_string();
        if self.remember_password && !username.is_empty() {
            self.auth_checking = true;
            self.auth_error = None;
            self.auth_captcha_url = None;
            self.send(Cmd::AutoLogin { username });
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

    /// 触发全局搜索。
    pub(crate) fn trigger_search(&mut self) {
        let keyword = self.filter.trim().to_string();
        tracing::info!("触发搜索: keyword='{}'", keyword);
        if keyword.is_empty() {
            self.exit_search();
            return;
        }
        self.search.start(&mut self.global, keyword);
    }

    /// 退出搜索模式。
    pub(crate) fn exit_search(&mut self) {
        self.search.exit();
        self.filter.clear();
    }

    /// 加载更多搜索结果。
    pub(crate) fn load_more_search_results(&mut self) {
        self.search.load_more(&mut self.global);
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
                // 换目录 / 刷新 = 重新开始: 清掉在途与失败的缩略图登记,
                // 让重新可见的文件有机会再请求一次(worker 侧也已作废旧任务)。
                self.thumbs.reset();
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
        self.exit_search();
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
        // 搜索模式下使用搜索结果
        let source: &[File] = if self.search.is_active() {
            self.search.results()
        } else {
            &self.files
        };

        let kw = if self.search.is_active() {
            String::new() // 搜索结果已经过滤过了
        } else {
            self.filter.trim().to_lowercase()
        };
        let cur = self.current_parent();
        let mut folders: Vec<&File> = Vec::new();
        let mut plain: Vec<&File> = Vec::new();
        for f in source {
            if !self.search.is_active() && self.hidden.get(&f.id).is_some_and(|hp| *hp == cur) {
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

    /// 选中项里的文件夹(id, name), 用于整目录下载。
    pub(crate) fn selected_folders(&self) -> Vec<(String, String)> {
        self.files
            .iter()
            .filter(|f| self.selected.contains(&f.id) && f.is_folder())
            .map(|f| (f.id.clone(), f.name.clone()))
            .collect()
    }

    /// 当前筛选下可见的顶层任务 id(不含目录的子文件), 按最新在前排序。
    pub(crate) fn visible_dl_ids(&self) -> Vec<u64> {
        let mut ids: Vec<u64> = self
            .jobs
            .iter()
            .filter(|(_, j)| j.parent.is_none() && self.dl_filter.matches(j))
            .map(|(id, _)| *id)
            .collect();
        ids.sort_unstable_by(|a, b| b.cmp(a));
        ids
    }

    /// 传输任务页当前要渲染的行(顶层任务 + 已展开目录的子文件)。
    pub(crate) fn dl_rows(&self) -> Vec<DlRow> {
        let mut rows = Vec::new();
        for id in self.visible_dl_ids() {
            let Some(j) = self.jobs.get(&id) else {
                continue;
            };
            if !j.is_folder() || !j.expanded {
                rows.push(DlRow::Job(id));
                continue;
            }
            // 先序遍历, 跳过被收起子目录的子孙。
            let mut visible: Vec<usize> = Vec::new();
            let mut skip_below: Option<u32> = None;
            for (i, n) in j.nodes.iter().enumerate() {
                if let Some(d) = skip_below {
                    if n.depth > d {
                        continue;
                    }
                    skip_below = None;
                }
                visible.push(i);
                if n.is_dir && !n.expanded {
                    skip_below = Some(n.depth);
                }
            }
            rows.push(DlRow::Tree(id, visible));
        }
        rows
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

    /// 是否有进行中的上传任务(排队或上传中), 用于加快进度轮询。
    pub(crate) fn has_active_uploads(&self) -> bool {
        self.ul_jobs
            .values()
            .any(|j| j.status == UlStatus::Queued || j.status == UlStatus::Running)
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

    /// 提交单个下载任务并登记任务行, 返回 req_id。parent 为所属目录任务的 req_id。
    pub(crate) fn enqueue_download_item(
        &mut self,
        file_id: String,
        name: String,
        dir: std::path::PathBuf,
        parent: Option<u64>,
    ) -> u64 {
        let req_id = self.alloc_req_id();
        let job = match parent {
            Some(p) => DlJob::child(file_id.clone(), name.clone(), dir.clone(), p),
            None => DlJob::queued(file_id.clone(), name.clone(), dir.clone()),
        };
        self.jobs.insert(req_id, job);
        self.send(Cmd::StartDownload {
            req_id,
            file_id,
            name,
            dest_dir: dir,
        });
        req_id
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
            self.enqueue_download_item(id.clone(), name.clone(), dir.clone(), None);
        }
        self.toast_ok(&format!("已加入下载队列 ({} 个文件)", items.len()));
    }

    /// 提交整目录下载: 后台先扫描目录树, 回 `Msg::FolderScanned` 后逐个入队。
    pub(crate) fn enqueue_download_folder(
        &mut self,
        folder_id: String,
        name: String,
        dir: std::path::PathBuf,
    ) -> u64 {
        let req_id = self.alloc_req_id();
        self.jobs.insert(
            req_id,
            DlJob::folder(folder_id.clone(), name.clone(), dir.clone()),
        );
        self.send(Cmd::StartDownloadFolder {
            req_id,
            folder_id,
            name,
            dest_dir: dir,
        });
        req_id
    }

    /// 递归下载单个云端目录。若已有默认下载目录则直接下载, 否则弹目录选择框。
    pub(crate) fn download_single_folder(&mut self, id: String, name: String) {
        let dir =
            if !self.download_dir.is_empty() && std::path::Path::new(&self.download_dir).is_dir() {
                std::path::PathBuf::from(&self.download_dir)
            } else {
                let Some(d) = self.choose_download_dir() else {
                    return;
                };
                d
            };
        self.enqueue_download_folder(id, name, dir);
        self.toast_ok("正在扫描目录…");
    }

    /// 依据子文件状态重算目录任务的聚合进度、各子目录计数与状态, 并在首次进入终态时写历史。
    pub(crate) fn recompute_folder(&mut self, folder: u64) {
        // 先把节点列表移出, 便于同时读取各子任务的进度而不产生借用冲突。
        let Some(mut nodes) = self
            .jobs
            .get_mut(&folder)
            .filter(|j| j.is_folder())
            .map(|j| std::mem::take(&mut j.nodes))
        else {
            return;
        };
        let kids: Vec<&DlJob> = nodes
            .iter()
            .filter_map(|n| n.rid)
            .filter_map(|rid| self.jobs.get(&rid))
            .collect();
        let (total, done, speed, status) = aggregate_children(kids.into_iter());
        // 同步文件节点的完成标记, 再自底向上累加每个子目录的计数。
        for n in nodes.iter_mut() {
            if let Some(rid) = n.rid {
                n.done = self
                    .jobs
                    .get(&rid)
                    .map(|j| j.status == DlStatus::Done)
                    .unwrap_or(false);
            }
        }
        compute_dir_counts(&mut nodes);
        let files_total = nodes.iter().filter(|n| !n.is_dir).count() as u32;
        let files_done = nodes.iter().filter(|n| !n.is_dir && n.done).count() as u32;
        let terminal = matches!(status, DlStatus::Done | DlStatus::Failed(_));
        let mut write_record = false;
        if let Some(f) = self.jobs.get_mut(&folder) {
            // 扫描时已知合计大小, 优先保留; 未知(0)时用子文件汇总兜底。
            if f.total == 0 {
                f.total = total;
            }
            f.done = done;
            f.speed = speed;
            f.status = status.clone();
            f.files_done = files_done;
            f.files_total = files_total;
            f.nodes = nodes;
            write_record = terminal && f.record_id.is_empty();
        }
        if write_record {
            self.persist_download_job(folder, dl_record_status(&status));
        }
    }

    /// 写一条下载历史记录, 并回填任务行的 record_id / at。
    /// 目录任务额外内联其子文件快照。
    fn persist_download_job(&mut self, rid: u64, status: DownloadRecordStatus) {
        let children = if self.jobs.get(&rid).map(|j| j.is_folder()).unwrap_or(false) {
            self.snapshot_children(rid)
        } else {
            Vec::new()
        };
        let rec_id = Self::chrono_now();
        let at = crate::format::now_unix();
        let Some(j) = self.jobs.get_mut(&rid) else {
            return;
        };
        j.record_id = rec_id.clone();
        j.at = Some(at);
        settings::append_download_record(DownloadRecord {
            file_id: j.file_id.clone(),
            name: j.name.clone(),
            dir: j.dir.clone(),
            total: j.total,
            done: j.done,
            status,
            at,
            timestamp: rec_id,
            is_folder: j.is_folder(),
            children,
        });
    }

    /// 目录任务的树节点记录快照(按先序, 含子目录)。
    fn snapshot_children(&self, folder: u64) -> Vec<DownloadChildRecord> {
        let nodes = self
            .jobs
            .get(&folder)
            .map(|j| j.nodes.clone())
            .unwrap_or_default();
        nodes
            .iter()
            .map(|n| {
                let c = n.rid.and_then(|rid| self.jobs.get(&rid));
                DownloadChildRecord {
                    file_id: c.map(|c| c.file_id.clone()).unwrap_or_default(),
                    name: n.name.clone(),
                    dir: c.map(|c| c.dir.clone()).unwrap_or_default(),
                    total: c.map(|c| c.total).unwrap_or(0),
                    done: c.map(|c| c.done).unwrap_or(0),
                    status: c
                        .map(|c| dl_record_status(&c.status))
                        .unwrap_or(DownloadRecordStatus::Done),
                    at: c.and_then(|c| c.at).unwrap_or(0),
                    is_dir: n.is_dir,
                    depth: n.depth,
                }
            })
            .collect()
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
        let start = helpers::picker_start_dir(&self.last_dir);
        let parent = self.current_parent();
        let stack = self.current_stack_pairs();
        self.upload_pick = Some((false, parent, stack, helpers::pick_files_async(&start)));
    }

    /// 选择本地文件夹并递归上传到当前网盘目录(异步弹框, 不阻塞 UI)。
    pub(crate) fn upload_dir_here(&mut self) {
        if self.upload_pick.is_some() {
            return;
        }
        let start = helpers::picker_start_dir(&self.last_dir);
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
                self.remember_picked_dir(&paths, is_dir);
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

    /// 记住本次选中的位置, 作为下次本地选择框的起始目录
    /// (目录选择记其自身, 文件选择记首个文件的父目录)。
    fn remember_picked_dir(&mut self, paths: &[std::path::PathBuf], is_dir: bool) {
        let Some(dir) = helpers::picked_dir(paths, is_dir) else {
            return;
        };
        let Some(dir) = dir.to_str() else {
            return;
        };
        if self.last_dir != dir {
            self.last_dir = dir.to_string();
            self.persist_settings();
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
                    && filetypes::is_subtitle(&f.name)
                    && helpers::subtitle_of(name, &f.name)
            })
            .map(|f| (f.id.clone(), f.name.clone()))
            .collect()
    }

    /// 当前列表里某文件的类型(查不到条目时按文件名回退)。
    pub(crate) fn file_type(&self, id: &str, name: &str) -> filetypes::FileType {
        match self.files.iter().find(|f| f.id == id) {
            Some(f) => filetypes::classify_file(f),
            None => filetypes::classify(name, None),
        }
    }

    /// 预览云端文件: 音/视频交给 mpv 流式播放, 其他下载后交给系统查看器。
    ///
    /// 所有入口(双击 / 右键菜单 / 工具栏)都汇到这里: 只下载类直接拒绝并提示;
    /// 非媒体预览要整份下载, 超过 [`PREVIEW_CONFIRM_BYTES`] 且未命中缓存时先确认。
    pub(crate) fn open_preview(&mut self, id: String, name: String) {
        let ft = self.file_type(&id, &name);
        tracing::debug!(
            "预览路由「{name}」: mime={:?} → {ft:?}",
            self.files
                .iter()
                .find(|f| f.id == id)
                .and_then(|f| f.mime_type.as_deref())
        );
        match filetypes::preview_kind(ft) {
            filetypes::PreviewKind::DownloadOnly => {
                self.toast_warn(&format!("「{name}」不支持预览, 可用「下载到本地」"));
                return;
            }
            filetypes::PreviewKind::Open => {
                let size = self.file_size(&id);
                if size > PREVIEW_CONFIRM_BYTES && !worker::preview_cached(&id, &name) {
                    self.preview.confirm_big(id, name, size);
                    return;
                }
            }
            filetypes::PreviewKind::Play => {}
        }
        self.start_preview(id, name);
    }

    /// 某文件的大小(查不到条目时按 0 处理, 不触发大文件确认)。
    fn file_size(&self, id: &str) -> i64 {
        self.files
            .iter()
            .find(|f| f.id == id)
            .map(|f| f.size)
            .unwrap_or(0)
    }

    /// 真正发起预览(供大文件确认通过后复用)。
    pub(crate) fn start_preview(&mut self, id: String, name: String) {
        let ft = self.file_type(&id, &name);
        let media = matches!(ft, filetypes::FileType::Video | filetypes::FileType::Audio);
        let req_id = self.alloc_req_id();
        // 同目录下的同集字幕, 播放时一并挂载(仅视频需要)。
        let subtitles = if ft == filetypes::FileType::Video {
            self.episode_subtitles(&name)
        } else {
            Vec::new()
        };
        self.preview
            .start(&mut self.global, req_id, id, name, media, subtitles);
    }

    /// 确保某媒体文件的可用清晰度已解析(供「播放」子菜单展示)。
    pub(crate) fn fetch_qualities(&mut self, id: String, name: String) {
        let subtitles = self.episode_subtitles(&name);
        self.preview
            .fetch_qualities(&mut self.global, id, subtitles);
    }

    /// 用某个已解析出的清晰度播放(挂载同集字幕)。
    pub(crate) fn play_option(&mut self, id: String, opt: crate::msg::QualityOption) {
        let name = self
            .files
            .iter()
            .find(|f| f.id == id)
            .map(|f| f.name.clone())
            .unwrap_or_else(|| opt.label.clone());
        self.preview.play_option(&mut self.global, id, name, opt);
    }

    // ---------- 我的分享 ----------

    /// 从当前选中项打开「创建分享」设置框。
    pub(crate) fn share_selection(&mut self) {
        let targets = self.selected_names();
        if targets.is_empty() {
            self.toast_warn("请先选择要分享的文件");
            return;
        }
        self.shares.open_dialog(targets);
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
        self.shares.open_dialog(targets);
    }
}

// ================= 主循环 =================

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain(ctx);
        self.poll_file_picker();
        self.preview.poll_open_probe(ctx, &mut self.global);
        self.global.poll_system_theme();

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
        } else if self.preview.has_inflight_qualities() {
            // 清晰度解析中, 加快轮询让「播放」子菜单尽快展开选项。
            ctx.request_repaint_after(Duration::from_millis(100));
        } else if self.upload_pick.is_some() {
            // 文件选择进行中, 加快轮询以尽快取回结果。
            ctx.request_repaint_after(Duration::from_millis(100));
        } else if self.has_active_downloads()
            || self.has_active_uploads()
            || self.preview.is_progressing()
            || self.dir_loading
            || !self.dir_inflight.is_empty()
            || self.shares.loading
            || self.trash.loading
            || !self.tasks_loading_more.is_empty()
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
        self.preview.draw_status(ctx, &mut self.global, &th);
        self.draw_toast(ctx);
        self.drop_overlay(ctx);
    }
}
