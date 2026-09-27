//! 「保存到」网盘目录选择器: 状态操作 + 弹窗渲染。

use eframe::egui::{self, Align2, Color32, RichText, Stroke};

use kichi_core::types::File;

use crate::msg::Cmd;
use crate::theme::Theme;

use super::super::files::Crumb;
use super::super::global::Global;
use super::super::helpers::folder_row;
use super::TasksPage;

impl TasksPage {
    /// 选择器当前所在目录。
    fn picker_parent(&self) -> Option<String> {
        self.picker_stack.last().and_then(|c| c.id.clone())
    }

    /// 打开「保存到」网盘目录选择器, 从根目录开始。
    pub(super) fn open_picker(&mut self, g: &mut Global) {
        self.picker_open = true;
        self.picker_stack = vec![Crumb {
            id: None,
            label: "我的云盘".into(),
        }];
        self.picker_list(g);
    }

    /// 请求选择器当前目录的子文件夹列表。
    fn picker_list(&mut self, g: &mut Global) {
        self.picker_loading = true;
        self.picker_folders.clear();
        self.picker_req += 1;
        let req_id = self.picker_req;
        g.send(Cmd::ListFolders {
            parent: self.picker_parent(),
            req_id,
        });
    }

    /// 该 req_id 是否属于「保存到」选择器。
    pub(crate) fn handles_picker(&self, req_id: u64) -> bool {
        self.picker_open && req_id == self.picker_req
    }

    /// 目录选择器响应(父目录已不是当前所在目录时丢弃)。
    pub(crate) fn on_folders(&mut self, parent: Option<String>, files: Vec<File>) {
        if parent != self.picker_parent() {
            return;
        }
        self.picker_loading = false;
        self.picker_folders = files;
    }

    /// 「选择保存到（网盘目录）」弹窗。
    pub(crate) fn draw_offline_picker(&mut self, ctx: &egui::Context, g: &mut Global, th: &Theme) {
        if !self.picker_open {
            return;
        }
        let crumbs = self.picker_stack.clone();
        let folders = self.picker_folders.clone();
        let loading = self.picker_loading;
        let cur_dest = self.dest.clone();
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
            self.picker_stack.truncate(i + 1);
            self.picker_list(g);
        }
        if let Some((id, name)) = enter {
            self.picker_stack.push(Crumb {
                id: Some(id),
                label: name,
            });
            self.picker_list(g);
        }
        if confirm {
            self.dest = self
                .picker_stack
                .last()
                .and_then(|c| c.id.clone().map(|id| (id, c.label.clone())));
            self.picker_open = false;
        }
        if close {
            self.picker_open = false;
        }
    }
}
