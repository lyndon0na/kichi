use eframe::egui::{self, Align, CornerRadius, Frame, Layout, Margin, RichText, Stroke, vec2};

use crate::format;
use crate::icons::{self, Glyph};
use crate::msg::Cmd;
use crate::theme::Theme;

use super::helpers::open_dir;
use super::types::{DlJob, DlOp, DlStatus};
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

impl App {
    pub(super) fn downloads_page(&mut self, ctx: &egui::Context, th: &Theme) {
        let mut ops: Vec<(u64, DlOp)> = Vec::new();
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
                    icons::paint(ui.painter(), r, Glyph::Download, th.accent);
                    ui.label(RichText::new("本地下载").size(19.0).strong().color(th.text));
                    let done = self.jobs.len().saturating_sub(running);
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
                ui.add_space(4.0);
                ui.label(
                    RichText::new("文件保存在本地下载目录, 可前往「设置」修改。")
                        .color(th.text_weak)
                        .size(12.5),
                );
                ui.add_space(14.0);

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
                        ui.label(RichText::new("暂无本地下载任务").color(th.text_weak).size(14.0));
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new("在「网盘文件」中选择文件, 右键或底部操作条下载到本地")
                                .color(th.text_faint)
                                .size(12.0),
                        );
                        ui.add_space(60.0);
                    });
                    return;
                }

                let scroll_h = ui.available_height().max(60.0);
                egui::ScrollArea::vertical()
                    .id_salt("downloads_scroll")
                    .auto_shrink([false, false])
                    .max_height(scroll_h)
                    .show(ui, |ui| {
                        let ids: Vec<u64> = self.jobs.keys().cloned().collect();
                        for rid in ids {
                            let Some(job) = self.jobs.get(&rid).cloned() else {
                                continue;
                            };
                            let mut op: Option<DlOp> = None;
                            ui.push_id(rid, |ui| {
                                egui::Frame::new()
                                    .fill(th.card)
                                    .stroke(Stroke::new(1.0, th.border))
                                    .corner_radius(CornerRadius::same(12))
                                    .inner_margin(Margin::same(12))
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            ui.vertical(|ui| {
                                                ui.label(
                                                    RichText::new(&job.name)
                                                        .strong()
                                                        .size(13.5)
                                                        .color(th.text),
                                                );
                                                let (col, txt) = status_line(&job);
                                                ui.label(RichText::new(txt).color(col).size(11.5));
                                            });
                                            ui.add_space(10.0);
                                            ui.vertical(|ui| {
                                                match &job.status {
                                                    DlStatus::Running if job.total > 0 => {
                                                        let frac = (job.done as f32 / job.total as f32)
                                                            .clamp(0.0, 1.0);
                                                        ui.add(
                                                            egui::ProgressBar::new(frac)
                                                                .desired_width(280.0)
                                                                .text(format!(
                                                                    "{} / {}",
                                                                    format::fmt_bytes(job.done as i64),
                                                                    format::fmt_bytes(job.total as i64)
                                                                )),
                                                        );
                                                    }
                                                    DlStatus::Running => {
                                                        ui.horizontal(|ui| {
                                                            ui.spinner();
                                                            if job.done > 0 {
                                                                ui.label(
                                                                    RichText::new(format!(
                                                                        "已接收 {}",
                                                                        format::fmt_bytes(job.done as i64)
                                                                    ))
                                                                    .color(th.text_weak),
                                                                );
                                                            } else {
                                                                ui.label(
                                                                    RichText::new("连接中…")
                                                                        .color(th.text_weak),
                                                                );
                                                            }
                                                        });
                                                    }
                                                    DlStatus::Done
                                                    | DlStatus::Cancelled
                                                    | DlStatus::Queued => {
                                                        if job.done > 0 {
                                                            ui.label(
                                                                RichText::new(format!(
                                                                    "已下载 {}",
                                                                    format::fmt_bytes(job.done as i64)
                                                                ))
                                                                .color(th.text_weak),
                                                            );
                                                        }
                                                    }
                                                    DlStatus::Failed(_) => {
                                                        ui.label(
                                                            RichText::new("下载未完成, 可移除后重试")
                                                                .color(th.text_weak),
                                                        );
                                                    }
                                                }
                                                if job.speed > 0 && job.status == DlStatus::Running {
                                                    ui.label(
                                                        RichText::new(format!(
                                                            "{} /s",
                                                            format::fmt_bytes(job.speed as i64)
                                                        ))
                                                        .color(th.text_weak),
                                                    );
                                                }
                                                if !job.dir.as_os_str().is_empty() {
                                                    ui.label(
                                                        RichText::new(format!(
                                                            "→ {}",
                                                            job.dir.display()
                                                        ))
                                                        .color(th.text_faint)
                                                        .size(11.0),
                                                    );
                                                }
                                            });
                                            ui.with_layout(
                                                Layout::right_to_left(Align::Center),
                                                |ui| match &job.status {
                                                    DlStatus::Queued | DlStatus::Running => {
                                                        if ui.button("取消").clicked() {
                                                            op = Some(DlOp::Cancel);
                                                        }
                                                    }
                                                    _ => {
                                                        if ui
                                                            .add(
                                                                egui::Button::new(
                                                                    RichText::new("移除")
                                                                        .color(th.text_weak),
                                                                ),
                                                            )
                                                            .clicked()
                                                        {
                                                            op = Some(DlOp::Remove);
                                                        }
                                                        if ui.button("打开目录").clicked() {
                                                            op = Some(DlOp::OpenDir);
                                                        }
                                                    }
                                                },
                                            );
                                        });
                                    });
                            });
                            if let Some(op) = op {
                                ops.push((rid, op));
                            }
                            ui.add_space(8.0);
                        }
                    });
            });

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
                }
            }
        }
    }
}
