//! 「我的分享」列表渲染与创建分享弹窗。

use std::collections::HashSet;

use eframe::egui::{self, vec2, Align, Align2, Color32, Frame, Layout, Margin, RichText, Stroke};

use crate::format;
use crate::icons::{self, Glyph};
use crate::msg::Cmd;
use crate::theme::{mix, Theme};

use super::super::global::Global;
use super::super::helpers;
use super::SharesPage;

impl SharesPage {
    pub(crate) fn draw(&mut self, ctx: &egui::Context, g: &mut Global, th: &Theme) {
        let mut refresh = false;
        let mut load_more = false;
        let mut copy: Option<(String, &'static str)> = None;
        let mut open: Option<String> = None;
        let mut delete_single: Option<(String, String)> = None;
        let mut delete_selected = false;
        let mut toggle: Option<(String, bool)> = None;
        let mut select_all: Option<bool> = None;

        // 复制列表快照, 避免在渲染闭包里与 self 的可变借用冲突。
        let shares = self.list.clone();
        let selected: HashSet<String> = self.selected.clone();
        let has_next = self.next.is_some();
        let loading = self.loading;
        let all_selected = !shares.is_empty() && selected.len() >= shares.len();
        let selected_count = selected.len();

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
                    icons::paint(ui.painter(), r, Glyph::Share, th.accent);
                    ui.label(RichText::new("我的分享").size(19.0).strong().color(th.text));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        // 转存分享按钮
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("转存分享").color(th.accent),
                                )
                                .frame(true),
                            )
                            .clicked()
                        {
                            self.save_open = true;
                            self.clear_save();
                        }
                        let refreshing = loading && !shares.is_empty();
                        let (rr, ico) =
                            ui.allocate_exact_size(vec2(22.0, 22.0), egui::Sense::click());
                        icons::paint(ui.painter(), rr, Glyph::Refresh, th.text_weak);
                        let ico_clicked = !refreshing && ico.clicked();
                        let btn = ui.add_enabled(
                            !refreshing,
                            egui::Button::new(
                                RichText::new(if refreshing { "正在刷新…" } else { "刷新" })
                                    .color(th.text_weak),
                            )
                            .frame(false),
                        );
                        if btn.clicked() || ico_clicked {
                            refresh = true;
                        }
                        if refreshing {
                            ui.add(egui::Spinner::new().size(14.0).color(th.text_weak));
                        }
                        if !shares.is_empty() {
                            let mut sel = all_selected;
                            if ui.checkbox(&mut sel, "全选").changed() {
                                select_all = Some(sel);
                            }
                            ui.label(
                                RichText::new(format!("共 {} 个", shares.len()))
                                    .color(th.text_faint)
                                    .size(12.0),
                            );
                        }
                    });
                });
                ui.add_space(4.0);
                ui.label(
                    RichText::new(
                        "在「我的文件」中选中文件后点击「分享」即可创建链接; 在此管理已创建的分享。",
                    )
                    .color(th.text_weak)
                    .size(12.5),
                );
                ui.add_space(12.0);

                if loading && shares.is_empty() {
                    ui.vertical_centered(|ui| {
                        ui.add_space(60.0);
                        ui.spinner();
                        ui.add_space(8.0);
                        ui.label(RichText::new("正在获取分享…").color(th.text_weak));
                    });
                    return;
                }

                if shares.is_empty() {
                    ui.vertical_centered(|ui| {
                        ui.add_space((ui.available_height() * 0.24).max(50.0));
                        let (r, _) = ui.allocate_exact_size(vec2(56.0, 56.0), egui::Sense::hover());
                        icons::paint(ui.painter(), r, Glyph::Share, th.text_faint);
                        ui.add_space(10.0);
                        ui.label(RichText::new("暂无分享内容").color(th.text_weak).size(13.5));
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new("在文件页选中文件后点击「分享」创建")
                                .color(th.text_faint)
                                .size(12.0),
                        );
                    });
                    return;
                }

                // 选中批量操作条
                if selected_count > 0 {
                    Frame::new()
                        .fill(th.accent_soft())
                        .corner_radius(th.cr(10))
                        .inner_margin(Margin::symmetric(12, 7))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(format!("已选 {selected_count} 项"))
                                        .color(th.accent)
                                        .size(12.5),
                                );
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new("取消选中分享")
                                                    .color(egui::Color32::WHITE),
                                            )
                                            .fill(th.danger)
                                            .stroke(Stroke::NONE)
                                            .corner_radius(th.cr(8)),
                                        )
                                        .clicked()
                                    {
                                        delete_selected = true;
                                    }
                                    if ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new("取消选择").color(th.text_weak),
                                            )
                                            .frame(false),
                                        )
                                        .clicked()
                                    {
                                        select_all = Some(false);
                                    }
                                });
                            });
                        });
                    ui.add_space(8.0);
                }

                let scroll_h = (ui.available_height() - 44.0).max(80.0);
                egui::ScrollArea::vertical()
                    .id_salt("shares_scroll")
                    .auto_shrink([false, false])
                    .max_height(scroll_h)
                    .show(ui, |ui| {
                        for s in &shares {
                            Frame::new()
                                .fill(th.card)
                                .stroke(Stroke::new(1.0_f32, th.border))
                                .corner_radius(th.cr(14))
                                .inner_margin(Margin::symmetric(16, 12))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        let mut checked = selected.contains(&s.share_id);
                                        if ui.checkbox(&mut checked, "").changed() {
                                            toggle = Some((s.share_id.clone(), checked));
                                        }
                                        ui.add_space(2.0);
                                        ui.vertical(|ui| {
                                            ui.horizontal(|ui| {
                                                let title = if s.title.is_empty() {
                                                    "未命名分享"
                                                } else {
                                                    s.title.as_str()
                                                };
                                                ui.label(
                                                    RichText::new(title)
                                                        .size(14.0)
                                                        .strong()
                                                        .color(th.text),
                                                );
                                                let (badge, color) = if s.is_unavailable() {
                                                    ("已失效", th.danger)
                                                } else if s.needs_pass_code() {
                                                    ("私密", th.warn)
                                                } else {
                                                    ("公开", th.ok)
                                                };
                                                Frame::new()
                                                    .fill(mix(
                                                        th.card,
                                                        color,
                                                        if th.dark { 0.22 } else { 0.14 },
                                                    ))
                                                    .corner_radius(th.cr(6))
                                                    .inner_margin(Margin::symmetric(6, 1))
                                                    .show(ui, |ui| {
                                                        ui.label(
                                                            RichText::new(badge)
                                                                .color(color)
                                                                .size(11.0),
                                                        );
                                                    });
                                            });
                                            ui.add_space(3.0);
                                            let views = if s.view_count.trim().is_empty() {
                                                "0"
                                            } else {
                                                s.view_count.trim()
                                            };
                                            let restores = if s.restore_count.trim().is_empty() {
                                                "0"
                                            } else {
                                                s.restore_count.trim()
                                            };
                                            let mut meta = format!(
                                                "{} 个文件 · 有效期 {} · 浏览 {} · 转存 {}",
                                                s.file_count(),
                                                s.expiry_label(),
                                                views,
                                                restores,
                                            );
                                            if !s.create_time.is_empty() {
                                                meta.push_str(&format!(
                                                    " · {}",
                                                    format::fmt_time(&s.create_time)
                                                ));
                                            }
                                            ui.label(
                                                RichText::new(meta).color(th.text_weak).size(12.0),
                                            );
                                            if !s.pass_code.is_empty() {
                                                ui.add_space(2.0);
                                                ui.label(
                                                    RichText::new(format!(
                                                        "提取码 {}",
                                                        s.pass_code
                                                    ))
                                                    .color(th.accent)
                                                    .size(12.0),
                                                );
                                            }
                                        });
                                        ui.with_layout(
                                            Layout::right_to_left(Align::Center),
                                            |ui| {
                                                if ui
                                                    .add(
                                                        egui::Button::new(
                                                            RichText::new("取消分享")
                                                                .color(th.danger),
                                                        )
                                                        .fill(egui::Color32::TRANSPARENT)
                                                        .stroke(Stroke::new(
                                                            1.0_f32,
                                                            mix(th.danger, th.bg, 0.35),
                                                        ))
                                                        .corner_radius(th.cr(8)),
                                                    )
                                                    .clicked()
                                                {
                                                    delete_single = Some((
                                                        s.share_id.clone(),
                                                        s.title.clone(),
                                                    ));
                                                }
                                                if ui
                                                    .add(
                                                        egui::Button::new(
                                                            RichText::new("复制链接")
                                                                .color(th.accent),
                                                        )
                                                        .fill(th.accent_soft())
                                                        .stroke(Stroke::NONE)
                                                        .corner_radius(th.cr(8)),
                                                    )
                                                    .clicked()
                                                {
                                                    copy = Some((
                                                        s.share_url.clone(),
                                                        "已复制分享链接",
                                                    ));
                                                }
                                                if !s.pass_code.is_empty()
                                                    && ui
                                                        .add(
                                                            egui::Button::new(
                                                                RichText::new("复制链接和提取码")
                                                                    .color(th.text_weak),
                                                            )
                                                            .corner_radius(th.cr(8)),
                                                        )
                                                        .clicked()
                                                {
                                                    copy = Some((
                                                        format!(
                                                            "{} 提取码: {}",
                                                            s.share_url, s.pass_code
                                                        ),
                                                        "已复制分享链接和提取码",
                                                    ));
                                                }
                                                let (or, ores) = ui.allocate_exact_size(
                                                    vec2(26.0, 26.0),
                                                    egui::Sense::click(),
                                                );
                                                if ores.hovered() {
                                                    ui.painter()
                                                        .rect_filled(or, th.cr(6), th.hover);
                                                }
                                                icons::paint(
                                                    ui.painter(),
                                                    or.shrink(5.0),
                                                    Glyph::OpenExternal,
                                                    if ores.hovered() {
                                                        th.accent
                                                    } else {
                                                        th.text_weak
                                                    },
                                                );
                                                let ores = ores.on_hover_text("在浏览器打开");
                                                if ores.clicked() {
                                                    open = Some(s.share_url.clone());
                                                }
                                            },
                                        );
                                    });
                                    ui.add_space(4.0);
                                    ui.label(
                                        RichText::new(s.share_url.as_str())
                                            .color(th.text_faint)
                                            .size(11.5),
                                    );
                                });
                            ui.add_space(8.0);
                        }

                        if has_next {
                            ui.add_space(4.0);
                            ui.vertical_centered(|ui| {
                                if ui.button("加载更多").clicked() {
                                    load_more = true;
                                }
                            });
                            ui.add_space(2.0);
                        }
                    });
            });

        if let Some((id, on)) = toggle {
            if on {
                self.selected.insert(id);
            } else {
                self.selected.remove(&id);
            }
        }
        if let Some(true) = select_all {
            self.selected = shares.iter().map(|s| s.share_id.clone()).collect();
        } else if let Some(false) = select_all {
            self.selected.clear();
        }
        if refresh {
            self.refresh(g);
        }
        if load_more {
            self.load_more(g);
        }
        if let Some((text, msg)) = copy {
            ctx.copy_text(text);
            g.toast_ok(msg);
        }
        if let Some(url) = open {
            if let Err(e) = helpers::open_url(&url) {
                g.toast_err(&format!("打开链接失败: {e}"));
            }
        }
        if delete_selected {
            let items: Vec<(String, String)> = shares
                .iter()
                .filter(|s| self.selected.contains(&s.share_id))
                .map(|s| (s.share_id.clone(), s.title.clone()))
                .collect();
            if !items.is_empty() {
                self.delete_confirm = Some(items);
            }
        }
        if let Some(item) = delete_single {
            self.delete_confirm = Some(vec![item]);
        }
    }

    /// 「创建分享」设置框 / 分享结果 / 取消分享确认。
    pub(crate) fn draw_dialogs(&mut self, ctx: &egui::Context, g: &mut Global, th: &Theme) {
        if let Some(targets) = self.create_dialog.clone() {
            let n = targets.len();
            let name = if n == 1 {
                targets[0].1.clone()
            } else {
                format!("选中的 {n} 项")
            };
            let mut confirm = false;
            let mut close = false;
            egui::Window::new("创建分享")
                .collapsible(false)
                .resizable(false)
                .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.add_space(4.0);
                    ui.label(RichText::new(format!("将分享: {name}")).color(th.text));
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("有效期").color(th.text_weak));
                        let label = match self.expiration_days {
                            1 => "1 天",
                            7 => "7 天",
                            30 => "30 天",
                            _ => "永久",
                        };
                        egui::ComboBox::from_id_salt("share_expiry")
                            .selected_text(label)
                            .show_ui(ui, |ui| {
                                for (d, l) in
                                    [(-1, "永久"), (1, "1 天"), (7, "7 天"), (30, "30 天")]
                                {
                                    ui.selectable_value(&mut self.expiration_days, d, l);
                                }
                            });
                    });
                    ui.add_space(6.0);
                    ui.checkbox(&mut self.need_password, "需要提取码");
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new("带提取码的分享更安全, 访问者需输入提取码。")
                            .color(th.text_faint)
                            .size(11.5),
                    );
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add(
                                egui::Button::new(RichText::new("创建分享").color(th.on_accent))
                                    .fill(th.accent)
                                    .stroke(Stroke::NONE)
                                    .corner_radius(th.cr(8)),
                            )
                            .clicked()
                        {
                            confirm = true;
                            close = true;
                        }
                        if ui.button("取消").clicked() {
                            close = true;
                        }
                    });
                });
            if confirm {
                let ids: Vec<String> = targets.iter().map(|(id, _)| id.clone()).collect();
                let label = if n == 1 {
                    targets[0].1.clone()
                } else {
                    format!("{n} 项")
                };
                g.send(Cmd::CreateShare {
                    file_ids: ids,
                    expiration_days: self.expiration_days,
                    need_password: self.need_password,
                    label,
                });
            }
            if close {
                self.create_dialog = None;
            }
        }

        if let Some(r) = self.result.clone() {
            let mut close = false;
            let mut copied: Option<&'static str> = None;
            let mut open = false;
            egui::Window::new("分享已创建")
                .collapsible(false)
                .resizable(false)
                .default_width(420.0)
                .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(format!("「{}」的分享链接", r.label))
                            .color(th.text_weak)
                            .size(12.5),
                    );
                    ui.add_space(6.0);
                    // 用可选中标签展示(只读文本域在 egui 中无法拖选)。
                    ui.label(
                        RichText::new(r.url.as_str())
                            .monospace()
                            .size(12.5)
                            .color(th.text),
                    );
                    if !r.pass_code.is_empty() {
                        ui.add_space(8.0);
                        ui.label(RichText::new("提取码").color(th.text_weak).size(12.5));
                        ui.label(
                            RichText::new(r.pass_code.as_str())
                                .monospace()
                                .size(12.5)
                                .color(th.text),
                        );
                    }
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add(
                                egui::Button::new(RichText::new("复制链接").color(th.on_accent))
                                    .fill(th.accent)
                                    .stroke(Stroke::NONE)
                                    .corner_radius(th.cr(8)),
                            )
                            .clicked()
                        {
                            ctx.copy_text(r.url.clone());
                            copied = Some("已复制分享链接");
                        }
                        if !r.pass_code.is_empty()
                            && ui
                                .add(egui::Button::new(
                                    RichText::new("复制链接和提取码").color(th.text_weak),
                                ))
                                .clicked()
                        {
                            // 服务端返回的 share_text 未必带提取码, 只有确认包含时才用,
                            // 否则自行拼出「链接 + 提取码」, 保证与按钮文案一致。
                            let text = if !r.share_text.is_empty()
                                && r.share_text.contains(r.pass_code.as_str())
                            {
                                r.share_text.clone()
                            } else {
                                format!("{} 提取码: {}", r.url, r.pass_code)
                            };
                            ctx.copy_text(text);
                            copied = Some("已复制分享链接和提取码");
                        }
                        if ui.button("在浏览器打开").clicked() {
                            open = true;
                        }
                        if ui.button("完成").clicked() {
                            close = true;
                        }
                    });
                    ui.add_space(4.0);
                });
            if let Some(msg) = copied {
                g.toast_ok(msg);
            }
            if open {
                if let Err(e) = helpers::open_url(&r.url) {
                    g.toast_err(&format!("打开链接失败: {e}"));
                }
            }
            if close {
                self.result = None;
            }
        }

        if let Some(items) = self.delete_confirm.clone() {
            let ids: Vec<String> = items.iter().map(|(id, _)| id.clone()).collect();
            let mut confirmed = false;
            let mut close = false;
            egui::Window::new("取消分享")
                .collapsible(false)
                .resizable(false)
                .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    if items.len() == 1 {
                        let title = if items[0].1.is_empty() {
                            "该分享"
                        } else {
                            items[0].1.as_str()
                        };
                        ui.label(format!("确定取消分享「{title}」吗? 分享链接将立即失效。"));
                    } else {
                        ui.label(format!("确定取消这 {} 个分享吗?", items.len()));
                    }
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add(
                                egui::Button::new(RichText::new("取消分享").color(Color32::WHITE))
                                    .fill(th.danger)
                                    .stroke(Stroke::NONE),
                            )
                            .clicked()
                        {
                            confirmed = true;
                            close = true;
                        }
                        if ui.button("关闭").clicked() {
                            close = true;
                        }
                    });
                });
            if confirmed {
                g.send(Cmd::DeleteShares { ids });
                // 等待后台 SharesDeleted 回执后再关闭，避免请求失败时丢失确认框。
            } else if close {
                self.delete_confirm = None;
            }
        }
    }
}
