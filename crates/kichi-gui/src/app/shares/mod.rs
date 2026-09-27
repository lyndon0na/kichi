//! 「我的分享」与「转存分享」域: 状态与消息处理。
//!
//! 本模块持有 [`SharesPage`] 的状态 / 生命周期与 `Msg` 处理; 渲染按角色分家 ——
//! 我的分享列表与创建分享弹窗在 `mine`, 转存分享与目标目录选择器在 `restore`。

use std::collections::HashSet;
use std::time::{Duration, Instant};

use kichi_core::types::{File, Share, ShareList};

use crate::msg::Cmd;

use super::files::Crumb;
use super::global::Global;

mod mine;
mod restore;

/// 分享创建成功后的结果展示。
#[derive(Clone)]
pub(crate) struct ShareResult {
    pub url: String,
    pub pass_code: String,
    pub share_text: String,
    pub label: String,
}

/// 「我的分享」列表新鲜期: 进入页面时命中则不发请求。
const SHARES_TTL: Duration = Duration::from_secs(60);

/// 分享页面状态(我的分享列表 + 创建分享弹窗 + 转存分享)。
pub(crate) struct SharesPage {
    // 我的分享
    list: Vec<Share>,
    next: Option<String>,
    /// 列表是否正在加载(刷新或加载更多)。
    pub(crate) loading: bool,
    /// 最近一次列表请求的 id, 用于丢弃乱序的旧响应。
    req: u64,
    /// 选中项 share_id。
    selected: HashSet<String>,
    /// 最近一次成功加载分享列表的时间, 用于 SWR 新鲜度判定。
    fetched_at: Option<Instant>,
    /// 待创建分享的选中项 (id, name); Some 表示「创建分享」设置框打开。
    create_dialog: Option<Vec<(String, String)>>,
    expiration_days: i64,
    need_password: bool,
    /// 分享创建成功后的结果框。
    result: Option<ShareResult>,
    /// 取消分享确认 (share_id, 标题)。
    delete_confirm: Option<Vec<(String, String)>>,

    // 转存分享
    /// 转存弹窗是否打开。
    save_open: bool,
    /// 用户输入的分享链接或分享 ID。
    save_input: String,
    /// 用户输入的提取码。
    save_pass_code: String,
    /// 是否正在解析分享链接。
    save_resolving: bool,
    /// 解析成功后的分享 ID。
    save_id: Option<String>,
    /// 解析成功后的分享标题。
    save_title: Option<String>,
    /// 解析成功后的 pass_code_token(转存时需要)。
    save_token: Option<String>,
    /// 解析出的文件列表。
    save_files: Vec<File>,
    /// 用户选中的文件 id。
    save_selected: HashSet<String>,
    /// 文件名搜索过滤。
    save_filter: String,
    /// 分享文件列表分页 token。
    save_next: Option<String>,
    /// 是否正在加载更多文件。
    save_loading_more: bool,
    /// 是否正在转存。
    save_saving: bool,
    /// 解析或转存的错误信息。
    save_error: Option<String>,
    /// 转存目标目录选择器是否打开。
    picker_open: bool,
    /// 转存目标目录选择器的面包屑导航。
    picker_stack: Vec<Crumb>,
    /// 转存目标目录选择器当前目录的子文件夹。
    picker_folders: Vec<File>,
    /// 转存目标目录选择器加载状态。
    picker_loading: bool,
    /// 转存目标目录选择器请求 ID。
    picker_req: u64,
    /// 用户选择的转存目标目录 (id, name); None 表示默认位置。
    save_dest: Option<(String, String)>,
    /// 自动移动失败时的目标目录信息, 用于重试。
    save_move_failed: Option<(String, String)>,
}

impl Default for SharesPage {
    fn default() -> Self {
        Self {
            list: Vec::new(),
            next: None,
            loading: false,
            req: 0,
            selected: HashSet::new(),
            fetched_at: None,
            create_dialog: None,
            // -1 = 永久有效。
            expiration_days: -1,
            need_password: false,
            result: None,
            delete_confirm: None,
            save_open: false,
            save_input: String::new(),
            save_pass_code: String::new(),
            save_resolving: false,
            save_id: None,
            save_title: None,
            save_token: None,
            save_files: Vec::new(),
            save_selected: HashSet::new(),
            save_filter: String::new(),
            save_next: None,
            save_loading_more: false,
            save_saving: false,
            save_error: None,
            picker_open: false,
            picker_stack: vec![Crumb {
                id: None,
                label: "我的云盘".to_string(),
            }],
            picker_folders: Vec::new(),
            picker_loading: false,
            picker_req: 0,
            save_dest: None,
            save_move_failed: None,
        }
    }
}

