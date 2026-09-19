use std::time::Duration;

use eframe::egui::{self, vec2, Align2, Color32, FontId, Key, Pos2, Rect, RichText, Stroke};

use crate::icons::{self, Glyph};
use crate::msg::Cmd;
use crate::theme::Theme;
use kichi_core::types::File;

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
    let icon_rect = Rect::from_center_size(
        Pos2::new(rect.min.x + 16.0, rect.center().y),
        vec2(18.0, 18.0),
    );
    icons::paint(
        &painter,
        icon_rect,
        Glyph::Folder,
        Color32::from_rgb(232, 178, 84),
    );
    let g = truncate_text(
        &painter,
        name,
        rect.width() - 34.0,
        FontId::proportional(13.5),
        th.text,
    );
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

        // 转存分享弹窗
        if self.save_share_open {
            let mut resolve = false;
            let mut save = false;
            let mut close = false;

            egui::Window::new("转存分享")
                .collapsible(false)
                .resizable(false)
                .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.set_min_width(380.0);
                    ui.add_space(4.0);

                    // 错误提示
                    if let Some(err) = &self.save_share_error {
                        ui.label(RichText::new(err).color(th.danger).size(12.0));
                        ui.add_space(6.0);
                    }

                    // 未解析: 显示输入框
                    if self.save_share_id.is_none() {
                        ui.label(RichText::new("分享链接或 ID").color(th.text_weak));
                        let resp = ui.add(
                            input(&mut self.save_share_input)
                                .desired_width(360.0)
                                .hint_text("https://mypikpak.com/s/xxx 或直接输入 ID"),
                        );
                        let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                        ui.add_space(6.0);
                        ui.label(RichText::new("提取码 (可选)").color(th.text_weak));
                        ui.add(
                            input(&mut self.save_share_pass_code)
                                .desired_width(360.0)
                                .hint_text("公开分享无需填写"),
                        );
                        ui.add_space(12.0);
                        ui.horizontal(|ui| {
                            let can_resolve = !self.save_share_input.trim().is_empty()
                                && !self.save_share_resolving;
                            if ui
                                .add_enabled(can_resolve, egui::Button::new("解析"))
                                .clicked()
                                || enter
                            {
                                resolve = true;
                            }
                            if ui.button("取消").clicked() {
                                close = true;
                            }
                            if self.save_share_resolving {
                                ui.add_space(8.0);
                                ui.spinner();
                                ui.label(RichText::new("正在解析…").color(th.text_weak));
                            }
                        });
                    } else {
                        // 已解析: 显示文件列表
                        let title = self.save_share_title.clone().unwrap_or_default();
                        if !title.is_empty() {
                            ui.label(
                                RichText::new(format!("分享: {title}"))
                                    .strong()
                                    .color(th.text),
                            );
                            ui.add_space(6.0);
                        }

                        let files = self.save_share_files.clone();
                        let filter = self.save_share_filter.clone();
                        let filtered: Vec<&File> = if filter.is_empty() {
                            files.iter().collect()
                        } else {
                            let lower = filter.to_lowercase();
                            files
                                .iter()
                                .filter(|f| f.name.to_lowercase().contains(&lower))
                                .collect()
                        };
                        let selected = self.save_share_selected.clone();
                        let filtered_selected =
                            filtered.iter().filter(|f| selected.contains(&f.id)).count();
                        let all_filtered_selected =
                            !filtered.is_empty() && filtered_selected >= filtered.len();

                        ui.label(
                            RichText::new(format!("共 {} 个文件", files.len()))
                                .color(th.text_weak)
                                .size(12.0),
                        );
                        ui.add_space(4.0);

                        // 搜索框
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("搜索").color(th.text_weak).size(12.0));
                            ui.add(
                                input(&mut self.save_share_filter)
                                    .desired_width(200.0)
                                    .hint_text("按文件名过滤"),
                            );
                            if !filter.is_empty() && ui.button("清除").clicked() {
                                self.save_share_filter.clear();
                            }
                        });
                        ui.add_space(4.0);

                        // 全选(仅对过滤后的文件生效)
                        let mut sel = all_filtered_selected;
                        if ui.checkbox(&mut sel, "全选").changed() {
                            if sel {
                                for f in &filtered {
                                    self.save_share_selected.insert(f.id.clone());
                                }
                            } else {
                                for f in &filtered {
                                    self.save_share_selected.remove(&f.id);
                                }
                            }
                        }
                        if !filter.is_empty() {
                            ui.label(
                                RichText::new(format!("(匹配 {} 个)", filtered.len()))
                                    .color(th.text_faint)
                                    .size(11.0),
                            );
                        }
                        ui.add_space(4.0);

                        // 文件列表
                        egui::ScrollArea::vertical()
                            .max_height(240.0)
                            .show(ui, |ui| {
                                if filtered.is_empty() && !files.is_empty() {
                                    ui.label(
                                        RichText::new("没有匹配的文件")
                                            .color(th.text_weak)
                                            .size(12.0),
                                    );
                                }
                                for file in &filtered {
                                    let is_sel = self.save_share_selected.contains(&file.id);
                                    let mut checked = is_sel;
                                    let icon = if file.is_folder() {
                                        Glyph::Folder
                                    } else {
                                        Glyph::File
                                    };
                                    ui.horizontal(|ui| {
                                        if ui.checkbox(&mut checked, "").changed() {
                                            if checked {
                                                self.save_share_selected.insert(file.id.clone());
                                            } else {
                                                self.save_share_selected.remove(&file.id);
                                            }
                                        }
                                        let (r, _) = ui.allocate_exact_size(
                                            vec2(16.0, 16.0),
                                            egui::Sense::hover(),
                                        );
                                        let color = if file.is_folder() {
                                            Color32::from_rgb(232, 178, 84)
                                        } else {
                                            th.text_weak
                                        };
                                        icons::paint(ui.painter(), r, icon, color);
                                        ui.label(
                                            RichText::new(&file.name).color(th.text).size(13.0),
                                        );
                                    });
                                }
                            });

                        // 加载更多按钮
                        if let Some(next_token) = self.save_share_next.clone() {
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                if self.save_share_loading_more {
                                    ui.spinner();
                                    ui.label(
                                        RichText::new("正在加载…").color(th.text_weak).size(12.0),
                                    );
                                } else if ui.button("加载更多文件").clicked() {
                                    if let (Some(share_id), Some(token)) =
                                        (&self.save_share_id, &self.save_share_token)
                                    {
                                        self.save_share_loading_more = true;
                                        self.save_share_error = None;
                                        self.send(Cmd::LoadMoreShareFiles {
                                            share_id: share_id.clone(),
                                            pass_code_token: token.clone(),
                                            page_token: next_token,
                                        });
                                    }
                                }
                            });
                        }

                        ui.add_space(12.0);

                        ui.horizontal(|ui| {
                            let can_save =
                                !self.save_share_selected.is_empty() && !self.save_share_saving;

                            // 左侧: 目录选择按钮, 显示当前目标目录名或"默认位置"
                            let dest_label = self
                                .save_share_dest
                                .as_ref()
                                .map(|(_, name)| name.as_str())
                                .unwrap_or("默认位置");
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new(dest_label).color(Color32::WHITE),
                                    )
                                    .fill(Color32::from_gray(45))
                                    .stroke(Stroke::new(1.0, Color32::from_gray(70)))
                                    .min_size(vec2(120.0, 0.0)),
                                )
                                .on_hover_text("点击选择保存目录")
                                .clicked()
                            {
                                self.open_save_share_picker();
                            }

                            ui.add_space(8.0);

                            // 右侧: 保存按钮
                            if ui
                                .add_enabled(
                                    can_save,
                                    egui::Button::new(RichText::new("保存").color(Color32::WHITE))
                                        .fill(th.accent)
                                        .stroke(Stroke::NONE),
                                )
                                .clicked()
                            {
                                save = true;
                            }

                            if ui.button("返回").clicked() {
                                // 返回输入状态
                                self.save_share_id = None;
                                self.save_share_title = None;
                                self.save_share_token = None;
                                self.save_share_files.clear();
                                self.save_share_selected.clear();
                                self.save_share_error = None;
                            }
                            if ui.button("取消").clicked() {
                                close = true;
                            }
                            if self.save_share_saving {
                                ui.add_space(8.0);
                                ui.spinner();
                                ui.label(RichText::new("正在转存…").color(th.text_weak));
                            }
                        });
                    }
                });

            if resolve {
                self.save_share_error = None;
                match Self::parse_share_input(&self.save_share_input) {
                    Some((share_id, extracted_pass)) => {
                        // 链接里带了提取码且用户未填写时自动回填。
                        if let Some(p) = extracted_pass {
                            if self.save_share_pass_code.trim().is_empty() {
                                self.save_share_pass_code = p;
                            }
                        }
                        self.save_share_resolving = true;
                        let pass_code = self.save_share_pass_code.clone();
                        self.send(Cmd::ResolveShare {
                            share_id,
                            pass_code,
                        });
                    }
                    None => {
                        self.save_share_error = Some(
                            "无法识别分享链接, 请检查格式 (例如 https://mypikpak.com/s/xxx 或直接输入 ID)"
                                .to_string(),
                        );
                    }
                }
            }
            if save {
                if let (Some(share_id), Some(token)) = (&self.save_share_id, &self.save_share_token)
                {
                    self.save_share_saving = true;
                    self.save_share_error = None;
                    let file_ids: Vec<String> = self.save_share_selected.iter().cloned().collect();
                    let dest = self.save_share_dest.as_ref().and_then(|(id, _)| {
                        if id.is_empty() {
                            None
                        } else {
                            Some(id.clone())
                        }
                    });
                    self.send(Cmd::SaveShare {
                        share_id: share_id.clone(),
                        pass_code_token: token.clone(),
                        file_ids,
                        dest,
                    });
                }
            }
            if close {
                self.save_share_open = false;
                self.clear_save_share_state();
            }
        }

        // 转存分享目录选择器
        if self.save_share_picker_open {
            let crumbs = self.save_share_picker_stack.clone();
            let folders = self.save_share_picker_folders.clone();
            let loading = self.save_share_picker_loading;
            let cur_dest = self.save_share_dest.clone();
            let mut nav_to: Option<usize> = None;
            let mut enter: Option<(String, String)> = None;
            let mut confirm = false;
            let mut close_picker = false;

            egui::Window::new("选择转存到（网盘目录）")
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
                        .id_salt("save_share_picker_scroll")
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
                            close_picker = true;
                        }
                        if let Some((_, name)) = cur_dest.as_ref() {
                            ui.label(RichText::new(format!("当前: {name}")).color(th.text_faint));
                        }
                    });
                });

            if let Some(i) = nav_to {
                self.save_share_picker_stack.truncate(i + 1);
                self.save_share_picker_list();
            }
            if let Some((id, name)) = enter {
                self.save_share_picker_stack.push(Crumb {
                    id: Some(id.clone()),
                    label: name,
                });
                self.save_share_picker_list();
            }
            if confirm {
                // 获取当前目录作为目标
                let parent = self.save_share_picker_parent();
                let label = self
                    .save_share_picker_stack
                    .last()
                    .map(|c| c.label.clone())
                    .unwrap_or_else(|| "我的云盘".to_string());
                // 选择根目录时, 用空字符串标记, 以便 UI 显示"我的云盘"而非"默认位置"
                self.save_share_dest = Some((parent.unwrap_or_default(), label));
                self.save_share_picker_open = false;
            }
            if close_picker {
                self.save_share_picker_open = false;
            }
        }

        // 自动移动失败重试对话框
        if let Some((dest_id, dest_name)) = self.save_share_move_failed.clone() {
            let mut retry = false;
            let mut dismiss = false;

            egui::Window::new("移动失败")
                .collapsible(false)
                .resizable(false)
                .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.set_min_width(320.0);
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(format!(
                            "文件已转存到「转存自分享」, 但自动移动到「{dest_name}」失败。"
                        ))
                        .color(th.text),
                    );
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new("您可以重试移动, 或稍后手动处理。")
                            .color(th.text_weak)
                            .size(12.0),
                    );
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add(
                                egui::Button::new(RichText::new("重试移动").color(Color32::WHITE))
                                    .fill(th.accent)
                                    .stroke(Stroke::NONE),
                            )
                            .clicked()
                        {
                            retry = true;
                        }
                        if ui.button("稍后处理").clicked() {
                            dismiss = true;
                        }
                    });
                });

            if retry {
                self.send(Cmd::RetryMoveShare { dest: dest_id });
                self.save_share_move_failed = None;
            }
            if dismiss {
                self.save_share_move_failed = None;
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
