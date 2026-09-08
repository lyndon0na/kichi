use eframe::egui::{self, Align, CornerRadius, FontId, Frame, Layout, Margin, Pos2, Rect, RichText, Stroke, vec2};

use pikpak_core::types::Quota;

use crate::format;
use crate::icons::{self, Glyph};
use crate::theme::{mix, Theme};

use super::helpers::{draw_sun_moon, truncate_text};
use super::types::{DlStatus, Page};
use super::App;

impl App {
    pub(super) fn app_shell(&mut self, ctx: &egui::Context, th: &Theme) {
        egui::SidePanel::left("app_sidebar")
            .resizable(true)
            .default_width(228.0)
            .width_range(196.0..=320.0)
            .frame(
                Frame::new()
                    .fill(th.panel)
                    .stroke(Stroke::new(1.0, th.border))
                    .inner_margin(Margin::symmetric(10, 12)),
            )
            .show(ctx, |ui| {
                self.sidebar(ui, th);
            });

        match self.page {
            Page::Files => self.files_page(ctx, th),
            Page::Tasks => self.tasks_page(ctx, th),
            Page::Downloads => self.downloads_page(ctx, th),
            Page::Settings => self.settings_page(ctx, th),
        }
    }

    /// 单个导航项; 返回是否被点击。可选显示角标。
    pub(super) fn nav_item(
        &mut self,
        ui: &mut egui::Ui,
        th: &Theme,
        glyph: Glyph,
        label: &str,
        selected: bool,
        badge: Option<usize>,
    ) -> bool {
        let height = 36.0;
        let (rect, resp) =
            ui.allocate_exact_size(vec2(ui.available_width(), height), egui::Sense::click());

        let bg = if selected {
            mix(th.panel, th.accent, if th.dark { 0.24 } else { 0.13 })
        } else if resp.hovered() {
            mix(th.panel, th.text, if th.dark { 0.08 } else { 0.06 })
        } else {
            egui::Color32::TRANSPARENT
        };
        let painter = ui.painter().clone();
        painter.rect_filled(rect, CornerRadius::same(9), bg);
        if selected {
            painter.rect_filled(
                Rect::from_min_max(
                    Pos2::new(rect.min.x + 4.0, rect.min.y + (height - 18.0) / 2.0),
                    Pos2::new(rect.min.x + 6.0, rect.min.y + (height + 18.0) / 2.0),
                ),
                CornerRadius::same(1),
                th.accent,
            );
        }
        let icon_rect =
            Rect::from_center_size(Pos2::new(rect.min.x + 20.0, rect.center().y), vec2(18.0, 18.0));
        let icon_color = if selected { th.accent } else { th.text_weak };
        icons::paint(&painter, icon_rect, glyph, icon_color);
        let txt_color = if selected { th.text } else { th.text_weak };
        let txt = painter.layout_no_wrap(
            label.to_string(),
            FontId::proportional(14.0),
            txt_color,
        );
        painter.galley(
            Pos2::new(rect.min.x + 42.0, rect.center().y - txt.size().y / 2.0),
            txt,
            txt_color,
        );
        if let Some(n) = badge {
            let s = format!("{n}");
            let g = painter.layout_no_wrap(s, FontId::proportional(11.0), egui::Color32::WHITE);
            let w = (g.size().x + 14.0).max(17.0);
            let r = Rect::from_min_size(
                Pos2::new(rect.right() - w - 8.0, rect.center().y - 9.0),
                vec2(w, 18.0),
            );
            painter.rect_filled(r, CornerRadius::same(9), mix(th.panel, th.accent, 0.9));
            painter.galley(Pos2::new(r.center().x - g.size().x / 2.0, r.min.y + 3.0), g, egui::Color32::WHITE);
        }
        resp.clicked()
    }

