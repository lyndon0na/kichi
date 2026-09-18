use eframe::egui::{self, Align, Frame, Layout, Margin, RichText, Stroke, vec2};

use crate::icons::{self, Glyph};
use crate::theme::{mix, Theme};

use super::App;

fn settings_card(
    ui: &mut egui::Ui,
    th: &Theme,
    title: &str,
    rows: &mut dyn FnMut(&mut egui::Ui),
) {
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

impl App {
    pub(super) fn settings_page(&mut self, ctx: &egui::Context, th: &Theme) {
        egui::CentralPanel::default()
            .frame(Frame::new().fill(th.bg).inner_margin(Margin { left: 20, right: 20, top: 16, bottom: 12 }))
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
                                RichText::new(if dl.is_empty() { "未设置, 将使用系统下载目录".into() } else { dl.clone() })
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
                    ui.label(RichText::new("Kichi · PikPak Third-Party Client v0.1.0").color(th.text_faint).size(11.5));
                });
            });
    }
}
