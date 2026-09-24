//! 传输页「上传」分栏: 顶部操作行、筛选汇总、任务列表与底部批量操作条。

use eframe::egui::{
    self, vec2, Align, FontId, Frame, Layout, Margin, Pos2, Rect, RichText, Stroke,
};

use crate::format;
use crate::icons::{self, Glyph};
use crate::msg::Cmd;
use crate::theme::{mix, Theme};

use super::super::global::Global;
use super::super::helpers::{card_shell, icon_action, paint_checkbox, truncate_text, CheckState};
use super::super::types::{DlSel, UlFilter, UlJob, UlOp, UlStatus};
use super::{TransfersAction, TransfersPage, DL_CARD_H};

/// 上传行状态文案。
fn upload_status_line(job: &UlJob) -> (egui::Color32, String) {
    match &job.status {
        UlStatus::Queued => (egui::Color32::from_gray(150), "排队中".into()),
        UlStatus::Running => (egui::Color32::from_rgb(60, 130, 200), "上传中".into()),
        UlStatus::Done => (egui::Color32::from_rgb(70, 150, 90), "已完成".into()),
        UlStatus::Failed(what) => (egui::Color32::from_rgb(217, 70, 60), what.clone()),
    }
}

/// 渲染单个上传任务卡片, 返回操作和选择请求。
#[allow(clippy::too_many_arguments)]
fn ul_card(
    ui: &mut egui::Ui,
    th: &Theme,
    rid: u64,
    job: &UlJob,
    is_sel: bool,
    ctrl: bool,
    shift: bool,
    now: u64,
) -> (Option<UlOp>, Option<DlSel>) {
    let mut op: Option<UlOp> = None;
    let mut sel: Option<DlSel> = None;
    let w = ui.available_width().max(320.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, DL_CARD_H), egui::Sense::click());
    let painter = ui.painter().clone();

    // 背景 / 选中态
    card_shell(&painter, th, rect, resp.hovered(), is_sel);

    let inner = rect.shrink2(vec2(12.0, 10.0));

    // 复选框
    let cb_rect = Rect::from_center_size(
        Pos2::new(inner.min.x + 8.0, inner.center().y),
        vec2(16.0, 16.0),
    );
    let cb_resp = ui.interact(
        cb_rect,
        ui.id().with(("ul_check", rid)),
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

    const CB_W: f32 = 26.0;
    const BTN_W: f32 = 92.0;
    let content_x = inner.min.x + CB_W;
    let right_start = inner.max.x - BTN_W;
    let left_w = ((right_start - content_x - 16.0) * 0.46).max(120.0);

    // 左列: 名称 / 状态(含相对时间) / 目标路径
    let name_g = truncate_text(
        &painter,
        &job.name,
        left_w,
        FontId::proportional(13.5),
        th.text,
    );
    painter.galley(Pos2::new(content_x, inner.min.y + 1.0), name_g, th.text);
    let (col, mut txt) = upload_status_line(job);
    if job.is_dir && job.files_total > 0 {
        txt = format!("{txt} · 文件 {}/{}", job.files_done, job.files_total);
    }
    if let Some(at) = job.at {
        let rel = format::fmt_rel(now, at);
        if !rel.is_empty() {
            txt = format!("{txt} · {rel}");
        }
    }
    let st_g = truncate_text(&painter, &txt, left_w, FontId::proportional(11.5), col);
    painter.galley(Pos2::new(content_x, inner.min.y + 18.0), st_g, col);
    let dest = job.dest_label();
    if !dest.is_empty() {
        let dg = truncate_text(
            &painter,
            &format!("→ {dest}"),
            left_w,
            FontId::proportional(10.5),
            th.text_faint,
        );
        painter.galley(Pos2::new(content_x, inner.min.y + 35.0), dg, th.text_faint);
    }

    // 中间: 进度 / 速率 / 剩余时间
    let mid_x = content_x + left_w + 14.0;
    let mid_w = (right_start - 14.0 - mid_x).max(0.0);
    if mid_w > 40.0 {
        match &job.status {
            UlStatus::Running if job.total > 0 => {
                let frac = (job.done as f32 / job.total as f32).clamp(0.0, 1.0);
                let bar = Rect::from_min_max(
                    Pos2::new(mid_x, inner.center().y - 10.0),
                    Pos2::new(mid_x + mid_w, inner.center().y + 2.0),
                );
                painter.rect_filled(bar, th.cr(3), mix(th.text_faint, th.bg, 0.7));
                if frac > 0.0 {
                    painter.rect_filled(
                        Rect::from_min_max(bar.min, Pos2::new(bar.min.x + mid_w * frac, bar.max.y)),
                        th.cr(3),
                        th.accent,
                    );
                }
                let mut info = format!(
                    "{} / {}",
                    format::fmt_bytes(job.done as i64),
                    format::fmt_bytes(job.total as i64)
                );
                if job.speed > 0 {
                    info.push_str(&format!("   {}/s", format::fmt_bytes(job.speed as i64)));
                    let eta = format::fmt_eta(job.total.saturating_sub(job.done), job.speed);
                    if !eta.is_empty() {
                        info.push_str(&format!("   剩余 {eta}"));
                    }
                }
                painter.text(
                    Pos2::new(mid_x, inner.center().y + 6.0),
                    egui::Align2::LEFT_TOP,
                    info,
                    FontId::proportional(10.5),
                    th.text_weak,
                );
            }
            UlStatus::Running => {
                let info = if job.is_dir {
                    if job.current.is_empty() {
                        "准备中…".to_string()
                    } else {
                        job.current.clone()
                    }
                } else if job.done > 0 {
                    format!("已上传 {}", format::fmt_bytes(job.done as i64))
                } else {
                    "计算哈希 / 连接中…".to_string()
                };
                painter.text(
                    Pos2::new(mid_x, inner.center().y - 7.0),
                    egui::Align2::LEFT_TOP,
                    info,
                    FontId::proportional(11.5),
                    th.text_weak,
                );
            }
            _ => {
                if job.done > 0 {
                    painter.text(
                        Pos2::new(mid_x, inner.center().y - 7.0),
                        egui::Align2::LEFT_TOP,
                        format!("已上传 {}", format::fmt_bytes(job.done as i64)),
                        FontId::proportional(11.5),
                        th.text_weak,
                    );
                }
            }
        }
    }

    // 图标操作按钮(右对齐)
    let mut btns: Vec<(Glyph, &str, UlOp)> = Vec::new();
    match &job.status {
        UlStatus::Queued | UlStatus::Running => btns.push((Glyph::Close, "取消上传", UlOp::Cancel)),
        UlStatus::Failed(_) => {
            btns.push((Glyph::Refresh, "重试上传", UlOp::Retry));
            btns.push((Glyph::Trash, "从列表移除", UlOp::Remove));
        }
        UlStatus::Done => {
            btns.push((Glyph::Folder, "在网盘中打开", UlOp::OpenInDrive));
            btns.push((Glyph::Trash, "从列表移除", UlOp::Remove));
        }
    }
    let btn_sz = 28.0;
    let btn_gap = 4.0;
    let btn_y = inner.center().y - btn_sz / 2.0;
    let mut bx = inner.max.x;
    for (glyph, tip, dop) in btns.into_iter().rev() {
        let r = Rect::from_min_max(Pos2::new(bx - btn_sz, btn_y), Pos2::new(bx, btn_y + btn_sz));
        bx -= btn_sz + btn_gap;
        let id = ui.id().with(("ul_btn", rid, tip));
        if icon_action(ui, &painter, th, r, id, glyph, tip, glyph == Glyph::Trash) {
            op = Some(dop);
        }
    }

    // 失败原因完整展示
    if let UlStatus::Failed(what) = &job.status {
        if !what.is_empty() {
            resp.clone().on_hover_text(what);
        }
    }

    if cb_clicked {
        sel = Some(DlSel::Toggle(rid));
    } else if op.is_none() && resp.clicked() {
        if shift {
            sel = Some(DlSel::Range(rid));
        } else if ctrl {
            sel = Some(DlSel::Toggle(rid));
        } else {
            sel = Some(DlSel::Replace(rid));
        }
    }

    (op, sel)
}

