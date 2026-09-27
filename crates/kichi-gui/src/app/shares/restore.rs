//! 转存分享: 解析他人分享链接 / 浏览文件 / 选择目标目录并保存。

use eframe::egui::{self, vec2, Align2, Color32, Key, RichText, Stroke};

use kichi_core::types::File;

use crate::icons::{self, Glyph};
use crate::msg::Cmd;
use crate::theme::Theme;

use super::super::files::Crumb;
use super::super::global::Global;
use super::super::helpers::{folder_row, input};
use super::SharesPage;

impl SharesPage {
    /// 转存分享弹窗 / 目标目录选择器 / 自动移动失败重试。
    pub(crate) fn draw_save_dialogs(&mut self, ctx: &egui::Context, g: &mut Global, th: &Theme) {
        // 转存分享弹窗
        if self.save_open {
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
                    if let Some(err) = &self.save_error {
                        ui.label(RichText::new(err).color(th.danger).size(12.0));
                        ui.add_space(6.0);
                    }

                    // 未解析: 显示输入框
                    if self.save_id.is_none() {
                        ui.label(RichText::new("分享链接或 ID").color(th.text_weak));
                        let resp = ui.add(
                            input(&mut self.save_input)
                                .desired_width(360.0)
                                .hint_text("https://mypikpak.com/s/xxx 或直接输入 ID"),
                        );
                        let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                        ui.add_space(6.0);
                        ui.label(RichText::new("提取码 (可选)").color(th.text_weak));
                        ui.add(
                            input(&mut self.save_pass_code)
                                .desired_width(360.0)
                                .hint_text("公开分享无需填写"),
                        );
                        ui.add_space(12.0);
                        ui.horizontal(|ui| {
                            let can_resolve =
                                !self.save_input.trim().is_empty() && !self.save_resolving;
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
                            if self.save_resolving {
                                ui.add_space(8.0);
                                ui.spinner();
                                ui.label(RichText::new("正在解析…").color(th.text_weak));
                            }
                        });
                    } else {
                        // 已解析: 显示文件列表
                        let title = self.save_title.clone().unwrap_or_default();
                        if !title.is_empty() {
                            ui.label(
                                RichText::new(format!("分享: {title}"))
                                    .strong()
                                    .color(th.text),
                            );
                            ui.add_space(6.0);
                        }

                        let files = self.save_files.clone();
                        let filter = self.save_filter.clone();
                        let filtered: Vec<&File> = if filter.is_empty() {
                            files.iter().collect()
                        } else {
                            let lower = filter.to_lowercase();
                            files
                                .iter()
                                .filter(|f| f.name.to_lowercase().contains(&lower))
                                .collect()
                        };
                        let selected = self.save_selected.clone();
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
                                input(&mut self.save_filter)
                                    .desired_width(200.0)
                                    .hint_text("按文件名过滤"),
                            );
                            if !filter.is_empty() && ui.button("清除").clicked() {
                                self.save_filter.clear();
                            }
                        });
                        ui.add_space(4.0);