impl SharesPage {
    // ---------- 我的分享: 状态 ----------

    /// 清空我的分享相关状态(退出登录 / 会话失效时调用)。
    pub(crate) fn clear(&mut self) {
        self.list.clear();
        self.next = None;
        self.loading = false;
        self.req = 0;
        self.selected.clear();
        self.fetched_at = None;
        self.create_dialog = None;
        self.result = None;
        self.delete_confirm = None;
    }

    /// 进入「我的分享」页面: 缓存新鲜则零请求, 否则刷新。
    pub(crate) fn enter(&mut self, g: &mut Global) {
        let fresh = self.fetched_at.is_some_and(|t| t.elapsed() < SHARES_TTL);
        if !fresh {
            self.refresh(g);
        }
    }

    /// 请求刷新「我的分享」首页列表(保留旧数据, 仅置加载态)。
    pub(crate) fn refresh(&mut self, g: &mut Global) {
        self.loading = true;
        self.next = None;
        self.selected.clear();
        self.req += 1;
        let req_id = self.req;
        g.send(Cmd::ListShares {
            token: None,
            append: false,
            req_id,
        });
    }

    /// 加载分享列表下一页。
    pub(crate) fn load_more(&mut self, g: &mut Global) {
        let Some(token) = self.next.clone() else {
            return;
        };
        self.loading = true;
        self.req += 1;
        let req_id = self.req;
        g.send(Cmd::ListShares {
            token: Some(token),
            append: true,
            req_id,
        });
    }

    /// 打开「创建分享」设置框, targets 为 (id, name)。
    pub(crate) fn open_dialog(&mut self, targets: Vec<(String, String)>) {
        if targets.is_empty() {
            return;
        }
        self.create_dialog = Some(targets);
    }

    // ---------- 我的分享: 消息 ----------

    /// 分享创建成功: 记下结果并作废列表缓存(是否立即刷新由调用方按页面判断)。
    pub(crate) fn on_created(
        &mut self,
        url: String,
        pass_code: String,
        share_text: String,
        label: String,
    ) {
        self.result = Some(ShareResult {
            url,
            pass_code,
            share_text,
            label,
        });
        self.fetched_at = None;
    }

    /// 列表响应(乱序的旧响应直接丢弃)。
    pub(crate) fn on_list(&mut self, req_id: u64, append: bool, list: ShareList) {
        if req_id != self.req {
            return;
        }
        self.loading = false;
        self.fetched_at = Some(Instant::now());
        self.next = list.next_page_token;
        if append {
            let known: HashSet<String> = self.list.iter().map(|s| s.share_id.clone()).collect();
            for s in list.shares {
                if !known.contains(&s.share_id) {
                    self.list.push(s);
                }
            }
        } else {
            self.list = list.shares;
        }
    }

    /// 列表请求失败。
    pub(crate) fn on_list_failed(&mut self, g: &mut Global, what: String) {
        self.loading = false;
        g.toast_err(&what);
    }

    /// 取消分享成功。
    pub(crate) fn on_deleted(&mut self, g: &mut Global, ids: &[String]) {
        self.list.retain(|s| !ids.contains(&s.share_id));
        self.selected.retain(|id| !ids.contains(id));
        self.delete_confirm = None;
        g.toast_ok(&format!("已取消 {} 个分享", ids.len()));
    }

    // ---------- 转存分享: 状态 ----------

    /// 清空转存分享弹窗状态。
    pub(crate) fn clear_save(&mut self) {
        self.save_input.clear();
        self.save_pass_code.clear();
        self.save_resolving = false;
        self.save_id = None;
        self.save_title = None;
        self.save_token = None;
        self.save_files.clear();
        self.save_selected.clear();
        self.save_filter.clear();
        self.save_next = None;
        self.save_loading_more = false;
        self.save_saving = false;
        self.save_error = None;
        self.picker_open = false;
        self.picker_stack = vec![Crumb {
            id: None,
            label: "我的云盘".to_string(),
        }];
        self.picker_folders.clear();
        self.picker_loading = false;
        self.save_dest = None;
    }

