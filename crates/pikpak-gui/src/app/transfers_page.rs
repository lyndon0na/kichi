use eframe::egui::{self, Align, FontId, Frame, Layout, Margin, Pos2, Rect, RichText, Stroke, vec2};

use crate::format;
use crate::icons::{self, Glyph};
use crate::msg::Cmd;
use crate::settings;
use crate::theme::{mix, Theme};

use super::helpers::{open_dir, open_path, truncate_text};
use super::types::{DlFilter, DlJob, DlOp, DlSel, DlStatus, TransferTab, UlJob, UlOp, UlStatus};
use super::App;

/// 下载卡片固定高度(虚拟滚动要求逐行等高)。
const DL_CARD_H: f32 = 72.0;

/// 下载行状态文案。
fn status_line(job: &DlJob) -> (egui::Color32, String) {
    match &job.status {
        DlStatus::Queued => (egui::Color32::from_gray(150), "排队中".into()),
        DlStatus::Running => (egui::Color32::from_rgb(60, 130, 200), "下载中".into()),
        DlStatus::Done => (egui::Color32::from_rgb(70, 150, 90), "已完成".into()),
        DlStatus::Failed(what) => (egui::Color32::from_rgb(217, 70, 60), what.clone()),
    }
}

/// 上传行状态文案。
fn upload_status_line(job: &UlJob) -> (egui::Color32, String) {
    match &job.status {
        UlStatus::Queued => (egui::Color32::from_gray(150), "排队中".into()),
        UlStatus::Running => (egui::Color32::from_rgb(60, 130, 200), "上传中".into()),
        UlStatus::Done => (egui::Color32::from_rgb(70, 150, 90), "已完成".into()),
        UlStatus::Failed(what) => (egui::Color32::from_rgb(217, 70, 60), what.clone()),
    }
}

/// 复选框三态。
#[derive(Clone, Copy, PartialEq)]
enum CheckState {
    Unchecked,
    Checked,
    Partial,
}