                        // 全选(仅对过滤后的文件生效)
                        let mut sel = all_filtered_selected;
                        if ui.checkbox(&mut sel, "全选").changed() {
                            if sel {
                                for f in &filtered {
                                    self.save_selected.insert(f.id.clone());
                                }
                            } else {
                                for f in &filtered {
                                    self.save_selected.remove(&f.id);
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
                                    let is_sel = self.save_selected.contains(&file.id);
                                    let mut checked = is_sel;
                                    let icon = if file.is_folder() {
                                        Glyph::Folder
                                    } else {
                                        Glyph::File
                                    };
                                    ui.horizontal(|ui| {
                                        if ui.checkbox(&mut checked, "").changed() {
                                            if checked {
                                                self.save_selected.insert(file.id.clone());
                                            } else {
                                                self.save_selected.remove(&file.id);
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
                        if let Some(next_token) = self.save_next.clone() {
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                if self.save_loading_more {
                                    ui.spinner();
                                    ui.label(
                                        RichText::new("正在加载…").color(th.text_weak).size(12.0),
                                    );
                                } else if ui.button("加载更多文件").clicked() {
                                    if let (Some(share_id), Some(token)) =
                                        (&self.save_id, &self.save_token)
                                    {
                                        self.save_loading_more = true;
                                        self.save_error = None;
                                        g.send(Cmd::LoadMoreShareFiles {
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
                            let can_save = !self.save_selected.is_empty() && !self.save_saving;

                            // 左侧: 目录选择按钮, 显示当前目标目录名或"默认位置"
                            let dest_label = self
                                .save_dest
                                .as_ref()
                                .map(|(_, name)| name.as_str())
                                .unwrap_or("默认位置");
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new(dest_label).color(Color32::WHITE),
                                    )
                                    .fill(Color32::from_gray(45))
                                    .stroke(Stroke::new(1.0_f32, Color32::from_gray(70)))
                                    .min_size(vec2(120.0, 0.0)),
                                )
                                .on_hover_text("点击选择保存目录")
                                .clicked()
                            {
                                self.open_picker(g);
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
                                self.save_id = None;
                                self.save_title = None;
                                self.save_token = None;
                                self.save_files.clear();
                                self.save_selected.clear();
                                self.save_error = None;
                            }
                            if ui.button("取消").clicked() {
                                close = true;
                            }
                            if self.save_saving {
                                ui.add_space(8.0);
                                ui.spinner();
                                ui.label(RichText::new("正在转存…").color(th.text_weak));
                            }
                        });
                    }
                });

            if resolve {
                self.save_error = None;
                match parse_share_input(&self.save_input) {
                    Some((share_id, extracted_pass)) => {
                        // 链接里带了提取码且用户未填写时自动回填。
                        if let Some(p) = extracted_pass {
                            if self.save_pass_code.trim().is_empty() {
                                self.save_pass_code = p;
                            }
                        }
                        self.save_resolving = true;
                        let pass_code = self.save_pass_code.clone();
                        g.send(Cmd::ResolveShare {
                            share_id,
                            pass_code,
                        });
                    }
                    None => {
                        self.save_error = Some(
                            "无法识别分享链接, 请检查格式 (例如 https://mypikpak.com/s/xxx 或直接输入 ID)"
                                .to_string(),
                        );
                    }
                }
            }
            if save {
                if let (Some(share_id), Some(token)) = (&self.save_id, &self.save_token) {
                    self.save_saving = true;
                    self.save_error = None;
                    let file_ids: Vec<String> = self.save_selected.iter().cloned().collect();
                    let dest = self.save_dest.as_ref().and_then(|(id, _)| {
                        if id.is_empty() {
                            None
                        } else {
                            Some(id.clone())
                        }
                    });
                    g.send(Cmd::SaveShare {
                        share_id: share_id.clone(),
                        pass_code_token: token.clone(),
                        file_ids,
                        dest,
                    });
                }
            }
            if close {
                self.save_open = false;
                self.clear_save();
            }
        }

        // 转存分享目录选择器
        if self.picker_open {
            let crumbs = self.picker_stack.clone();
            let folders = self.picker_folders.clone();
            let loading = self.picker_loading;
            let cur_dest = self.save_dest.clone();
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
                self.picker_stack.truncate(i + 1);
                self.picker_list(g);
            }
            if let Some((id, name)) = enter {
                self.picker_stack.push(Crumb {
                    id: Some(id.clone()),
                    label: name,
                });
                self.picker_list(g);
            }
            if confirm {
                // 获取当前目录作为目标
                let parent = self.picker_parent();
                let label = self
                    .picker_stack
                    .last()
                    .map(|c| c.label.clone())
                    .unwrap_or_else(|| "我的云盘".to_string());
                // 选择根目录时, 用空字符串标记, 以便 UI 显示"我的云盘"而非"默认位置"
                self.save_dest = Some((parent.unwrap_or_default(), label));
                self.picker_open = false;
            }
            if close_picker {
                self.picker_open = false;
            }
        }

        // 自动移动失败重试对话框
        if let Some((dest_id, dest_name)) = self.save_move_failed.clone() {
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
                g.send(Cmd::RetryMoveShare { dest: dest_id });
                self.save_move_failed = None;
            }
            if dismiss {
                self.save_move_failed = None;
            }
        }
    }
}

/// 解析分享输入, 返回 `(share_id, 链接里携带的提取码)`。
///
/// 支持的形态:
/// - 完整链接: `https://mypikpak.com/s/<id>`
/// - 带查询参数 / 片段: `.../s/<id>?password=abcd#frag`(顺带提取 `password`/`pass_code`)
/// - 复制链接时附带的前后缀文字: 只取 `/s/` 之后的一段
/// - 裸 ID: `<id>`
///
/// 无法识别(如缺少 `/s/` 的其它域名链接、含非法字符)时返回 `None`, 由调用方给出提示。
fn parse_share_input(input: &str) -> Option<(String, Option<String>)> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }

    // 截出 `/s/` 之后的一段: ID 到第一个非法字符为止, 之后按查询串解析提取码。
    let (id_candidate, query) = if let Some(idx) = input.rfind("/s/") {
        let rest = &input[idx + 3..];
        let id: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        let query = rest.find('?').map(|q| {
            let after = &rest[q + 1..];
            let end = after
                .find(|c: char| c == '#' || c.is_whitespace())
                .unwrap_or(after.len());
            &after[..end]
        });
        (id, query)
    } else if input.contains("://") {
        // 是一个链接但不含 `/s/` 段, 无法定位分享 ID。
        return None;
    } else {
        // 视为裸 ID。
        (input.to_string(), None)
    };

    if !is_valid_share_id(&id_candidate) {
        return None;
    }
    let pass_code = query.and_then(pass_code_from_query);
    Some((id_candidate, pass_code))
}

/// share_id 只由字母、数字、`-`、`_` 组成且非空。
fn is_valid_share_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// 从 URL 查询串里取出提取码 (`password` / `pass_code` / `passcode`)。
fn pass_code_from_query(query: &str) -> Option<String> {
    for pair in query.split('&') {
        let mut kv = pair.splitn(2, '=');
        let key = kv.next().unwrap_or("");
        if matches!(key, "password" | "pass_code" | "passcode") {
            if let Some(v) = kv.next() {
                let v = v.trim();
                if !v.is_empty() {
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_share_id_from_url_forms() {
        let cases = [
            ("https://mypikpak.com/s/VO8B-abc", Some(("VO8B-abc", None))),
            (
                "  https://mypikpak.com/s/VO8B-abc?password=abcd  ",
                Some(("VO8B-abc", Some("abcd"))),
            ),
            (
                "https://mypikpak.com/s/VO8B-abc#frag",
                Some(("VO8B-abc", None)),
            ),
            // 复制链接时附带的前后缀文字。
            (
                "打开链接 https://mypikpak.com/s/AbC_123 查看",
                Some(("AbC_123", None)),
            ),
            ("xyz-1", Some(("xyz-1", None))),
            // 无法识别的形态。
            ("https://example.com/download/abc", None),
            ("https://mypikpak.com/s/", None),
            ("not a valid id!", None),
            ("", None),
        ];
        for (input, want) in cases {
            let got = parse_share_input(input);
            let got = got.as_ref().map(|(id, p)| (id.as_str(), p.as_deref()));
            assert_eq!(got, want, "input: {input:?}");
        }
    }
}
