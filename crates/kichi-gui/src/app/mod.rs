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

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32};
use kichi_core::session;
use kichi_core::types::{task_id, File, FileList, Quota, Share, Task};

use crate::filetypes;
use crate::kde;
use crate::msg::{Cmd, Msg};
use crate::settings::{
    self, DownloadChildRecord, DownloadRecord, DownloadRecordStatus, UploadRecord,
    UploadRecordStatus,
};
use crate::theme::{self, Theme};
use crate::worker;

use self::helpers::install_fonts;
use self::types::{
    ClipKind, Clipboard, ColDrag, Crumb, DirEntry, DlFilter, DlJob, DlNode, DlRow, DlStatus,
    OfflineTab, Page, PendingOpen, PreviewConfirm, PreviewProgress, QualityReady, ShareResult,
    SortBy, TransferTab, UlFilter, UlJob, UlStatus, UploadPick, ViewMode,
};

/// 非媒体预览的确认阈值: 预览需先整份下载到本地缓存, 超过则先弹确认。
const PREVIEW_CONFIRM_BYTES: i64 = 64 * 1024 * 1024;

/// 下载任务状态 -> 持久化记录状态(非终态仅在异常情况下出现, 兜底标记未完成)。
fn dl_record_status(s: &DlStatus) -> DownloadRecordStatus {
    match s {
        DlStatus::Done => DownloadRecordStatus::Done,
        DlStatus::Failed(w) => DownloadRecordStatus::Failed(w.clone()),
        DlStatus::Queued | DlStatus::Running => DownloadRecordStatus::Failed("未完成".into()),
    }
}

/// 汇总目录任务下所有子文件的进度与状态: (合计大小, 已下载, 合计速率, 聚合状态)。
fn aggregate_children<'a>(children: impl Iterator<Item = &'a DlJob>) -> (u64, u64, u64, DlStatus) {
    let mut total = 0u64;
    let mut done = 0u64;
    let mut speed = 0u64;
    let (mut n, mut done_n, mut fail_n) = (0u32, 0u32, 0u32);
    let mut active = false;
    for c in children {
        n += 1;
        total = total.saturating_add(c.total);
        done = done.saturating_add(c.done);
        match &c.status {
            DlStatus::Done => done_n += 1,
            DlStatus::Failed(_) => fail_n += 1,
            DlStatus::Running => {
                active = true;
                speed = speed.saturating_add(c.speed);
            }
            DlStatus::Queued => {}
        }
    }
    let status = if fail_n > 0 {
        DlStatus::Failed(format!("{fail_n} 个文件失败"))
    } else if n > 0 && done_n == n {
        DlStatus::Done
    } else if active {
        DlStatus::Running
    } else {
        DlStatus::Queued
    };
    (total, done, speed, status)
}

