//! 「文件浏览」域: 导航栈 / 目录缓存(含 SWR 校正) / 选中集 / 排序过滤 / 视图与列宽 / 剪贴板。
//!
//! 页面渲染留在 `files_page.rs`(继续按渲染角色拆进 `app/files/`, 见 TODO.md 的 P2-11);
//! 本模块负责文件列表数据、目录缓存与导航状态。

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use eframe::egui;

use kichi_core::types::{File, FileList};

use crate::msg::Cmd;

use super::global::Global;
use super::search::SearchPage;
use super::thumbs::ThumbsPage;
use super::types::{ClipKind, Clipboard, ColDrag, Crumb, DirEntry, SortBy, ViewMode};

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
