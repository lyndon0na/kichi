use std::time::Duration;

use eframe::egui::{self, vec2, Align2, Color32, Key, RichText, Stroke};

use crate::msg::Cmd;
use crate::theme::Theme;

use super::helpers::input;
use super::App;

impl App {
    pub(super) fn dialogs(&mut self, ctx: &egui::Context, th: &Theme) {
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
                        input(&mut name)
                            .desired_width(300.0)
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
            self.mkdir_name = name;
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
                    let resp = ui.add(input(&mut name).desired_width(300.0));
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
            self.rename_name = name;
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
                            .add(
                                egui::Button::new(
                                    RichText::new("移入回收站").color(Color32::WHITE),
                                )
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
                let src = self.current_parent();
                for id in &ids {
                    self.hidden.insert(id.clone(), src.clone());
                    self.selected.remove(id);
                }
                if let Some(entry) = self.dir_cache.get_mut(&src) {
                    entry.files.retain(|f| !ids.contains(&f.id));
                }
                self.files.retain(|f| !ids.contains(&f.id));
                self.send(Cmd::Trash { ids });
            }
            if close {
                self.trash_confirm = None;
            }
        }

        if let Some((id, name)) = self.preview.draw_confirm(ctx, th) {
            // 用户已确认大文件预览: 跳过尺寸检查直接发起。
            self.start_preview(id, name);
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
                // 显式退出登录时一并清除密钥环里保存的密码, 避免下次启动又自动登录。
                if !self.username.is_empty() {
                    self.send(Cmd::ForgetPassword {
                        username: self.username.clone(),
                    });
                }
                self.send(Cmd::Logout);
            }
            if close {
                self.logout_confirm = false;
            }
        }

        self.tasks.draw_offline_picker(ctx, &mut self.global, th);

        self.shares.draw_dialogs(ctx, &mut self.global, th);

        self.trash.draw_confirms(ctx, &mut self.global, th);

        self.shares.draw_save_dialogs(ctx, &mut self.global, th);
    }
    pub(super) fn draw_toast(&mut self, ctx: &egui::Context) {
        let Some((color, msg, since)) = self.global.toast.clone() else {
            return;
        };
        if since.elapsed() > Duration::from_secs(6) {
            self.global.toast = None;
            return;
        }
        let cr = self.theme().cr(10);
        egui::Area::new(egui::Id::new("toast"))
            .anchor(Align2::RIGHT_BOTTOM, [-16.0, -16.0])
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style())
                    .corner_radius(cr)
                    .show(ui, |ui| {
                        ui.add_space(2.0);
                        ui.horizontal(|ui| {
                            let (dr, _) =
                                ui.allocate_exact_size(vec2(10.0, 10.0), egui::Sense::hover());
                            ui.painter().circle_filled(dr.center(), 3.5, color);
                            ui.add_space(2.0);
                            ui.label(RichText::new(msg).color(color));
                        });
                        ui.add_space(2.0);
                    });
            });
    }
}
