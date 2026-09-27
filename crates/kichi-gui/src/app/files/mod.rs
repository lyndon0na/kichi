//! 「文件浏览」域: 导航栈 / 目录缓存(含 SWR 校正) / 选中集 / 排序过滤 / 视图与列宽 / 剪贴板。
//!
//! 本模块持有 `FilesPage` 的状态与导航逻辑, 以及渲染入口 `show`(渲染与动作分离,
//! 跨域动作见 [`FilesAction`]); 各渲染角色分别落在 `list` / `grid` / `row` / `toolbar`。

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use eframe::egui::{self, Frame, Key, Margin, Pos2, RichText, Stroke, UiBuilder};

use kichi_core::types::{File, FileList};

use crate::filetypes;
use crate::msg::{Cmd, QualityOption};
use crate::theme::Theme;

use super::global::Global;
use super::preview::PreviewPage;
use super::search::SearchPage;
use super::thumbs::ThumbsPage;

pub(super) mod grid;
pub(super) mod list;
pub(super) mod row;
pub(super) mod toolbar;

/// 目录缓存新鲜期: 命中后超过该时长, 先展示旧数据再后台静默校正。
const DIR_TTL: Duration = Duration::from_secs(60);
/// 目录缓存上限, 超出按 LRU 淘汰(不淘汰当前目录)。
const DIR_CACHE_CAP: usize = 64;

#[derive(PartialEq, Eq, Clone, Copy, PartialOrd, Ord, Hash)]
pub(crate) enum SortBy {
    Name,
    Size,
    Modified,
}

#[derive(Clone)]
pub(crate) struct Crumb {
    pub id: Option<String>,
    pub label: String,
}

/// 目录缓存条目: 某目录已加载的文件列表与分页游标。
pub(crate) struct DirEntry {
    pub files: Vec<File>,
    pub next_token: Option<String>,
    /// 最近一次写入对应的请求 id, 用于丢弃乱序到达的旧响应。
    pub req: u64,
    /// 最近一次成功加载的时间, 用于 TTL 新鲜度判定。
    pub fetched_at: Instant,
    /// 最近一次访问时间, 用于 LRU 淘汰。
    pub last_used: Instant,
}

impl Default for DirEntry {
    fn default() -> Self {
        let now = Instant::now();
        Self {
            files: Vec::new(),
            next_token: None,
            req: 0,
            fetched_at: now,
            last_used: now,
        }
    }
}

#[derive(Clone)]
pub(crate) enum RowAction {
    OpenFolder(String, String),
    /// 打开/播放(音视频为原画流式, 其他为下载后用系统查看器)。
    OpenFile(String, String),
    /// 确保某媒体文件的可用清晰度已解析(供「播放」子菜单展示)。
    FetchQualities(String, String),
    /// 用已解析出的某个清晰度播放。
    PlayOption(String, crate::msg::QualityOption),
    DownloadFile(String, String),
    /// 递归下载整个云端目录 (folder_id, 目录名)。
    DownloadFolder(String, String),
    CopyName(String),
    Rename(String, String),
    CopyItem(String),
    CutItem(String),
    /// 为该项(若在多选内则为整个选中集)创建分享。
    Share(String),
    PasteInto(String),
    Trash(String),
}

/// 文件列表视图模式。
#[derive(PartialEq, Eq, Clone, Copy)]
pub(crate) enum ViewMode {
    /// 列表视图(表格样式)。
    List,
    /// 图标视图(网格缩略图)。
    Icon,
}

/// 剪贴板操作类型。
#[derive(PartialEq, Eq, Clone, Copy)]
pub(crate) enum ClipKind {
    /// 复制: 粘贴后保留剪贴板内容, 可继续粘贴到别处。
    Copy,
    /// 剪切: 粘贴成功后清空剪贴板。
    Cut,
}

/// 内部文件剪贴板: 复制/剪切选中项, 切换到目标目录后粘贴。
#[derive(Clone)]
pub(crate) struct Clipboard {
    pub kind: ClipKind,
    pub ids: Vec<String>,
    /// 源目录, 用于剪切时判断目标是否与原目录相同。
    pub src_parent: Option<String>,
    /// 展示用描述(单文件为文件名, 多项为 "N 项")。
    pub label: String,
}

/// 列拖拽状态。
pub(crate) struct ColDrag {
    /// 拖拽的是哪条分割线 (1 = 名称|大小, 2 = 大小|修改时间)
    pub handle: u8,
    /// 拖拽开始时鼠标 x
    pub start_x: f32,
    /// 拖拽开始时 col_size_w
    pub orig_size_w: f32,
    /// 拖拽开始时 col_time_w
    pub orig_time_w: f32,
}

