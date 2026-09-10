use std::time::Duration;

use eframe::egui::{
    self, vec2, Align2, Color32, CornerRadius, FontId, Key, Pos2, Rect, RichText, Sense, Stroke,
};

use crate::icons::{self, Glyph};
use crate::msg::Cmd;
use crate::theme::{mix, Theme};

use super::helpers::truncate_text;
use super::types::MoveMode;
use super::App;

impl App {
    pub(super) fn dialogs(&mut self, ctx: &egui::Context, th: &Theme) {
        let _ = th;
        if self.mkdir_open {
            let parent = self.current_parent();
            let mut name = self.mkdir_name.clone();
            let mut confirmed = false;
            let mut close = false;
            egui::Window::new("新建文件夹")
                .collapsible(false)
                .resizable(false)
                .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.add_space(4.0);
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut name)
                            .desired_width(260.0)
                            .hint_text("文件夹名称"),
                    );
                    let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        let ok = !name.trim().is_empty();
                        if ui.add_enabled(ok, egui::Button::new("创建")).clicked() || enter {
                            confirmed = true;
                            close = true;
                        }
                        if ui.button("取消").clicked() {
                            close = true;
                        }
                    });
                });
            if confirmed {
                let name = name.trim().to_string();
                self.send(Cmd::CreateFolder { name, parent });
            }
            self.mkdir_open = !close;
        }

        if let Some(id) = self.rename_id.clone() {
            let mut name = self.rename_name.clone();
            let mut confirmed = false;
            let mut close = false;
            egui::Window::new("重命名")
                .collapsible(false)
                .resizable(false)
                .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.add_space(4.0);
                    let resp = ui.add(egui::TextEdit::singleline(&mut name).desired_width(260.0));
                    let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        let ok = !name.trim().is_empty();
                        if ui.add_enabled(ok, egui::Button::new("确定")).clicked() || enter {
                            confirmed = true;
                            close = true;
                        }
                        if ui.button("取消").clicked() {
                            close = true;
                        }
                    });
                });
            if confirmed {
                let name = name.trim().to_string();
                self.send(Cmd::Rename { id, name });
            }
            if close {
                self.rename_id = None;
            }
        }

        if let Some(items) = self.trash_confirm.clone() {
            let ids: Vec<String> = items.iter().map(|(id, _)| id.clone()).collect();
            let mut confirmed = false;
            let mut close = false;
            egui::Window::new("移入回收站")
                .collapsible(false)
                .resizable(false)
                .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    if items.len() == 1 {
                        ui.label(format!("确定将「{}」移入回收站吗?", items[0].1));
                    } else {
                        ui.label(format!("确定将这 {} 项移入回收站吗?", items.len()));
                        if let Some((_, first)) = items.first() {
                            ui.label(RichText::new(first.clone()).weak());
                        }
                        if items.len() > 1 {
                            ui.label(RichText::new("……").weak());
                        }
                    }
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add(egui::Button::new(RichText::new("移入回收站").color(Color32::WHITE)).fill(th.danger).stroke(Stroke::NONE))
                            .clicked()
                        {
                            confirmed = true;
                            close = true;
                        }
                        if ui.button("取消").clicked() {
                            close = true;
                        }
                    });
                });
            if confirmed {
                for id in &ids {
                    self.hidden.insert(id.clone());
                    self.selected.remove(id);
                }
                self.files.retain(|f| !ids.contains(&f.id));
                self.send(Cmd::Trash { ids });
            }
            if close {
                self.trash_confirm = None;
            }
        }

        if self.logout_confirm {
            let mut confirmed = false;
            let mut close = false;
            egui::Window::new("退出登录")
                .collapsible(false)
                .resizable(false)
                .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label("确定要退出登录吗?");
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add(
                                egui::Button::new(RichText::new("退出").color(Color32::WHITE))
                                    .fill(th.danger)
                                    .stroke(Stroke::NONE),
                            )
                            .clicked()
                        {
                            confirmed = true;
                            close = true;
                        }
                        if ui.button("取消").clicked() {
                            close = true;
                        }
                    });
                });
            if confirmed {
                self.send(Cmd::Logout);
            }
            if close {
                self.logout_confirm = false;
            }
        }

        self.folder_picker(ctx, th);
    }

    /// 「移动/复制到…」: 弹窗内浏览目标目录并确认。
    fn folder_picker(&mut self, ctx: &egui::Context, th: &Theme) {
        enum Act {
            Open(String, String),
            Up,
            Jump(usize),
            Refresh,
            LoadMore,
            Confirm,
            Cancel,
        }
        let mut act: Vec<Act> = Vec::new();

        {
            let Some(dlg) = self.move_dialog.as_ref() else {
                return;
            };
            let verb = dlg.mode.verb();
            let at_source = self.pick_at_source();
            let top_name = dlg
                .stack
                .last()
                .map(|c| c.label.clone())
                .unwrap_or_default();

            egui::Window::new(dlg.mode.title())
                .collapsible(false)
                .resizable(false)
                .default_width(460.0)
                .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.add_space(2.0);
                    ui.horizontal(|ui| {
                        let n = dlg.ids.len();
                        ui.label(
                            RichText::new(format!("{verb} {n} 项 · 选择目标目录"))
                                .size(13.0)
                                .color(th.text_weak),
                        );
                    });
                    ui.add_space(6.0);

                    // 路径: 返回上级 + 面包屑 + 刷新
                    ui.horizontal(|ui| {
                        let can_up = dlg.stack.len() > 1;
                        let (ur, up_resp) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
                        ui.painter().rect_filled(ur, CornerRadius::same(5), th.card);
                        let color = if can_up {
                            if up_resp.hovered() { th.text } else { th.text_weak }
                        } else {
                            th.text_faint
                        };
                        icons::paint(ui.painter(), ur.shrink(4.0), Glyph::Up, color);
                        if can_up && up_resp.clicked() {
                            act.push(Act::Up);
                        }
                        up_resp.on_hover_text("返回上级目录");
                        ui.add_space(2.0);
                        for (i, crumb) in dlg.stack.iter().enumerate() {
                            if i > 0 {
                                ui.label(RichText::new("›").color(th.text_faint));
                            }
                            let last = i == dlg.stack.len() - 1;
                            let text = RichText::new(&crumb.label)
                                .size(13.5)
                                .color(if last { th.text } else { th.text_weak });
                            let resp = ui.add(egui::Button::new(text).frame(false));
                            if !last && resp.clicked() {
                                act.push(Act::Jump(i));
                            }
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let (rr, rr_resp) =
                                ui.allocate_exact_size(vec2(20.0, 20.0), Sense::click());
                            ui.painter().rect_filled(
                                rr,
                                CornerRadius::same(5),
                                if rr_resp.hovered() { th.hover } else { th.card },
                            );
                            icons::paint(
                                ui.painter(),
                                rr.shrink(3.5),
                                Glyph::Refresh,
                                th.text_weak,
                            );
                            if rr_resp.clicked() {
                                act.push(Act::Refresh);
                            }
                            rr_resp.on_hover_text("刷新此目录");
                        });
                    });

                    ui.add_space(6.0);
                    ui.separator();

                    // 子文件夹列表
                    let row_h = 34.0;
                    egui::ScrollArea::vertical()
                        .id_salt("move_pick_scroll")
                        .auto_shrink([false, false])
                        .max_height(280.0)
                        .show(ui, |ui| {
                            ui.set_min_height(140.0);
                            if dlg.loading && dlg.folders.is_empty() {
                                ui.vertical_centered(|ui| {
                                    ui.spinner();
                                    ui.label(RichText::new("加载目录中…").color(th.text_weak));
                                });
                                return;
                            }
                            let iter: Vec<&pikpak_core::types::File> =
                                dlg.folders.iter().filter(|f| !dlg.blocked.contains(&f.id)).collect();
                            if iter.is_empty() {
                                ui.vertical_centered(|ui| {
                                    let msg = if dlg.folders.is_empty() {
                                        "此目录下没有子文件夹"
                                    } else {
                                        "没有可进入的子文件夹(所选文件夹被排除)"
                                    };
                                    ui.label(RichText::new(msg).color(th.text_weak));
                                });
                            }
                            let row_w = ui.available_width().max(200.0);
                            let folder_color = Color32::from_rgb(232, 178, 84);
                            for f in &iter {
                                let (rect, resp) =
                                    ui.allocate_exact_size(vec2(row_w, row_h), Sense::click());
                                let painter = ui.painter().clone();
                                let hovered = resp.hovered();
                                let bg = if hovered {
                                    mix(th.card, th.text, if th.dark { 0.08 } else { 0.05 })
                                } else {
                                    Color32::TRANSPARENT
                                };
                                painter.rect_filled(
                                    rect.shrink2(vec2(3.0, 2.0)),
                                    CornerRadius::same(6),
                                    bg,
                                );
                                let ic = Rect::from_center_size(
                                    Pos2::new(rect.min.x + 21.0, rect.center().y),
                                    vec2(18.0, 18.0),
                                );
                                icons::paint(&painter, ic.shrink(1.5), Glyph::Folder, folder_color);
                                let g = truncate_text(
                                    &painter,
                                    &f.name,
                                    rect.width() - 48.0,
                                    FontId::proportional(13.5),
                                    if hovered { th.text } else { th.text_weak },
                                );
                                painter.galley(
                                    Pos2::new(rect.min.x + 36.0, rect.center().y - g.size().y / 2.0),
                                    g,
                                    th.text,
                                );
                                // 单击进入目录; 双击不产生第二次进入, 避免误开子项。
                                if !resp.double_clicked() && resp.clicked() {
                                    act.push(Act::Open(f.id.clone(), f.name.clone()));
                                }
                            }
                            ui.add_space(4.0);
                            if dlg.next.is_some() {
                                ui.vertical_centered(|ui| {
                                    if ui.button("加载更多目录…").clicked() {
                                        act.push(Act::LoadMore);
                                    }
                                });
                            }
                        });

                    ui.separator();
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        let confirm_lbl = match dlg.mode {
                            MoveMode::Move => "移动到这里",
                            MoveMode::Copy => "复制到这里",
                        };
                        let resp = ui.add_enabled(
                            !at_source,
                            egui::Button::new(RichText::new(confirm_lbl).color(th.on_accent))
                                .fill(th.accent)
                                .stroke(Stroke::NONE)
                                .corner_radius(CornerRadius::same(8)),
                        );
                        if resp.clicked() {
                            act.push(Act::Confirm);
                        }
                        if at_source {
                            ui.label(
                                RichText::new(format!("已在「{top_name}」中"))
                                    .color(th.text_faint)
                                    .size(12.5),
                            );
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("取消").clicked() {
                                act.push(Act::Cancel);
                            }
                        });
                    });
                    ui.add_space(2.0);
                });
        }

        for a in act {
            match a {
                Act::Open(id, name) => self.pick_open(id, name),
                Act::Up => self.pick_up(),
                Act::Jump(i) => self.pick_jump(i),
                Act::Refresh => self.pick_refresh(),
                Act::LoadMore => self.pick_load_more(),
                Act::Confirm => self.pick_confirm(),
                Act::Cancel => self.close_move_dialog(),
            }
        }
    }

    pub(super) fn draw_toast(&mut self, ctx: &egui::Context) {
        let Some((color, msg, since)) = self.toast.clone() else {
            return;
        };
        if since.elapsed() > Duration::from_secs(6) {
            self.toast = None;
            return;
        }
        egui::Area::new(egui::Id::new("toast"))
            .anchor(Align2::RIGHT_BOTTOM, [-16.0, -16.0])
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style())
                    .corner_radius(CornerRadius::same(10))
                    .show(ui, |ui| {
                        ui.add_space(2.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("● ").color(color));
                            ui.label(RichText::new(msg).color(color));
                        });
                        ui.add_space(2.0);
                    });
            });
    }
}
