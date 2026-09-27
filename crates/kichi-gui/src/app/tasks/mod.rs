//! 「离线下载任务」域: 分桶列表 / 选择 / 新建表单与「保存到」目录选择器。
//!
//! 本模块持有 `TasksPage` 的状态与生命周期、`Msg` 处理, 以及跨域动作类型
//! [`TasksAction`] 与 picker 状态字段; 渲染与 picker 交互分别落在 `list` / `card` / `picker`。

use std::collections::{BTreeMap, BTreeSet, HashSet};

use kichi_core::types::{task_id, File, Task};

use crate::msg::Cmd;

use super::files::Crumb;
use super::global::Global;
use super::types::OfflineTab;

mod card;
mod list;
mod picker;

/// 离线任务页本帧产生的跨域动作(渲染与动作分离, 由 `App::apply_tasks_action` 执行)。
pub(crate) enum TasksAction {
    /// 把已完成的任务文件下载到本地(需要 `App` 的下载目录设置)。
    Download { id: String, name: String },
}

/// 离线任务状态(分桶列表 + 选择 + 新建表单 + 保存到选择器)。
#[derive(Default)]
pub(crate) struct TasksPage {
    /// 按 phase 分桶的离线任务(空分桶不保留)。
    pub(crate) buckets: BTreeMap<String, Vec<Task>>,
    /// 每个 phase 的下一页游标; 缺失/None 表示没有更多。
    pub(crate) next_tokens: BTreeMap<String, Option<String>>,
    /// 正在「加载更多」的 phase 集合, 用于按钮禁用/转圈。
    pub(crate) loading_more: BTreeSet<String>,
    loading: bool,
    /// 用户点击「刷新任务」后的进行中状态, 用于给出可见反馈。
    pub(crate) refreshing: bool,
    /// 阶段页签; None 表示尚未按已有数据自动选定。
    pub(crate) tab: Option<OfflineTab>,
    /// 选中项(task id)。
    pub(crate) selected: HashSet<String>,
    /// Shift 范围选择的锚点(task id)。
    pub(crate) anchor: Option<String>,

    // 新建离线下载
    /// 链接 / 磁力。
    pub(crate) url: String,
    /// 文件名(留空自动识别)。
    pub(crate) name: String,
    /// 保存到的网盘目录 (id, 名称); None = 离线默认目录。
    pub(crate) dest: Option<(String, String)>,

    // 「保存到」目录选择器
    picker_open: bool,
    picker_stack: Vec<Crumb>,
    picker_folders: Vec<File>,
    picker_loading: bool,
    /// 最近一次选择器请求 id, 用于丢弃乱序的旧响应。
    picker_req: u64,
}

impl TasksPage {
    /// 清空分桶与选择(退出登录 / 会话失效时调用)。
    pub(crate) fn clear(&mut self) {
        self.buckets.clear();
        self.next_tokens.clear();
        self.loading_more.clear();
        self.selected.clear();
        self.anchor = None;
    }

    /// 当前生效的页签: 未手动选择时按已有数据自动挑一个(优先「下载中」)。
    pub(crate) fn active_tab(&mut self) -> OfflineTab {
        if let Some(t) = self.tab {
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
            self.tab = Some(pick);
        }
        pick
    }

    /// 加载某个 phase 的下一页任务。
    pub(crate) fn load_more(&mut self, g: &mut Global, phase: &str) {
        if self.loading_more.contains(phase) {
            return;
        }
        if self
            .next_tokens
            .get(phase)
            .and_then(|t| t.as_ref())
            .is_none()
        {
            return;
        }
        self.loading_more.insert(phase.to_string());
        g.send(Cmd::LoadMoreTasks {
            phase: phase.to_string(),
        });
    }

    // ---------- 消息 ----------

    /// 离线任务提交成功: 清空表单并刷新列表。
    pub(crate) fn on_created(&mut self, g: &mut Global) {
        g.toast_ok("已提交离线下载");
        self.url.clear();
        self.name.clear();
        g.send(Cmd::RefreshTasks);
    }

    /// 离线任务重试 / 删除后重新同步列表。
    pub(crate) fn on_changed(&mut self, g: &mut Global) {
        g.send(Cmd::RefreshTasks);
    }

    /// 全量分桶响应: 合并而非整体替换, 某个分桶刷新失败时保留其旧数据。
    pub(crate) fn on_all(
        &mut self,
        buckets: BTreeMap<String, Vec<Task>>,
        next_tokens: BTreeMap<String, Option<String>>,
    ) {
        for (phase, tasks) in buckets {
            if tasks.is_empty() {
                self.buckets.remove(&phase);
            } else {
                self.buckets.insert(phase.clone(), tasks);
            }
            if let Some(tok) = next_tokens.get(&phase) {
                self.next_tokens.insert(phase, tok.clone());
            }
        }
        self.loading = false;
        self.refreshing = false;
    }

    /// 某分桶的下一页响应: 按 task_id 去重追加, 避免与刷新回包交叠时重复。
    pub(crate) fn on_more(
        &mut self,
        phase: String,
        tasks: Vec<Task>,
        next_page_token: Option<String>,
    ) {
        self.loading_more.remove(&phase);
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
        self.next_tokens.insert(phase, next_page_token);
    }

    /// 分桶加载更多失败。
    pub(crate) fn on_more_failed(&mut self, g: &mut Global, phase: String, what: String) {
        self.loading_more.remove(&phase);
        g.toast_err(&what);
    }
}
