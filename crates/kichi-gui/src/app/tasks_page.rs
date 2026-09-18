use eframe::egui::{self, vec2, Align, Frame, Key, Layout, Margin, RichText, Stroke};

use kichi_core::types::{task_file_id, task_id, task_name, task_size};

use crate::format;
use crate::icons::{self, Glyph};
use crate::msg::Cmd;
use crate::theme::Theme;

use super::helpers::input;
use super::App;

impl App {
    pub(super) fn tasks_page(&mut self, ctx: &egui::Context, th: &Theme) {
        let mut create = false;
        let mut do_refresh = false;
        let mut download_target: Option<(String, String)> = None;
        let mut retry: Option<String> = None;
        let mut del: Option<String> = None;
        let mut clear_all: Vec<Vec<String>> = Vec::new();

        egui::CentralPanel::default()
            .frame(Frame::new().fill(th.bg).inner_margin(Margin {
                left: 20,
                right: 20,
                top: 16,
                bottom: 12,
            }))
            .show(ctx, |ui| {
                // 标题
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(22.0, 22.0), egui::Sense::hover());
                    icons::paint(ui.painter(), r, Glyph::Transfer, th.accent);
                    ui.label(RichText::new("离线下载").size(19.0).strong().color(th.text));
                });
                ui.add_space(4.0);
                ui.label(
                    RichText::new("将磁力 / 直链先转存到云端, 完成后在「我的文件」中查看。")
                        .color(th.text_weak)
                        .size(12.5),
                );
                ui.add_space(14.0);

