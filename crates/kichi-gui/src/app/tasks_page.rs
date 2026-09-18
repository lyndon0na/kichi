use std::collections::HashSet;

use eframe::egui::{
    self, vec2, Align, FontId, Frame, Key, Layout, Margin, Pos2, Rect, RichText, Stroke,
};

use kichi_core::types::{task_created, task_file_id, task_id, task_name, task_size, Task};

use crate::format;
use crate::icons::{self, Glyph};
use crate::msg::Cmd;
use crate::theme::{mix, Theme};

use super::helpers::{card_shell, icon_action, input, paint_checkbox, truncate_text, CheckState};
use super::types::{OfflineTab, TaskOp, TaskSel};
use super::App;

/// 离线任务卡片固定高度(虚拟滚动要求逐行等高)。
const TASK_CARD_H: f32 = 64.0;

/// 渲染单张离线任务卡片, 返回行级操作与选择请求。
#[allow(clippy::too_many_arguments)]
fn task_card(
    ui: &mut egui::Ui,
    th: &Theme,
    tab: OfflineTab,
    idx: usize,
    t: &Task,
    is_sel: bool,
    ctrl: bool,
    shift: bool,
) -> (Option<TaskOp>, Option<TaskSel>) {
    let mut op: Option<TaskOp> = None;
    let mut sel: Option<TaskSel> = None;
    let w = ui.available_width().max(320.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, TASK_CARD_H), egui::Sense::click());
    let painter = ui.painter().clone();

    card_shell(&painter, th, rect, resp.hovered(), is_sel);

    let inner = rect.shrink2(vec2(12.0, 10.0));

    // 复选框
    let cb_rect = Rect::from_center_size(
        Pos2::new(inner.min.x + 8.0, inner.center().y),
        vec2(16.0, 16.0),
    );
    let cb_resp = ui.interact(
        cb_rect,
        ui.id().with(("task_check", idx)),
        egui::Sense::click(),
    );
    if cb_resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    paint_checkbox(
        &painter,
        th,
        cb_rect,
        if is_sel {
            CheckState::Checked
        } else {
            CheckState::Unchecked
        },
        resp.hovered() || cb_resp.hovered(),
    );
    let cb_clicked = cb_resp.clicked();

    // 列宽: 右侧统一预留两个图标按钮位, 保证各卡片列对齐。
    const CB_W: f32 = 26.0;
    const BTN_SZ: f32 = 28.0;
    const BTN_GAP: f32 = 4.0;
    const BTN_RSV: f32 = BTN_SZ * 2.0 + BTN_GAP;
    let content_x = inner.min.x + CB_W;
    let right_start = inner.max.x - BTN_RSV - 4.0;
    let text_w = (right_start - content_x - 12.0).max(80.0);

    let name = task_name(t).unwrap_or_else(|| "未知任务".into());
    let name_g = truncate_text(&painter, &name, text_w, FontId::proportional(13.5), th.text);
    let truncated = name_g.text() != name.as_str();
    painter.galley(Pos2::new(content_x, inner.min.y + 2.0), name_g, th.text);

    // 次要行: 大小 · 创建时间
    let mut meta = String::new();
    if let Some(sz) = task_size(t) {
        if sz > 0 {
            meta.push_str(&format::fmt_bytes(sz));
        }
    }
    if let Some(created) = task_created(t) {
        let c = format::fmt_time(&created);
        if !c.is_empty() {
            if !meta.is_empty() {
                meta.push_str(" · ");
            }
            meta.push_str(&c);
        }
    }
    if !meta.is_empty() {
        let mg = truncate_text(
            &painter,
            &meta,
            text_w,
            FontId::proportional(11.5),
            th.text_weak,
        );
        painter.galley(Pos2::new(content_x, inner.min.y + 22.0), mg, th.text_weak);
    }

    // 右侧图标操作(按阶段决定)
    let tid = task_id(t);
    let fid = task_file_id(t).filter(|s| !s.is_empty());
    let mut btns: Vec<(Glyph, &str, TaskOp)> = Vec::new();
    match tab {
        OfflineTab::Complete => {
            if let Some(fid) = fid {
                btns.push((
                    Glyph::Download,
                    "下载到本地",
                    TaskOp::Download(fid, name.clone()),
                ));
            }
            if let Some(id) = tid.clone() {
                btns.push((Glyph::Trash, "删除任务记录", TaskOp::Delete(id)));
            }
        }
        OfflineTab::Error => {
            if let Some(id) = tid.clone() {
                btns.push((Glyph::Refresh, "重试", TaskOp::Retry(id)));
            }
            if let Some(id) = tid.clone() {
                btns.push((Glyph::Trash, "删除任务记录", TaskOp::Delete(id)));
            }
        }
        OfflineTab::Pending | OfflineTab::Running => {
            if let Some(id) = tid.clone() {
                btns.push((Glyph::Trash, "删除任务记录", TaskOp::Delete(id)));
            }
        }
    }
    let btn_y = inner.center().y - BTN_SZ / 2.0;
    let mut bx = inner.max.x;
    for (glyph, tip, dop) in btns.into_iter().rev() {
        let r = Rect::from_min_max(Pos2::new(bx - BTN_SZ, btn_y), Pos2::new(bx, btn_y + BTN_SZ));
        bx -= BTN_SZ + BTN_GAP;
        let id = ui.id().with(("task_btn", idx, tip));
        if icon_action(ui, &painter, th, r, id, glyph, tip, glyph == Glyph::Trash) {
            op = Some(dop);
        }
    }

    // 名称被截断时, hover 整行显示完整文件名。
    if truncated {
        resp.clone().on_hover_text(&name);
    }

    if cb_clicked {
        if let Some(id) = tid {
            sel = Some(TaskSel::Toggle(id));
        }
    } else if op.is_none() && resp.clicked() {
        if let Some(id) = tid {
            sel = Some(if shift {
                TaskSel::Range(id)
            } else if ctrl {
                TaskSel::Toggle(id)
            } else {
                TaskSel::Replace(id)
            });
        }
    }

    (op, sel)
}

