//! 我的分享 / 回收站等占位页面: 目前仅提供 UI, 功能待接入。

use eframe::egui::{self, Frame, Margin, RichText, vec2};

use crate::icons::{self, Glyph};
use crate::theme::Theme;

use super::App;

impl App {
    /// 通用占位页: 顶部标题 + 居中空状态。
    pub(super) fn placeholder_page(
        &mut self,
        ctx: &egui::Context,
        th: &Theme,
        glyph: Glyph,
        title: &str,
        empty: &str,
    ) {
        egui::CentralPanel::default()
            .frame(Frame::new().fill(th.card).inner_margin(Margin {
                left: 20,
                right: 20,
                top: 16,
                bottom: 12,
            }))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(22.0, 22.0), egui::Sense::hover());
                    icons::paint(ui.painter(), r, glyph, th.accent);
                    ui.label(RichText::new(title).size(19.0).strong().color(th.text));
                });

                ui.add_space((ui.available_height() * 0.28).max(60.0));
                ui.vertical_centered(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(60.0, 60.0), egui::Sense::hover());
                    icons::paint(ui.painter(), r, glyph, th.text_faint);
                    ui.add_space(12.0);
                    ui.label(RichText::new(empty).color(th.text_weak).size(13.5));
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new("该功能正在开发中")
                            .color(th.text_faint)
                            .size(12.0),
                    );
                });
            });
    }
}