impl TransfersPage {
    /// 传输任务页「上传」分栏。
    pub(super) fn upload_tab(
        &mut self,
        ui: &mut egui::Ui,
        th: &Theme,
        g: &mut Global,
        actions: &mut Vec<TransfersAction>,
    ) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("上传本地文件到当前网盘目录。")
                    .color(th.text_weak)
                    .size(12.5),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add(
                        egui::Button::new(RichText::new("上传文件").color(th.on_accent))
                            .fill(th.accent)
                            .stroke(Stroke::NONE)
                            .corner_radius(th.cr(8)),
                    )
                    .clicked()
                {
                    actions.push(TransfersAction::PickFiles);
                }
                if ui
                    .add(
                        egui::Button::new(RichText::new("上传文件夹").color(th.text_weak))
                            .stroke(Stroke::new(1.0, th.border))
                            .fill(egui::Color32::TRANSPARENT)
                            .corner_radius(th.cr(8)),
                    )
                    .clicked()
                {
                    actions.push(TransfersAction::PickFolder);
                }
            });
        });
        ui.add_space(8.0);

        if self.ul_jobs.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.add_space(60.0);
                let (r, _) = ui.allocate_exact_size(vec2(64.0, 64.0), egui::Sense::hover());
                icons::paint(ui.painter(), r, Glyph::Upload, th.text_faint);
                ui.add_space(10.0);
                ui.label(RichText::new("暂无上传任务").color(th.text_weak).size(14.0));
                ui.add_space(4.0);
                ui.label(
                    RichText::new("在「我的文件」中选择文件后上传, 或点右上角「上传文件」")
                        .color(th.text_faint)
                        .size(12.0),
                );
                ui.add_space(60.0);
            });
            return;
        }

        // 筛选 + 汇总 + 全选 + 清除已完成
        let now = format::now_unix();
        let mut clear_done = false;
        ui.horizontal(|ui| {
            let total = self.ul_jobs.len();
            let active = self
                .ul_jobs
                .values()
                .filter(|j| matches!(j.status, UlStatus::Queued | UlStatus::Running))
                .count();
            let done_c = self
                .ul_jobs
                .values()
                .filter(|j| j.status == UlStatus::Done)
                .count();
            let failed = self
                .ul_jobs
                .values()
                .filter(|j| matches!(j.status, UlStatus::Failed(_)))
                .count();
            self.ul_filter_button(ui, th, UlFilter::All, &format!("全部 {total}"));
            self.ul_filter_button(ui, th, UlFilter::Active, &format!("进行中 {active}"));
            self.ul_filter_button(ui, th, UlFilter::Done, &format!("已完成 {done_c}"));
            self.ul_filter_button(ui, th, UlFilter::Failed, &format!("失败 {failed}"));

            let ids = self.visible_ul_ids();
            self.selected_ul.retain(|id| ids.contains(id));
            let vis_total = ids.len();
            let vis_selected = ids
                .iter()
                .filter(|id| self.selected_ul.contains(id))
                .count();
            let master = if vis_total == 0 || vis_selected == 0 {
                CheckState::Unchecked
            } else if vis_selected >= vis_total {
                CheckState::Checked
            } else {
                CheckState::Partial
            };

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if done_c > 0
                    && ui
                        .add(
                            egui::Button::new(RichText::new("清除已完成").color(th.text_weak))
                                .frame(false),
                        )
                        .clicked()
                {
                    clear_done = true;
                }
                ui.add_space(10.0);
                let (cb_rect, cb_resp) =
                    ui.allocate_exact_size(vec2(16.0, 16.0), egui::Sense::click());
                paint_checkbox(ui.painter(), th, cb_rect, master, cb_resp.hovered());
                if cb_resp.clicked() {
                    if master == CheckState::Checked {
                        for id in &ids {
                            self.selected_ul.remove(id);
                        }
                    } else {
                        for id in &ids {
                            self.selected_ul.insert(*id);
                        }
                    }
                }
                ui.add_space(12.0);
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
                ui.add_space(12.0);
                let (sum_done, sum_total, speed) =
                    self.ul_jobs
                        .values()
                        .fold((0u64, 0u64, 0u64), |(d, t, s), j| {
                            let sp = if matches!(j.status, UlStatus::Running) {
                                j.speed
                            } else {
                                0
                            };
                            (d + j.done, t + j.total, s + sp)
                        });
                if sum_total > 0 {
                    let pct = (sum_done as f32 / sum_total as f32 * 100.0).round() as u32;
                    let mut agg = format!(
                        "整体 {pct}% · {}/{}",
                        format::fmt_bytes(sum_done as i64),
                        format::fmt_bytes(sum_total as i64)
                    );
                    if speed > 0 {
                        agg.push_str(&format!(" · {}/s", format::fmt_bytes(speed as i64)));
                    }
                    ui.label(RichText::new(agg).color(th.text_faint).size(12.0));
                }
            });
        });
        ui.add_space(4.0);

        if clear_done {
            let done_ids: Vec<u64> = self
                .ul_jobs
                .iter()
                .filter(|(_, j)| j.status == UlStatus::Done)
                .map(|(id, _)| *id)
                .collect();
            for rid in done_ids {
                self.remove_upload_job(g, rid);
            }
        }

        let ids = self.visible_ul_ids();
        if ids.is_empty() {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new("该筛选下暂无任务")
                        .color(th.text_weak)
                        .size(13.0),
                );
            });
            return;
        }

        let mut ops: Vec<(u64, UlOp)> = Vec::new();
        let mut sel_reqs: Vec<DlSel> = Vec::new();
        let ctrl = ui.input(|i| i.modifiers.ctrl);
        let shift = ui.input(|i| i.modifiers.shift);
        let scroll_h = ui.available_height();
        egui::ScrollArea::vertical()
            .id_salt("uploads_scroll")
            .auto_shrink([false, false])
            .max_height(scroll_h.max(60.0))
            .show_rows(ui, DL_CARD_H, ids.len(), |ui, range| {
                for i in range {
                    let rid = ids[i];
                    let Some(job) = self.ul_jobs.get(&rid) else {
                        continue;
                    };
                    let is_sel = self.selected_ul.contains(&rid);
                    let (op, sel) = ul_card(ui, th, rid, job, is_sel, ctrl, shift, now);
                    if let Some(op) = op {
                        ops.push((rid, op));
                    }
                    if let Some(sel) = sel {
                        sel_reqs.push(sel);
                    }
                }
            });

        for sel in sel_reqs {
            match sel {
                DlSel::Replace(rid) => {
                    self.selected_ul.clear();
                    self.selected_ul.insert(rid);
                    self.ul_last_clicked = Some(rid);
                }
                DlSel::Toggle(rid) => {
                    if self.selected_ul.contains(&rid) {
                        self.selected_ul.remove(&rid);
                    } else {
                        self.selected_ul.insert(rid);
                    }
                    self.ul_last_clicked = Some(rid);
                }
                DlSel::Range(rid) => {
                    if let Some(anchor) = self.ul_last_clicked {
                        let start = ids.iter().position(|x| *x == anchor).unwrap_or(0);
                        let end = ids.iter().position(|x| *x == rid).unwrap_or(0);
                        let (from, to) = if start <= end {
                            (start, end)
                        } else {
                            (end, start)
                        };
                        if !ctrl {
                            self.selected_ul.clear();
                        }
                        for id in &ids[from..=to] {
                            self.selected_ul.insert(*id);
                        }
                    } else {
                        self.selected_ul.clear();
                        self.selected_ul.insert(rid);
                    }
                    self.ul_last_clicked = Some(rid);
                }
            }
        }

        for (rid, op) in ops {
            match op {
                UlOp::Cancel => g.send(Cmd::CancelUpload { req_id: rid }),
                UlOp::Remove => self.remove_upload_job(g, rid),
                UlOp::Retry => self.retry_upload_job(g, rid),
                UlOp::OpenInDrive => {
                    let stack = self.ul_jobs.get(&rid).map(|j| j.dest_stack.clone());
                    if let Some(stack) = stack {
                        Self::navigate_to_stack(actions, stack);
                    }
                }
            }
        }
    }

    /// 上传页底部批量操作条(与下载页一致); 选中非空时显示。
    pub(super) fn upload_action_bar(&mut self, ctx: &egui::Context, th: &Theme, g: &mut Global) {
        if self.selected_ul.is_empty() {
            return;
        }
        egui::TopBottomPanel::bottom("ul_action_bar")
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
                let retryable: Vec<u64> = self
                    .selected_ul
                    .iter()
                    .copied()
                    .filter(|rid| {
                        self.ul_jobs
                            .get(rid)
                            .map(|j| matches!(j.status, UlStatus::Failed(_)))
                            .unwrap_or(false)
                    })
                    .collect();
                let selected: Vec<u64> = self.selected_ul.iter().copied().collect();
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("已选择 {} 项", selected.len()))
                            .size(13.0)
                            .color(th.text),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add(
                                egui::Button::new(RichText::new("移除选中").color(th.danger))
                                    .stroke(Stroke::new(1.0, mix(th.danger, th.bg, 0.35)))
                                    .fill(egui::Color32::TRANSPARENT)
                                    .corner_radius(th.cr(8)),
                            )
                            .clicked()
                        {
                            for rid in selected.iter().copied() {
                                self.remove_upload_job(g, rid);
                            }
                        }
                        if !retryable.is_empty()
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
                            for rid in retryable.iter().copied() {
                                self.retry_upload_job(g, rid);
                            }
                        }
                        if ui
                            .add(
                                egui::Button::new(RichText::new("取消选中").color(th.text_weak))
                                    .frame(false),
                            )
                            .clicked()
                        {
                            self.selected_ul.clear();
                            self.ul_last_clicked = None;
                        }
                    });
                });
                ui.add_space(8.0);
            });
    }
}
