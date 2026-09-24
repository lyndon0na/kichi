use std::time::Duration;

use eframe::egui::{self, vec2, Align2, Color32, Key, RichText, Stroke};

use crate::icons::{self, Glyph};
use crate::msg::Cmd;
use crate::theme::Theme;

use super::helpers::{self, input};
use super::types::Crumb;
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

        if let Some(pc) = self.preview_confirm.clone() {
            let mut confirmed = false;
            let mut close = false;
            egui::Window::new("预览大文件")
                .collapsible(false)
                .resizable(false)
                .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.set_max_width(360.0);
                    ui.label(format!(
                        "「{}」大小为 {}, 预览前需要先完整下载到本地缓存。",
                        pc.name,
                        crate::format::fmt_bytes(pc.size)
                    ));
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add(
                                egui::Button::new(RichText::new("继续预览").color(th.on_accent))
                                    .fill(th.accent)
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
                self.start_preview(pc.id.clone(), pc.name.clone());
            }
            if close {
                self.preview_confirm = None;
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

        if self.offline_picker_open {
            let crumbs = self.offline_picker_stack.clone();
            let folders = self.offline_picker_folders.clone();
            let loading = self.offline_picker_loading;
            let cur_dest = self.offline_dest.clone();
            let mut nav_to: Option<usize> = None;
            let mut enter: Option<(String, String)> = None;
            let mut confirm = false;
            let mut close = false;
            egui::Window::new("选择保存到（网盘目录）")
                .collapsible(false)
                .resizable(false)
                .default_width(420.0)
                .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.add_space(4.0);
                    ui.horizontal_wrapped(|ui| {
                        let last = crumbs.len().saturating_sub(1);
                        for (i, c) in crumbs.iter().enumerate() {
                            if i > 0 {
                                ui.label(RichText::new("/").color(th.text_faint));
                            }
                            if ui.selectable_label(i == last, &c.label).clicked() && i != last {
                                nav_to = Some(i);
                            }
                        }
                    });
                    ui.add_space(6.0);
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .id_salt("offline_picker_scroll")
                        .auto_shrink([false, false])
                        .max_height(260.0)
                        .show(ui, |ui| {
                            ui.set_min_width(380.0);
                            if loading {
                                ui.vertical_centered(|ui| {
                                    ui.add_space(24.0);
                                    ui.spinner();
                                    ui.add_space(24.0);
                                });
                            } else if folders.is_empty() {
                                ui.vertical_centered(|ui| {
                                    ui.add_space(24.0);
                                    ui.label(RichText::new("此目录下没有子文件夹").weak());
                                    ui.add_space(24.0);
                                });
                            } else {
                                for f in &folders {
                                    if helpers::folder_row(ui, th, &f.name) {
                                        enter = Some((f.id.clone(), f.name.clone()));
                                    }
                                    ui.add_space(2.0);
                                }
                            }
                        });
                    ui.separator();
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("选择此目录").color(Color32::WHITE),
                                )
                                .fill(th.accent)
                                .stroke(Stroke::NONE),
                            )
                            .clicked()
                        {
                            confirm = true;
                        }
                        if ui.button("取消").clicked() {
                            close = true;
                        }
                        if let Some((_, name)) = cur_dest.as_ref() {
                            ui.label(RichText::new(format!("当前: {name}")).color(th.text_faint));
                        }
                    });
                });
            if let Some(i) = nav_to {
                self.offline_picker_stack.truncate(i + 1);
                self.offline_picker_list();
            }
            if let Some((id, name)) = enter {
                self.offline_picker_stack.push(Crumb {
                    id: Some(id),
                    label: name,
                });
                self.offline_picker_list();
            }
            if confirm {
                self.offline_dest = self
                    .offline_picker_stack
                    .last()
                    .and_then(|c| c.id.clone().map(|id| (id, c.label.clone())));
                self.offline_picker_open = false;
            }
            if close {
                self.offline_picker_open = false;
            }
        }

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

    /// 非媒体预览的缓存下载状态条(常驻于 toast 上方, 带进度与取消)。
    pub(super) fn draw_preview_status(&mut self, ctx: &egui::Context) {
        let Some(p) = self.preview_progress.clone() else {
            return;
        };
        let th = self.theme();
        let cr = th.cr(10);
        let shown: String = if p.name.chars().count() > 22 {
            format!("{}…", p.name.chars().take(20).collect::<String>())
        } else {
            p.name.clone()
        };
        let frac = if p.total > 0 {
            (p.done as f64 / p.total as f64).clamp(0.0, 1.0) as f32
        } else {
            0.0
        };
        let text = if p.total > 0 {
            format!(
                "{} / {}",
                crate::format::fmt_bytes(p.done as i64),
                crate::format::fmt_bytes(p.total as i64)
            )
        } else {
            crate::format::fmt_bytes(p.done as i64)
        };
        let mut cancel = false;
        egui::Area::new(egui::Id::new("preview-status"))
            .anchor(Align2::RIGHT_BOTTOM, [-16.0, -64.0])
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style())
                    .corner_radius(cr)
                    .show(ui, |ui| {
                        ui.set_max_width(300.0);
                        ui.add_space(2.0);
                        ui.label(
                            RichText::new(format!("预览下载中「{shown}」"))
                                .color(th.text)
                                .size(12.5),
                        );
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::ProgressBar::new(frac)
                                    .desired_width(240.0)
                                    .corner_radius(th.cr(4))
                                    .fill(th.accent)
                                    .animate(p.total == 0)
                                    .text(RichText::new(text).size(11.0)),
                            );
                            ui.add_space(2.0);
                            let (r, resp) =
                                ui.allocate_exact_size(vec2(20.0, 20.0), egui::Sense::click());
                            icons::paint(ui.painter(), r, Glyph::Close, th.text_weak);
                            if resp.on_hover_text("取消预览").clicked() {
                                cancel = true;
                            }
                        });
                        ui.add_space(2.0);
                    });
            });
        if cancel {
            self.cancel_preview();
        }
    }
}
