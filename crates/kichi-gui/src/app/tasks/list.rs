//! 离线任务页渲染: 阶段页签 / 新建表单 / 批量操作条 / 任务列表与「加载更多」。

use std::collections::HashSet;

use eframe::egui::{self, vec2, Align, FontId, Frame, Key, Layout, Margin, RichText, Stroke};

use kichi_core::types::{task_id, Task};

use crate::format;
use crate::icons::{self, Glyph};
use crate::msg::Cmd;
use crate::theme::{mix, Theme};

use super::super::global::Global;
use super::super::helpers::{input, paint_checkbox, CheckState};
use super::card::{task_card, TASK_CARD_H};
use super::{OfflineTab, TaskOp, TaskSel};
use super::{TasksAction, TasksPage};

impl TasksPage {
    /// 离线任务页顶部的阶段页签按钮。
    fn task_tab_button(
        &mut self,
        ui: &mut egui::Ui,
        th: &Theme,
        tab: OfflineTab,
        active: OfflineTab,
    ) {
        let selected = tab == active;
        let count = self.buckets.get(tab.phase()).map_or(0, |v| v.len());
        let text = RichText::new(format!("{} {count}", format::phase_label(tab.phase())))
            .size(12.5)
            .color(if selected { th.on_accent } else { th.text_weak });
        let btn = egui::Button::new(text)
            .fill(if selected {
                th.accent
            } else {
                egui::Color32::TRANSPARENT
            })
            .stroke(Stroke::new(
                1.0,
                if selected { th.accent } else { th.border },
            ))
            .corner_radius(th.cr(8));
        if ui.add(btn).clicked() && !selected {
            self.tab = Some(tab);
            self.selected.clear();
            self.anchor = None;
        }
    }

