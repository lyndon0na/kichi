//! 列表行的绘制与右键菜单: `file_row` + 预览 / 播放子菜单。
//!
//! 行内的所有用户操作都只落进 `RowAction`, 由入口统一转成 `Cmd` 或状态变更。

use eframe::egui::{self, vec2, FontId, Pos2, Rect, RichText, Stroke};

use kichi_core::types::File;

use crate::filetypes::{self, file_visual, FileType, PreviewKind};
use crate::format;
use crate::icons;
use crate::theme::{mix, Theme};

use super::super::helpers::truncate_text;
use super::super::preview::QualityMenuState;
use super::RowAction;

/// 「播放」子菜单: 原画直达 + 已解析出的清晰度。
/// 子菜单打开时会请求(若尚未缓存)该文件的清晰度列表。
pub(crate) fn play_menu(
    ui: &mut egui::Ui,
    id: &str,
    name: &str,
    state: QualityMenuState<'_>,
    actions: &mut Vec<RowAction>,
) {
    actions.push(RowAction::FetchQualities(id.to_string(), name.to_string()));
    ui.menu_button("播放", |ui| match state {
        QualityMenuState::Loading => {
            if ui.button("原画（直接播放）").clicked() {
                actions.push(RowAction::OpenFile(id.to_string(), name.to_string()));
                ui.close_menu();
            }
            ui.add_enabled(false, egui::Button::new("加载清晰度…"));
        }
        QualityMenuState::Ready(r) if !r.options.is_empty() => {
            for opt in &r.options {
                if ui.button(&opt.label).clicked() {
                    actions.push(RowAction::PlayOption(id.to_string(), opt.clone()));
                    ui.close_menu();
                }
            }
        }
        QualityMenuState::Ready(_) => {
            if ui.button("原画（直接播放）").clicked() {
                actions.push(RowAction::OpenFile(id.to_string(), name.to_string()));
                ui.close_menu();
            }
            ui.add_enabled(false, egui::Button::new("无可用清晰度"));
        }
    });
}

/// 预览入口: 视频给「播放」子菜单(含清晰度), 音频给「播放」, 其余给「打开」;
/// 压缩包 / 镜像 / 可执行 / 种子不给入口, 只留「下载到本地」。
pub(crate) fn preview_menu_items(
    ui: &mut egui::Ui,
    f: &File,
    quality: QualityMenuState<'_>,
    actions: &mut Vec<RowAction>,
) {
    let ft = filetypes::classify_file(f);
    match filetypes::preview_kind(ft) {
        PreviewKind::DownloadOnly => {}
        PreviewKind::Play if ft == FileType::Video => {
            play_menu(ui, &f.id, &f.name, quality, actions);
        }
        kind => {
            let label = if kind == PreviewKind::Play {
                "播放"
            } else {
                "打开"
            };
            if ui.button(label).clicked() {
                actions.push(RowAction::OpenFile(f.id.clone(), f.name.clone()));
                ui.close_menu();
            }
        }
    }
}

