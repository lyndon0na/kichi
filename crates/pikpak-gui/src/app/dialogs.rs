use std::time::Duration;

use eframe::egui::{self, Align2, Color32, FontId, Key, Pos2, Rect, RichText, Stroke, vec2};

use crate::icons::{self, Glyph};
use crate::msg::Cmd;
use crate::theme::Theme;

use super::helpers::{self, input, truncate_text};
use super::types::Crumb;
use super::App;

/// 目录选择器里的一行文件夹(矢量文件夹图标 + 名称, 超长截断)。
fn folder_row(ui: &mut egui::Ui, th: &Theme, name: &str) -> bool {
    let h = 30.0;
    let w = ui.available_width().max(120.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, h), egui::Sense::click());
    let painter = ui.painter().clone();
    if resp.hovered() {
        painter.rect_filled(rect, th.cr(6), th.hover);
    }
    let icon_rect =
        Rect::from_center_size(Pos2::new(rect.min.x + 16.0, rect.center().y), vec2(18.0, 18.0));
    icons::paint(&painter, icon_rect, Glyph::Folder, Color32::from_rgb(232, 178, 84));
    let g = truncate_text(&painter, name, rect.width() - 34.0, FontId::proportional(13.5), th.text);
    painter.galley(
        Pos2::new(rect.min.x + 32.0, rect.center().y - g.size().y / 2.0),
        g,
        th.text,
    );
    resp.clicked()
}

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
                                    if folder_row(ui, th, &f.name) {
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
                                egui::Button::new(RichText::new("选择此目录").color(Color32::WHITE))
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
                self.offline_picker_stack.push(Crumb { id: Some(id), label: name });
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

        if let Some(targets) = self.share_dialog.clone() {
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
                        let label = match self.share_expiration_days {
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
                                    ui.selectable_value(&mut self.share_expiration_days, d, l);
                                }
                            });
                    });
                    ui.add_space(6.0);
                    ui.checkbox(&mut self.share_need_password, "需要提取码");
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
                self.send(Cmd::CreateShare {
                    file_ids: ids,
                    expiration_days: self.share_expiration_days,
                    need_password: self.share_need_password,
                    label,
                });
            }
            if close {
                self.share_dialog = None;
            }
        }

        if let Some(r) = self.share_result.clone() {
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
                self.toast_ok(msg);
            }
            if open {
                if let Err(e) = helpers::open_url(&r.url) {
                    self.toast_err(&format!("打开链接失败: {e}"));
                }
            }
            if close {
                self.share_result = None;
            }
        }

        if let Some(items) = self.share_delete_confirm.clone() {
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
                self.send(Cmd::DeleteShares { ids });
                // 等待后台 SharesDeleted 回执后再关闭，避免请求失败时丢失确认框。
            } else if close {
                self.share_delete_confirm = None;
            }
        }

        if let Some(items) = self.trash_delete_confirm.clone() {
            let ids: Vec<String> = items.iter().map(|(id, _)| id.clone()).collect();
            let mut confirmed = false;
            let mut close = false;
            egui::Window::new("彻底删除")
                .collapsible(false)
                .resizable(false)
                .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    if items.len() == 1 {
                        ui.label(format!("确定彻底删除「{}」吗?", items[0].1));
                    } else {
                        ui.label(format!("确定彻底删除这 {} 项吗?", items.len()));
                    }
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new("彻底删除后无法恢复。")
                            .color(th.danger)
                            .size(12.0),
                    );
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add(
                                egui::Button::new(RichText::new("彻底删除").color(Color32::WHITE))
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
                self.send(Cmd::DeleteTrash { ids });
                // 等待后台 TrashDeleted 回执后再关闭，避免请求失败时丢失确认框。
            } else if close {
                self.trash_delete_confirm = None;
            }
        }

        if self.trash_empty_confirm {
            let mut confirmed = false;
            let mut close = false;
            egui::Window::new("清空回收站")
                .collapsible(false)
                .resizable(false)
                .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label("确定清空回收站吗?");
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new("回收站中的全部文件将被彻底删除, 无法恢复。")
                            .color(th.danger)
                            .size(12.0),
                    );
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add(
                                egui::Button::new(RichText::new("清空").color(Color32::WHITE))
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
                self.send(Cmd::EmptyTrash);
                // 等待后台 TrashEmptied 回执后再关闭。
            } else if close {
                self.trash_empty_confirm = false;
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
        let cr = self.theme().cr(10);
        egui::Area::new(egui::Id::new("toast"))
            .anchor(Align2::RIGHT_BOTTOM, [-16.0, -16.0])
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style())
                    .corner_radius(cr)
                    .show(ui, |ui| {
                        ui.add_space(2.0);
                        ui.horizontal(|ui| {
                            let (dr, _) = ui.allocate_exact_size(vec2(10.0, 10.0), egui::Sense::hover());
                            ui.painter().circle_filled(dr.center(), 3.5, color);
                            ui.add_space(2.0);
                            ui.label(RichText::new(msg).color(color));
                        });
                        ui.add_space(2.0);
                    });
            });
    }
}