                // 新建离线下载卡片
                egui::Frame::new()
                    .fill(th.card)
                    .stroke(Stroke::new(1.0, th.border))
                    .corner_radius(th.cr(14))
                    .inner_margin(Margin::same(16))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new("新建离线下载")
                                .size(14.0)
                                .strong()
                                .color(th.text),
                        );
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("链接 / 磁力").color(th.text_weak));
                            let resp = ui.add(
                                input(&mut self.offline_url)
                                    .desired_width(f32::INFINITY)
                                    .hint_text("magnet:?xt=... 或 https://..."),
                            );
                            if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                                create = true;
                            }
                        });
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("文件名").color(th.text_weak));
                            ui.add(
                                input(&mut self.offline_name)
                                    .desired_width(280.0)
                                    .hint_text("可选, 留空自动识别"),
                            );
                            ui.add_space(8.0);
                            let enabled = !self.offline_url.trim().is_empty();
                            if ui
                                .add_enabled(
                                    enabled,
                                    egui::Button::new(
                                        RichText::new("提交下载").color(th.on_accent),
                                    )
                                    .fill(th.accent)
                                    .stroke(Stroke::NONE)
                                    .corner_radius(th.cr(8)),
                                )
                                .clicked()
                            {
                                create = true;
                            }
                        });
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("保存到").color(th.text_weak));
                            let label = match &self.offline_dest {
                                Some((_, name)) => format!("网盘目录 · {name}"),
                                None => "离线默认目录".to_string(),
                            };
                            if ui.button(RichText::new(label).color(th.accent)).clicked() {
                                self.open_offline_picker();
                            }
                        });
                    });

                ui.add_space(14.0);

                // 任务列表区
                let scroll_h = (ui.available_height() - 36.0).max(80.0);
                egui::ScrollArea::vertical()
                    .id_salt("tasks_scroll")
                    .auto_shrink([false, false])
                    .max_height(scroll_h)
                    .show(ui, |ui| {
                        if self.tasks_loading && self.buckets.is_empty() {
                            ui.vertical_centered(|ui| {
                                ui.add_space(30.0);
                                ui.spinner();
                                ui.add_space(6.0);
                                ui.label(RichText::new("正在获取任务…").color(th.text_weak));
                            });
                            return;
                        }

                        // 任务分组列表
                        egui::Frame::new()
                            .fill(th.card)
                            .stroke(Stroke::new(1.0, th.border))
                            .corner_radius(th.cr(14))
                            .inner_margin(Margin::symmetric(14, 8))
                            .show(ui, |ui| {
                                let mut any = false;
                                for phase in format::PHASE_ORDER {
                                    let Some(tasks) = self.buckets.get(phase) else {
                                        continue;
                                    };
                                    if tasks.is_empty() {
                                        continue;
                                    }
                                    any = true;
                                    let color = match phase {
                                        "PHASE_TYPE_COMPLETE" => th.ok,
                                        "PHASE_TYPE_ERROR" => th.danger,
                                        "PHASE_TYPE_RUNNING" => {
                                            egui::Color32::from_rgb(96, 146, 235)
                                        }
                                        _ => th.warn,
                                    };
                                    let header =
                                        format!("{} ({})", format::phase_label(phase), tasks.len());
                                    egui::CollapsingHeader::new(
                                        RichText::new(header).color(color).strong().size(13.5),
                                    )
                                    .default_open(phase != "PHASE_TYPE_COMPLETE")
                                    .show(ui, |ui| {
                                        for t in tasks {
                                            let name =
                                                task_name(t).unwrap_or_else(|| "未知任务".into());
                                            let id = task_id(t).unwrap_or_default();
                                            let size = task_size(t).unwrap_or(0);
                                            let file_id = task_file_id(t).unwrap_or_default();
                                            ui.horizontal(|ui| {
                                                ui.add_space(2.0);
                                                ui.label(
                                                    RichText::new(&name).size(13.5).color(th.text),
                                                );
                                                if size > 0 {
                                                    ui.label(
                                                        RichText::new(format::fmt_bytes(size))
                                                            .color(th.text_faint)
                                                            .size(12.0),
                                                    );
                                                }
                                                ui.with_layout(
                                                    Layout::right_to_left(Align::Center),
                                                    |ui| {
                                                        if !file_id.is_empty()
                                                            && phase == "PHASE_TYPE_COMPLETE"
                                                            && ui
                                                                .add(
                                                                    egui::Button::new(
                                                                        RichText::new("下载")
                                                                            .color(th.accent),
                                                                    )
                                                                    .fill(th.accent_soft())
                                                                    .stroke(Stroke::NONE)
                                                                    .corner_radius(th.cr(7)),
                                                                )
                                                                .clicked()
                                                        {
                                                            download_target = Some((
                                                                file_id.clone(),
                                                                name.clone(),
                                                            ));
                                                        }
                                                        if !id.is_empty()
                                                            && ui
                                                                .button(
                                                                    RichText::new("删除")
                                                                        .color(th.text_weak),
                                                                )
                                                                .clicked()
                                                        {
                                                            del = Some(id.clone());
                                                        }
                                                        if phase == "PHASE_TYPE_ERROR"
                                                            && !id.is_empty()
                                                            && ui
                                                                .button(
                                                                    RichText::new("重试")
                                                                        .color(th.warn),
                                                                )
                                                                .clicked()
                                                        {
                                                            retry = Some(id.clone());
                                                        }
                                                    },
                                                );
                                            });
                                            ui.add_space(2.0);
                                            ui.separator();
                                        }
                                        if phase != "PHASE_TYPE_RUNNING" {
                                            let ids: Vec<String> =
                                                tasks.iter().filter_map(task_id).collect();
                                            if !ids.is_empty() {
                                                ui.add_space(2.0);
                                                ui.horizontal(|ui| {
                                                    ui.add_space(4.0);
                                                    if ui
                                                        .button(
                                                            RichText::new(
                                                                "清空本组(仅移除任务记录)",
                                                            )
                                                            .color(th.text_faint)
                                                            .size(12.0),
                                                        )
                                                        .clicked()
                                                    {
                                                        clear_all.push(ids);
                                                    }
                                                });
                                                ui.add_space(2.0);
                                            }
                                        }
                                    });
                                }
                                if !any && self.buckets.values().all(|v| v.is_empty()) {
                                    ui.centered_and_justified(|ui| {
                                        ui.add_space(30.0);
                                        ui.label(RichText::new("暂无离线任务").color(th.text_weak));
                                        ui.add_space(30.0);
                                    });
                                }
                            });
                    });

                // 刷新按钮 (固定在滚动区下方, 始终可见可点)
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let refreshing = self.tasks_refreshing;
                    let (r, ico) = ui.allocate_exact_size(vec2(22.0, 22.0), egui::Sense::click());
                    icons::paint(ui.painter(), r, Glyph::Refresh, th.text_weak);
                    let ico_clicked = !refreshing && ico.clicked();
                    let btn = ui.add_enabled(
                        !refreshing,
                        egui::Button::new(
                            RichText::new(if refreshing {
                                "正在刷新…"
                            } else {
                                "刷新任务"
                            })
                            .color(th.text_weak),
                        )
                        .frame(false),
                    );
                    if btn.clicked() || ico_clicked {
                        do_refresh = true;
                    }
                    if refreshing {
                        ui.add(egui::Spinner::new().size(14.0).color(th.text_weak));
                    }
                });
            });

        if create {
            let url = self.offline_url.trim().to_string();
            let name = {
                let n = self.offline_name.trim();
                if n.is_empty() {
                    None
                } else {
                    Some(n.to_string())
                }
            };
            let parent = self.offline_dest.as_ref().map(|(id, _)| id.clone());
            self.send(Cmd::OfflineCreate { url, name, parent });
        }
        if do_refresh {
            self.tasks_refreshing = true;
            self.send(Cmd::RefreshTasks);
        }
        if let Some((fid, fname)) = download_target {
            self.download_single(fid, fname);
        }
        if let Some(tid) = retry {
            let tx = self.tx.clone();
            let _ = tx.send(Cmd::OfflineRetry { task_id: tid });
        }
        if let Some(tid) = del {
            let tx = self.tx.clone();
            let _ = tx.send(Cmd::OfflineDelete {
                task_ids: vec![tid],
                delete_files: false,
            });
        }
        for ids in clear_all {
            let tx = self.tx.clone();
            let _ = tx.send(Cmd::OfflineDelete {
                task_ids: ids,
                delete_files: false,
            });
        }
    }
}