impl App {
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
            self.tasks_tab = Some(tab);
            self.tasks_selected.clear();
            self.tasks_anchor = None;
        }
    }

    pub(super) fn tasks_page(&mut self, ctx: &egui::Context, th: &Theme) {
        let mut create = false;
        let mut do_refresh = false;
        let mut load_more = false;
        let mut ops: Vec<TaskOp> = Vec::new();

        // 选中项批量操作条(底部固定, 与传输任务一致)
        if !self.tasks_selected.is_empty() {
            let active = self.active_tasks_tab();
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
                    let selected: Vec<String> = self.tasks_selected.iter().cloned().collect();
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
                                self.send(Cmd::OfflineDelete {
                                    task_ids: selected.clone(),
                                    delete_files: false,
                                });
                                self.tasks_selected.clear();
                                self.tasks_anchor = None;
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
                                    self.send(Cmd::OfflineRetry {
                                        task_id: id.clone(),
                                    });
                                }
                                self.tasks_selected.clear();
                                self.tasks_anchor = None;
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
                                self.tasks_selected.clear();
                                self.tasks_anchor = None;
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

                // 新建离线下载(可折叠, 默认收起以让位给列表)
                egui::CollapsingHeader::new(
                    RichText::new("新建离线下载")
                        .size(13.5)
                        .strong()
                        .color(th.text),
                )
                .default_open(false)
                .show(ui, |ui| {
                    egui::Frame::new()
                        .fill(th.card)
                        .stroke(Stroke::new(1.0, th.border))
                        .corner_radius(th.cr(14))
                        .inner_margin(Margin::same(16))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("链接 / 磁力").color(th.text_weak));
                                let resp = ui.add(
                                    input(&mut self.offline_url)
                                        .desired_width(f32::INFINITY)
                                        .hint_text("magnet:?xt=... 或 https://..."),
                                );
                                if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                                    create = true;
                                }
                            });
                            ui.add_space(8.0);
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("文件名").color(th.text_weak));
                                ui.add(
                                    input(&mut self.offline_name)
                                        .desired_width(280.0)
                                        .hint_text("可选, 留空自动识别"),
                                );
                                ui.add_space(8.0);
                                let enabled = !self.offline_url.trim().is_empty();
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
                            ui.add_space(8.0);
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("保存到").color(th.text_weak));
                                let label = match &self.offline_dest {
                                    Some((_, name)) => format!("网盘目录 · {name}"),
                                    None => "离线默认目录".to_string(),
                                };
                                if ui.button(RichText::new(label).color(th.accent)).clicked() {
                                    self.open_offline_picker();
                                }
                            });
                        });
                });
                ui.add_space(8.0);

                // 阶段页签 + 刷新
                ui.horizontal(|ui| {
                    let active = self.active_tasks_tab();
                    for tab in OfflineTab::ALL {
                        self.task_tab_button(ui, th, tab, active);
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let refreshing = self.tasks_refreshing;
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
                    });
                });
                ui.add_space(6.0);

                // 点击页签后本帧立即生效(重新读取当前页签)。
                let active = self.active_tasks_tab();
                let phase = active.phase();
                let ids: Vec<String> = self
                    .buckets
                    .get(phase)
                    .map(|v| v.iter().filter_map(task_id).collect())
                    .unwrap_or_default();
                let task_count = self.buckets.get(phase).map_or(0, |v| v.len());

                // 剔除已不在当前页签的选中项。
                let id_set: HashSet<&str> = ids.iter().map(|s| s.as_str()).collect();
                self.tasks_selected
                    .retain(|id| id_set.contains(id.as_str()));

                // 全选 + 计数
                if task_count > 0 {
                    ui.horizontal(|ui| {
                        let vis_total = ids.len();
                        let vis_selected = ids
                            .iter()
                            .filter(|id| self.tasks_selected.contains(*id))
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
                                for id in &ids {
                                    self.tasks_selected.remove(id);
                                }
                            } else {
                                for id in &ids {
                                    self.tasks_selected.insert(id.clone());
                                }
                            }
                        }
                        ui.add_space(6.0);
                        ui.label(RichText::new("全选").color(th.text_weak).size(12.5));
                        ui.add_space(12.0);
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
                    });
                    ui.add_space(4.0);
                }

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
                            let is_sel =
                                task_id(t).is_some_and(|id| self.tasks_selected.contains(&id));
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
                            self.tasks_selected.clear();
                            self.tasks_selected.insert(id.clone());
                            self.tasks_anchor = Some(id);
                        }
                        TaskSel::Toggle(id) => {
                            if !self.tasks_selected.remove(&id) {
                                self.tasks_selected.insert(id.clone());
                            }
                            self.tasks_anchor = Some(id);
                        }
                        TaskSel::Range(id) => {
                            if let Some(anchor) = self.tasks_anchor.clone() {
                                let start = ids.iter().position(|x| *x == anchor).unwrap_or(0);
                                let end = ids.iter().position(|x| *x == id).unwrap_or(0);
                                let (from, to) = if start <= end {
                                    (start, end)
                                } else {
                                    (end, start)
                                };
                                if !ctrl {
                                    self.tasks_selected.clear();
                                }
                                for x in &ids[from..=to] {
                                    self.tasks_selected.insert(x.clone());
                                }
                            } else {
                                self.tasks_selected.clear();
                                self.tasks_selected.insert(id.clone());
                            }
                            self.tasks_anchor = Some(id);
                        }
                    }
                }

                // 加载更多(当前页签各自的分页游标)
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let has_next = self
                        .buckets_next
                        .get(phase)
                        .and_then(|t| t.as_ref())
                        .is_some();
                    if has_next {
                        if self.tasks_loading_more.contains(phase) {
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
            let url = self.offline_url.trim().to_string();
            let name = {
                let n = self.offline_name.trim();
                if n.is_empty() {
                    None
                } else {
                    Some(n.to_string())
                }
            };
            let parent = self.offline_dest.as_ref().map(|(id, _)| id.clone());
            self.send(Cmd::OfflineCreate { url, name, parent });
        }
        if do_refresh {
            self.tasks_refreshing = true;
            self.send(Cmd::RefreshTasks);
        }
        if load_more {
            let phase = self.active_tasks_tab().phase();
            self.load_more_tasks(phase);
        }
        for op in ops {
            match op {
                TaskOp::Download(fid, name) => self.download_single(fid, name),
                TaskOp::Retry(id) => {
                    let tx = self.tx.clone();
                    let _ = tx.send(Cmd::OfflineRetry { task_id: id });
                }
                TaskOp::Delete(id) => {
                    let tx = self.tx.clone();
                    let _ = tx.send(Cmd::OfflineDelete {
                        task_ids: vec![id],
                        delete_files: false,
                    });
                }
            }
        }
    }
}
