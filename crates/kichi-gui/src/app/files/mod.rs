//! 「文件浏览」域: 导航栈 / 目录缓存(含 SWR 校正) / 选中集 / 排序过滤 / 视图与列宽 / 剪贴板。
//!
//! 页面渲染留在 `files_page.rs`(继续按渲染角色拆进 `app/files/`, 见 TODO.md 的 P2-11);
//! 本模块负责文件列表数据、目录缓存与导航状态。

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use eframe::egui::{
    self, pos2, vec2, Align, Color32, FontId, Frame, Key, Layout, Margin, Pos2, Rect, RichText,
    Stroke, UiBuilder,
};

use kichi_core::types::{File, FileList};

use crate::filetypes::{self, file_visual, FileType, PreviewKind};
use crate::icons::{self, Glyph};
use crate::msg::{Cmd, QualityOption};
use crate::theme::{mix, Theme};

use super::global::Global;
use super::helpers::truncate_text;
use super::preview::PreviewPage;
use super::search::SearchPage;
use super::thumbs::ThumbsPage;
use super::types::{
    ClipKind, Clipboard, ColDrag, Crumb, DirEntry, QualityMenuState, RowAction, SortBy, ViewMode,
};

pub(super) mod grid;
pub(super) mod row;
pub(super) mod toolbar;

/// 目录缓存新鲜期: 命中后超过该时长, 先展示旧数据再后台静默校正。
const DIR_TTL: Duration = Duration::from_secs(60);
/// 目录缓存上限, 超出按 LRU 淘汰(不淘汰当前目录)。
const DIR_CACHE_CAP: usize = 64;

/// 文件浏览状态(导航栈 / 列表数据 / 目录缓存 / 选中集 / 排序过滤 / 视图与列宽 / 剪贴板)。
pub(crate) struct FilesPage {
    /// 导航栈; 首项固定为根目录。
    pub(crate) stack: Vec<Crumb>,
    /// 列表请求 id 分配器(与传输任务 id 共用命名空间, 由 `App::restore_req_id` 修正起点)。
    pub(crate) req_id: u64,
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
            req_id: 0,
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
        self.req_id += 1;
        let req_id = self.req_id;
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

    /// 布局坐标。返回 (name_x, size_left, time_left)。
    /// 名称列占据剩余空间; size_w / time_w 来自自身状态。
    fn col_layout(&self, w: f32) -> (f32, f32, f32) {
        let time_left = (w - self.col_time_w - Self::RIGHT_PAD).max(Self::NAME_X + 60.0);
        let size_left = (time_left - self.col_size_w - 16.0).max(Self::NAME_X + 60.0);
        (Self::NAME_X, size_left, time_left)
    }

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

        let mut up = false;
        let mut jumped: Option<usize> = None;
        let mut mkdir = false;
        let mut upload = false;
        let mut upload_dir = false;
        let mut refresh = false;
        let mut want_download = false;
        let mut clear_clip = false;
        let mut ask_rename = false;
        let mut ask_preview = false;
        let mut ask_trash = false;
        let mut ask_share = false;
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

        // -------- 顶部: 面包屑 + 搜索 + 操作 --------
        egui::TopBottomPanel::top("file_head")
            .frame(Frame::new().fill(th.bg).inner_margin(Margin {
                left: 20,
                right: 20,
                top: 14,
                bottom: 10,
            }))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.set_min_height(Self::HEAD_H);