/// 列表行; 返回勾选变更的 id(仅点击复选框时)。
#[allow(clippy::too_many_arguments)]
pub(crate) fn file_row(
    ui: &mut egui::Ui,
    th: &Theme,
    f: &File,
    is_sel: bool,
    even: bool,
    has_clip: bool,
    aria2: bool,
    quality: QualityMenuState<'_>,
    actions: &mut Vec<RowAction>,
    name_x: f32,
    size_left: f32,
    time_left: f32,
) -> Option<String> {
    let row_h = 40.0;
    let w = ui.available_width().max(320.0);
    let (rect, row_resp) = ui.allocate_exact_size(vec2(w, row_h), egui::Sense::click());
    let painter = ui.painter().clone();

    let hovered = row_resp.hovered();

    let bg = if is_sel {
        if th.breeze {
            th.accent
        } else {
            mix(th.card, th.accent, if th.dark { 0.22 } else { 0.12 })
        }
    } else if hovered {
        mix(th.card, th.text, if th.dark { 0.07 } else { 0.045 })
    } else if even && !th.breeze {
        mix(th.card, th.text, if th.dark { 0.018 } else { 0.012 })
    } else {
        egui::Color32::TRANSPARENT
    };
    painter.rect_filled(rect.shrink2(vec2(2.0, 2.0)), th.cr(8), bg);
    if is_sel && !th.breeze {
        painter.rect_filled(
            Rect::from_min_max(
                Pos2::new(rect.min.x + 3.0, rect.min.y + 7.0),
                Pos2::new(rect.min.x + 5.0, rect.max.y - 7.0),
            ),
            th.cr(2),
            th.accent,
        );
    }

    // 复选框
    let cb_size = 16.0;
    let cb_x = rect.min.x + 8.0;
    let cb_y = rect.center().y - cb_size / 2.0;
    let cb_rect = Rect::from_min_max(
        Pos2::new(cb_x, cb_y),
        Pos2::new(cb_x + cb_size, cb_y + cb_size),
    );
    let cb_resp = ui.interact(
        cb_rect,
        ui.id().with(("cb", f.id.clone())),
        egui::Sense::click(),
    );

    // 复选框背景
    let cb_on_accent = is_sel && th.breeze;
    let cb_bg = if cb_on_accent {
        th.on_accent
    } else if is_sel {
        th.accent
    } else {
        egui::Color32::TRANSPARENT
    };
    let cb_stroke = if is_sel {
        Stroke::NONE
    } else {
        Stroke::new(1.5_f32, th.text_faint)
    };
    painter.rect_filled(cb_rect, th.cr(3), cb_bg);
    painter.rect_stroke(cb_rect, th.cr(3), cb_stroke, egui::StrokeKind::Inside);

    // 选中时绘制勾号
    if is_sel {
        let check_pts = [
            Pos2::new(cb_x + 3.5, cb_y + cb_size / 2.0),
            Pos2::new(cb_x + 6.5, cb_y + cb_size / 2.0 + 3.0),
            Pos2::new(cb_x + cb_size - 3.0, cb_y + 3.0),
        ];
        let tick = if cb_on_accent {
            th.accent
        } else {
            th.on_accent
        };
        painter.add(egui::Shape::line(
            check_pts.to_vec(),
            Stroke::new(2.0_f32, tick),
        ));
    }

    // 复选框悬停效果
    if cb_resp.hovered() {
        painter.rect_filled(cb_rect, th.cr(3), mix(th.accent, th.bg, 0.85));
    }

    let yc = rect.center().y;
    let (glyph, color) = file_visual(f);
    let icon_rect = Rect::from_center_size(Pos2::new(rect.min.x + 38.0, yc), vec2(22.0, 22.0));
    painter.rect_filled(
        icon_rect,
        th.cr(6),
        mix(th.card, color, if th.dark { 0.16 } else { 0.10 }),
    );
    icons::paint(&painter, icon_rect.shrink(2.5), glyph, color);

    let x0 = rect.min.x;
    let name_color = if is_sel && th.breeze {
        th.on_accent
    } else {
        th.text
    };
    let dim_color = if is_sel && th.breeze {
        mix(th.on_accent, th.accent, 0.25)
    } else {
        th.text_weak
    };

    let name_w = (size_left - 12.0 - name_x).max(24.0);
    let name_g = truncate_text(
        &painter,
        &f.name,
        name_w,
        FontId::proportional(14.0),
        name_color,
    );
    painter.galley(
        Pos2::new(x0 + name_x, yc - name_g.size().y / 2.0),
        name_g,
        name_color,
    );

    if !f.is_folder() {
        let g = painter.layout_no_wrap(
            format::fmt_bytes(f.size),
            FontId::proportional(12.5),
            dim_color,
        );
        painter.galley(
            Pos2::new(x0 + size_left, yc - g.size().y / 2.0),
            g,
            dim_color,
        );
    }
    let t = f
        .modified_time
        .as_deref()
        .or(f.created_time.as_deref())
        .unwrap_or("");
    let g = painter.layout_no_wrap(format::fmt_time(t), FontId::proportional(12.5), dim_color);
    painter.galley(
        Pos2::new(x0 + time_left, yc - g.size().y / 2.0),
        g,
        dim_color,
    );

    // 右键菜单
    let f_ctx = f.clone();
    let dbl = row_resp.double_clicked();
    let _menu = row_resp.context_menu(|ui| {
        if f_ctx.is_folder() {
            if ui.button("打开").clicked() {
                actions.push(RowAction::OpenFolder(f_ctx.id.clone(), f_ctx.name.clone()));
                ui.close_menu();
            }
            if ui.button("下载到本地…").clicked() {
                actions.push(RowAction::DownloadFolder(
                    f_ctx.id.clone(),
                    f_ctx.name.clone(),
                ));
                ui.close_menu();
            }
            if aria2 && ui.button("发送到 aria2").clicked() {
                actions.push(RowAction::Aria2Folder(f_ctx.id.clone(), f_ctx.name.clone()));
                ui.close_menu();
            }
        } else {
            if ui.button("下载到本地…").clicked() {
                actions.push(RowAction::DownloadFile(
                    f_ctx.id.clone(),
                    f_ctx.name.clone(),
                ));
                ui.close_menu();
            }
            if aria2 && ui.button("发送到 aria2").clicked() {
                actions.push(RowAction::Aria2File(f_ctx.id.clone(), f_ctx.name.clone()));
                ui.close_menu();
            }
            preview_menu_items(ui, &f_ctx, quality, actions);
        }
        ui.separator();
        if ui.button("分享").clicked() {
            actions.push(RowAction::Share(f_ctx.id.clone()));
            ui.close_menu();
        }
        if ui.button("复制").clicked() {
            actions.push(RowAction::CopyItem(f_ctx.id.clone()));
            ui.close_menu();
        }
        if ui.button("剪切").clicked() {
            actions.push(RowAction::CutItem(f_ctx.id.clone()));
            ui.close_menu();
        }
        if has_clip && f_ctx.is_folder() && ui.button("粘贴到此处").clicked() {
            actions.push(RowAction::PasteInto(f_ctx.id.clone()));
            ui.close_menu();
        }
        ui.separator();
        if ui.button("重命名").clicked() {
            actions.push(RowAction::Rename(f_ctx.id.clone(), f_ctx.name.clone()));
            ui.close_menu();
        }
        if ui.button("复制名称").clicked() {
            actions.push(RowAction::CopyName(f_ctx.name.clone()));
            ui.close_menu();
        }
        ui.separator();
        if ui
            .button(RichText::new("移入回收站").color(th.danger))
            .clicked()
        {
            actions.push(RowAction::Trash(f_ctx.id.clone()));
            ui.close_menu();
        }
    });

    let mut sel: Option<String> = None;
    // 只有点击复选框才会勾选/取消; 单击行本身不改变选择。
    if cb_resp.clicked() {
        sel = Some(f.id.clone());
    } else if dbl {
        if f.is_folder() {
            actions.push(RowAction::OpenFolder(f.id.clone(), f.name.clone()));
        } else {
            actions.push(RowAction::OpenFile(f.id.clone(), f.name.clone()));
        }
    }
    sel
}