    /// 转存分享目录选择器当前所在目录。
    pub(crate) fn picker_parent(&self) -> Option<String> {
        self.picker_stack.last().and_then(|c| c.id.clone())
    }

    /// 打开转存分享「保存到」网盘目录选择器, 从根目录开始。
    pub(crate) fn open_picker(&mut self, g: &mut Global) {
        self.picker_open = true;
        self.picker_stack = vec![Crumb {
            id: None,
            label: "我的云盘".into(),
        }];
        self.picker_list(g);
    }

    /// 请求转存分享选择器当前目录的子文件夹列表。
    pub(crate) fn picker_list(&mut self, g: &mut Global) {
        self.picker_loading = true;
        self.picker_folders.clear();
        self.picker_req += 1;
        let req_id = self.picker_req;
        g.send(Cmd::ListFolders {
            parent: self.picker_parent(),
            req_id,
        });
    }

    /// 该 req_id 是否属于转存分享目录选择器。
    pub(crate) fn handles_picker(&self, req_id: u64) -> bool {
        self.picker_open && req_id == self.picker_req
    }

    /// 目录选择器响应(父目录已不是当前所在目录时丢弃)。
    pub(crate) fn on_folders(&mut self, parent: Option<String>, files: Vec<File>) {
        if parent != self.picker_parent() {
            return;
        }
        self.picker_loading = false;
        self.picker_folders = files;
    }

    // ---------- 转存分享: 消息 ----------

    /// 分享链接解析成功。
    pub(crate) fn on_resolved(
        &mut self,
        share_id: String,
        title: String,
        pass_code_token: String,
        files: Vec<File>,
        next_page_token: Option<String>,
    ) {
        self.save_resolving = false;
        self.save_id = Some(share_id);
        self.save_title = Some(title);
        self.save_token = Some(pass_code_token);
        self.save_files = files;
        self.save_next = next_page_token;
        self.save_selected = HashSet::new();
    }

    /// 分享文件列表加载更多成功。
    pub(crate) fn on_files_loaded(&mut self, files: Vec<File>, next_page_token: Option<String>) {
        self.save_loading_more = false;
        self.save_files.extend(files);
        self.save_next = next_page_token;
    }

    /// 分享文件列表加载更多失败。
    pub(crate) fn on_files_failed(&mut self, what: String) {
        self.save_loading_more = false;
        self.save_error = Some(what);
    }

    /// 分享链接解析失败。
    pub(crate) fn on_resolve_failed(&mut self, what: String) {
        self.save_resolving = false;
        self.save_error = Some(what);
    }

    /// 转存成功: 关闭弹窗; auto_move_failed 时保留目标目录以便重试移动。
    pub(crate) fn on_saved(&mut self, g: &mut Global, auto_move_failed: bool) {
        self.save_saving = false;
        self.save_open = false;
        let dest = self.save_dest.clone();
        if auto_move_failed {
            // 保存失败信息以便重试
            self.save_move_failed = dest.clone();
        }
        self.clear_save();
        if auto_move_failed {
            if let Some((_, name)) = dest {
                g.toast_warn(&format!(
                    "转存成功, 但自动移动到「{name}」失败。文件仍在「转存自分享」中"
                ));
            } else {
                g.toast_warn("转存成功, 但自动移动失败, 请在「转存自分享」中查看");
            }
        } else if dest.is_some() {
            g.toast_ok("转存成功, 文件已移动到目标目录");
        } else {
            g.toast_ok("转存成功, 文件已保存到「转存自分享」");
        }
    }

    /// 转存失败。
    pub(crate) fn on_save_failed(&mut self, what: String) {
        self.save_saving = false;
        self.save_error = Some(what);
    }

    /// 自动移动重试成功(目录重列由调用方负责)。
    pub(crate) fn on_move_retried(&mut self, g: &mut Global) {
        self.save_move_failed = None;
        g.toast_ok("移动成功");
    }

    /// 自动移动重试失败。
    pub(crate) fn on_move_retry_failed(&mut self, g: &mut Global, what: String) {
        g.toast_err(&what);
    }
}
