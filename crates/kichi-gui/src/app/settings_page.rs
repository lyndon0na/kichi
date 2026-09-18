use eframe::egui::{self, vec2, Align, Frame, Layout, Margin, RichText, Stroke};

use crate::icons::{self, Glyph};
use crate::msg::Cmd;
use crate::theme::{mix, Theme};

use super::App;

fn settings_card(ui: &mut egui::Ui, th: &Theme, title: &str, rows: &mut dyn FnMut(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(th.card)
        .stroke(Stroke::new(1.0, th.border))
        .corner_radius(th.cr(14))
        .inner_margin(Margin::same(18))
        .show(ui, |ui| {
            ui.label(RichText::new(title).size(14.0).strong().color(th.text));
            ui.add_space(10.0);
            rows(ui);
        });
}

/// 设置卡片内的一行: 左侧标题+说明, 右侧 − 数值 + 步进器。值变化时返回 true。
fn transfer_row(
    ui: &mut egui::Ui,
    th: &Theme,
    label: &str,
    help: &str,
    value: &mut i64,
    range: (usize, usize),
) -> bool {
    let lo = range.0 as i64;
    let hi = range.1 as i64;
    let step = |txt: &str| {
        egui::Button::new(RichText::new(txt).strong().color(th.text))
            .corner_radius(th.cr(6))
            .min_size(vec2(24.0, 24.0))
    };
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).color(th.text_weak));
        ui.label(RichText::new(help).color(th.text_faint).size(11.5));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.add_enabled(*value < hi, step("+")).clicked() {
                *value += 1;
                changed = true;
            }
            // 等宽占位保证 1→2 位数切换时按钮不跳动。
            ui.label(
                RichText::new(format!("{value:^2}"))
                    .monospace()
                    .size(12.5)
                    .color(th.text),
            );
            if ui.add_enabled(*value > lo, step("−")).clicked() {
                *value -= 1;
                changed = true;
            }
        });
    });
    changed
}

impl App {
    pub(super) fn settings_page(&mut self, ctx: &egui::Context, th: &Theme) {
        egui::CentralPanel::default()
            .frame(Frame::new().fill(th.bg).inner_margin(Margin {
                left: 20,
                right: 20,
                top: 16,
                bottom: 12,
            }))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(22.0, 22.0), egui::Sense::hover());
                    icons::paint(ui.painter(), r, Glyph::Gear, th.accent);
                    ui.label(RichText::new("设置").size(19.0).strong().color(th.text));
                });
                ui.add_space(14.0);

                // 下载目录
                let dl = self.download_dir.clone();
                settings_card(ui, th, "下载", &mut |ui| {
                    ui.horizontal(|ui| {
                        ui.add_space(2.0);
                        ui.vertical(|ui| {
                            ui.label(RichText::new("本地下载目录").color(th.text_weak));
                            ui.label(
                                RichText::new(if dl.is_empty() {
                                    "未设置, 将使用系统下载目录".into()
                                } else {
                                    dl.clone()
                                })
                                .color(th.text_faint)
                                .size(12.0),
                            );
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.button("选择目录…").clicked() {
                                self.choose_download_dir();
                            }
                        });
                    });
                });
                ui.add_space(14.0);

                // 传输并发 / 重试
                let mut dl_conc = self.dl_concurrency as i64;
                let mut ul_conc = self.ul_concurrency as i64;
                let mut part_conc = self.part_concurrency as i64;
                let mut attempts = self.max_attempts as i64;
                let mut limits_changed = false;
                settings_card(ui, th, "传输", &mut |ui| {
                    limits_changed |= transfer_row(
                        ui,
                        th,
                        "下载并发",
                        "同时下载的文件数上限",
                        &mut dl_conc,
                        crate::settings::DL_CONCURRENCY_RANGE,
                    );
                    ui.add_space(8.0);
                    limits_changed |= transfer_row(
                        ui,
                        th,
                        "上传并发",
                        "同时上传的文件数上限",
                        &mut ul_conc,
                        crate::settings::UL_CONCURRENCY_RANGE,
                    );
                    ui.add_space(8.0);
                    limits_changed |= transfer_row(
                        ui,
                        th,
                        "上传分片并发",
                        "单个上传任务内同时传输的分片数",
                        &mut part_conc,
                        crate::settings::PART_CONCURRENCY_RANGE,
                    );
                    ui.add_space(8.0);
                    limits_changed |= transfer_row(
                        ui,
                        th,
                        "单任务重试次数",
                        "网络抖动等瞬时错误下的最大尝试次数",
                        &mut attempts,
                        crate::settings::MAX_ATTEMPTS_RANGE,
                    );
                    ui.add_space(10.0);
                    ui.label(
                        RichText::new("并发调整即时生效; 重试与分片并发对新启动的任务生效。")
                            .color(th.text_faint)
                            .size(11.5),
                    );
                });
                if limits_changed {
                    self.dl_concurrency = dl_conc as usize;
                    self.ul_concurrency = ul_conc as usize;
                    self.part_concurrency = part_conc as usize;
                    self.max_attempts = attempts as usize;
                    self.persist_settings();
                    self.send(Cmd::SetTransferLimits {
                        dl_concurrency: self.dl_concurrency,
                        ul_concurrency: self.ul_concurrency,
                        part_concurrency: self.part_concurrency,
                        max_attempts: self.max_attempts,
                    });
                }
                ui.add_space(14.0);

                // 账户
                let name = self.username.clone();
                settings_card(ui, th, "账户", &mut |ui| {
                    ui.horizontal(|ui| {
                        ui.add_space(2.0);
                        ui.vertical(|ui| {
                            ui.label(RichText::new("当前登录").color(th.text_weak));
                            ui.label(RichText::new(&name).color(th.text).size(13.0));
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui
                                .add(
                                    egui::Button::new(RichText::new("退出登录").color(th.danger))
                                        .stroke(Stroke::new(1.0, mix(th.danger, th.bg, 0.3)))
                                        .fill(egui::Color32::TRANSPARENT)
                                        .corner_radius(th.cr(8)),
                                )
                                .clicked()
                            {
                                self.logout_confirm = true;
                            }
                        });
                    });
                });
                ui.add_space(24.0);
                ui.centered_and_justified(|ui| {
                    ui.label(
                        RichText::new(concat!(
                            "Kichi · PikPak Third-Party Client v",
                            env!("CARGO_PKG_VERSION")
                        ))
                        .color(th.text_faint)
                        .size(11.5),
                    );
                });
            });
    }
}