/// 在给定矩形内绘制现代化复选框(不处理点击)。
fn paint_checkbox(painter: &egui::Painter, th: &Theme, rect: Rect, state: CheckState, hovered: bool) {
    let size = rect.width();
    let (fill, border) = match state {
        CheckState::Checked | CheckState::Partial => (th.accent, th.accent),
        CheckState::Unchecked => {
            if hovered {
                (th.card, mix(th.border, th.text_weak, 0.65))
            } else {
                (th.card, mix(th.border, th.text_faint, 0.45))
            }
        }
    };
    painter.rect_filled(rect, th.cr(4), fill);
    painter.rect_stroke(rect, th.cr(4), Stroke::new(1.0, border), egui::StrokeKind::Inside);

    match state {
        CheckState::Checked => {
            let p1 = Pos2::new(rect.min.x + size * 0.26, rect.center().y + size * 0.02);
            let p2 = Pos2::new(rect.min.x + size * 0.43, rect.max.y - size * 0.28);
            let p3 = Pos2::new(rect.max.x - size * 0.24, rect.min.y + size * 0.30);
            painter.add(egui::Shape::line(vec![p1, p2, p3], Stroke::new(1.8, th.on_accent)));
        }
        CheckState::Partial => {
            let y = rect.center().y;
            painter.line_segment(
                [
                    Pos2::new(rect.min.x + size * 0.28, y),
                    Pos2::new(rect.max.x - size * 0.28, y),
                ],
                Stroke::new(1.8, th.on_accent),
            );
        }
        CheckState::Unchecked => {}
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
    let h = DL_CARD_H;
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

    // 复选框
    let cb_rect = Rect::from_center_size(
        Pos2::new(inner.min.x + 8.0, inner.center().y),
        vec2(16.0, 16.0),
    );
    let cb_resp = ui.interact(cb_rect, ui.id().with(("dl_check", rid)), egui::Sense::click());
    if cb_resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    paint_checkbox(
        &painter,
        th,
        cb_rect,
        if is_sel { CheckState::Checked } else { CheckState::Unchecked },
        resp.hovered() || cb_resp.hovered(),
    );
    let cb_clicked = cb_resp.clicked();

    // 布局: [复选框] [名称/状态] [进度] [按钮]
    const CB_W: f32 = 26.0;
    // 预留最宽按钮组(3 个图标)的宽度, 保证各卡片列对齐。
    const BTN_W: f32 = 92.0;
    const GAP: f32 = 16.0;
    let content_x = inner.min.x + CB_W;
    let right_start = inner.max.x - BTN_W;
    let avail = (right_start - content_x - GAP).max(0.0);
    let left_w = (avail * 0.5).max(80.0);
    let left_rect = Rect::from_min_max(
        Pos2::new(content_x, inner.min.y),
        Pos2::new(content_x + left_w, inner.max.y),
    );
    let name_g = truncate_text(&painter, &job.name, left_rect.width(), FontId::proportional(13.5), th.text);
    painter.galley(Pos2::new(left_rect.min.x, left_rect.min.y + 2.0), name_g, th.text);
    let (col, txt) = status_line(job);
    let status_g = truncate_text(&painter, &txt, left_rect.width(), FontId::proportional(11.5), col);
    painter.galley(Pos2::new(left_rect.min.x, left_rect.min.y + 20.0), status_g, col);

    // 中间: 进度/速度
    let mid_x = left_rect.max.x + GAP;
    let mid_w = (right_start - GAP) - mid_x;
    if mid_w > 40.0 {
        match &job.status {
            DlStatus::Running if job.total > 0 => {
                let frac = (job.done as f32 / job.total as f32).clamp(0.0, 1.0);
                let bar_rect = Rect::from_min_max(
                    Pos2::new(mid_x, inner.center().y - 10.0),
                    Pos2::new(mid_x + mid_w, inner.center().y + 2.0),
                );
                painter.rect_filled(bar_rect, th.cr(3), mix(th.text_faint, th.bg, 0.7));
                let fill_w = mid_w * frac;
                if fill_w > 0.0 {
                    let fill_rect = Rect::from_min_max(bar_rect.min, Pos2::new(bar_rect.min.x + fill_w, bar_rect.max.y));
                    painter.rect_filled(fill_rect, th.cr(3), th.accent);
                }
                let mut info = format!(
                    "{} / {}",
                    format::fmt_bytes(job.done as i64),
                    format::fmt_bytes(job.total as i64)
                );
                if job.speed > 0 {
                    info.push_str(&format!("   {}/s", format::fmt_bytes(job.speed as i64)));
                }
                painter.text(
                    Pos2::new(mid_x, inner.center().y + 6.0),
                    egui::Align2::LEFT_TOP,
                    info,
                    FontId::proportional(10.5),
                    th.text_weak,
                );
            }
            DlStatus::Running => {
                let mut info = if job.done > 0 {
                    format!("已接收 {}", format::fmt_bytes(job.done as i64))
                } else {
                    "连接中…".to_string()
                };
                if job.speed > 0 {
                    info.push_str(&format!("   {}/s", format::fmt_bytes(job.speed as i64)));
                }
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
                        format!("已下载 {}", format::fmt_bytes(job.done as i64)),
                        FontId::proportional(11.5),
                        th.text_weak,
                    );
                }
            }
        }
    }

    // 右侧: 图标操作按钮(右对齐, 统一预留宽度保证各卡片列对齐)
    let mut btns: Vec<(Glyph, &str, DlOp)> = Vec::new();
    match &job.status {
        DlStatus::Queued | DlStatus::Running => {
            btns.push((Glyph::Close, "取消下载", DlOp::Cancel))
        }
        DlStatus::Done => {
            btns.push((Glyph::OpenExternal, "打开文件", DlOp::OpenFile));
            btns.push((Glyph::Folder, "打开所在目录", DlOp::OpenDir));
            btns.push((Glyph::Trash, "从列表移除", DlOp::Remove));
        }
        DlStatus::Failed(_) => {
            if !job.file_id.is_empty() {
                btns.push((Glyph::Refresh, "重试下载", DlOp::Retry));
            }
            btns.push((Glyph::Folder, "打开所在目录", DlOp::OpenDir));
            btns.push((Glyph::Trash, "从列表移除", DlOp::Remove));
        }
    }

    let btn_sz = 28.0;
    let btn_gap = 4.0;
    let btn_y = inner.center().y - btn_sz / 2.0;
    let mut bx = inner.max.x;
    for (glyph, tip, dop) in btns.into_iter().rev() {
        let rect = Rect::from_min_max(Pos2::new(bx - btn_sz, btn_y), Pos2::new(bx, btn_y + btn_sz));
        bx -= btn_sz + btn_gap;
        let bresp = ui.interact(rect, ui.id().with(("dl_btn", rid, tip)), egui::Sense::click());
        if bresp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            painter.rect_filled(rect, th.cr(6), th.hover);
        }
        let color = if glyph == Glyph::Trash {
            th.danger
        } else {
            th.text_weak
        };
        icons::paint(&painter, rect.shrink(6.0), glyph, color);
        let bresp = bresp.on_hover_text(tip);
        if bresp.clicked() {
            op = Some(dop);
        }
    }

    // 点击处理(按钮已消费的操作不改变选中)
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

    /// 下载列表的状态筛选按钮。
    fn dl_filter_button(&mut self, ui: &mut egui::Ui, th: &Theme, filter: DlFilter, label: &str) {
        let selected = self.dl_filter == filter;
        let text = RichText::new(label)
            .size(12.0)
            .color(if selected { th.on_accent } else { th.text_weak });
        let btn = egui::Button::new(text)
            .fill(if selected { th.accent } else { egui::Color32::TRANSPARENT })
            .stroke(Stroke::new(1.0, if selected { th.accent } else { th.border }))
            .corner_radius(th.cr(8));
        if ui.add(btn).clicked() && !selected {
            self.dl_filter = filter;
            // 切换筛选时清空选择, 避免被筛掉的项仍处于选中状态
            self.selected_dl.clear();
            self.last_clicked_dl = None;
        }
    }

    /// 传输任务页「上传」分栏。
    fn upload_tab(&mut self, ui: &mut egui::Ui, th: &Theme) {
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
                    self.upload_here();
                }
            });
        });
        ui.add_space(8.0);

        if self.ul_jobs.is_empty() {
            ui.add_space((ui.available_height() * 0.25).max(60.0));
            ui.vertical_centered(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(60.0, 60.0), egui::Sense::hover());
                icons::paint(ui.painter(), r, Glyph::Upload, th.text_faint);
                ui.add_space(12.0);
                ui.label(RichText::new("暂无上传任务").color(th.text_weak).size(14.0));
                ui.add_space(4.0);
                ui.label(
                    RichText::new("在「我的文件」点「上传文件」, 或点右上角按钮")
                        .color(th.text_faint)
                        .size(12.0),
                );
            });
            return;
        }

        let mut ops: Vec<(u64, UlOp)> = Vec::new();
        egui::ScrollArea::vertical()
            .id_salt("uploads_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let ids: Vec<u64> = self.ul_jobs.keys().copied().collect();
                for rid in ids {
                    let Some(job) = self.ul_jobs.get(&rid) else {
                        continue;
                    };
                    let w = ui.available_width().max(320.0);
                    let (rect, _) =
                        ui.allocate_exact_size(vec2(w, DL_CARD_H), egui::Sense::hover());
                    let painter = ui.painter().clone();
                    painter.rect_filled(rect, th.cr(12), th.card);
                    painter.rect_stroke(
                        rect,
                        th.cr(12),
                        Stroke::new(1.0, th.border),
                        egui::StrokeKind::Inside,
                    );

                    let inner = rect.shrink2(vec2(12.0, 10.0));
                    let left_w = (inner.width() * 0.42).max(120.0);
                    let name_g = truncate_text(
                        &painter,
                        &job.name,
                        left_w,
                        FontId::proportional(13.5),
                        th.text,
                    );
                    painter.galley(Pos2::new(inner.min.x, inner.min.y + 2.0), name_g, th.text);
                    let (col, txt) = upload_status_line(job);
                    let st_g = truncate_text(&painter, &txt, left_w, FontId::proportional(11.5), col);
                    painter.galley(Pos2::new(inner.min.x, inner.min.y + 20.0), st_g, col);

                    // 进度 / 速率
                    let mid_x = inner.min.x + left_w + 16.0;
                    let right_start = inner.max.x - 96.0;
                    let mid_w = (right_start - mid_x - 16.0).max(0.0);
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
                                        Rect::from_min_max(
                                            bar.min,
                                            Pos2::new(bar.min.x + mid_w * frac, bar.max.y),
                                        ),
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
                                    info.push_str(&format!(
                                        "   {}/s",
                                        format::fmt_bytes(job.speed as i64)
                                    ));
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
                                let info = if job.done > 0 {
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
                        UlStatus::Queued | UlStatus::Running => {
                            btns.push((Glyph::Close, "取消上传", UlOp::Cancel))
                        }
                        UlStatus::Failed(_) => {
                            btns.push((Glyph::Refresh, "重试上传", UlOp::Retry));
                            btns.push((Glyph::Trash, "从列表移除", UlOp::Remove));
                        }
                        UlStatus::Done => {
                            btns.push((Glyph::Trash, "从列表移除", UlOp::Remove))
                        }
                    }
                    let btn_sz = 28.0;
                    let btn_gap = 4.0;
                    let btn_y = inner.center().y - btn_sz / 2.0;
                    let mut bx = inner.max.x;
                    for (glyph, tip, op) in btns.into_iter().rev() {
                        let r = Rect::from_min_max(
                            Pos2::new(bx - btn_sz, btn_y),
                            Pos2::new(bx, btn_y + btn_sz),
                        );
                        bx -= btn_sz + btn_gap;
                        let bresp = ui.interact(
                            r,
                            ui.id().with(("ul_btn", rid, tip)),
                            egui::Sense::click(),
                        );
                        if bresp.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            painter.rect_filled(r, th.cr(6), th.hover);
                        }
                        let color = if glyph == Glyph::Trash {
                            th.danger
                        } else {
                            th.text_weak
                        };
                        icons::paint(&painter, r.shrink(6.0), glyph, color);
                        if bresp.on_hover_text(tip).clicked() {
                            ops.push((rid, op));
                        }
                    }
                }
            });

        for (rid, op) in ops {
            match op {
                UlOp::Cancel => self.send(Cmd::CancelUpload { req_id: rid }),
                UlOp::Remove => {
                    if let Some(job) = self.ul_jobs.get(&rid) {
                        settings::remove_upload_record(&job.record_id, &job.local_path, &job.name);
                    }
                    self.ul_jobs.remove(&rid);
                }
                UlOp::Retry => {
                    let info = self.ul_jobs.get(&rid).map(|j| {
                        (
                            j.local_path.clone(),
                            j.parent.clone(),
                            j.record_id.clone(),
                            j.name.clone(),
                        )
                    });
                    if let Some((path, parent, rec_id, name)) = info {
                        settings::remove_upload_record(&rec_id, &path, &name);
                        self.ul_jobs.remove(&rid);
                        self.enqueue_upload(vec![path], parent);
                    }
                }
            }
        }
    }

    pub(super) fn transfers_page(&mut self, ctx: &egui::Context, th: &Theme) {
        let mut ops: Vec<(u64, DlOp)> = Vec::new();
        let mut sel_reqs: Vec<DlSel> = Vec::new();

        // 底部批量操作条: 底部面板保证布局高度正确; 用页面同色铺底避免露出窗口
        // 底色, 顶部加一条分隔线, 整体是单层扁平工具条, 不再有嵌套盒子。
        if self.transfer_tab == TransferTab::Download
            && !self.jobs.is_empty()
            && !self.selected_dl.is_empty()
        {
            egui::TopBottomPanel::bottom("dl_action_bar")
                .frame(
                    Frame::new()
                        .fill(th.bg)
                        .inner_margin(Margin {
                            left: 20,
                            right: 20,
                            top: 0,
                            bottom: 0,
                        }),
                )
                .show(ctx, |ui| {
                    let top = ui.max_rect().min.y;
                    ui.painter().hline(
                        ui.max_rect().x_range(),
                        top,
                        Stroke::new(1.0, th.border),
                    );
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("已选择 {} 项", self.selected_dl.len()))
                                .size(13.0)
                                .color(th.text),
                        );
                        // 选中项里可重试(已知云端 id 的取消/失败任务)的数量
                        let retryable: Vec<u64> = self
                            .selected_dl
                            .iter()
                            .copied()
                            .filter(|rid| {
                                self.jobs
                                    .get(rid)
                                    .map(|j| {
                                        !j.file_id.is_empty()
                                            && matches!(j.status, DlStatus::Failed(_))
                                    })
                                    .unwrap_or(false)
                            })
                            .collect();
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
                                for rid in &self.selected_dl {
                                    let running = self
                                        .jobs
                                        .get(rid)
                                        .map(|j| {
                                            matches!(j.status, DlStatus::Queued | DlStatus::Running)
                                        })
                                        .unwrap_or(false);
                                    ops.push((*rid, if running { DlOp::Cancel } else { DlOp::Remove }));
                                }
                            }
                            if !retryable.is_empty()
                                && ui
                                    .add(
                                        egui::Button::new(RichText::new("重试选中").color(th.on_accent))
                                            .fill(th.accent)
                                            .stroke(Stroke::NONE)
                                            .corner_radius(th.cr(8)),
                                    )
                                    .clicked()
                            {
                                for rid in &retryable {
                                    ops.push((*rid, DlOp::Retry));
                                }
                            }
                            if ui
                                .add(
                                    egui::Button::new(RichText::new("取消选中").color(th.text_weak))
                                        .frame(false),
                                )
                                .clicked()
                            {
                                self.selected_dl.clear();
                            }
                        });
                    });
                    ui.add_space(8.0);
                });
        }

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

                // -------- 上传 --------
                if self.transfer_tab == TransferTab::Upload {
                    self.upload_tab(ui, th);
                    return;
                }

                // -------- 下载 --------
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("文件保存在本地下载目录, 可前往「设置」修改。")
                            .color(th.text_weak)
                            .size(12.5),
                    );
                });
                ui.add_space(8.0);

                // 筛选栏: 状态分段 + 主复选框(全选/全不选) + 计数 (仅有任务时显示)
                let mut dl_ids: Vec<u64> = Vec::new();
                if !self.jobs.is_empty() {
                    ui.horizontal(|ui| {
                        let total = self.jobs.len();
                        let active = self
                            .jobs
                            .values()
                            .filter(|j| matches!(j.status, DlStatus::Queued | DlStatus::Running))
                            .count();
                        let done_c = self
                            .jobs
                            .values()
                            .filter(|j| j.status == DlStatus::Done)
                            .count();
                        let failed = self
                            .jobs
                            .values()
                            .filter(|j| matches!(j.status, DlStatus::Failed(_)))
                            .count();

                        self.dl_filter_button(ui, th, DlFilter::All, &format!("全部 {total}"));
                        self.dl_filter_button(ui, th, DlFilter::Active, &format!("进行中 {active}"));
                        self.dl_filter_button(ui, th, DlFilter::Done, &format!("已完成 {done_c}"));
                        self.dl_filter_button(ui, th, DlFilter::Failed, &format!("失败 {failed}"));

                        // 当前筛选下可见项
                        dl_ids = self.visible_dl_ids();
                        // 剔除已不在当前筛选中的选中项(如进行中任务完成后被筛掉)
                        self.selected_dl.retain(|id| dl_ids.contains(id));
                        let vis_total = dl_ids.len();
                        let vis_selected =
                            dl_ids.iter().filter(|id| self.selected_dl.contains(id)).count();
                        let master = if vis_total == 0 || vis_selected == 0 {
                            CheckState::Unchecked
                        } else if vis_selected >= vis_total {
                            CheckState::Checked
                        } else {
                            CheckState::Partial
                        };

                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
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
                            let (cb_rect, cb_resp) =
                                ui.allocate_exact_size(vec2(16.0, 16.0), egui::Sense::click());
                            paint_checkbox(ui.painter(), th, cb_rect, master, cb_resp.hovered());
                            if cb_resp.clicked() {
                                if master == CheckState::Checked {
                                    for id in &dl_ids {
                                        self.selected_dl.remove(id);
                                    }
                                } else {
                                    for id in &dl_ids {
                                        self.selected_dl.insert(*id);
                                    }
                                }
                            }
                            ui.add_space(6.0);
                            ui.label(RichText::new("全选").color(th.text_weak).size(12.5));
                        });
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

                // 当前筛选下没有任务
                if dl_ids.is_empty() {
                    ui.add_space(40.0);
                    ui.vertical_centered(|ui| {
                        ui.label(
                            RichText::new("该筛选下暂无任务").color(th.text_weak).size(13.0),
                        );
                    });
                    return;
                }

                let ctrl = ui.input(|i| i.modifiers.ctrl);
                let shift = ui.input(|i| i.modifiers.shift);

                let scroll_h = ui.available_height();
                egui::ScrollArea::vertical()
                    .id_salt("downloads_scroll")
                    .auto_shrink([false, false])
                    .max_height(scroll_h.max(60.0))
                    .show_rows(ui, DL_CARD_H, dl_ids.len(), |ui, range| {
                        for i in range {
                            let rid = dl_ids[i];
                            let Some(job) = self.jobs.get(&rid) else {
                                continue;
                            };
                            let is_sel = self.selected_dl.contains(&rid);
                            let (op, sel) = dl_card(ui, th, rid, job, is_sel, ctrl, shift);
                            if let Some(op) = op {
                                ops.push((rid, op));
                            }
                            if let Some(sel) = sel {
                                sel_reqs.push(sel);
                            }
                        }
                    });

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
                                for id in &dl_ids[from..=to] {
                                    self.selected_dl.insert(*id);
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
                DlOp::OpenFile => {
                    if let Some(job) = self.jobs.get(&rid) {
                        let path = job.dir.join(&job.name);
                        if let Err(e) = open_path(&path) {
                            let msg = format!("打开文件失败: {e}");
                            self.toast_err(&msg);
                        }
                    }
                }
                DlOp::Retry => {
                    // 取出旧条目信息后移除旧行, 再重新入队, 避免同一文件被重复重试。
                    let info = self.jobs.get(&rid).and_then(|j| {
                        if j.file_id.is_empty() {
                            None
                        } else {
                            Some((
                                j.file_id.clone(),
                                j.name.clone(),
                                j.dir.clone(),
                                j.record_id.clone(),
                            ))
                        }
                    });
                    if let Some((file_id, name, dir, rec_id)) = info {
                        self.enqueue_downloads(vec![(file_id.clone(), name.clone())], dir.clone());
                        self.jobs.remove(&rid);
                        self.selected_dl.remove(&rid);
                        if self.last_clicked_dl == Some(rid) {
                            self.last_clicked_dl = None;
                        }
                        settings::remove_download_record(&rec_id, &file_id, &name, &dir);
                    }
                }
                DlOp::Remove => {
                    if let Some(job) = self.jobs.get(&rid) {
                        // 仅从列表/历史记录中移除, 不删除本地已下载的文件
                        settings::remove_download_record(
                            &job.record_id,
                            &job.file_id,
                            &job.name,
                            &job.dir,
                        );
                    }
                    self.jobs.remove(&rid);
                    self.selected_dl.remove(&rid);
                    if self.last_clicked_dl == Some(rid) {
                        self.last_clicked_dl = None;
                    }
                }
            }
        }
    }
}