/// 依据文件节点的 `done` 标记, 自底向上累加每个子目录节点的子树文件计数。
/// `nodes` 必须按先序排列(父节点先于其子孙)。
fn compute_dir_counts(nodes: &mut [DlNode]) {
    for n in nodes.iter_mut() {
        if n.is_dir {
            n.files_done = 0;
            n.files_total = 0;
        }
    }
    // 栈内为当前仍「开放」的祖先目录下标(按 depth 递增)。
    let mut stack: Vec<usize> = Vec::new();
    for i in 0..nodes.len() {
        while let Some(&top) = stack.last() {
            if nodes[top].depth >= nodes[i].depth {
                stack.pop();
            } else {
                break;
            }
        }
        if nodes[i].is_dir {
            stack.push(i);
        } else if nodes[i].rid.is_some() {
            let done = nodes[i].done;
            for &d in &stack {
                nodes[d].files_total += 1;
                if done {
                    nodes[d].files_done += 1;
                }
            }
        }
    }
}

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
    /// 登录被要求人机验证时, 服务端给出的验证页链接(可在浏览器打开)。
    pub(crate) auth_captcha_url: Option<String>,
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

    // 全局搜索
    /// 是否处于搜索模式(搜索模式下显示搜索结果而非当前目录)
    pub(crate) search_mode: bool,
    /// 搜索关键词
    pub(crate) search_keyword: String,
    /// 搜索结果
    pub(crate) search_results: Vec<File>,
    /// 搜索结果分页 token
    pub(crate) search_next: Option<String>,
    /// 搜索是否进行中
    pub(crate) search_loading: bool,
    /// 当前搜索请求 ID
    pub(crate) search_req_id: u64,

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

    /// 正在准备中的预览任务 (req_id, 文件名); 用于给出加载反馈。
    pub(crate) preview_pending: Option<(u64, String)>,
    /// 待确认的大文件预览(非媒体预览需先整份下载, 超过阈值先问一次)。
    pub(crate) preview_confirm: Option<PreviewConfirm>,
    /// 非媒体预览的缓存下载进度(常驻进度条 + 取消)。
    pub(crate) preview_progress: Option<PreviewProgress>,
    /// 待回传的「用系统程序打开」探针(避免 xdg-open 假成功)。
    pub(crate) pending_open: Option<PendingOpen>,

    /// 已解析的媒体文件清晰度缓存(file_id -> 清晰度+字幕)。
    pub(crate) quality_cache: HashMap<String, QualityReady>,
    /// 正在解析清晰度的文件 id。
    pub(crate) quality_inflight: HashSet<String>,

    /// 已加载的缩略图纹理(file_id -> TextureHandle)。
    pub(crate) thumbnail_textures: HashMap<String, egui::TextureHandle>,
    /// 正在加载缩略图的文件 id。
    pub(crate) thumbnail_inflight: HashSet<String>,
    /// 缩略图重试到底仍失败的文件 id(本次会话不再重复请求; 刷新目录 /
    /// 重新搜索时清空以再试一次)。
    pub(crate) thumbnail_failed: HashSet<String>,

    /// 磁盘缓存(预览 + 缩略图)占用; None = 未查询或查询中。
    pub(crate) cache_usage: Option<types::CacheUsage>,
    /// 是否已发出占用查询(避免每帧重复发)。
    pub(crate) cache_usage_pending: bool,
    /// 是否正在执行手动清理。
    pub(crate) cache_sweeping: bool,
    /// 网格视图卡片大小(80-160)。
    pub(crate) grid_card_size: f32,

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