    /// 渲染离线任务页, 返回本帧产生的跨域动作。
    pub(crate) fn show(
        &mut self,
        ctx: &egui::Context,
        th: &Theme,
        g: &mut Global,
    ) -> Vec<TasksAction> {
        let mut create = false;
        let mut do_refresh = false;
        let mut load_more = false;
        let mut ops: Vec<TaskOp> = Vec::new();
        let mut actions: Vec<TasksAction> = Vec::new();

        // 选中项批量操作条(底部固定, 与传输任务一致)
        if !self.selected.is_empty() {
            let active = self.active_tab();
            egui::TopBottomPanel::bottom("tasks_action_bar")
                .frame(Frame::new().fill(th.bg).inner_margin(Margin {
                    left: 20,
                    right: 20,
                    top: 0,
                    bottom: 0,
                }))
                .show(ctx, |ui| {
                    let top = ui.max_rect().min.y;
                    ui.painter()
                        .hline(ui.max_rect().x_range(), top, Stroke::new(1.0, th.border));
                    ui.add_space(8.0);
                    let selected: Vec<String> = self.selected.iter().cloned().collect();
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("已选择 {} 项", selected.len()))
                                .size(13.0)
                                .color(th.text),
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui
                                .add(
                                    egui::Button::new(RichText::new("删除选中").color(th.danger))
                                        .stroke(Stroke::new(1.0, mix(th.danger, th.bg, 0.35)))
                                        .fill(egui::Color32::TRANSPARENT)
                                        .corner_radius(th.cr(8)),
                                )
                                .clicked()
                            {
                                g.send(Cmd::OfflineDelete {
                                    task_ids: selected.clone(),
                                    delete_files: false,
                                });
                                self.selected.clear();
                                self.anchor = None;
                            }
                            if active == OfflineTab::Error
                                && ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new("重试选中").color(th.on_accent),
                                        )
                                        .fill(th.accent)
                                        .stroke(Stroke::NONE)
                                        .corner_radius(th.cr(8)),
                                    )
                                    .clicked()
                            {
                                for id in &selected {
                                    g.send(Cmd::OfflineRetry {
                                        task_id: id.clone(),
                                    });
                                }
                                self.selected.clear();
                                self.anchor = None;
                            }
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("取消选中").color(th.text_weak),
                                    )
                                    .frame(false),
                                )
                                .clicked()
                            {
                                self.selected.clear();
                                self.anchor = None;
                            }
                        });
                    });
                    ui.add_space(8.0);
                });
        }

        egui::CentralPanel::default()
            .frame(Frame::new().fill(th.bg).inner_margin(Margin {
                left: 20,
                right: 20,
                top: 16,
                bottom: 12,
            }))
            .show(ctx, |ui| {
                // 标题
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(22.0, 22.0), egui::Sense::hover());
                    icons::paint(ui.painter(), r, Glyph::Transfer, th.accent);
                    ui.label(RichText::new("离线下载").size(19.0).strong().color(th.text));
                });
                ui.add_space(4.0);
                ui.label(
                    RichText::new("将磁力 / 直链先转存到云端, 完成后在「我的文件」中查看。")
                        .color(th.text_weak)
                        .size(12.5),
                );
                ui.add_space(10.0);

                // 新建离线下载(常显, 使用频率较高)
                egui::Frame::new()
                    .fill(th.card)
                    .stroke(Stroke::new(1.0, th.border))
                    .corner_radius(th.cr(14))
                    .inner_margin(Margin::same(16))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new("新建离线下载")
                                .size(14.0)
                                .strong()
                                .color(th.text),
                        );
                        ui.add_space(10.0);
                        // 链接 + 文件名同一行: 链接自适应, 右侧固定宽给文件名。
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("链接 / 磁力").color(th.text_weak));
                            const NAME_W: f32 = 200.0;
                            let name_label_w = ui
                                .painter()
                                .layout_no_wrap(
                                    "文件名".to_string(),
                                    FontId::proportional(14.0),
                                    th.text_weak,
                                )
                                .size()
                                .x;
                            let url_w =
                                (ui.available_width() - name_label_w - NAME_W - 24.0).max(160.0);
                            let resp = ui.add(
                                input(&mut self.url)
                                    .desired_width(url_w)
                                    .hint_text("magnet:?xt=... 或 https://..."),
                            );
                            if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                                create = true;
                            }
                            ui.add_space(10.0);
                            ui.label(RichText::new("文件名").color(th.text_weak));
                            ui.add(
                                input(&mut self.name)
                                    .desired_width(NAME_W)
                                    .hint_text("可选, 留空自动识别"),
                            );
                        });
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("保存到").color(th.text_weak));
                            let label = match &self.dest {
                                Some((_, name)) => format!("网盘目录 · {name}"),
                                None => "离线默认目录".to_string(),
                            };
                            if ui.button(RichText::new(label).color(th.accent)).clicked() {
                                self.open_picker(g);
                            }
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                let enabled = !self.url.trim().is_empty();
                                if ui
                                    .add_enabled(
                                        enabled,
                                        egui::Button::new(
                                            RichText::new("提交下载").color(th.on_accent),
                                        )
                                        .fill(th.accent)
                                        .stroke(Stroke::NONE)
                                        .corner_radius(th.cr(8)),
                                    )
                                    .clicked()
                                {
                                    create = true;
                                }
                            });
                        });
                    });
                ui.add_space(8.0);

                // 页签行的可见项(全选 / 计数用); 按点击前的页签计算, 点击后下方列表本帧即切换。
                let row_active = self.active_tab();
                let row_ids: Vec<String> = self
                    .buckets
                    .get(row_active.phase())
                    .map(|v| v.iter().filter_map(task_id).collect())
                    .unwrap_or_default();

                // 阶段页签 + 全选 / 计数 + 刷新(同一行, 与传输任务一致)
                ui.horizontal(|ui| {
                    for tab in OfflineTab::ALL {
                        self.task_tab_button(ui, th, tab, row_active);
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let refreshing = self.refreshing;
                        let (r, ico) =
                            ui.allocate_exact_size(vec2(22.0, 22.0), egui::Sense::click());
                        icons::paint(ui.painter(), r, Glyph::Refresh, th.text_weak);
                        let ico_clicked = !refreshing && ico.clicked();
                        let btn = ui.add_enabled(
                            !refreshing,
                            egui::Button::new(
                                RichText::new(if refreshing {
                                    "正在刷新…"
                                } else {
                                    "刷新任务"
                                })
                                .color(th.text_weak),
                            )
                            .frame(false),
                        );
                        if btn.clicked() || ico_clicked {
                            do_refresh = true;
                        }
                        if refreshing {
                            ui.add(egui::Spinner::new().size(14.0).color(th.text_weak));
                        }

                        if !row_ids.is_empty() {
                            ui.add_space(14.0);
                            let vis_total = row_ids.len();
                            let vis_selected = row_ids
                                .iter()
                                .filter(|id| self.selected.contains(*id))
                                .count();
                            let master = if vis_total == 0 || vis_selected == 0 {
                                CheckState::Unchecked
                            } else if vis_selected >= vis_total {
                                CheckState::Checked
                            } else {
                                CheckState::Partial
                            };
                            let (cb_rect, cb_resp) =
                                ui.allocate_exact_size(vec2(16.0, 16.0), egui::Sense::click());
                            paint_checkbox(ui.painter(), th, cb_rect, master, cb_resp.hovered());
                            if cb_resp.clicked() {
                                if master == CheckState::Checked {
                                    for id in &row_ids {
                                        self.selected.remove(id);
                                    }
                                } else {
                                    for id in &row_ids {
                                        self.selected.insert(id.clone());
                                    }
                                }
                            }
                            ui.add_space(6.0);
                            ui.label(RichText::new("全选").color(th.text_weak).size(12.5));
                            ui.add_space(14.0);
                            if vis_selected > 0 {
                                ui.label(
                                    RichText::new(format!("已选 {vis_selected}/{vis_total}"))
                                        .color(th.accent)
                                        .size(12.5),
                                );
                            } else {
                                ui.label(
                                    RichText::new(format!("共 {vis_total} 个"))
                                        .color(th.text_faint)
                                        .size(12.5),
                                );
                            }
                        }
                    });
                });
                ui.add_space(6.0);

                // 点击页签后本帧立即生效(重新读取当前页签)。
                let active = self.active_tab();
                let phase = active.phase();
                let ids: Vec<String> = self
                    .buckets
                    .get(phase)
                    .map(|v| v.iter().filter_map(task_id).collect())
                    .unwrap_or_default();
                let task_count = self.buckets.get(phase).map_or(0, |v| v.len());

                // 剔除已不在当前页签的选中项。
                let id_set: HashSet<&str> = ids.iter().map(|s| s.as_str()).collect();
                self.selected.retain(|id| id_set.contains(id.as_str()));

                if task_count == 0 {
                    ui.centered_and_justified(|ui| {
                        ui.add_space(60.0);
                        let (r, _) = ui.allocate_exact_size(vec2(64.0, 64.0), egui::Sense::hover());
                        icons::paint(ui.painter(), r, Glyph::Transfer, th.text_faint);
                        ui.add_space(10.0);
                        ui.label(
                            RichText::new(format!("「{}」下暂无任务", format::phase_label(phase)))
                                .color(th.text_weak)
                                .size(14.0),
                        );
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new("展开上方「新建离线下载」提交磁力 / 直链")
                                .color(th.text_faint)
                                .size(12.0),
                        );
                        ui.add_space(60.0);
                    });
                    return;
                }

                let ctrl = ui.input(|i| i.modifiers.ctrl);
                let shift = ui.input(|i| i.modifiers.shift);
                let mut sel_reqs: Vec<TaskSel> = Vec::new();

                let tasks: &[Task] = self.buckets.get(phase).map(|v| v.as_slice()).unwrap_or(&[]);
                let scroll_h = (ui.available_height() - 36.0).max(60.0);
                egui::ScrollArea::vertical()
                    .id_salt("tasks_scroll")
                    .auto_shrink([false, false])
                    .max_height(scroll_h)
                    .show_rows(ui, TASK_CARD_H, tasks.len(), |ui, range| {
                        for i in range {
                            let t = &tasks[i];
                            let is_sel = task_id(t).is_some_and(|id| self.selected.contains(&id));
                            let (op, sel) = task_card(ui, th, active, i, t, is_sel, ctrl, shift);
                            if let Some(op) = op {
                                ops.push(op);
                            }
                            if let Some(sel) = sel {
                                sel_reqs.push(sel);
                            }
                        }
                    });

                // 应用选择请求
                for sel in sel_reqs {
                    match sel {
                        TaskSel::Replace(id) => {
                            self.selected.clear();
                            self.selected.insert(id.clone());
                            self.anchor = Some(id);
                        }
                        TaskSel::Toggle(id) => {
                            if !self.selected.remove(&id) {
                                self.selected.insert(id.clone());
                            }
                            self.anchor = Some(id);
                        }
                        TaskSel::Range(id) => {
                            if let Some(anchor) = self.anchor.clone() {
                                let start = ids.iter().position(|x| *x == anchor).unwrap_or(0);
                                let end = ids.iter().position(|x| *x == id).unwrap_or(0);
                                let (from, to) = if start <= end {
                                    (start, end)
                                } else {
                                    (end, start)
                                };
                                if !ctrl {
                                    self.selected.clear();
                                }
                                for x in &ids[from..=to] {
                                    self.selected.insert(x.clone());
                                }
                            } else {
                                self.selected.clear();
                                self.selected.insert(id.clone());
                            }
                            self.anchor = Some(id);
                        }
                    }
                }

                // 加载更多(当前页签各自的分页游标)
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let has_next = self
                        .next_tokens
                        .get(phase)
                        .and_then(|t| t.as_ref())
                        .is_some();
                    if has_next {
                        if self.loading_more.contains(phase) {
                            ui.add(egui::Spinner::new().size(14.0).color(th.text_weak));
                            ui.label(RichText::new("正在加载…").color(th.text_weak).size(12.0));
                        } else if ui
                            .button(RichText::new("加载更多").color(th.accent).size(12.5))
                            .clicked()
                        {
                            load_more = true;
                        }
                    }
                });
            });

        if create {
            let url = self.url.trim().to_string();
            let name = {
                let n = self.name.trim();
                if n.is_empty() {
                    None
                } else {
                    Some(n.to_string())
                }
            };
            let parent = self.dest.as_ref().map(|(id, _)| id.clone());
            g.send(Cmd::OfflineCreate { url, name, parent });
        }
        if do_refresh {
            self.refreshing = true;
            g.send(Cmd::RefreshTasks);
        }
        if load_more {
            let phase = self.active_tab().phase();
            self.load_more(g, phase);
        }
        for op in ops {
            match op {
                TaskOp::Download(fid, name) => {
                    actions.push(TasksAction::Download { id: fid, name })
                }
                TaskOp::Retry(id) => {
                    g.send(Cmd::OfflineRetry { task_id: id });
                }
                TaskOp::Delete(id) => {
                    g.send(Cmd::OfflineDelete {
                        task_ids: vec![id],
                        delete_files: false,
                    });
                }
            }
        }
        actions
    }
}