/// 文件浏览状态(导航栈 / 列表数据 / 目录缓存 / 选中集 / 排序过滤 / 视图与列宽 / 剪贴板)。
pub(crate) struct FilesPage {
    /// 导航栈; 首项固定为根目录。
    pub(crate) stack: Vec<Crumb>,
    /// 选中项 id 集合。
    pub(crate) selected: HashSet<String>,
    pub(crate) sort_by: SortBy,
    pub(crate) sort_desc: bool,
    /// 本地过滤关键词(仅当前目录; 全局搜索走 `SearchPage`)。
    pub(crate) filter: String,
    /// 当前目录的列表数据(过滤/排序前的原始顺序)。
    pub(crate) items: Vec<File>,
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

    /// 视图模式(列表 / 图标)。
    pub(crate) view_mode: ViewMode,
    /// 网格视图卡片大小(80-160)。
    pub(crate) grid_card_size: f32,

    /// 等待服务端列表同步、本地先行隐藏的 id -> 其应隐藏的目录(回收/移出的源目录)。
    /// 用目录区分, 避免移动后的文件在目标目录里也被隐藏。
    pub(crate) hidden: HashMap<String, Option<String>>,
    /// 移动/复制成功后延迟重列目录的时间点(规避服务端列表最终一致性)。
    pub(crate) relist_at: Option<Instant>,
    /// 复制/剪切剪贴板(在目标目录粘贴)。
    pub(crate) clipboard: Option<Clipboard>,
}

impl Default for FilesPage {
    fn default() -> Self {
        Self {
            stack: vec![Crumb {
                id: None,
                label: "我的云盘".into(),
            }],
            selected: HashSet::new(),
            sort_by: SortBy::Name,
            sort_desc: false,
            filter: String::new(),
            items: Vec::new(),
            dir_next: None,
            dir_loading: false,
            dir_cache: HashMap::new(),
            dir_inflight: HashMap::new(),
            col_size_w: 100.0,
            col_time_w: 160.0,
            col_dragging: None,
            view_mode: ViewMode::List,
            grid_card_size: 104.0,
            hidden: HashMap::new(),
            relist_at: None,
            clipboard: None,
        }
    }
}

impl FilesPage {
    // ---------- 会话 ----------

    /// 会话失效: 作废目录缓存与剪贴板。
    pub(crate) fn invalidate_session(&mut self) {
        self.clipboard = None;
        self.dir_cache.clear();
        self.dir_inflight.clear();
    }

    /// 退出登录: 清空浏览状态。
    pub(crate) fn clear(&mut self) {
        self.invalidate_session();
        self.items.clear();
        self.selected.clear();
        self.hidden.clear();
        self.reset_stack();
    }

    // ---------- 导航与目录数据 ----------

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

    pub(crate) fn reset_stack(&mut self) {
        self.stack = vec![Crumb {
            id: None,
            label: "我的云盘".into(),
        }];
        self.dir_next = None;
        self.dir_loading = true;
    }

    pub(crate) fn reset_browse(&mut self, g: &mut Global) {
        self.dir_cache.clear();
        self.dir_inflight.clear();
        self.reset_stack();
        self.selected.clear();
        self.fetch_dir(g, None);
    }

    /// 发送一次 ListFiles 并登记在途请求(首屏才登记, 用于去重)。
    fn send_list(
        &mut self,
        g: &mut Global,
        parent: Option<String>,
        token: Option<String>,
        append: bool,
    ) {
        let req_id = g.alloc_req_id();
        if !append {
            self.dir_inflight.insert(parent.clone(), req_id);
        }
        g.send(Cmd::ListFiles {
            parent,
            token,
            append,
            req_id,
        });
    }

    /// 冷加载: 清空视图并显示加载态。
    fn fetch_dir(&mut self, g: &mut Global, parent: Option<String>) {
        self.items.clear();
        self.dir_next = None;
        self.dir_loading = true;
        self.send_list(g, parent, None, false);
    }

    /// 静默校正: 保留当前视图(继续展示旧数据), 仅后台刷新缓存。
    /// 该目录已有在途请求时跳过, 避免重复请求。
    fn revalidate_dir(&mut self, g: &mut Global, parent: Option<String>) {
        if self.dir_inflight.contains_key(&parent) {
            return;
        }
        self.send_list(g, parent, None, false);
    }

