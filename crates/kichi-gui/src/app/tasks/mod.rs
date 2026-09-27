//! 「离线下载任务」域: 分桶列表 / 选择 / 新建表单与「保存到」目录选择器。
//!
//! 本模块持有 `TasksPage` 的状态与生命周期、`Msg` 处理, 以及渲染入口
//! `show`(渲染与动作分离, 跨域动作见 [`TasksAction`]); 页面渲染落在 `list`。

use std::collections::{BTreeMap, BTreeSet, HashSet};

use eframe::egui::{self, Align2, Color32, RichText, Stroke};

use kichi_core::types::{task_id, File, Task};

use crate::msg::Cmd;
use crate::theme::Theme;

use super::global::Global;
use super::helpers::folder_row;
use super::types::{Crumb, OfflineTab};

mod list;

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

    // ---------- 「保存到」目录选择器 ----------

    /// 选择器当前所在目录。
    fn picker_parent(&self) -> Option<String> {
        self.picker_stack.last().and_then(|c| c.id.clone())
    }

    /// 打开「保存到」网盘目录选择器, 从根目录开始。
    pub(crate) fn open_picker(&mut self, g: &mut Global) {
        self.picker_open = true;
        self.picker_stack = vec![Crumb {
            id: None,
            label: "我的云盘".into(),
        }];
        self.picker_list(g);
    }

    /// 请求选择器当前目录的子文件夹列表。
    fn picker_list(&mut self, g: &mut Global) {
        self.picker_loading = true;
        self.picker_folders.clear();
        self.picker_req += 1;
        let req_id = self.picker_req;
        g.send(Cmd::ListFolders {
            parent: self.picker_parent(),
            req_id,
        });
    }

    /// 该 req_id 是否属于「保存到」选择器。
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

    // ---------- 渲染 ----------

    /// 「选择保存到（网盘目录）」弹窗。
    pub(crate) fn draw_offline_picker(&mut self, ctx: &egui::Context, g: &mut Global, th: &Theme) {
        if !self.picker_open {
            return;
        }
        let crumbs = self.picker_stack.clone();
        let folders = self.picker_folders.clone();
        let loading = self.picker_loading;
        let cur_dest = self.dest.clone();
        let mut nav_to: Option<usize> = None;
        let mut enter: Option<(String, String)> = None;
        let mut confirm = false;
        let mut close = false;
        egui::Window::new("选择保存到（网盘目录）")
            .collapsible(false)
            .resizable(false)
            .default_width(420.0)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    let last = crumbs.len().saturating_sub(1);
                    for (i, c) in crumbs.iter().enumerate() {
                        if i > 0 {
                            ui.label(RichText::new("/").color(th.text_faint));
                        }
                        if ui.selectable_label(i == last, &c.label).clicked() && i != last {
                            nav_to = Some(i);
                        }
                    }
                });
                ui.add_space(6.0);
                ui.separator();
                egui::ScrollArea::vertical()
                    .id_salt("offline_picker_scroll")
                    .auto_shrink([false, false])
                    .max_height(260.0)
                    .show(ui, |ui| {
                        ui.set_min_width(380.0);
                        if loading {
                            ui.vertical_centered(|ui| {
                                ui.add_space(24.0);
                                ui.spinner();
                                ui.add_space(24.0);
                            });
                        } else if folders.is_empty() {
                            ui.vertical_centered(|ui| {
                                ui.add_space(24.0);
                                ui.label(RichText::new("此目录下没有子文件夹").weak());
                                ui.add_space(24.0);
                            });
                        } else {
                            for f in &folders {
                                if folder_row(ui, th, &f.name) {
                                    enter = Some((f.id.clone(), f.name.clone()));
                                }
                                ui.add_space(2.0);
                            }
                        }
                    });
                ui.separator();
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Button::new(RichText::new("选择此目录").color(Color32::WHITE))
                                .fill(th.accent)
                                .stroke(Stroke::NONE),
                        )
                        .clicked()
                    {
                        confirm = true;
                    }
                    if ui.button("取消").clicked() {
                        close = true;
                    }
                    if let Some((_, name)) = cur_dest.as_ref() {
                        ui.label(RichText::new(format!("当前: {name}")).color(th.text_faint));
                    }
                });
            });
        if let Some(i) = nav_to {
            self.picker_stack.truncate(i + 1);
            self.picker_list(g);
        }
        if let Some((id, name)) = enter {
            self.picker_stack.push(Crumb {
                id: Some(id),
                label: name,
            });
            self.picker_list(g);
        }
        if confirm {
            self.dest = self
                .picker_stack
                .last()
                .and_then(|c| c.id.clone().map(|id| (id, c.label.clone())));
            self.picker_open = false;
        }
        if close {
            self.picker_open = false;
        }
    }
}