    pub(super) fn sidebar(&mut self, ui: &mut egui::Ui, th: &Theme) {
        // ---------- 顶部品牌 ----------
        let row_h = 40.0;
        let (brect, _) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), egui::Sense::hover());
        let bp = ui.painter().clone();
        let logo = Rect::from_center_size(Pos2::new(brect.min.x + 18.0, brect.center().y), vec2(30.0, 30.0));
        bp.rect_filled(logo, CornerRadius::same(9), th.accent);
        let ltxt = bp.layout_no_wrap(
            "P".into(),
            FontId::proportional(17.0),
            th.on_accent,
        );
        bp.galley(
            Pos2::new(logo.center().x - ltxt.size().x / 2.0, logo.center().y - ltxt.size().y / 2.0),
            ltxt,
            th.on_accent,
        );
        let brand = bp.layout_no_wrap("PikPak".into(), FontId::proportional(17.0), th.text);
        bp.galley(
            Pos2::new(brect.min.x + 42.0, brect.min.y + 3.0),
            brand,
            th.text,
        );
        let sub = bp.layout_no_wrap(
            "Linux 客户端".into(),
            FontId::proportional(11.0),
            th.text_faint,
        );
        bp.galley(Pos2::new(brect.min.x + 42.0, brect.min.y + 24.0), sub, th.text_faint);

        // 主题切换
        let theme_btn = Rect::from_min_size(
            Pos2::new(brect.right() - 34.0, brect.center().y - 17.0),
            vec2(28.0, 28.0),
        );
        let tresp = ui.interact(theme_btn, ui.id().with("theme_toggle"), egui::Sense::click());
        let tb = if tresp.hovered() {
            mix(th.panel, th.text, if th.dark { 0.12 } else { 0.1 })
        } else {
            egui::Color32::TRANSPARENT
        };
        bp.rect_filled(theme_btn, CornerRadius::same(7), tb);
        let sun = !self.dark;
        draw_sun_moon(&bp, theme_btn, if sun { egui::Color32::from_rgb(226, 162, 54) } else { egui::Color32::from_rgb(140, 150, 190) });
        if tresp.clicked() {
            self.toggle_theme();
        }

        ui.add_space(18.0);

        // ---------- 主导航 ----------
        let running = [
            "PHASE_TYPE_PENDING",
            "PHASE_TYPE_RUNNING",
            "PHASE_TYPE_ERROR",
        ]
        .iter()
        .map(|p| self.buckets.get(*p).map(|v| v.len()).unwrap_or(0))
        .sum::<usize>();

        let mut nav: Option<Page> = None;
        if self.nav_item(
            ui,
            th,
            Glyph::Folder,
            "网盘文件",
            self.page == Page::Files,
            None,
        ) {
            nav = Some(Page::Files);
        }
        if self.nav_item(
            ui,
            th,
            Glyph::Transfer,
            "离线下载",
            self.page == Page::Tasks,
            (running > 0).then_some(running),
        ) {
            nav = Some(Page::Tasks);
        }
        if self.nav_item(
            ui,
            th,
            Glyph::Gear,
            "设置",
            self.page == Page::Settings,
            None,
        ) {
            nav = Some(Page::Settings);
        }
        if let Some(p) = nav {
            self.page = p;
        }

        // 本地下载(常驻入口, 有进行中任务时显示角标)
        {
            ui.add_space(6.0);
            let active = self
                .jobs
                .values()
                .filter(|j| matches!(j.status, DlStatus::Queued | DlStatus::Running))
                .count();
            let done = self.jobs.len().saturating_sub(active);
            let clicked = self.nav_item(
                ui,
                th,
                Glyph::Download,
                "本地下载",
                self.page == Page::Downloads,
                (active > 0).then_some(active),
            );
            if clicked {
                self.page = Page::Downloads;
            }
            if !self.jobs.is_empty() && active == 0 && done > 0 {
                ui.add_space(2.0);
                ui.label(
                    RichText::new("全部任务已完成, 点击可查看")
                        .color(th.text_faint)
                        .size(11.0),
                );
            }
        }

        // ---------- 底部: 存储配额 + 账户 ----------
        let quota = self.quota.clone();
        let username = self.username.clone();
        ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
            ui.add_space(8.0);
            self.account_card(ui, th, &username);
            ui.add_space(8.0);
            self.quota_card(ui, th, quota.as_ref());
        });
    }

    /// 底部账户行(头像 + 用户名 + 退出登录)。
    pub(super) fn account_card(&mut self, ui: &mut egui::Ui, th: &Theme, username: &str) -> egui::Response {
        let h = 40.0;
        let (rect, resp) =
            ui.allocate_exact_size(vec2(ui.available_width(), h), egui::Sense::click());
        let painter = ui.painter().clone();
        painter.rect_filled(rect, CornerRadius::same(10), egui::Color32::TRANSPARENT);
        let initial = username
            .chars()
            .next()
            .map(|c| c.to_uppercase().collect::<String>())
            .unwrap_or_else(|| "P".into());
        let av = Rect::from_center_size(Pos2::new(rect.min.x + 19.0, rect.center().y), vec2(30.0, 30.0));
        painter.rect_filled(av, CornerRadius::same(15), mix(th.panel, th.accent, 0.28));
        let g = painter.layout_no_wrap(initial, FontId::proportional(14.0), th.accent);
        painter.galley(Pos2::new(av.center().x - g.size().x / 2.0, av.center().y - g.size().y / 2.0), g, th.accent);
        let name_rect = Rect::from_min_max(Pos2::new(rect.min.x + 42.0, rect.min.y + 4.0), Pos2::new(rect.right() - 44.0, rect.max.y - 4.0));
        painter.rect_filled(Rect::from_min_max(Pos2::new(rect.min.x + 42.0, rect.min.y + 5.0), Pos2::new(rect.right() - 40.0, rect.max.y - 5.0)), CornerRadius::same(8), egui::Color32::TRANSPARENT);
        let uname = truncate_text(&painter, username, name_rect.width(), FontId::proportional(13.5), th.text);
        painter.galley(Pos2::new(name_rect.min.x, rect.center().y - uname.size().y / 2.0), uname, th.text);
        let out_rect = Rect::from_center_size(Pos2::new(rect.right() - 17.0, rect.center().y), vec2(24.0, 24.0));
        let ores = ui.interact(out_rect, ui.id().with("logout_btn"), egui::Sense::click());
        if ores.hovered() {
            painter.rect_filled(out_rect, CornerRadius::same(12), mix(th.panel, th.danger, 0.18));
        }
        icons::paint(&painter, out_rect.shrink(3.0), Glyph::Logout, if ores.hovered() { th.danger } else { th.text_weak });
        let clicked = ores.clicked();
        ores.on_hover_text("退出登录");
        if clicked {
            self.logout_confirm = true;
        }
        resp
    }

    pub(super) fn quota_card(&mut self, ui: &mut egui::Ui, th: &Theme, quota: Option<&Quota>) -> egui::Response {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 74.0), egui::Sense::hover());
        let card = rect;
        let painter = ui.painter().clone();
        let (r, q) = match quota {
            Some(q) if q.limit > 0 => (q.usage as f32 / q.limit as f32, Some(q)),
            _ => (0.0, None),
        };
        let frac = r.clamp(0.0, 1.0);
        let bg = mix(th.bg, th.text, if th.dark { 0.05 } else { 0.03 });
        painter.rect_filled(card, CornerRadius::same(12), bg);
        painter.rect_stroke(card, CornerRadius::same(12), Stroke::new(1.0, th.border), egui::StrokeKind::Inside);
        let inner = card.shrink(14.0);
        let title = painter.layout_no_wrap("存储空间".into(), FontId::proportional(12.0), th.text_weak);
        painter.galley(Pos2::new(inner.min.x, inner.min.y + 2.0), title, th.text_weak);
        if quota.is_some() {
            let pct = format!("{}%", (frac * 100.0).round() as u32);
            let gp = painter.layout_no_wrap(pct, FontId::proportional(11.5), th.text_faint);
            painter.galley(Pos2::new(inner.right() - gp.size().x, inner.min.y + 2.0), gp, th.text_faint);
        }
        let bar = Rect::from_min_max(
            Pos2::new(inner.min.x, inner.min.y + 22.0),
            Pos2::new(inner.right(), inner.min.y + 26.0),
        );
        painter.rect_filled(bar, CornerRadius::same(2), mix(th.bg, th.text, if th.dark { 0.12 } else { 0.1 }));
        if frac > 0.0 {
            let fw = (bar.width() * frac).max(3.0);
            painter.rect_filled(
                Rect::from_min_size(bar.min, vec2(fw, bar.height())),
                CornerRadius::same(2),
                th.accent,
            );
        }
        let usage_txt = match q {
            Some(q) => format!("已用 {} / 共 {}", format::fmt_bytes(q.usage), format::fmt_bytes(q.limit)),
            None => "正在获取…".into(),
        };
        let g = painter.layout_no_wrap(usage_txt, FontId::proportional(11.0), th.text_weak);
        painter.galley(Pos2::new(inner.min.x, inner.min.y + 34.0), g, th.text_weak);
        ui.add_space(0.0);
        ui.interact(card, ui.id().with("quota_card"), egui::Sense::hover())
    }
}
