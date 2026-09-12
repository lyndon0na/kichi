use eframe::egui::{self, Align, FontId, Frame, Layout, Margin, Pos2, Rect, RichText, Stroke, vec2};

use crate::format;
use crate::icons::{self, Glyph};
use crate::msg::Cmd;
use crate::theme::{mix, Theme};

use super::helpers::{open_dir, truncate_text};
use super::types::{DlJob, DlOp, DlSel, DlStatus, TransferTab};
use super::App;

/// 下载行状态文案。
fn status_line(job: &DlJob) -> (egui::Color32, String) {
    match &job.status {
        DlStatus::Queued => (egui::Color32::from_gray(150), "排队中".into()),
        DlStatus::Running => (egui::Color32::from_rgb(60, 130, 200), "下载中".into()),
        DlStatus::Done => (
            egui::Color32::from_rgb(70, 150, 90),
            format!("已完成 · 保存于 {}", job.dir.display()),
        ),
        DlStatus::Cancelled => (egui::Color32::from_gray(150), "已取消(保留 .part 可续传)".into()),
        DlStatus::Failed(what) => (egui::Color32::from_rgb(217, 70, 60), what.clone()),
    }
}

/// 渲染单个下载任务卡片, 返回操作和选择请求。
fn dl_card(
    ui: &mut egui::Ui,
    th: &Theme,
    rid: u64,
    job: &DlJob,
    is_sel: bool,
    ctrl: bool,
    shift: bool,
) -> (Option<DlOp>, Option<DlSel>) {
    let mut op: Option<DlOp> = None;
    let mut sel: Option<DlSel> = None;
    let w = ui.available_width().max(320.0);
    let h = 72.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w, h), egui::Sense::click());
    let painter = ui.painter().clone();

    // 背景
    let bg = if is_sel {
        mix(th.card, th.accent, if th.dark { 0.22 } else { 0.12 })
    } else if resp.hovered() {
        mix(th.card, th.text, if th.dark { 0.05 } else { 0.03 })
    } else {
        th.card
    };
    painter.rect_filled(rect, th.cr(12), bg);
    painter.rect_stroke(rect, th.cr(12), Stroke::new(1.0, th.border), egui::StrokeKind::Inside);

    // 选中时左侧 accent 条
    if is_sel {
        painter.rect_filled(
            Rect::from_min_max(
                Pos2::new(rect.min.x + 3.0, rect.min.y + 12.0),
                Pos2::new(rect.min.x + 5.0, rect.max.y - 12.0),
            ),
            th.cr(2),
            th.accent,
        );
    }

    let inner = rect.shrink2(vec2(12.0, 10.0));
    // 左侧: 文件名 + 状态(超长截断, 避免溢出到进度/按钮区域)
    let left_w = (inner.width() - 120.0).max(120.0);
    let left_rect = Rect::from_min_max(inner.min, Pos2::new(inner.min.x + left_w, inner.max.y));
    let name_g = truncate_text(&painter, &job.name, left_rect.width(), FontId::proportional(13.5), th.text);
    painter.galley(Pos2::new(left_rect.min.x, left_rect.min.y + 2.0), name_g, th.text);
    let (col, txt) = status_line(job);
    let status_g = truncate_text(&painter, &txt, left_rect.width(), FontId::proportional(11.5), col);
    painter.galley(Pos2::new(left_rect.min.x, left_rect.min.y + 20.0), status_g, col);

    // 中间: 进度/速度
    let mid_x = left_rect.max.x + 16.0;
    let mid_w = inner.width() - left_w - 120.0;
    if mid_w > 50.0 {
        match &job.status {
            DlStatus::Running if job.total > 0 => {
                let frac = (job.done as f32 / job.total as f32).clamp(0.0, 1.0);
                let bar_rect = Rect::from_min_max(
                    Pos2::new(mid_x, inner.center().y - 8.0),
                    Pos2::new(mid_x + mid_w, inner.center().y + 8.0),
                );
                // 背景条
                painter.rect_filled(bar_rect, th.cr(4), mix(th.text_faint, th.bg, 0.7));
                // 进度条
                let fill_w = mid_w * frac;
                if fill_w > 0.0 {
                    let fill_rect = Rect::from_min_max(bar_rect.min, Pos2::new(bar_rect.min.x + fill_w, bar_rect.max.y));
                    painter.rect_filled(fill_rect, th.cr(4), th.accent);
                }
                // 文字
                painter.text(
                    Pos2::new(mid_x, inner.center().y + 12.0),
                    egui::Align2::LEFT_TOP,
                    format!("{} / {}", format::fmt_bytes(job.done as i64), format::fmt_bytes(job.total as i64)),
                    FontId::proportional(10.5),
                    th.text_weak,
                );
            }
            DlStatus::Running => {
                if job.done > 0 {
                    painter.text(
                        Pos2::new(mid_x, inner.center().y - 6.0),
                        egui::Align2::LEFT_TOP,
                        format!("已接收 {}", format::fmt_bytes(job.done as i64)),
                        FontId::proportional(11.5),
                        th.text_weak,
                    );
                }
            }
            _ => {
                if job.done > 0 {
                    painter.text(
                        Pos2::new(mid_x, inner.center().y - 6.0),
                        egui::Align2::LEFT_TOP,
                        format!("已下载 {}", format::fmt_bytes(job.done as i64)),
                        FontId::proportional(11.5),
                        th.text_weak,
                    );
                }
            }
        }
        if job.speed > 0 && job.status == DlStatus::Running {
            painter.text(
                Pos2::new(mid_x, inner.center().y + 10.0),
                egui::Align2::LEFT_TOP,
                format!("{} /s", format::fmt_bytes(job.speed as i64)),
                FontId::proportional(10.5),
                th.text_faint,
            );
        }
    }

    // 右侧: 按钮区域
    let right_x = inner.max.x - 100.0;
    let btn_y = inner.max.y - 24.0;
    let btn_rect = Rect::from_min_max(
        Pos2::new(right_x, btn_y),
        Pos2::new(right_x + 100.0, btn_y + 22.0),
    );

    match &job.status {
        DlStatus::Queued | DlStatus::Running => {
            // 取消按钮
            let cancel_rect = Rect::from_min_max(
                Pos2::new(btn_rect.center().x - 20.0, btn_rect.min.y),
                Pos2::new(btn_rect.center().x + 20.0, btn_rect.max.y),
            );
            let cancel_resp = ui.interact(cancel_rect, ui.id().with(("cancel", rid)), egui::Sense::click());
            painter.rect_filled(cancel_rect, th.cr(6), if cancel_resp.hovered() { th.hover } else { egui::Color32::TRANSPARENT });
            painter.text(
                cancel_rect.center(),
                egui::Align2::CENTER_CENTER,
                "取消",
                FontId::proportional(12.0),
                th.text_weak,
            );
            if cancel_resp.clicked() {
                op = Some(DlOp::Cancel);
            }
        }
        _ => {
            // 移除按钮
            let remove_rect = Rect::from_min_max(
                Pos2::new(btn_rect.min.x, btn_rect.min.y),
                Pos2::new(btn_rect.center().x - 4.0, btn_rect.max.y),
            );
            let remove_resp = ui.interact(remove_rect, ui.id().with(("remove", rid)), egui::Sense::click());
            painter.rect_filled(remove_rect, th.cr(6), if remove_resp.hovered() { th.hover } else { egui::Color32::TRANSPARENT });
            painter.text(
                remove_rect.center(),
                egui::Align2::CENTER_CENTER,
                "移除",
                FontId::proportional(12.0),
                th.text_weak,
            );
            if remove_resp.clicked() {
                op = Some(DlOp::Remove);
            }
            // 打开目录按钮
            let open_rect = Rect::from_min_max(
                Pos2::new(btn_rect.center().x + 4.0, btn_rect.min.y),
                Pos2::new(btn_rect.max.x, btn_rect.max.y),
            );
            let open_resp = ui.interact(open_rect, ui.id().with(("open", rid)), egui::Sense::click());
            painter.rect_filled(open_rect, th.cr(6), if open_resp.hovered() { th.hover } else { egui::Color32::TRANSPARENT });
            painter.text(
                open_rect.center(),
                egui::Align2::CENTER_CENTER,
                "打开目录",
                FontId::proportional(12.0),
                th.text_weak,
            );
            if open_resp.clicked() {
                op = Some(DlOp::OpenDir);
            }
        }
    }

    // 点击处理
    if resp.clicked() {
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

impl App {
    /// 传输任务页顶部的「上传 / 下载」分栏按钮。
    fn transfer_tab_button(&mut self, ui: &mut egui::Ui, th: &Theme, tab: TransferTab, label: &str) {
        let selected = self.transfer_tab == tab;
        let text = RichText::new(label)
            .size(13.0)
            .color(if selected { th.on_accent } else { th.text_weak });
        let btn = egui::Button::new(text)
            .fill(if selected { th.accent } else { egui::Color32::TRANSPARENT })
            .stroke(Stroke::new(1.0, if selected { th.accent } else { th.border }))
            .corner_radius(th.cr(8));
        if ui.add(btn).clicked() {
            self.transfer_tab = tab;
        }
    }

    pub(super) fn transfers_page(&mut self, ctx: &egui::Context, th: &Theme) {
        let mut ops: Vec<(u64, DlOp)> = Vec::new();
        let mut sel_reqs: Vec<DlSel> = Vec::new();
        let running = self
            .jobs
            .values()
            .filter(|j| j.status == DlStatus::Queued || j.status == DlStatus::Running)
            .count();

        egui::CentralPanel::default()
            .frame(Frame::new().fill(th.bg).inner_margin(Margin { left: 20, right: 20, top: 16, bottom: 12 }))
            .show(ctx, |ui| {
                // 标题行
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(22.0, 22.0), egui::Sense::hover());
                    icons::paint(ui.painter(), r, Glyph::Transfer, th.accent);
                    ui.label(RichText::new("传输任务").size(19.0).strong().color(th.text));
                });
                ui.add_space(10.0);

                // 上传 / 下载 分栏
                ui.horizontal(|ui| {
                    self.transfer_tab_button(ui, th, TransferTab::Upload, "上传");
                    self.transfer_tab_button(ui, th, TransferTab::Download, "下载");
                });
                ui.add_space(10.0);

                // -------- 上传(占位, 功能待实现) --------
                if self.transfer_tab == TransferTab::Upload {
                    ui.add_space((ui.available_height() * 0.25).max(60.0));
                    ui.vertical_centered(|ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(60.0, 60.0), egui::Sense::hover());
                        icons::paint(ui.painter(), r, Glyph::Transfer, th.text_faint);
                        ui.add_space(12.0);
                        ui.label(RichText::new("暂无上传任务").color(th.text_weak).size(14.0));
                        ui.add_space(4.0);
                        ui.label(RichText::new("上传功能正在开发中").color(th.text_faint).size(12.0));
                    });
                    return;
                }

                // -------- 下载 --------
                let done = self.jobs.len().saturating_sub(running);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("文件保存在本地下载目录, 可前往「设置」修改。")
                            .color(th.text_weak)
                            .size(12.5),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if !self.jobs.is_empty() {
                            ui.label(
                                RichText::new(format!(
                                    "进行中 {running} · 已完成 {done} · 共 {} 个任务",
                                    self.jobs.len()
                                ))
                                .color(th.text_weak)
                                .size(12.0),
                            );
                        }
                    });
                });
                ui.add_space(8.0);

                // 工具栏（仅有任务时显示）
                if !self.jobs.is_empty() {
                    ui.horizontal(|ui| {
                        // 选择操作按钮
                        if ui.add(egui::Button::new(RichText::new("全选").color(th.text_weak).size(12.5)).frame(false)).clicked() {
                            self.select_all_dl();
                        }
                        if ui.add(egui::Button::new(RichText::new("取消全选").color(th.text_weak).size(12.5)).frame(false)).clicked() {
                            self.deselect_all_dl();
                        }
                        if ui.add(egui::Button::new(RichText::new("反选").color(th.text_weak).size(12.5)).frame(false)).clicked() {
                            self.invert_selection_dl();
                        }

                        // 选中计数
                        if !self.selected_dl.is_empty() {
                            ui.label(RichText::new(format!("已选 {}/{}", self.selected_dl.len(), self.jobs.len())).color(th.accent).size(12.5));
                        }
                    });
                    ui.add_space(4.0);
                }

                if self.jobs.is_empty() {
                    ui.centered_and_justified(|ui| {
                        ui.add_space(60.0);
                        let (r, _) = ui.allocate_exact_size(vec2(64.0, 64.0), egui::Sense::hover());
                        icons::paint(
                            ui.painter(),
                            r,
                            Glyph::Download,
                            th.text_faint,
                        );
                        ui.add_space(10.0);
                        ui.label(RichText::new("暂无下载任务").color(th.text_weak).size(14.0));
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new("在「我的文件」中选择文件后下载到本地")
                                .color(th.text_faint)
                                .size(12.0),
                        );
                        ui.add_space(60.0);
                    });
                    return;
                }

                // 收集可见任务 ID 用于范围选择
                let dl_ids: Vec<u64> = self.jobs.keys().cloned().collect();
                let ctrl = ui.input(|i| i.modifiers.ctrl);
                let shift = ui.input(|i| i.modifiers.shift);

                let scroll_h = ui.available_height() - if !self.selected_dl.is_empty() { 44.0 } else { 0.0 };
                egui::ScrollArea::vertical()
                    .id_salt("downloads_scroll")
                    .auto_shrink([false, false])
                    .max_height(scroll_h.max(60.0))
                    .show(ui, |ui| {
                        for rid in &dl_ids {
                            let Some(job) = self.jobs.get(rid).cloned() else {
                                continue;
                            };
                            let is_sel = self.selected_dl.contains(rid);
                            let (op, sel) = dl_card(ui, th, *rid, &job, is_sel, ctrl, shift);
                            if let Some(op) = op {
                                ops.push((*rid, op));
                            }
                            if let Some(sel) = sel {
                                sel_reqs.push(sel);
                            }
                            ui.add_space(6.0);
                        }
                    });

                // 底部批量操作条
                if !self.selected_dl.is_empty() {
                    ui.add_space(4.0);
                    egui::Frame::new()
                        .fill(th.card)
                        .stroke(Stroke::new(1.0, th.border))
                        .corner_radius(th.cr(8))
                        .inner_margin(Margin::symmetric(12, 8))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(format!("已选择 {} 项", self.selected_dl.len())).size(13.0).color(th.text));
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if ui.add(
                                        egui::Button::new(RichText::new("移除选中").color(th.danger))
                                            .stroke(Stroke::new(1.0, mix(th.danger, th.bg, 0.35)))
                                            .fill(egui::Color32::TRANSPARENT)
                                            .corner_radius(th.cr(8)),
                                    ).clicked() {
                                        for rid in &self.selected_dl {
                                            ops.push((*rid, DlOp::Remove));
                                        }
                                    }
                                    if ui.add(
                                        egui::Button::new(RichText::new("取消选中").color(th.text_weak))
                                            .frame(false),
                                    ).clicked() {
                                        self.selected_dl.clear();
                                    }
                                });
                            });
                        });
                }

                // 处理选择请求（在 CentralPanel 闭包内，确保 dl_ids 和 ctrl 可见）
                for sel in sel_reqs {
                    match sel {
                        DlSel::Replace(rid) => {
                            self.selected_dl.clear();
                            self.selected_dl.insert(rid);
                            self.last_clicked_dl = Some(rid);
                        }
                        DlSel::Toggle(rid) => {
                            if self.selected_dl.contains(&rid) {
                                self.selected_dl.remove(&rid);
                            } else {
                                self.selected_dl.insert(rid);
                            }
                            self.last_clicked_dl = Some(rid);
                        }
                        DlSel::Range(rid) => {
                            if let Some(anchor) = self.last_clicked_dl {
                                let start_idx = dl_ids.iter().position(|x| *x == anchor).unwrap_or(0);
                                let end_idx = dl_ids.iter().position(|x| *x == rid).unwrap_or(0);
                                let (from, to) = if start_idx <= end_idx {
                                    (start_idx, end_idx)
                                } else {
                                    (end_idx, start_idx)
                                };
                                if !ctrl {
                                    self.selected_dl.clear();
                                }
                                for i in from..=to {
                                    self.selected_dl.insert(dl_ids[i]);
                                }
                            } else {
                                self.selected_dl.clear();
                                self.selected_dl.insert(rid);
                            }
                            self.last_clicked_dl = Some(rid);
                        }
                    }
                }
            });

        // 处理操作
        for (rid, op) in ops {
            match op {
                DlOp::Cancel => self.send(Cmd::CancelDownload { req_id: rid }),
                DlOp::OpenDir => {
                    if let Some(job) = self.jobs.get(&rid) {
                        open_dir(&job.dir);
                    }
                }
                DlOp::Remove => {
                    self.jobs.remove(&rid);
                    self.selected_dl.remove(&rid);
                }
            }
        }
    }
}