                    // 左侧区域限制在右侧控件之外, 面包屑/计数超长时截断, 避免溢出重叠。
                    let right_reserve = 360.0;
                    let left_w = (ui.available_width() - right_reserve).max(140.0);
                    ui.allocate_ui_with_layout(
                        vec2(left_w, Self::HEAD_H),
                        Layout::left_to_right(Align::Center),
                        |ui| {
                            let at_root = self.stack.len() <= 1;
                            let (r, rresp) =
                                ui.allocate_exact_size(vec2(30.0, 30.0), egui::Sense::click());
                            ui.painter().rect_filled(
                                r,
                                th.cr(8),
                                if rresp.hovered() && !at_root {
                                    th.hover
                                } else {
                                    egui::Color32::TRANSPARENT
                                },
                            );
                            icons::paint(
                                ui.painter(),
                                r.shrink(6.0),
                                Glyph::Up,
                                if at_root { th.text_faint } else { th.text_weak },
                            );
                            if rresp.clicked() && !at_root {
                                up = true;
                            }
                            rresp.clone().on_hover_text("返回上级");
                            ui.add_space(4.0);

                            let count_reserve = 76.0;
                            let clip_reserve = if clip_info.is_some() { 150.0 } else { 0.0 };
                            let crumbs_budget =
                                (ui.available_width() - count_reserve - clip_reserve).max(48.0);

                            // 搜索模式指示器
                            if search.is_active() {
                                egui::Frame::new()
                                    .fill(th.accent_soft())
                                    .corner_radius(th.cr(7))
                                    .inner_margin(Margin::symmetric(8, 3))
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            ui.label(
                                                RichText::new(format!(
                                                    "搜索: {}",
                                                    search.keyword()
                                                ))
                                                .color(th.accent)
                                                .size(12.0),
                                            );
                                        });
                                    });
                                ui.add_space(6.0);
                            } else if let Some(i) =
                                toolbar::breadcrumbs(ui, th, &self.stack, crumbs_budget)
                            {
                                jumped = Some(i);
                            }

                            if sel_meta.is_empty() {
                                ui.label(
                                    RichText::new(format!("· {visible_total} 项"))
                                        .color(th.text_faint)
                                        .size(12.5),
                                );
                            } else {
                                ui.label(
                                    RichText::new(format!(
                                        "· 已选 {}/{} 项",
                                        sel_meta.len(),
                                        visible_total
                                    ))
                                    .color(th.accent)
                                    .size(12.5),
                                );
                            }

                            // 剪贴板 chip(只显示数量, 避免文件名过长溢出)
                            if let Some((_label, n)) = &clip_info {
                                ui.add_space(8.0);
                                egui::Frame::new()
                                    .fill(th.accent_soft())
                                    .corner_radius(th.cr(7))
                                    .inner_margin(Margin::symmetric(8, 3))
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            ui.label(
                                                RichText::new(format!("剪贴板 · {n} 项"))
                                                    .color(th.accent)
                                                    .size(11.5),
                                            );
                                            let (xr, xresp) = ui.allocate_exact_size(
                                                vec2(14.0, 14.0),
                                                egui::Sense::click(),
                                            );
                                            icons::paint(
                                                ui.painter(),
                                                xr.shrink(2.5),
                                                Glyph::Close,
                                                if xresp.hovered() {
                                                    th.accent
                                                } else {
                                                    th.text_faint
                                                },
                                            );
                                            if xresp.clicked() {
                                                clear_clip = true;
                                            }
                                        });
                                    });
                            }
                        },
                    );

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if sel_meta.is_empty() {
                            // ---- 普通模式: 搜索 + 刷新 + 新建 + 视图 ----
                            let focused = ui.memory(|m| m.has_focus(egui::Id::new("file_search")));
                            egui::Frame::new()
                                .fill(th.card)
                                .stroke(Stroke::new(
                                    1.0,
                                    if focused { th.accent } else { th.border },
                                ))
                                .corner_radius(th.cr(9))
                                .inner_margin(Margin::symmetric(10, 5))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        let (ir, _) = ui.allocate_exact_size(
                                            vec2(15.0, 15.0),
                                            egui::Sense::hover(),
                                        );
                                        icons::paint(
                                            ui.painter(),
                                            ir,
                                            Glyph::Search,
                                            th.text_faint,
                                        );
                                        ui.add_space(1.0);
                                        let _search_response = ui.add(
                                            egui::TextEdit::singleline(&mut self.filter)
                                                .id(egui::Id::new("file_search"))
                                                .frame(false)
                                                .desired_width(150.0)
                                                .hint_text(if search.is_active() {
                                                    "搜索中..."
                                                } else {
                                                    "按 Enter 全局搜索"
                                                })
                                                .font(FontId::proportional(13.5)),
                                        );
                                        if !self.filter.is_empty() {
                                            let (xr, xresp) = ui.allocate_exact_size(
                                                vec2(14.0, 14.0),
                                                egui::Sense::click(),
                                            );
                                            icons::paint(
                                                ui.painter(),
                                                xr.shrink(2.5),
                                                Glyph::Close,
                                                if xresp.hovered() {
                                                    th.accent
                                                } else {
                                                    th.text_faint
                                                },
                                            );
                                            if xresp.clicked() {
                                                self.exit_search(search);
                                            }
                                        }
                                    });
                                });
                            ui.add_space(6.0);

                            // 刷新
                            let (rr, rresp) =
                                ui.allocate_exact_size(vec2(30.0, 30.0), egui::Sense::click());
                            ui.painter().rect_filled(
                                rr,
                                th.cr(8),
                                if rresp.hovered() {
                                    th.hover
                                } else {
                                    egui::Color32::TRANSPARENT
                                },
                            );
                            icons::paint(
                                ui.painter(),
                                rr.shrink(7.0),
                                Glyph::Refresh,
                                th.text_weak,
                            );
                            if rresp.clicked() {
                                refresh = true;
                            }
                            rresp.on_hover_text("刷新 (F5)");

                            ui.add_space(4.0);
                            // 上传菜单(上传文件 / 上传文件夹)
                            ui.menu_button(
                                RichText::new("上传").size(13.0).color(th.text_weak),
                                |ui| {
                                    if ui.button("上传文件").clicked() {
                                        upload = true;
                                        ui.close_menu();
                                    }
                                    if ui.button("上传文件夹").clicked() {
                                        upload_dir = true;
                                        ui.close_menu();
                                    }
                                },
                            );

                            ui.add_space(4.0);
                            // 新建文件夹(矢量图标 + 悬浮提示)
                            let (pr, presp) =
                                ui.allocate_exact_size(vec2(30.0, 30.0), egui::Sense::click());
                            ui.painter().rect_filled(
                                pr,
                                th.cr(8),
                                if presp.hovered() {
                                    th.accent_soft()
                                } else {
                                    egui::Color32::TRANSPARENT
                                },
                            );
                            icons::paint(ui.painter(), pr.shrink(7.0), Glyph::Plus, th.accent);
                            if presp.clicked() {
                                mkdir = true;
                            }
                            presp.on_hover_text("新建文件夹");

                            ui.add_space(6.0);
                            // 视图切换
                            let icon_bg = |active: bool, hovered: bool| {
                                if active {
                                    th.accent_soft()
                                } else if hovered {
                                    th.hover
                                } else {
                                    egui::Color32::TRANSPARENT
                                }
                            };
                            // 图标视图: 2x2 网格
                            let (ri, ri_resp) =
                                ui.allocate_exact_size(vec2(28.0, 28.0), egui::Sense::click());
                            ui.painter().rect_filled(
                                ri,
                                th.cr(6),
                                icon_bg(self.view_mode == ViewMode::Icon, ri_resp.hovered()),
                            );
                            let p = ui.painter();
                            let s = 3.0;
                            let gap = 2.0;
                            let cx = ri.center().x;
                            let cy = ri.center().y;
                            let color = if self.view_mode == ViewMode::Icon {
                                th.accent
                            } else {
                                th.text_weak
                            };
                            for dx in [-(s + gap / 2.0), s + gap / 2.0] {
                                for dy in [-(s + gap / 2.0), s + gap / 2.0] {
                                    p.rect_filled(
                                        Rect::from_center_size(
                                            Pos2::new(cx + dx, cy + dy),
                                            vec2(s, s),
                                        ),
                                        th.cr(1),
                                        color,
                                    );
                                }
                            }
                            if ri_resp.clicked() {
                                self.view_mode = ViewMode::Icon;
                            }
                            ri_resp.on_hover_text("图标视图");

                            // 列表视图: 三条横线
                            let (li, li_resp) =
                                ui.allocate_exact_size(vec2(28.0, 28.0), egui::Sense::click());
                            ui.painter().rect_filled(
                                li,
                                th.cr(6),
                                icon_bg(self.view_mode == ViewMode::List, li_resp.hovered()),
                            );
                            let p = ui.painter();
                            let color = if self.view_mode == ViewMode::List {
                                th.accent
                            } else {
                                th.text_weak
                            };
                            let lx = li.center().x - 5.0;
                            let lw = 10.0;
                            for dy in [-3.5, 0.0, 3.5] {
                                p.line_segment(
                                    [
                                        Pos2::new(lx, li.center().y + dy),
                                        Pos2::new(lx + lw, li.center().y + dy),
                                    ],
                                    Stroke::new(1.5, color),
                                );
                            }
                            if li_resp.clicked() {
                                self.view_mode = ViewMode::List;
                            }
                            li_resp.on_hover_text("列表视图");
                        } else {
                            // ---- 选中模式: 就地显示操作, 不新增行/不改变列表位置 ----
                            let single = sel_meta.len() == 1;
                            let open_sel = if single && !dl_candidates.is_empty() {
                                let (id, name) = sel_meta[0].clone();
                                let ft = self.file_type(&id, &name);
                                // 只下载类不给入口(双击由 open_preview 兜底提示)。
                                (filetypes::preview_kind(ft) != PreviewKind::DownloadOnly)
                                    .then_some((ft, id, name))
                            } else {
                                None
                            };

                            // 取消(最右)
                            if ui
                                .add(egui::Button::new(RichText::new("取消").color(th.text_weak)))
                                .clicked()
                            {
                                self.selected.clear();
                            }
                            if ui
                                .add(
                                    egui::Button::new(RichText::new("移入回收站").color(th.danger))
                                        .stroke(Stroke::new(1.0, mix(th.danger, th.bg, 0.35)))
                                        .fill(egui::Color32::TRANSPARENT)
                                        .corner_radius(th.cr(8)),
                                )
                                .clicked()
                            {
                                ask_trash = true;
                            }
                            if self.clipboard.is_some()
                                && ui
                                    .add(egui::Button::new(
                                        RichText::new("粘贴").color(th.text_weak),
                                    ))
                                    .clicked()
                            {
                                self.paste_clipboard(g);
                            }
                            if ui
                                .add(egui::Button::new(RichText::new("剪切").color(th.text_weak)))
                                .clicked()
                            {
                                self.clip_selection(g, ClipKind::Cut);
                            }
                            if ui
                                .add(egui::Button::new(RichText::new("复制").color(th.text_weak)))
                                .clicked()
                            {
                                self.clip_selection(g, ClipKind::Copy);
                            }
                            if ui
                                .add(egui::Button::new(RichText::new("分享").color(th.text_weak)))
                                .clicked()
                            {
                                ask_share = true;
                            }
                            if single
                                && ui
                                    .add(egui::Button::new(
                                        RichText::new("重命名").color(th.text_weak),
                                    ))
                                    .clicked()
                            {
                                ask_rename = true;
                            }
                            if let Some((ft, id, name)) = open_sel {
                                if ft == FileType::Video {
                                    let quality = match preview.quality(&id) {
                                        Some(r) => QualityMenuState::Ready(r),
                                        None => QualityMenuState::Loading,
                                    };
                                    row::play_menu(ui, &id, &name, quality, &mut actions);
                                } else {
                                    let label = if ft == FileType::Audio {
                                        "播放"
                                    } else {
                                        "打开"
                                    };
                                    if ui
                                        .add(egui::Button::new(
                                            RichText::new(label).color(th.text_weak),
                                        ))
                                        .clicked()
                                    {
                                        ask_preview = true;
                                    }
                                }
                            }
                            if (!dl_candidates.is_empty() || !dl_folders.is_empty())
                                && ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new("下载到本地").color(th.on_accent),
                                        )
                                        .fill(th.accent)
                                        .stroke(Stroke::NONE)
                                        .corner_radius(th.cr(8)),
                                    )
                                    .clicked()
                            {
                                want_download = true;
                            }
                        }
                    });
                });
            });

        if up {
            self.exit_search(search);
            self.stack.pop();
            self.show_dir(g);
        }
        if let Some(i) = jumped {
            self.exit_search(search);
            self.stack.truncate(i + 1);
            self.show_dir(g);
        }
        // 键盘 Delete 先于顶部栏动作生效(与旧实现的事件顺序一致)。
        if let Some(sel) = key_trash {
            effects.push(FilesAction::ConfirmTrash(sel));
        }
        if mkdir {
            effects.push(FilesAction::NewFolder);
        }
        if refresh {
            self.refresh_dir(g);
        }
        if upload {
            effects.push(FilesAction::UploadFiles);
        }
        if upload_dir {
            effects.push(FilesAction::UploadFolder);
        }

        // 处理顶部栏产生的操作
        if clear_clip {
            self.clipboard = None;
        }
        if ask_rename && sel_meta.len() == 1 {
            effects.push(FilesAction::Rename {
                id: sel_meta[0].0.clone(),
                name: sel_meta[0].1.clone(),
            });
        }
        if ask_preview && sel_meta.len() == 1 {
            effects.push(FilesAction::Preview {
                id: sel_meta[0].0.clone(),
                name: sel_meta[0].1.clone(),
            });
        }
        if ask_trash {
            effects.push(FilesAction::ConfirmTrash(sel_meta.clone()));
        }
        if ask_share {
            effects.push(FilesAction::ShareSelection);
        }
        if trigger_search {
            effects.push(FilesAction::Search);
        }

        if want_download && (!dl_candidates.is_empty() || !dl_folders.is_empty()) {
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
                        Stroke::new(1.0, th.border),
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

                        let mut even = false;
                        let (cn_x, cs_x, ct_x) = self.col_layout(inner.width());
                        let all_files: Vec<&File> = folders.iter().chain(plain.iter()).collect();

                        if self.view_mode == ViewMode::List {
                            // 列表视图
                            for f in &all_files {
                                let is_sel = self.selected.contains(&f.id);
                                let quality = match preview.quality(&f.id) {
                                    Some(r) => QualityMenuState::Ready(r),
                                    None => QualityMenuState::Loading,
                                };
                                if let Some(id) = row::file_row(
                                    ui,
                                    th,
                                    f,
                                    is_sel,
                                    even,
                                    has_clip,
                                    quality,
                                    &mut actions,
                                    cn_x,
                                    cs_x,
                                    ct_x,
                                ) {
                                    sel_reqs.push(id);
                                }
                                even = !even;
                            }
                        } else {
                            // 图标视图
                            let card_w = self.grid_card_size;
                            let card_h = self.grid_card_size * 1.15;
                            let gap = 8.0;
                            let avail_w = inner.width();
                            let cols = ((avail_w + gap) / (card_w + gap)).floor().max(1.0) as usize;
                            let total_rows = all_files.len().div_ceil(cols);

                            // 只请求「可见行 ± 一屏」的缩略图: 整目录一次性入队会让
                            // worker 长时间啃已经滚出视野的图, 新滚到的位置反而排在后面。
                            let clip = ui.clip_rect();
                            let row_h = card_h + ui.spacing().item_spacing.y;
                            let row_range =
                                grid::thumb_row_range(clip, ui.cursor().min.y, row_h, total_rows);
                            let max_edge = grid::thumb_max_edge(ui.ctx().pixels_per_point());
                            let now = Instant::now();
                            for row in row_range {
                                for col in 0..cols {
                                    let idx = row * cols + col;
                                    if idx >= all_files.len() {
                                        break;
                                    }
                                    let f = all_files[idx];
                                    if f.is_folder() {
                                        continue;
                                    }
                                    // 纹理在 = 图正被看着: 刷新 LRU, 使其免于本轮淘汰。
                                    if thumbs.textures.contains(&f.id) {
                                        thumbs.textures.mark_used(&f.id, now);
                                        continue;
                                    }
                                    let Some(url) = &f.thumbnail_link else {
                                        continue;
                                    };
                                    if !thumbs.needs_request(&f.id) {
                                        continue;
                                    }
                                    thumbs.mark_inflight(f.id.clone());
                                    g.send(Cmd::LoadThumbnail {
                                        file_id: f.id.clone(),
                                        url: url.clone(),
                                        max_edge,
                                    });
                                }
                            }

                            for row in 0..total_rows {
                                ui.horizontal(|ui| {
                                    ui.add_space(4.0);
                                    for col in 0..cols {
                                        let idx = row * cols + col;
                                        if idx >= all_files.len() {
                                            break;
                                        }
                                        let f = all_files[idx];
                                        let is_sel = self.selected.contains(&f.id);
                                        let quality = match preview.quality(&f.id) {
                                            Some(r) => QualityMenuState::Ready(r),
                                            None => QualityMenuState::Loading,
                                        };
                                        let (rect, resp) = ui.allocate_exact_size(
                                            vec2(card_w, card_h),
                                            egui::Sense::click(),
                                        );
                                        let painter = ui.painter().clone();

                                        // 背景
                                        let bg = if is_sel {
                                            if th.breeze {
                                                th.accent
                                            } else {
                                                mix(
                                                    th.card,
                                                    th.accent,
                                                    if th.dark { 0.22 } else { 0.12 },
                                                )
                                            }
                                        } else if resp.hovered() {
                                            mix(
                                                th.card,
                                                th.text,
                                                if th.dark { 0.07 } else { 0.045 },
                                            )
                                        } else {
                                            egui::Color32::TRANSPARENT
                                        };
                                        painter.rect_filled(rect, th.cr(10), bg);
                                        if is_sel && !th.breeze {
                                            painter.rect_stroke(
                                                rect,
                                                th.cr(10),
                                                Stroke::new(2.0, th.accent),
                                                egui::StrokeKind::Inside,
                                            );
                                        }

                                        // 左上角复选框
                                        let cb_size = 14.0;
                                        let cb_x = rect.min.x + 6.0;
                                        let cb_y = rect.min.y + 6.0;
                                        let cb_rect = Rect::from_min_max(
                                            Pos2::new(cb_x, cb_y),
                                            Pos2::new(cb_x + cb_size, cb_y + cb_size),
                                        );
                                        let cb_resp = ui.interact(
                                            cb_rect,
                                            ui.id().with(("cb", f.id.clone())),
                                            egui::Sense::click(),
                                        );
                                        let cb_on_accent = is_sel && th.breeze;
                                        let cb_bg = if cb_on_accent {
                                            th.on_accent
                                        } else if is_sel {
                                            th.accent
                                        } else {
                                            mix(th.card, th.text, if th.dark { 0.3 } else { 0.2 })
                                        };
                                        let cb_stroke = if is_sel {
                                            Stroke::NONE
                                        } else {
                                            Stroke::new(1.0, th.text_faint)
                                        };
                                        painter.rect_filled(cb_rect, th.cr(3), cb_bg);
                                        painter.rect_stroke(
                                            cb_rect,
                                            th.cr(3),
                                            cb_stroke,
                                            egui::StrokeKind::Inside,
                                        );
                                        if is_sel {
                                            let check_pts = [
                                                Pos2::new(cb_x + 3.0, cb_y + cb_size / 2.0),
                                                Pos2::new(cb_x + 5.5, cb_y + cb_size / 2.0 + 2.5),
                                                Pos2::new(cb_x + cb_size - 2.5, cb_y + 2.5),
                                            ];
                                            let tick = if cb_on_accent {
                                                th.accent
                                            } else {
                                                th.on_accent
                                            };
                                            painter.add(egui::Shape::line(
                                                check_pts.to_vec(),
                                                Stroke::new(1.8, tick),
                                            ));
                                        }

                                        // 文件图标或缩略图（居中偏上）
                                        let icon_size = card_w * 0.45;
                                        let icon_rect = Rect::from_center_size(
                                            Pos2::new(rect.center().x, rect.min.y + card_h * 0.35),
                                            vec2(icon_size, icon_size),
                                        );

                                        if let Some(texture) = thumbs.textures.get(&f.id) {
                                            // 渲染缩略图（保持宽高比）
                                            let max_size = card_w * grid::THUMB_MAX_CARD_RATIO;
                                            let tex_size = texture.size_vec2();
                                            let aspect = tex_size.x / tex_size.y;

                                            let (w, h) = if aspect > 1.0 {
                                                // 横向图片
                                                (max_size, max_size / aspect)
                                            } else {
                                                // 纵向图片
                                                (max_size * aspect, max_size)
                                            };

                                            let thumb_rect = Rect::from_center_size(
                                                Pos2::new(
                                                    rect.center().x,
                                                    rect.min.y + card_h * 0.4,
                                                ),
                                                vec2(w, h),
                                            );
                                            painter.image(
                                                texture.id(),
                                                thumb_rect,
                                                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                                                Color32::WHITE,
                                            );
                                        } else {
                                            // 回退到类型图标
                                            let (glyph, color) = file_visual(f);
                                            let tile_bg = if is_sel && th.breeze {
                                                mix(
                                                    th.on_accent,
                                                    color,
                                                    if th.dark { 0.22 } else { 0.14 },
                                                )
                                            } else {
                                                mix(
                                                    th.card,
                                                    color,
                                                    if th.dark { 0.16 } else { 0.10 },
                                                )
                                            };
                                            painter.rect_filled(icon_rect, th.cr(8), tile_bg);
                                            icons::paint(
                                                &painter,
                                                icon_rect.shrink(4.0),
                                                glyph,
                                                color,
                                            );
                                        }

                                        // 文件名（底部居中，最多 2 行）
                                        let name = &f.name;
                                        let max_w = card_w - 8.0;
                                        let name_color = if is_sel && th.breeze {
                                            th.on_accent
                                        } else {
                                            th.text
                                        };
                                        let font_size = (card_w * 0.11).clamp(9.0, 14.0);
                                        let name_g = truncate_text(
                                            &painter,
                                            name,
                                            max_w,
                                            FontId::proportional(font_size),
                                            name_color,
                                        );
                                        let name_h = name_g.size().y.min(card_h * 0.25);
                                        let name_y = rect.max.y - 8.0 - name_h;
                                        painter.galley(
                                            Pos2::new(
                                                rect.center().x - name_g.size().x / 2.0,
                                                name_y,
                                            ),
                                            name_g,
                                            name_color,
                                        );

                                        // 右键菜单
                                        let f_ctx = f.clone();
                                        let _menu = resp.context_menu(|ui| {
                                            if f_ctx.is_folder() {
                                                if ui.button("打开").clicked() {
                                                    actions.push(RowAction::OpenFolder(
                                                        f_ctx.id.clone(),
                                                        f_ctx.name.clone(),
                                                    ));
                                                    ui.close_menu();
                                                }
                                                if ui.button("下载到本地…").clicked() {
                                                    actions.push(RowAction::DownloadFolder(
                                                        f_ctx.id.clone(),
                                                        f_ctx.name.clone(),
                                                    ));
                                                    ui.close_menu();
                                                }
                                            } else {
                                                if ui.button("下载到本地…").clicked() {
                                                    actions.push(RowAction::DownloadFile(
                                                        f_ctx.id.clone(),
                                                        f_ctx.name.clone(),
                                                    ));
                                                    ui.close_menu();
                                                }
                                                row::preview_menu_items(
                                                    ui,
                                                    &f_ctx,
                                                    quality,
                                                    &mut actions,
                                                );
                                            }
                                            ui.separator();
                                            if ui.button("分享").clicked() {
                                                actions.push(RowAction::Share(f_ctx.id.clone()));
                                                ui.close_menu();
                                            }
                                            if ui.button("复制").clicked() {
                                                actions.push(RowAction::CopyItem(f_ctx.id.clone()));
                                                ui.close_menu();
                                            }
                                            if ui.button("剪切").clicked() {
                                                actions.push(RowAction::CutItem(f_ctx.id.clone()));
                                                ui.close_menu();
                                            }
                                            if has_clip
                                                && f_ctx.is_folder()
                                                && ui.button("粘贴到此处").clicked()
                                            {
                                                actions
                                                    .push(RowAction::PasteInto(f_ctx.id.clone()));
                                                ui.close_menu();
                                            }
                                            ui.separator();
                                            if ui.button("重命名").clicked() {
                                                actions.push(RowAction::Rename(
                                                    f_ctx.id.clone(),
                                                    f_ctx.name.clone(),
                                                ));
                                                ui.close_menu();
                                            }
                                            if ui.button("复制名称").clicked() {
                                                actions
                                                    .push(RowAction::CopyName(f_ctx.name.clone()));
                                                ui.close_menu();
                                            }
                                            ui.separator();
                                            if ui
                                                .button(
                                                    RichText::new("移入回收站").color(th.danger),
                                                )
                                                .clicked()
                                            {
                                                actions.push(RowAction::Trash(f_ctx.id.clone()));
                                                ui.close_menu();
                                            }
                                        });

                                        // 点击处理
                                        let dbl = resp.double_clicked();
                                        // 只有点击复选框才勾选; 单击卡片不改变选择。
                                        if cb_resp.clicked() {
                                            sel_reqs.push(f.id.clone());
                                        } else if dbl {
                                            if f.is_folder() {
                                                actions.push(RowAction::OpenFolder(
                                                    f.id.clone(),
                                                    f.name.clone(),
                                                ));
                                            } else {
                                                actions.push(RowAction::OpenFile(
                                                    f.id.clone(),
                                                    f.name.clone(),
                                                ));
                                            }
                                        }
                                        ui.add_space(4.0);
                                    }
                                });
                            }
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

    fn file_list_header(&mut self, ui: &mut egui::Ui, th: &Theme, w: f32, search: &SearchPage) {
        let (name_x, size_left, time_left) = self.col_layout(w);
        let h = 30.0;
        let (rect, _) = ui.allocate_exact_size(vec2(w, h), egui::Sense::hover());
        let painter = ui.painter().clone();
        let top = rect.min.y;
        let x0 = rect.min.x;

        // 表头全选复选框
        let (folders, plain) = self.visible_rows(search);
        let total_visible = folders.len() + plain.len();
        let all_selected = total_visible > 0 && self.selected.len() >= total_visible;
        let some_selected = !self.selected.is_empty() && !all_selected;

        let cb_size = 14.0;
        let cb_x = x0 + 6.0;
        let cb_y = top + (h - cb_size) / 2.0;
        let cb_rect = Rect::from_min_max(
            Pos2::new(cb_x, cb_y),
            Pos2::new(cb_x + cb_size, cb_y + cb_size),
        );
        let cb_resp = ui.interact(cb_rect, ui.id().with("header_cb"), egui::Sense::click());

        // 复选框背景
        let cb_bg = if all_selected {
            th.accent
        } else if some_selected {
            mix(th.accent, th.bg, 0.5)
        } else {
            egui::Color32::TRANSPARENT
        };
        let cb_stroke = if all_selected || some_selected {
            Stroke::NONE
        } else {
            Stroke::new(1.0, th.text_faint)
        };
        painter.rect_filled(cb_rect, th.cr(3), cb_bg);
        painter.rect_stroke(cb_rect, th.cr(3), cb_stroke, egui::StrokeKind::Inside);

        // 全选时绘制勾号
        if all_selected {
            let check_pts = [
                Pos2::new(cb_x + 3.0, cb_y + cb_size / 2.0),
                Pos2::new(cb_x + 5.5, cb_y + cb_size / 2.0 + 2.5),
                Pos2::new(cb_x + cb_size - 2.5, cb_y + 2.5),
            ];
            painter.add(egui::Shape::line(
                check_pts.to_vec(),
                Stroke::new(1.8, th.on_accent),
            ));
        }
        // 部分选中时绘制横线
        if some_selected {
            painter.line_segment(
                [
                    Pos2::new(cb_x + 3.0, cb_y + cb_size / 2.0),
                    Pos2::new(cb_x + cb_size - 3.0, cb_y + cb_size / 2.0),
                ],
                Stroke::new(2.0, th.on_accent),
            );
        }

        // 悬停效果
        if cb_resp.hovered() {
            painter.rect_filled(cb_rect, th.cr(3), mix(th.accent, th.bg, 0.85));
        }

        // 点击切换全选/取消全选(仅当前可见/过滤后的行)
        if cb_resp.clicked() {
            if all_selected {
                self.selected.clear();
            } else {
                let (folders, plain) = self.visible_rows(search);
                for f in folders.iter().chain(plain.iter()) {
                    self.selected.insert(f.id.clone());
                }
            }
        }

        struct Col {
            x: f32,
            label: &'static str,
            by: SortBy,
            w: f32,
        }
        let cols = [
            Col {
                x: name_x,
                label: "名称",
                by: SortBy::Name,
                w: size_left - name_x,
            },
            Col {
                x: size_left,
                label: "大小",
                by: SortBy::Size,
                w: time_left - size_left,
            },
            Col {
                x: time_left,
                label: "修改时间",
                by: SortBy::Modified,
                w: w - time_left - Self::RIGHT_PAD,
            },
        ];
        for col in &cols {
            let active = self.sort_by == col.by;
            let color = if active { th.accent } else { th.text_faint };
            let crect = Rect::from_min_max(
                Pos2::new(x0 + col.x - 8.0, top),
                Pos2::new((x0 + col.x + col.w).min(rect.right()), top + h),
            );
            let resp = ui.interact(
                crect,
                ui.id().with(("header", col.by)),
                egui::Sense::click(),
            );
            if resp.hovered() {
                painter.rect_filled(crect, th.cr(6), th.hover);
            }
            let g =
                painter.layout_no_wrap(col.label.to_string(), FontId::proportional(12.5), color);
            let label_w = g.size().x;
            painter.galley(
                Pos2::new(x0 + col.x, top + (h - g.size().y) / 2.0),
                g,
                color,
            );
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

        // ---- 拖拽分割线 ----
        let cursor_range = 6.0f32;
        let handle_w = 4.0f32;

        // Handle 1: 名称 | 大小
        let h1_x = x0 + size_left;
        let h1_rect = Rect::from_min_max(
            Pos2::new(h1_x - cursor_range, top),
            Pos2::new(h1_x + cursor_range, top + h),
        );
        let h1_resp = ui.interact(h1_rect, ui.id().with("col_drag_1"), egui::Sense::drag());
        let h1_active = h1_resp.hovered()
            || h1_resp.is_pointer_button_down_on()
            || self.col_dragging.as_ref().is_some_and(|d| d.handle == 1);
        if h1_active {
            painter.rect_filled(
                Rect::from_center_size(Pos2::new(h1_x, top + h / 2.0), vec2(handle_w, h - 8.0)),
                th.cr(2),
                th.text_faint,
            );
        }
        if h1_resp.drag_started() {
            self.col_dragging = Some(ColDrag {
                handle: 1,
                start_x: h1_resp.interact_pointer_pos().map(|p| p.x).unwrap_or(h1_x),
                orig_size_w: self.col_size_w,
                orig_time_w: self.col_time_w,
            });
        }

        // Handle 2: 大小 | 修改时间
        let h2_x = x0 + time_left;
        let h2_rect = Rect::from_min_max(
            Pos2::new(h2_x - cursor_range, top),
            Pos2::new(h2_x + cursor_range, top + h),
        );
        let h2_resp = ui.interact(h2_rect, ui.id().with("col_drag_2"), egui::Sense::drag());
        let h2_active = h2_resp.hovered()
            || h2_resp.is_pointer_button_down_on()
            || self.col_dragging.as_ref().is_some_and(|d| d.handle == 2);
        if h2_active {
            painter.rect_filled(
                Rect::from_center_size(Pos2::new(h2_x, top + h / 2.0), vec2(handle_w, h - 8.0)),
                th.cr(2),
                th.text_faint,
            );
        }
        if h2_resp.drag_started() {
            self.col_dragging = Some(ColDrag {
                handle: 2,
                start_x: h2_resp.interact_pointer_pos().map(|p| p.x).unwrap_or(h2_x),
                orig_size_w: self.col_size_w,
                orig_time_w: self.col_time_w,
            });
        }

        // 设置拖拽时的鼠标样式
        if h1_resp.hovered() || h2_resp.hovered() || self.col_dragging.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
    }
}