    /// 打开当前目录。命中且新鲜则同帧渲染、零请求; 命中但过期先展示旧数据
    /// 再后台静默校正; 未命中才冷加载。
    pub(crate) fn show_dir(&mut self, g: &mut Global) {
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
                self.items = files;
                self.dir_next = next;
                self.dir_loading = false;
                if let Some(entry) = self.dir_cache.get_mut(&parent) {
                    entry.last_used = Instant::now();
                }
                if age > DIR_TTL {
                    self.revalidate_dir(g, parent);
                }
            }
            None if self.dir_inflight.contains_key(&parent) => {
                // 已有请求在途: 保持加载态等待, 不重复发起。
                self.items.clear();
                self.dir_next = None;
                self.dir_loading = true;
            }
            None => self.fetch_dir(g, parent),
        }
    }

    /// 强制重新加载当前目录(F5 / 刷新按钮), 清空视图并显示加载态。
    pub(crate) fn refresh_dir(&mut self, g: &mut Global) {
        let parent = self.current_parent();
        self.dir_cache.remove(&parent);
        self.selected.clear();
        self.fetch_dir(g, parent);
    }

    /// 变更后就地作废缓存并静默重列当前目录: 保留现有列表(避免闪烁),
    /// 后台重新拉取以反映增删改。
    pub(crate) fn reload_dir(&mut self, g: &mut Global) {
        let parent = self.current_parent();
        self.dir_cache.remove(&parent);
        self.selected.clear();
        self.send_list(g, parent, None, false);
    }

    /// 加载当前目录的下一页。
    pub(crate) fn load_more(&mut self, g: &mut Global) {
        if self.dir_next.is_none() {
            return;
        }
        let parent = self.current_parent();
        let token = self.dir_next.clone();
        self.send_list(g, parent, token, true);
    }

    pub(crate) fn goto_folder(
        &mut self,
        g: &mut Global,
        search: &mut SearchPage,
        id: &str,
        name: &str,
    ) {
        self.exit_search(search);
        self.stack.push(Crumb {
            id: Some(id.to_string()),
            label: name.to_string(),
        });
        self.show_dir(g);
    }

    /// 退出搜索模式: 清空关键词与本地过滤。
    pub(crate) fn exit_search(&mut self, search: &mut SearchPage) {
        search.exit();
        self.filter.clear();
    }

    /// 作废目录缓存(回收站还原可能落回原目录)。
    pub(crate) fn invalidate_cache(&mut self) {
        self.dir_cache.clear();
    }

    /// 当前目录内过滤后的可见文件(文件夹在前, 组内按当前排序)。
    pub(crate) fn visible_rows(&self, search: &SearchPage) -> (Vec<File>, Vec<File>) {
        // 搜索模式下使用搜索结果
        let source: &[File] = if search.is_active() {
            search.results()
        } else {
            &self.items
        };

        let kw = if search.is_active() {
            String::new() // 搜索结果已经过滤过了
        } else {
            self.filter.trim().to_lowercase()
        };
        let cur = self.current_parent();
        let mut folders: Vec<&File> = Vec::new();
        let mut plain: Vec<&File> = Vec::new();
        for f in source {
            if !search.is_active() && self.hidden.get(&f.id).is_some_and(|hp| *hp == cur) {
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
        self.items
            .iter()
            .filter(|f| self.selected.contains(&f.id))
            .map(|f| (f.id.clone(), f.name.clone()))
            .collect()
    }

    /// 选中项里可下载的文件(id, name), 文件夹除外。
    pub(crate) fn selected_plain_files(&self) -> Vec<(String, String)> {
        self.items
            .iter()
            .filter(|f| self.selected.contains(&f.id) && !f.is_folder())
            .map(|f| (f.id.clone(), f.name.clone()))
            .collect()
    }

    /// 选中项里的文件夹(id, name), 用于整目录下载。
    pub(crate) fn selected_folders(&self) -> Vec<(String, String)> {
        self.items
            .iter()
            .filter(|f| self.selected.contains(&f.id) && f.is_folder())
            .map(|f| (f.id.clone(), f.name.clone()))
            .collect()
    }

    // ---------- 复制/剪切/粘贴 ----------

    /// 把当前选中项放入剪贴板。
    pub(crate) fn clip_selection(&mut self, g: &mut Global, kind: ClipKind) {
        let ids: Vec<String> = self
            .items
            .iter()
            .filter(|f| self.selected.contains(&f.id))
            .map(|f| f.id.clone())
            .collect();
        if ids.is_empty() {
            g.toast_warn("请先选择要操作的文件");
            return;
        }
        self.set_clipboard(g, kind, ids);
    }

    /// 右键单项: 若该项在多选中则操作整个选中集, 否则仅操作该项。
    pub(crate) fn clip_item(&mut self, g: &mut Global, kind: ClipKind, id: String) {
        let ids = if self.selected.contains(&id) && self.selected.len() > 1 {
            self.items
                .iter()
                .filter(|f| self.selected.contains(&f.id))
                .map(|f| f.id.clone())
                .collect()
        } else {
            vec![id]
        };
        self.set_clipboard(g, kind, ids);
    }

    fn set_clipboard(&mut self, g: &mut Global, kind: ClipKind, ids: Vec<String>) {
        let label = if ids.len() == 1 {
            self.items
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
        g.toast_ok(&format!("已{act}「{label}」, 进入目标目录后粘贴"));
    }

    /// 粘贴到当前目录。
    pub(crate) fn paste_clipboard(&mut self, g: &mut Global) {
        let dest = self.current_parent();
        self.paste_into(g, dest);
    }

    /// 粘贴到指定目录(None = 根目录)。
    pub(crate) fn paste_into(&mut self, g: &mut Global, dest: Option<String>) {
        let Some(clip) = self.clipboard.clone() else {
            g.toast_warn("剪贴板为空, 请先复制或剪切");
            return;
        };
        if let Some(d) = &dest {
            if clip.ids.contains(d) {
                g.toast_warn("不能粘贴到被操作的文件夹自身");
                return;
            }
        }
        if clip.kind == ClipKind::Cut && dest == clip.src_parent {
            g.toast_warn("已在原目录, 无需粘贴");
            return;
        }
        match clip.kind {
            ClipKind::Copy => g.send(Cmd::CopyTo {
                ids: clip.ids,
                dest,
            }),
            ClipKind::Cut => {
                g.send(Cmd::MoveTo {
                    ids: clip.ids,
                    dest,
                    src: clip.src_parent,
                });
                // 剪切只生效一次, 粘贴后清空剪贴板。
                self.clipboard = None;
            }
        }
    }

    // ---------- 消息 ----------

    /// 把一次 ListFiles 响应写入缓存, 并在其属于当前目录时同步到可见列表。
    /// `entry.req` 保证乱序到达的旧响应不会覆盖新数据。
    pub(crate) fn on_files(
        &mut self,
        parent: Option<String>,
        req_id: u64,
        append: bool,
        list: FileList,
        thumbs: &mut ThumbsPage,
    ) {
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
                    self.items = entry.files.clone();
                    self.dir_next = entry.next_token.clone();
                }
            }
            if !append {
                self.selected.clear();
                // 换目录 / 刷新 = 重新开始: 清掉在途与失败的缩略图登记,
                // 让重新可见的文件有机会再请求一次(worker 侧也已作废旧任务)。
                thumbs.reset();
            }
        }

        // 服务端列表已更新: 解除该目录中不再出现的隐藏项(最终一致性收敛)。
        if !self.hidden.is_empty() {
            let present: HashSet<String> = if is_current {
                self.items.iter().map(|f| f.id.clone()).collect()
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

    /// 当前目录列表加载失败: 结束加载态并释放在途登记。
    pub(crate) fn on_files_failed(&mut self, g: &mut Global, parent: Option<String>, what: String) {
        // 若仍停留在该目录, 无缓存数据时会显示空目录提示, 而非一直转圈。
        self.dir_inflight.remove(&parent);
        if parent == self.current_parent() {
            self.dir_loading = false;
        }
        g.toast_err(&what);
    }

    /// 移动成功: 源目录就地移除并作废目标目录缓存, 稍后重列。
    pub(crate) fn on_moved(
        &mut self,
        g: &mut Global,
        ids: &[String],
        src: &Option<String>,
        dest: &Option<String>,
    ) {
        g.toast_ok("移动成功");
        // batchMove 返回后服务端列表未必立即同步, 先把被移走的项从源
        // 目录隐藏(服务端列表不再含该 id 后自动解除), 避免刷新前文件仍
        // 显示在原目录; 隐藏按源目录区分, 目标目录里仍会正常显示。
        for id in ids {
            self.selected.remove(id);
            self.hidden.insert(id.clone(), src.clone());
        }
        // 同步更新源目录缓存, 并对目标目录作废缓存, 稍后重列。
        if let Some(entry) = self.dir_cache.get_mut(src) {
            entry.files.retain(|f| !ids.contains(&f.id));
        }
        self.items.retain(|f| !ids.contains(&f.id));
        self.dir_cache.remove(dest);
        self.relist_at = Some(Instant::now() + Duration::from_millis(1500));
    }

    /// 复制成功: 目标目录当前可能正在展示, 稍后重列以显示新文件。
    pub(crate) fn on_copied(&mut self, g: &mut Global, dest: Option<String>) {
        g.toast_ok("复制成功");
        self.dir_cache.remove(&dest);
        self.relist_at = Some(Instant::now() + Duration::from_millis(1500));
    }

    /// 移入回收站成功: 清空选中并静默重列当前目录。
    pub(crate) fn on_trashed(&mut self, g: &mut Global) {
        self.selected.clear();
        self.reload_dir(g);
    }

    /// 移动/复制成功后的延迟重列(服务端列表存在最终一致性)。
    /// 用静默校正而非强制刷新, 避免清空列表导致闪烁。
    pub(crate) fn poll_relist(&mut self, ctx: &egui::Context, g: &mut Global) {
        let Some(at) = self.relist_at else {
            return;
        };
        if Instant::now() >= at {
            self.relist_at = None;
            let parent = self.current_parent();
            self.revalidate_dir(g, parent);
        } else {
            ctx.request_repaint_after(Duration::from_millis(150));
        }
    }

    /// 当前列表里某文件的类型(查不到条目时按文件名回退)。
    pub(crate) fn file_type(&self, id: &str, name: &str) -> filetypes::FileType {
        match self.items.iter().find(|f| f.id == id) {
            Some(f) => filetypes::classify_file(f),
            None => filetypes::classify(name, None),
        }
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
}

/// 文件页渲染产生的跨域动作。
///
/// 渲染与动作分离: `FilesPage::show` 只读写自身状态, 其余(B 域状态 / 弹窗 /
/// 传输队列)一律 push 进本列表, 由 `App` 在本帧渲染结束后按序执行。
pub(crate) enum FilesAction {
    /// 打开「新建文件夹」弹窗。
    NewFolder,
    /// 打开「重命名」弹窗。
    Rename { id: String, name: String },
    /// 打开「移入回收站」确认弹窗(单个或多选)。
    ConfirmTrash(Vec<(String, String)>),
    /// 预览 / 播放某个云端文件。
    Preview { id: String, name: String },
    /// 解析某文件的可用清晰度(「播放」子菜单展开时)。
    FetchQualities { id: String, name: String },
    /// 用指定清晰度播放。
    PlayOption { id: String, opt: QualityOption },
    /// 把选中的文件与文件夹加入下载队列(默认目录不可用时先让用户选)。
    DownloadSelection {
        files: Vec<(String, String)>,
        folders: Vec<(String, String)>,
    },
    /// 下载单个文件。
    DownloadFile { id: String, name: String },
    /// 下载单个文件夹。
    DownloadFolder { id: String, name: String },
    /// 分享选中的多项。
    ShareSelection,
    /// 分享单项。
    ShareItem(String),
    /// 上传本地文件到当前目录。
    UploadFiles,
    /// 上传本地文件夹到当前目录。
    UploadFolder,
    /// 触发全局搜索。
    Search,
    /// 加载更多搜索结果。
    LoadMoreSearch,
}

/// 文件页布局常量与渲染入口。
impl FilesPage {
    const NAME_X: f32 = 54.0; // 复选框(8+16=24) + 图标(38+11=49) + 间距
    const RIGHT_PAD: f32 = 8.0;
    const MIN_SIZE_W: f32 = 60.0;
    const MIN_TIME_W: f32 = 80.0;
    /// 标题栏统一行高: 选中态与普通态使用同一高度, 避免切换时列表位移。
    const HEAD_H: f32 = 44.0;

    /// 文件页入口: 顶部栏 + 列表 / 网格 + 空态与加载更多。
    /// 返回本帧产生的跨域动作, 由 `App` 按序执行。
    pub(crate) fn show(
        &mut self,
        ctx: &egui::Context,
        th: &Theme,
        g: &mut Global,
        search: &mut SearchPage,
        thumbs: &mut ThumbsPage,
        preview: &PreviewPage,
    ) -> Vec<FilesAction> {
        // 处理列拖拽 (在渲染之前, 确保 header 和 rows 看到一致的列宽)
        if let Some(drag) = &self.col_dragging {
            let delta = ctx.input(|i| {
                i.pointer.hover_pos().map(|p| p.x).unwrap_or(drag.start_x) - drag.start_x
            });
            match drag.handle {
                1 => {
                    self.col_size_w = (drag.orig_size_w - delta).max(Self::MIN_SIZE_W);
                }
                2 => {
                    self.col_time_w = (drag.orig_time_w - delta).max(Self::MIN_TIME_W);
                }
                _ => {}
            }
            if ctx.input(|i| i.pointer.any_released()) {
                self.col_dragging = None;
            }
            ctx.request_repaint();
        }

        let mut trigger_search = false;
        let mut key_trash: Option<Vec<(String, String)>> = None;
        // 搜索框 Enter 键检测（在 TextEdit 消费事件之前检查）
        let search_focused = ctx.memory(|m| m.has_focus(egui::Id::new("file_search")));
        if search_focused {
            ctx.input(|i| {
                if i.key_pressed(Key::Enter) {
                    trigger_search = true;
                }
            });
        }

        // 键盘快捷键
        if !ctx.wants_keyboard_input() {
            ctx.input(|i| {
                if i.key_pressed(Key::F5) {
                    self.refresh_dir(g);
                }
                if i.key_pressed(Key::A) && i.modifiers.ctrl {
                    let (folders, plain) = self.visible_rows(search);
                    self.selected = folders
                        .iter()
                        .chain(plain.iter())
                        .map(|f| f.id.clone())
                        .collect();
                }
                if (i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace))
                    && !self.selected.is_empty()
                {
                    let sel = self.selected_names();
                    if !sel.is_empty() {
                        key_trash = Some(sel);
                    }
                }
                if i.key_pressed(Key::C) && i.modifiers.ctrl {
                    self.clip_selection(g, ClipKind::Copy);
                }
                if i.key_pressed(Key::X) && i.modifiers.ctrl {
                    self.clip_selection(g, ClipKind::Cut);
                }
                if i.key_pressed(Key::V) && i.modifiers.ctrl {
                    self.paste_clipboard(g);
                }
                if i.key_pressed(Key::Escape) && !self.selected.is_empty() {
                    self.selected.clear();
                }
            });
        }

        // Ctrl + 滚轮调整网格视图大小
        if matches!(self.view_mode, ViewMode::Icon) {
            ctx.input(|i| {
                for event in &i.events {
                    if let egui::Event::MouseWheel {
                        modifiers, delta, ..
                    } = event
                    {
                        if modifiers.ctrl && delta.y.abs() > 0.0 {
                            let step = if delta.y > 0.0 { 5.0 } else { -5.0 };
                            self.grid_card_size = (self.grid_card_size + step)
                                .clamp(grid::GRID_CARD_MIN, grid::GRID_CARD_MAX);
                        }
                    }
                }
            });
        }

        // 各菜单/操作栏产生的行级操作, 统一在最后处理。
        let mut actions: Vec<RowAction> = Vec::new();
        // 本帧产生的跨域动作, 按产生顺序交给 `App` 执行。
        let mut effects: Vec<FilesAction> = Vec::new();

        // 当前可见行统计(过滤后)与选中项信息, 供顶部栏与选中栏使用。
        let (folders, plain) = self.visible_rows(search);
        let visible_total = folders.len() + plain.len();
        let sel_meta = self.selected_names();
        let dl_candidates = self.selected_plain_files();
        let dl_folders = self.selected_folders();
        let clip_info = self
            .clipboard
            .as_ref()
            .map(|c| (c.label.clone(), c.ids.len()));

        let req = self.top_bar(
            ctx,
            toolbar::TopBar {
                th,
                g,
                search,
                preview,
                visible_total,
                sel_meta: &sel_meta,
                dl_files: &dl_candidates,
                dl_folders: &dl_folders,
                clip_info,
                actions: &mut actions,
            },
        );

        if req.up {
            self.exit_search(search);
            self.stack.pop();
            self.show_dir(g);
        }
        if let Some(i) = req.jumped {
            self.exit_search(search);
            self.stack.truncate(i + 1);
            self.show_dir(g);
        }
        // 键盘 Delete 先于顶部栏动作生效(与旧实现的事件顺序一致)。
        if let Some(sel) = key_trash {
            effects.push(FilesAction::ConfirmTrash(sel));
        }
        if req.refresh {
            self.refresh_dir(g);
        }
        if req.upload {
            effects.push(FilesAction::UploadFiles);
        }
        if req.upload_dir {
            effects.push(FilesAction::UploadFolder);
        }

        // 处理顶部栏产生的操作
        if req.clear_clip {
            self.clipboard = None;
        }
        if req.ask_rename && sel_meta.len() == 1 {
            effects.push(FilesAction::Rename {
                id: sel_meta[0].0.clone(),
                name: sel_meta[0].1.clone(),
            });
        }
        if req.ask_preview && sel_meta.len() == 1 {
            effects.push(FilesAction::Preview {
                id: sel_meta[0].0.clone(),
                name: sel_meta[0].1.clone(),
            });
        }
        if req.ask_trash {
            effects.push(FilesAction::ConfirmTrash(sel_meta.clone()));
        }
        if req.ask_share {
            effects.push(FilesAction::ShareSelection);
        }
        if trigger_search {
            effects.push(FilesAction::Search);
        }

        if req.want_download && (!dl_candidates.is_empty() || !dl_folders.is_empty()) {
            effects.push(FilesAction::DownloadSelection {
                files: dl_candidates.clone(),
                folders: dl_folders.clone(),
            });
        }

        // -------- 文件列表 --------
        let mut open_folder: Option<(String, String)> = None;
        let mut sel_reqs: Vec<String> = Vec::new();

        egui::CentralPanel::default()
            .frame(Frame::new().fill(th.card).inner_margin(Margin {
                left: 20,
                right: 20,
                top: 4,
                bottom: 12,
            }))
            .show(ctx, |ui| {
                if self.dir_loading && self.items.is_empty() {
                    ui.vertical_centered(|ui| {
                        ui.add_space(90.0);
                        ui.spinner();
                        ui.add_space(8.0);
                        ui.label(RichText::new("加载中…").color(th.text_weak));
                    });
                    return;
                }

                // Breeze/Dolphin 风格: 视图区直接平铺, 不再套外层卡片与边框。
                let inner = ui.max_rect();
                let mut inner_ui = ui.new_child(UiBuilder::new().max_rect(inner));

                inner_ui.add_space(6.0);
                // 列表视图时显示表头
                if self.view_mode == ViewMode::List {
                    self.file_list_header(&mut inner_ui, th, inner.width(), search);
                    let sep_y = inner_ui.cursor().min.y;
                    ui.painter().line_segment(
                        [
                            Pos2::new(inner.min.x + 6.0, sep_y + 3.0),
                            Pos2::new(inner.max.x - 6.0, sep_y + 3.0),
                        ],
                        Stroke::new(1.0_f32, th.border),
                    );
                }

                let list_avail_h = inner.max.y - (inner_ui.cursor().min.y) - 4.0;

                egui::ScrollArea::vertical()
                    .id_salt("file_list_scroll")
                    .auto_shrink([false, false])
                    .max_height(list_avail_h.max(40.0))
                    .show(&mut inner_ui, |ui| {
                        ui.set_min_height(list_avail_h.max(40.0));
                        ui.set_width(inner.width());

                        // 目录空白处右键菜单(粘贴 / 新建文件夹 / 刷新)。先于行注册,
                        // 后续行的右键菜单优先级更高, 空白处才会落到这里。
                        let bg_resp = ui.interact(
                            ui.clip_rect(),
                            ui.id().with("file_list_bg"),
                            egui::Sense::click(),
                        );
                        let has_clip = self.clipboard.is_some();
                        bg_resp.context_menu(|ui| {
                            if has_clip && ui.button("粘贴").clicked() {
                                self.paste_clipboard(g);
                                ui.close_menu();
                            }
                            if ui.button("上传文件").clicked() {
                                effects.push(FilesAction::UploadFiles);
                                ui.close_menu();
                            }
                            if ui.button("上传文件夹").clicked() {
                                effects.push(FilesAction::UploadFolder);
                                ui.close_menu();
                            }
                            if ui.button("新建文件夹").clicked() {
                                effects.push(FilesAction::NewFolder);
                                ui.close_menu();
                            }
                            if ui.button("刷新").clicked() {
                                self.refresh_dir(g);
                                ui.close_menu();
                            }
                        });

                        let all_files: Vec<&File> = folders.iter().chain(plain.iter()).collect();

                        if self.view_mode == ViewMode::List {
                            self.list_rows(
                                ui,
                                list::ListCtx {
                                    th,
                                    preview,
                                    files: &all_files,
                                    has_clip,
                                    col_w: inner.width(),
                                    actions: &mut actions,
                                    sel_reqs: &mut sel_reqs,
                                },
                            );
                        } else {
                            self.grid_cards(
                                ui,
                                grid::GridCtx {
                                    th,
                                    g,
                                    thumbs,
                                    preview,
                                    files: &all_files,
                                    avail_w: inner.width(),
                                    has_clip,
                                    actions: &mut actions,
                                    sel_reqs: &mut sel_reqs,
                                },
                            );
                        }

                        // 纹理上限淘汰放在绘制之后: 本帧点亮过的(可见 ± 一屏)还在
                        // 宽限期内受保护, 只有久未露面且超限的纹理在此释放显存。
                        thumbs.textures.evict(Instant::now());

                        // 加载更多按钮
                        let has_more = if search.is_active() {
                            search.has_more()
                        } else {
                            self.dir_next.is_some()
                        };
                        if has_more {
                            ui.add_space(4.0);
                            ui.vertical_centered(|ui| {
                                let btn_text = if search.is_loading() {
                                    "搜索中..."
                                } else {
                                    "加载更多"
                                };
                                if ui
                                    .add_enabled(!search.is_loading(), egui::Button::new(btn_text))
                                    .clicked()
                                {
                                    if search.is_active() {
                                        effects.push(FilesAction::LoadMoreSearch);
                                    } else {
                                        self.load_more(g);
                                    }
                                }
                            });
                            ui.add_space(2.0);
                        }
                        // 空状态提示
                        if search.is_active() {
                            if search.is_empty() && !search.is_loading() {
                                ui.add_space((list_avail_h * 0.3).max(20.0));
                                ui.vertical_centered(|ui| {
                                    ui.label(
                                        RichText::new(format!(
                                            "没有找到匹配「{}」的文件",
                                            search.keyword()
                                        ))
                                        .color(th.text_weak),
                                    );
                                });
                            }
                        } else if self.items.is_empty() && !self.dir_loading {
                            ui.add_space((list_avail_h * 0.3).max(20.0));
                            ui.vertical_centered(|ui| {
                                ui.label(RichText::new("此文件夹为空").color(th.text_weak));
                            });
                        } else if !self.filter.is_empty() && folders.is_empty() && plain.is_empty()
                        {
                            ui.add_space((list_avail_h * 0.3).max(20.0));
                            ui.vertical_centered(|ui| {
                                ui.label(
                                    RichText::new(format!("没有匹配「{}」的文件", self.filter))
                                        .color(th.text_weak),
                                );
                            });
                        }

                        // 处理复选框勾选请求
                        for id in sel_reqs {
                            if self.selected.contains(&id) {
                                self.selected.remove(&id);
                            } else {
                                self.selected.insert(id);
                            }
                        }
                    });

                for action in actions {
                    match action {
                        RowAction::OpenFolder(id, name) => open_folder = Some((id, name)),
                        RowAction::OpenFile(id, name) => {
                            effects.push(FilesAction::Preview { id, name });
                        }
                        RowAction::FetchQualities(id, name) => {
                            effects.push(FilesAction::FetchQualities { id, name });
                        }
                        RowAction::PlayOption(id, opt) => {
                            effects.push(FilesAction::PlayOption { id, opt });
                        }
                        RowAction::DownloadFile(id, name) => {
                            effects.push(FilesAction::DownloadFile { id, name });
                        }
                        RowAction::DownloadFolder(id, name) => {
                            effects.push(FilesAction::DownloadFolder { id, name });
                        }
                        RowAction::CopyName(name) => {
                            let ctx2 = ctx.clone();
                            ctx2.copy_text(name);
                            g.toast_ok("已复制名称");
                        }
                        RowAction::Rename(id, name) => {
                            effects.push(FilesAction::Rename { id, name });
                        }
                        RowAction::CopyItem(id) => {
                            self.clip_item(g, ClipKind::Copy, id);
                        }
                        RowAction::CutItem(id) => {
                            self.clip_item(g, ClipKind::Cut, id);
                        }
                        RowAction::Share(id) => {
                            effects.push(FilesAction::ShareItem(id));
                        }
                        RowAction::PasteInto(id) => {
                            self.paste_into(g, Some(id));
                        }
                        RowAction::Trash(id) => {
                            if let Some(name) = self
                                .items
                                .iter()
                                .find(|f| f.id == id)
                                .map(|f| f.name.clone())
                            {
                                effects.push(FilesAction::ConfirmTrash(vec![(id, name)]));
                            }
                        }
                    }
                }
            });

        if let Some((id, name)) = open_folder {
            self.goto_folder(g, search, &id, &name);
        }

        effects
    }
}
