mod dialogs;
mod files;
mod global;
mod helpers;
mod login;
mod preview;
mod search;
mod settings_page;
mod shares;
mod sidebar;
mod tasks;
mod tasks_page;
mod thumbs;
mod transfers;
mod trash;
pub(crate) mod types;

use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32};
use kichi_core::session;
use kichi_core::types::Quota;

use crate::filetypes;
use crate::msg::{Cmd, Msg};
use crate::settings;
use crate::theme::{self, Theme};
use crate::worker;

use self::files::{FilesAction, FilesPage};
use self::global::Global;
use self::helpers::install_fonts;
use self::preview::PreviewPage;
use self::search::SearchPage;
use self::shares::SharesPage;
use self::tasks::TasksPage;
use self::thumbs::ThumbsPage;
use self::transfers::{TransfersAction, TransfersPage};
use self::trash::TrashPage;
use self::types::{Crumb, Page};

/// 非媒体预览的确认阈值: 预览需先整份下载到本地缓存, 超过则先弹确认。
const PREVIEW_CONFIRM_BYTES: i64 = 64 * 1024 * 1024;

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
    /// 传输任务域: 上传 / 下载任务列表与选中、筛选、展开, 本地上传选择框。
    pub(crate) transfers: TransfersPage,

    // 文件浏览
    /// 文件浏览域: 导航栈 / 目录缓存(含 SWR 校正) / 列表数据 / 选中集 / 视图与列宽 / 剪贴板。
    pub(crate) files: FilesPage,

    // 全局搜索
    pub(crate) search: SearchPage,

    /// 离线任务域: 分桶列表 / 选择 / 新建表单与「保存到」选择器。
    pub(crate) tasks: TasksPage,

    pub(crate) quota: Option<Quota>,

    // 对话框
    pub(crate) mkdir_open: bool,
    pub(crate) mkdir_name: String,
    pub(crate) rename_id: Option<String>,
    pub(crate) rename_name: String,
    pub(crate) trash_confirm: Option<Vec<(String, String)>>,

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
            transfers: TransfersPage::from_settings(),
            files: FilesPage::default(),
            search: SearchPage::default(),
            tasks: TasksPage::default(),
            quota: None,
            mkdir_open: false,
            mkdir_name: String::new(),
            rename_id: None,
            rename_name: String::new(),
            trash_confirm: None,
            logout_confirm: false,
            download_dir: saved.download_dir.clone(),
            last_dir: saved.last_dir.clone(),
            dl_concurrency: saved.dl_concurrency,
            ul_concurrency: saved.ul_concurrency,
            part_concurrency: saved.part_concurrency,
            max_attempts: saved.max_attempts,
            preview: PreviewPage::default(),
            thumbs: ThumbsPage::default(),
            cache_usage: None,
            cache_usage_pending: false,
            cache_sweeping: false,
            shares: SharesPage::default(),
            trash: TrashPage::default(),
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
        self.global.req_id = self.transfers.max_job_id();
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
                    self.tasks.clear();
                    self.quota = None;
                    self.files.reset_browse(&mut self.global);
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
                    self.transfers.clear();
                    self.files.invalidate_session();
                    self.preview.clear();
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
                    self.tasks.clear();
                    self.files.clear();
                    self.transfers.clear();
                    self.preview.clear();
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
                    self.files
                        .on_files(parent, req_id, append, list, &mut self.thumbs);
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
                    self.files.reload_dir(&mut self.global);
                }
                Msg::Renamed => {
                    self.rename_id = None;
                    self.toast_ok("重命名成功");
                    self.files.reload_dir(&mut self.global);
                }
                Msg::Trashed => {
                    self.trash_confirm = None;
                    // 回收站内容已变, 作废缓存。
                    self.trash.invalidate_cache();
                    self.files.on_trashed(&mut self.global);
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
                    self.files.invalidate_cache();
                }
                Msg::TrashDeleted { ids } => self.trash.on_deleted(&mut self.global, &ids),
                Msg::TrashEmptied => self.trash.on_emptied(&mut self.global),
                Msg::Moved { ids, src, dest } => {
                    self.files.on_moved(&mut self.global, &ids, &src, &dest);
                }
                Msg::Copied { dest } => self.files.on_copied(&mut self.global, dest),
                Msg::OfflineCreated => self.tasks.on_created(&mut self.global),
                Msg::OfflineRetried => self.tasks.on_changed(&mut self.global),
                Msg::OfflineDeleted => self.tasks.on_changed(&mut self.global),
                Msg::Folders {
                    parent,
                    req_id,
                    files,
                } => {
                    // 路由到对应的目录选择器
                    if self.shares.handles_picker(req_id) {
                        self.shares.on_folders(parent, files);
                    } else if self.tasks.handles_picker(req_id) {
                        self.tasks.on_folders(parent, files);
                    }
                }
                Msg::Quota(quota) => self.quota = quota,
                Msg::TasksAll {
                    buckets,
                    next_tokens,
                } => self.tasks.on_all(buckets, next_tokens),
                Msg::TasksMore {
                    phase,
                    tasks,
                    next_page_token,
                } => self.tasks.on_more(phase, tasks, next_page_token),
                Msg::TasksMoreFailed { phase, what } => {
                    self.tasks.on_more_failed(&mut self.global, phase, what)
                }
                Msg::DlProgress {
                    req_id,
                    total,
                    done,
                } => self.transfers.on_dl_progress(req_id, total, done),
                Msg::DlFinished { req_id, bytes } => self.transfers.on_dl_finished(req_id, bytes),
                Msg::DlCancelled { req_id } => self.transfers.on_dl_cancelled(req_id),
                Msg::DlFailed { req_id, what } => self.transfers.on_dl_failed(req_id, what),
                Msg::FolderScanned {
                    req_id,
                    items,
                    total_bytes,
                } => self
                    .transfers
                    .on_folder_scanned(&mut self.global, req_id, items, total_bytes),
                Msg::FolderScanFailed { req_id, what } => {
                    self.transfers
                        .on_folder_scan_failed(&mut self.global, req_id, what)
                }
                Msg::UlProgress {
                    req_id,
                    total,
                    done,
                } => self.transfers.on_ul_progress(req_id, total, done),
                Msg::UlFiles {
                    req_id,
                    done,
                    total,
                    current,
                } => self.transfers.on_ul_files(req_id, done, total, current),
                Msg::UlFinished { req_id } => {
                    if let Some(parent) = self.transfers.on_ul_finished(&mut self.global, req_id) {
                        // 上传完成后目标目录内容已变, 作废缓存并按需刷新。
                        self.files.dir_cache.remove(&parent);
                        if parent == self.files.current_parent() {
                            self.files.reload_dir(&mut self.global);
                        }
                    }
                    // 上传占用空间, 显式刷新配额(自动轮询已降频)。
                    self.send(Cmd::RefreshQuota);
                }
                Msg::UlCancelled { req_id } => self.transfers.on_ul_cancelled(req_id),
                Msg::UlFailed { req_id, what } => {
                    self.transfers.on_ul_failed(&mut self.global, req_id, what)
                }
                Msg::FilesFailed { parent, what } => {
                    self.files.on_files_failed(&mut self.global, parent, what);
                }
                Msg::Error { what } => {
                    self.tasks.refreshing = false;
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
                    self.files.reload_dir(&mut self.global);
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

    /// 触发全局搜索。
    pub(crate) fn trigger_search(&mut self) {
        let keyword = self.files.filter.trim().to_string();
        tracing::info!("触发搜索: keyword='{}'", keyword);
        if keyword.is_empty() {
            self.files.exit_search(&mut self.search);
            return;
        }
        self.search.start(&mut self.global, keyword);
    }

    /// 加载更多搜索结果。
    pub(crate) fn load_more_search_results(&mut self) {
        self.search.load_more(&mut self.global);
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
        self.transfers
            .enqueue_download_folder(&mut self.global, id, name, dir);
        self.toast_ok("正在扫描目录…");
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
        self.transfers
            .enqueue_downloads(&mut self.global, vec![(id, name)], dir);
    }

    /// 选择本地文件并上传到当前网盘目录(异步弹框, 不阻塞 UI)。
    pub(crate) fn upload_here(&mut self) {
        let start = helpers::picker_start_dir(&self.last_dir);
        let parent = self.files.current_parent();
        let stack = self.files.current_stack_pairs();
        self.transfers
            .start_pick(false, parent, stack, helpers::pick_files_async(&start));
    }

    /// 选择本地文件夹并递归上传到当前网盘目录(异步弹框, 不阻塞 UI)。
    pub(crate) fn upload_dir_here(&mut self) {
        let start = helpers::picker_start_dir(&self.last_dir);
        let parent = self.files.current_parent();
        let stack = self.files.current_stack_pairs();
        self.transfers
            .start_pick(true, parent, stack, helpers::pick_dir_async(&start));
    }

    /// 处理拖拽到窗口的本地文件/文件夹(上传到当前网盘目录)。
    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        if dropped.is_empty() {
            return;
        }
        let parent = self.files.current_parent();
        let stack = self.files.current_stack_pairs();
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
            self.transfers
                .enqueue_upload(&mut self.global, files, parent.clone(), stack.clone());
        }
        for d in dirs {
            self.transfers
                .enqueue_upload_dir(&mut self.global, d, parent.clone(), stack.clone());
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
    /// 新选中的起始目录由 `App` 落盘(`last_dir` 是 App 的持久化设置)。
    fn poll_file_picker(&mut self) {
        let Some(dir) = self.transfers.poll_file_picker(&mut self.global) else {
            return;
        };
        if self.last_dir != dir {
            self.last_dir = dir;
            self.persist_settings();
        }
    }

    /// 执行文件页本帧产生的跨域动作(渲染与动作分离, 见 `files::FilesAction`)。
    fn apply_files_action(&mut self, a: FilesAction) {
        match a {
            FilesAction::NewFolder => {
                self.mkdir_open = true;
                self.mkdir_name.clear();
            }
            FilesAction::Rename { id, name } => {
                self.rename_id = Some(id);
                self.rename_name = name;
            }
            FilesAction::ConfirmTrash(items) => self.trash_confirm = Some(items),
            FilesAction::Preview { id, name } => self.open_preview(id, name),
            FilesAction::FetchQualities { id, name } => self.fetch_qualities(id, name),
            FilesAction::PlayOption { id, opt } => self.play_option(id, opt),
            FilesAction::DownloadSelection { files, folders } => {
                self.download_selection(files, folders)
            }
            FilesAction::DownloadFile { id, name } => self.download_single(id, name),
            FilesAction::DownloadFolder { id, name } => self.download_single_folder(id, name),
            FilesAction::ShareSelection => self.share_selection(),
            FilesAction::ShareItem(id) => self.share_item(id),
            FilesAction::UploadFiles => self.upload_here(),
            FilesAction::UploadFolder => self.upload_dir_here(),
            FilesAction::Search => self.trigger_search(),
            FilesAction::LoadMoreSearch => self.load_more_search_results(),
        }
    }

    /// 执行传输页本帧产生的跨域动作(渲染与动作分离, 见 `transfers::TransfersAction`)。
    fn apply_transfers_action(&mut self, a: TransfersAction) {
        match a {
            TransfersAction::OpenPath(path, label, quiet_ok) => {
                self.preview.open_with_system(path, label, quiet_ok)
            }
            TransfersAction::PickFiles => self.upload_here(),
            TransfersAction::PickFolder => self.upload_dir_here(),
            TransfersAction::NavigateTo(stack) => {
                self.files.stack = stack
                    .into_iter()
                    .map(|(id, label)| Crumb { id, label })
                    .collect();
                self.page = Page::Files;
                self.files.show_dir(&mut self.global);
            }
            TransfersAction::OpenDownloadDir => {
                let dir = if !self.download_dir.is_empty()
                    && std::path::Path::new(&self.download_dir).is_dir()
                {
                    Some(std::path::PathBuf::from(&self.download_dir))
                } else {
                    dirs::download_dir().filter(|d| d.is_dir())
                };
                match dir {
                    Some(d) => {
                        self.preview
                            .open_with_system(d.clone(), d.display().to_string(), true)
                    }
                    None => self.toast_warn("无法定位下载目录, 请在「设置」中手动选择保存位置"),
                }
            }
        }
    }

    /// 把选中的文件 / 文件夹加入下载队列; 默认下载目录不可用时先让用户选一个。
    fn download_selection(&mut self, files: Vec<(String, String)>, folders: Vec<(String, String)>) {
        let dir =
            if !self.download_dir.is_empty() && std::path::Path::new(&self.download_dir).is_dir() {
                std::path::PathBuf::from(&self.download_dir)
            } else {
                let Some(d) = self.choose_download_dir() else {
                    return;
                };
                d
            };
        self.transfers
            .enqueue_downloads(&mut self.global, files, dir.clone());
        for (id, name) in &folders {
            self.transfers.enqueue_download_folder(
                &mut self.global,
                id.clone(),
                name.clone(),
                dir.clone(),
            );
        }
        if !folders.is_empty() {
            self.toast_ok("正在扫描目录…");
        }
    }

    /// 当前目录下与 `name` 同集的外挂字幕 (id, 文件名)。
    fn episode_subtitles(&self, name: &str) -> Vec<(String, String)> {
        self.files
            .items
            .iter()
            .filter(|f| {
                !f.is_folder()
                    && filetypes::is_subtitle(&f.name)
                    && helpers::subtitle_of(name, &f.name)
            })
            .map(|f| (f.id.clone(), f.name.clone()))
            .collect()
    }

    /// 预览云端文件: 音/视频交给 mpv 流式播放, 其他下载后交给系统查看器。
    ///
    /// 所有入口(双击 / 右键菜单 / 工具栏)都汇到这里: 只下载类直接拒绝并提示;
    /// 非媒体预览要整份下载, 超过 [`PREVIEW_CONFIRM_BYTES`] 且未命中缓存时先确认。
    pub(crate) fn open_preview(&mut self, id: String, name: String) {
        let ft = self.files.file_type(&id, &name);
        tracing::debug!(
            "预览路由「{name}」: mime={:?} → {ft:?}",
            self.files
                .items
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
            .items
            .iter()
            .find(|f| f.id == id)
            .map(|f| f.size)
            .unwrap_or(0)
    }

    /// 真正发起预览(供大文件确认通过后复用)。
    pub(crate) fn start_preview(&mut self, id: String, name: String) {
        let ft = self.files.file_type(&id, &name);
        let media = matches!(ft, filetypes::FileType::Video | filetypes::FileType::Audio);
        let req_id = self.global.alloc_req_id();
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
            .items
            .iter()
            .find(|f| f.id == id)
            .map(|f| f.name.clone())
            .unwrap_or_else(|| opt.label.clone());
        self.preview.play_option(&mut self.global, id, name, opt);
    }

    // ---------- 我的分享 ----------

    /// 从当前选中项打开「创建分享」设置框。
    pub(crate) fn share_selection(&mut self) {
        let targets = self.files.selected_names();
        if targets.is_empty() {
            self.toast_warn("请先选择要分享的文件");
            return;
        }
        self.shares.open_dialog(targets);
    }

    /// 右键单项分享: 若该项在多选内则分享整个选中集, 否则仅分享该项。
    pub(crate) fn share_item(&mut self, id: String) {
        let targets = if self.files.selected.contains(&id) && self.files.selected.len() > 1 {
            self.files.selected_names()
        } else {
            self.files
                .items
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

        self.files.poll_relist(ctx, &mut self.global);

        let th = self.theme();
        theme::configure(ctx, &th);

        if self.auth_checking {
            ctx.request_repaint_after(Duration::from_millis(120));
        } else if self.preview.has_inflight_qualities() {
            // 清晰度解析中, 加快轮询让「播放」子菜单尽快展开选项。
            ctx.request_repaint_after(Duration::from_millis(100));
        } else if self.transfers.upload_pick.is_some() {
            // 文件选择进行中, 加快轮询以尽快取回结果。
            ctx.request_repaint_after(Duration::from_millis(100));
        } else if self.transfers.has_active_downloads()
            || self.transfers.has_active_uploads()
            || self.preview.is_progressing()
            || self.files.dir_loading
            || !self.files.dir_inflight.is_empty()
            || self.shares.loading
            || self.trash.loading
            || !self.tasks.loading_more.is_empty()
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