/// 用时间加权 EMA 刷新任务速率。
///
/// `drain()` 会在单帧内一次性消费积压的多条进度消息, 若逐条按 `Instant::now()`
/// 取样会出现 `dt≈0` 而使瞬时速率爆炸(截图里的 389 MB/s)。这里仅当距上次取样
/// 满 `MIN_SAMPLE` 秒才计算一次, 短间隔消息只推进 `done` 不动速率。
fn sample_speed(speed: &mut u64, last_done: &mut u64, last_at: &mut Option<Instant>, done: u64) {
    const MIN_SAMPLE: f64 = 0.25;
    let now = Instant::now();
    match *last_at {
        None => {
            *last_at = Some(now);
            *last_done = done;
        }
        Some(at) => {
            let dt = now.duration_since(at).as_secs_f64();
            if dt < MIN_SAMPLE {
                return;
            }
            if done >= *last_done {
                let inst = ((done - *last_done) as f64 / dt) as u64;
                *speed = if *speed == 0 {
                    inst
                } else {
                    ((*speed as f64) * 0.6 + (inst as f64) * 0.4) as u64
                };
            }
            *last_at = Some(now);
            *last_done = done;
        }
    }
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
            auth_captcha_url: None,
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
            search_mode: false,
            search_keyword: String::new(),
            search_results: Vec::new(),
            search_next: None,
            search_loading: false,
            search_req_id: 0,
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
            preview_pending: None,
            preview_confirm: None,
            preview_progress: None,
            pending_open: None,
            quality_cache: HashMap::new(),
            quality_inflight: HashSet::new(),
            thumbnail_textures: HashMap::new(),
            thumbnail_inflight: HashSet::new(),
            thumbnail_failed: HashSet::new(),
            cache_usage: None,
            cache_usage_pending: false,
            cache_sweeping: false,
            grid_card_size: 104.0,
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
                    self.preview_pending = None;
                    self.preview_confirm = None;
                    self.preview_progress = None;
                    self.pending_open = None;
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
                    self.preview_pending = None;
                    self.preview_confirm = None;
                    self.preview_progress = None;
                    self.pending_open = None;
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
                Msg::SearchResults {
                    req_id,
                    append,
                    list,
                } => {
                    if req_id != self.search_req_id {
                        continue;
                    }
                    self.search_loading = false;
                    self.search_next = list.next_page_token;
                    if append {
                        let known: HashSet<String> =
                            self.search_results.iter().map(|f| f.id.clone()).collect();
                        for f in list.files {
                            if !known.contains(&f.id) {
                                self.search_results.push(f);
                            }
                        }
                    } else {
                        self.search_results = list.files;
                        // 新一次搜索 = 重新开始, 与换目录同理。
                        self.thumbnail_inflight.clear();
                        self.thumbnail_failed.clear();
                    }
                }
                Msg::SearchFailed { what } => {
                    self.search_loading = false;
                    self.toast_err(&what);
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
                } => {
                    if self
                        .preview_pending
                        .as_ref()
                        .is_some_and(|(id, _)| *id == req_id)
                    {
                        self.preview_pending = None;
                    }
                    if self
                        .preview_progress
                        .as_ref()
                        .is_some_and(|p| p.req_id == req_id)
                    {
                        self.preview_progress = None;
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
                Msg::PreviewProgress {
                    req_id,
                    total,
                    done,
                } => {
                    // 只认当前在途的预览任务, 避免过期消息把进度条拉回来。
                    let name = self
                        .preview_pending
                        .as_ref()
                        .filter(|(id, _)| *id == req_id)
                        .map(|(_, name)| name.clone());
                    if let Some(name) = name {
                        self.preview_progress = Some(PreviewProgress {
                            req_id,
                            name,
                            total,
                            done,
                        });
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
                    if self
                        .preview_progress
                        .as_ref()
                        .is_some_and(|p| p.req_id == req_id)
                    {
                        self.preview_progress = None;
                    }
                    // 结果由后台探针回传(xdg-open 失败不再是假成功)。
                    self.pending_open = Some(PendingOpen {
                        rx: helpers::open_async(path),
                        label: name,
                        quiet_ok: false,
                    });
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
                    if self
                        .preview_progress
                        .as_ref()
                        .is_some_and(|p| p.req_id == req_id)
                    {
                        self.preview_progress = None;
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
                Msg::ThumbnailReady {
                    file_id,
                    width,
                    height,
                    pixels,
                } => {
                    self.thumbnail_inflight.remove(&file_id);
                    let color_image = egui::ColorImage {
                        size: [width as usize, height as usize],
                        pixels,
                    };
                    let texture = ctx.load_texture(
                        format!("thumb_{file_id}"),
                        color_image,
                        egui::TextureOptions::LINEAR,
                    );
                    self.thumbnail_textures.insert(file_id, texture);
                }
                Msg::ThumbnailFailed { file_id } => {
                    // 结束在途登记并记住失败: 既不留下永不结束的 inflight,
                    // 也不在下一帧立刻重新请求同一个失败的图。
                    self.thumbnail_inflight.remove(&file_id);
                    self.thumbnail_failed.insert(file_id);
                }
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

    /// 触发全局搜索。
    pub(crate) fn trigger_search(&mut self) {
        let keyword = self.filter.trim().to_string();
        tracing::info!("触发搜索: keyword='{}'", keyword);
        if keyword.is_empty() {
            self.exit_search();
            return;
        }
        self.search_mode = true;
        self.search_keyword = keyword.clone();
        self.search_results.clear();
        self.search_next = None;
        self.search_loading = true;
        self.search_req_id += 1;
        tracing::info!("发送搜索命令: req_id={}", self.search_req_id);
        let _ = self.tx.send(Cmd::SearchFiles {
            keyword,
            token: None,
            append: false,
            req_id: self.search_req_id,
        });
    }

    /// 退出搜索模式。
    pub(crate) fn exit_search(&mut self) {
        self.search_mode = false;
        self.search_keyword.clear();
        self.search_results.clear();
        self.search_next = None;
        self.search_loading = false;
        self.filter.clear();
    }

    /// 加载更多搜索结果。
    pub(crate) fn load_more_search_results(&mut self) {
        if self.search_loading || self.search_next.is_none() {
            return;
        }
        self.search_loading = true;
        let token = self.search_next.clone().unwrap();
        let _ = self.tx.send(Cmd::SearchFiles {
            keyword: self.search_keyword.clone(),
            token: Some(token),
            append: true,
            req_id: self.search_req_id,
        });
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
                self.thumbnail_inflight.clear();
                self.thumbnail_failed.clear();
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
        let source = if self.search_mode {
            &self.search_results
        } else {
            &self.files
        };

        let kw = if self.search_mode {
            String::new() // 搜索结果已经过滤过了
        } else {
            self.filter.trim().to_lowercase()
        };
        let cur = self.current_parent();
        let mut folders: Vec<&File> = Vec::new();
        let mut plain: Vec<&File> = Vec::new();
        for f in source {
            if !self.search_mode && self.hidden.get(&f.id).is_some_and(|hp| *hp == cur) {
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

    /// 回收「用系统程序打开」的探针结果(见 [`helpers::open_async`]), 给出诚实提示。
    fn poll_pending_open(&mut self, ctx: &egui::Context) {
        let Some(pending) = self.pending_open.take() else {
            return;
        };
        match pending.rx.try_recv() {
            Ok(helpers::OpenOutcome::Launched) => {
                if !pending.quiet_ok {
                    self.toast_ok(&format!("已打开「{}」", pending.label));
                }
            }
            Ok(helpers::OpenOutcome::NoHandler) => {
                self.toast_warn(&format!("系统未关联打开「{}」的程序", pending.label));
            }
            Ok(helpers::OpenOutcome::Failed(e)) => {
                self.toast_err(&format!("打开「{}」失败: {e}", pending.label));
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                // 探针还在等 xdg-open 退出(最多 1s), 保留结果下次再收。
                self.pending_open = Some(pending);
                ctx.request_repaint_after(Duration::from_millis(100));
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {}
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
                    self.preview_confirm = Some(PreviewConfirm { id, name, size });
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
    fn start_preview(&mut self, id: String, name: String) {
        let ft = self.file_type(&id, &name);
        let media = matches!(ft, filetypes::FileType::Video | filetypes::FileType::Audio);
        let req_id = self.alloc_req_id();
        self.preview_pending = Some((req_id, name.clone()));
        // 同目录下的同集字幕, 播放时一并挂载(仅视频需要)。
        let subtitles = if ft == filetypes::FileType::Video {
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

    /// 取消正在准备的非媒体预览(丢弃未完成的缓存下载)。
    pub(crate) fn cancel_preview(&mut self) {
        let Some(p) = self.preview_progress.take() else {
            return;
        };
        self.send(Cmd::CancelPreview { req_id: p.req_id });
        if self
            .preview_pending
            .as_ref()
            .is_some_and(|(id, _)| *id == p.req_id)
        {
            self.preview_pending = None;
        }
        self.toast("已取消预览", self.theme().text_weak);
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

    /// 解析分享输入, 返回 `(share_id, 链接里携带的提取码)`。
    ///
    /// 支持的形态:
    /// - 完整链接: `https://mypikpak.com/s/<id>`
    /// - 带查询参数 / 片段: `.../s/<id>?password=abcd#frag`(顺带提取 `password`/`pass_code`)
    /// - 复制链接时附带的前后缀文字: 只取 `/s/` 之后的一段
    /// - 裸 ID: `<id>`
    ///
    /// 无法识别(如缺少 `/s/` 的其它域名链接、含非法字符)时返回 `None`, 由调用方给出提示。
    pub(crate) fn parse_share_input(input: &str) -> Option<(String, Option<String>)> {
        let input = input.trim();
        if input.is_empty() {
            return None;
        }

        // 截出 `/s/` 之后的一段: ID 到第一个非法字符为止, 之后按查询串解析提取码。
        let (id_candidate, query) = if let Some(idx) = input.rfind("/s/") {
            let rest = &input[idx + 3..];
            let id: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                .collect();
            let query = rest.find('?').map(|q| {
                let after = &rest[q + 1..];
                let end = after
                    .find(|c: char| c == '#' || c.is_whitespace())
                    .unwrap_or(after.len());
                &after[..end]
            });
            (id, query)
        } else if input.contains("://") {
            // 是一个链接但不含 `/s/` 段, 无法定位分享 ID。
            return None;
        } else {
            // 视为裸 ID。
            (input.to_string(), None)
        };

        if !Self::is_valid_share_id(&id_candidate) {
            return None;
        }
        let pass_code = query.and_then(Self::pass_code_from_query);
        Some((id_candidate, pass_code))
    }

    /// share_id 只由字母、数字、`-`、`_` 组成且非空。
    fn is_valid_share_id(id: &str) -> bool {
        !id.is_empty()
            && id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    }

    /// 从 URL 查询串里取出提取码 (`password` / `pass_code` / `passcode`)。
    fn pass_code_from_query(query: &str) -> Option<String> {
        for pair in query.split('&') {
            let mut kv = pair.splitn(2, '=');
            let key = kv.next().unwrap_or("");
            if matches!(key, "password" | "pass_code" | "passcode") {
                if let Some(v) = kv.next() {
                    let v = v.trim();
                    if !v.is_empty() {
                        return Some(v.to_string());
                    }
                }
            }
        }
        None
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
        self.drain(ctx);
        self.poll_file_picker();
        self.poll_pending_open(ctx);
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
            || self.has_active_uploads()
            || self.preview_progress.is_some()
            || self.dir_loading
            || !self.dir_inflight.is_empty()
            || self.shares_loading
            || self.trash_loading
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
        self.draw_preview_status(ctx);
        self.draw_toast(ctx);
        self.drop_overlay(ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dl_job(status: DlStatus, total: u64, done: u64) -> DlJob {
        let mut j = DlJob::queued("id".into(), "n".into(), std::path::PathBuf::from("/tmp"));
        j.status = status;
        j.total = total;
        j.done = done;
        j
    }

    #[test]
    fn aggregate_sums_and_running_status() {
        let a = dl_job(DlStatus::Running, 100, 40);
        let b = dl_job(DlStatus::Done, 50, 50);
        let (total, done, _speed, status) = aggregate_children([&a, &b].into_iter());
        assert_eq!((total, done), (150, 90));
        assert_eq!(status, DlStatus::Running);
    }

    #[test]
    fn aggregate_all_done_is_done() {
        let a = dl_job(DlStatus::Done, 10, 10);
        let b = dl_job(DlStatus::Done, 5, 5);
        let (total, done, _speed, status) = aggregate_children([&a, &b].into_iter());
        assert_eq!((total, done), (15, 15));
        assert_eq!(status, DlStatus::Done);
    }

    #[test]
    fn aggregate_any_failure_wins() {
        let a = dl_job(DlStatus::Done, 10, 10);
        let f = dl_job(DlStatus::Failed("x".into()), 0, 0);
        let (_t, _d, _s, status) = aggregate_children([&a, &f].into_iter());
        assert!(matches!(status, DlStatus::Failed(_)));
    }

    #[test]
    fn aggregate_queued_before_any_activity() {
        let a = dl_job(DlStatus::Queued, 10, 0);
        let (_t, _d, _s, status) = aggregate_children([&a].into_iter());
        assert_eq!(status, DlStatus::Queued);
    }

    fn dir_node(name: &str, depth: u32) -> DlNode {
        DlNode {
            is_dir: true,
            name: name.into(),
            depth,
            rid: None,
            expanded: true,
            files_done: 0,
            files_total: 0,
            done: false,
        }
    }

    fn file_node(depth: u32, done: bool) -> DlNode {
        DlNode {
            is_dir: false,
            name: "f".into(),
            depth,
            rid: Some(1),
            expanded: false,
            files_done: 0,
            files_total: 0,
            done,
        }
    }

    #[test]
    fn dir_counts_roll_up_subtree() {
        // A/ (1): 自身 2 个文件(1 完成) + 子目录 B/ (2): 1 个文件(完成); C/ (1): 空。
        let mut nodes = vec![
            dir_node("A", 1),
            file_node(2, true),
            file_node(2, false),
            dir_node("B", 2),
            file_node(3, true),
            dir_node("C", 1),
        ];
        compute_dir_counts(&mut nodes);
        // A 汇总其整个子树: 3 个文件、2 个完成。
        assert_eq!((nodes[0].files_done, nodes[0].files_total), (2, 3));
        assert_eq!((nodes[3].files_done, nodes[3].files_total), (1, 1));
        assert_eq!((nodes[5].files_done, nodes[5].files_total), (0, 0));
    }

    #[test]
    fn parse_share_id_from_url_forms() {
        let cases = [
            ("https://mypikpak.com/s/VO8B-abc", Some(("VO8B-abc", None))),
            (
                "  https://mypikpak.com/s/VO8B-abc?password=abcd  ",
                Some(("VO8B-abc", Some("abcd"))),
            ),
            (
                "https://mypikpak.com/s/VO8B-abc#frag",
                Some(("VO8B-abc", None)),
            ),
            // 复制链接时附带的前后缀文字。
            (
                "打开链接 https://mypikpak.com/s/AbC_123 查看",
                Some(("AbC_123", None)),
            ),
            ("xyz-1", Some(("xyz-1", None))),
            // 无法识别的形态。
            ("https://example.com/download/abc", None),
            ("https://mypikpak.com/s/", None),
            ("not a valid id!", None),
            ("", None),
        ];
        for (input, want) in cases {
            let got = App::parse_share_input(input);
            let got = got.as_ref().map(|(id, p)| (id.as_str(), p.as_deref()));
            assert_eq!(got, want, "input: {input:?}");
        }
    }
}
