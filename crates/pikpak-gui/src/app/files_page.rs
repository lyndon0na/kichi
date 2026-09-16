use eframe::egui::{self, Align, FontId, Frame, Key, Layout, Margin, Pos2, Rect, RichText, Stroke, vec2, UiBuilder};

use pikpak_core::types::File;

use crate::format;
use crate::icons::{self, Glyph};
use crate::theme::{mix, Theme};

use super::helpers::{is_media_file, is_video_file, truncate_text};
use super::types::{ClipKind, ColDrag, Crumb, QualityMenuState, RowAction, SortBy, ViewMode};
use super::App;

/// 文件类型 -> 图标 / 颜色。
pub(super) fn file_visual(f: &File) -> (Glyph, egui::Color32) {
    let light_gray = egui::Color32::from_rgb(120, 126, 140);
    if f.is_folder() {
        return (Glyph::Folder, egui::Color32::from_rgb(232, 178, 84));
    }
    let ext = f
        .name
        .rsplit_once('.')
        .map(|(_, e): (&str, &str)| e.to_lowercase())
        .unwrap_or_default();
    let videos = [
        "mp4", "mkv", "avi", "mov", "wmv", "flv", "webm", "ts", "rmvb", "m4v",
    ];
    let audio = ["mp3", "flac", "wav", "aac", "ogg", "m4a", "opus", "ape"];
    let images = [
        "jpg", "jpeg", "png", "gif", "bmp", "webp", "heic", "svg", "tiff",
    ];
    let docs = [
        "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "txt", "md", "epub", "csv",
    ];
    let arch = ["zip", "rar", "7z", "tar", "gz", "bz2", "xz", "iso"];
    if videos.contains(&ext.as_str()) {
        (Glyph::Video, egui::Color32::from_rgb(196, 130, 220))
    } else if audio.contains(&ext.as_str()) {
        (Glyph::Audio, egui::Color32::from_rgb(104, 196, 136))
    } else if images.contains(&ext.as_str()) {
        (Glyph::Image, egui::Color32::from_rgb(96, 184, 200))
    } else if docs.contains(&ext.as_str()) {
        (Glyph::Doc, egui::Color32::from_rgb(214, 178, 96))
    } else if arch.contains(&ext.as_str()) {
        (Glyph::Archive, egui::Color32::from_rgb(208, 142, 110))
    } else {
        (Glyph::File, light_gray)
    }
}

/// 布局坐标。返回 (name_x, size_left, time_left)。
/// 名称列占据剩余空间; size_w / time_w 从 App 状态读取。
impl App {
    const NAME_X: f32 = 54.0;  // 复选框(8+16=24) + 图标(38+11=49) + 间距
    const RIGHT_PAD: f32 = 8.0;
    const MIN_SIZE_W: f32 = 60.0;
    const MIN_TIME_W: f32 = 80.0;
    /// 标题栏统一行高: 选中态与普通态使用同一高度, 避免切换时列表位移。
    const HEAD_H: f32 = 44.0;

    fn col_layout(&self, w: f32) -> (f32, f32, f32) {
        let time_left = (w - self.col_time_w - Self::RIGHT_PAD).max(Self::NAME_X + 60.0);
        let size_left = (time_left - self.col_size_w - 16.0).max(Self::NAME_X + 60.0);
        (Self::NAME_X, size_left, time_left)
    }
}

/// 面包屑导航: 宽度不足时从左侧省略中间层级, 始终保留当前目录(必要时截断)。
/// 返回被点击的层级索引。
fn breadcrumbs(ui: &mut egui::Ui, th: &Theme, crumbs: &[Crumb], budget: f32) -> Option<usize> {
    if crumbs.is_empty() {
        return None;
    }
    let painter = ui.painter().clone();
    let font = FontId::proportional(15.0);
    let pad = 8.0f32;
    let sep = 16.0f32;
    let row_h = 26.0f32;
    let n = crumbs.len();

    let mut galleys: Vec<_> = crumbs
        .iter()
        .map(|c| painter.layout_no_wrap(c.label.clone(), font.clone(), th.text))
        .collect();
    let mut widths: Vec<f32> = galleys.iter().map(|g| g.size().x + pad * 2.0).collect();

    // 从最后一项往前塞, 放不下就省略更早的层级。
    let ell_w = painter
        .layout_no_wrap("…".to_string(), font.clone(), th.text_faint)
        .size()
        .x
        + pad * 2.0;
    let mut start = n - 1;
    let mut used = widths[start];
    while start > 0 {
        let cand = widths[start - 1] + sep;
        let extra_ell = if start - 1 > 0 { ell_w + sep } else { 0.0 };
        if used + cand + extra_ell <= budget {
            used += cand;
            start -= 1;
        } else {
            break;
        }
    }

    // 当前目录仍放不下时单独截断。
    if start == n - 1 && widths[n - 1] > budget {
        let w = (budget - pad * 2.0).max(24.0);
        galleys[n - 1] = truncate_text(&painter, &crumbs[n - 1].label, w, font.clone(), th.text);
        widths[n - 1] = galleys[n - 1].size().x + pad * 2.0;
    }

    let mut clicked = None;
    if start > 0 {
        let (er, _) = ui.allocate_exact_size(vec2(ell_w, row_h), egui::Sense::hover());
        let ec = er.center();
        for dx in [-4.0f32, 0.0, 4.0] {
            painter.circle_filled(Pos2::new(ec.x + dx, ec.y), 1.4, th.text_faint);
        }
        let (sr, _) = ui.allocate_exact_size(vec2(sep, row_h), egui::Sense::hover());
        icons::paint(&painter, Rect::from_center_size(sr.center(), vec2(11.0, 11.0)), Glyph::ChevronRight, th.text_faint);
    }
    for i in start..n {
        if i > start {
            let (sr, _) = ui.allocate_exact_size(vec2(sep, row_h), egui::Sense::hover());
            icons::paint(&painter, Rect::from_center_size(sr.center(), vec2(11.0, 11.0)), Glyph::ChevronRight, th.text_faint);
        }
        let last = i == n - 1;
        let (rect, resp) = ui.allocate_exact_size(vec2(widths[i], row_h), egui::Sense::click());
        if !last && resp.hovered() {
            painter.rect_filled(rect, th.cr(6), th.hover);
        }
        let color = if last { th.text } else { th.text_weak };
        let g = galleys[i].clone();
        painter.galley(
            Pos2::new(rect.center().x - g.size().x / 2.0, rect.center().y - g.size().y / 2.0),
            g,
            color,
        );
        if !last && resp.clicked() {
            clicked = Some(i);
        }
    }
    clicked
}

/// 「播放」子菜单: 原画直达 + 已解析出的清晰度。
/// 子菜单打开时会请求(若尚未缓存)该文件的清晰度列表。
fn play_menu(
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

/// 列表行; 返回勾选变更的 id(仅点击复选框时)。
#[allow(clippy::too_many_arguments)]
fn file_row(
    ui: &mut egui::Ui,
    th: &Theme,
    f: &File,
    is_sel: bool,
    even: bool,
    has_clip: bool,
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
    let cb_rect = Rect::from_min_max(Pos2::new(cb_x, cb_y), Pos2::new(cb_x + cb_size, cb_y + cb_size));
    let cb_resp = ui.interact(cb_rect, ui.id().with(("cb", f.id.clone())), egui::Sense::click());

    // 复选框背景
    let cb_on_accent = is_sel && th.breeze;
    let cb_bg = if cb_on_accent {
        th.on_accent
    } else if is_sel {
        th.accent
    } else {
        egui::Color32::TRANSPARENT
    };
    let cb_stroke = if is_sel { Stroke::NONE } else { Stroke::new(1.5, th.text_faint) };
    painter.rect_filled(cb_rect, th.cr(3), cb_bg);
    painter.rect_stroke(cb_rect, th.cr(3), cb_stroke, egui::StrokeKind::Inside);

    // 选中时绘制勾号
    if is_sel {
        let check_pts = [
            Pos2::new(cb_x + 3.5, cb_y + cb_size / 2.0),
            Pos2::new(cb_x + 6.5, cb_y + cb_size / 2.0 + 3.0),
            Pos2::new(cb_x + cb_size - 3.0, cb_y + 3.0),
        ];
        let tick = if cb_on_accent { th.accent } else { th.on_accent };
        painter.add(egui::Shape::line(check_pts.to_vec(), Stroke::new(2.0, tick)));
    }

    // 复选框悬停效果
    if cb_resp.hovered() {
        painter.rect_filled(cb_rect, th.cr(3), mix(th.accent, th.bg, 0.85));
    }

    let yc = rect.center().y;
    let (glyph, color) = file_visual(f);
    let icon_rect = Rect::from_center_size(Pos2::new(rect.min.x + 38.0, yc), vec2(22.0, 22.0));
    painter.rect_filled(icon_rect, th.cr(6), mix(th.card, color, if th.dark { 0.16 } else { 0.10 }));
    icons::paint(&painter, icon_rect.shrink(2.5), glyph, color);

    let x0 = rect.min.x;
    let name_color = if is_sel && th.breeze { th.on_accent } else { th.text };
    let dim_color = if is_sel && th.breeze {
        mix(th.on_accent, th.accent, 0.25)
    } else {
        th.text_weak
    };

    let name_w = (size_left - 12.0 - name_x).max(24.0);
    let name_g = truncate_text(&painter, &f.name, name_w, FontId::proportional(14.0), name_color);
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
    let g = painter.layout_no_wrap(
        format::fmt_time(t),
        FontId::proportional(12.5),
        dim_color,
    );
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
        } else {
            if ui.button("下载到本地…").clicked() {
                actions.push(RowAction::DownloadFile(f_ctx.id.clone(), f_ctx.name.clone()));
                ui.close_menu();
            }
            if is_video_file(&f_ctx.name) {
                play_menu(ui, &f_ctx.id, &f_ctx.name, quality, actions);
            } else {
                let label = if is_media_file(&f_ctx.name) { "播放" } else { "打开" };
                if ui.button(label).clicked() {
                    actions.push(RowAction::OpenFile(f_ctx.id.clone(), f_ctx.name.clone()));
                    ui.close_menu();
                }
            }
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

impl App {
    pub(super) fn files_page(&mut self, ctx: &egui::Context, th: &Theme) {
        // 处理列拖拽 (在渲染之前, 确保 header 和 rows 看到一致的列宽)
        if let Some(drag) = &self.col_dragging {
            let delta = ctx.input(|i| i.pointer.hover_pos().map(|p| p.x).unwrap_or(drag.start_x) - drag.start_x);
            match drag.handle {
                1 => {
                    self.col_size_w = (drag.orig_size_w - delta).max(Self::MIN_SIZE_W);
                }
                2 => {
                    self.col_time_w = (drag.orig_time_w - delta).max(Self::MIN_TIME_W);
                }
                _ => {}
            }
            if ctx.input(|i| i.pointer.any_released()) {
                self.col_dragging = None;
            }
            ctx.request_repaint();
        }

        // 键盘快捷键
        if !ctx.wants_keyboard_input() {
            ctx.input(|i| {
                if i.key_pressed(Key::F5) {
                    self.refresh_dir();
                }
                if i.key_pressed(Key::A) && i.modifiers.ctrl {
                    let (folders, plain) = self.visible_rows();
                    self.selected = folders
                        .iter()
                        .chain(plain.iter())
                        .map(|f| f.id.clone())
                        .collect();
                }
                if (i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace))
                    && !self.selected.is_empty()
                {
                    let sel = self.selected_names();
                    if !sel.is_empty() {
                        self.trash_confirm = Some(sel);
                    }
                }
                if i.key_pressed(Key::C) && i.modifiers.ctrl {
                    self.clip_selection(ClipKind::Copy);
                }
                if i.key_pressed(Key::X) && i.modifiers.ctrl {
                    self.clip_selection(ClipKind::Cut);
                }
                if i.key_pressed(Key::V) && i.modifiers.ctrl {
                    self.paste_clipboard();
                }
                if i.key_pressed(Key::Escape) && !self.selected.is_empty() {
                    self.selected.clear();
                }
            });
        }

        let mut up = false;
        let mut jumped: Option<usize> = None;
        let mut mkdir = false;
        let mut upload = false;
        let mut upload_dir = false;
        let mut refresh = false;
        let mut want_download = false;
        let mut clear_clip = false;
        let mut ask_rename = false;
        let mut ask_preview = false;
        let mut ask_trash = false;
        let mut ask_share = false;
        // 各菜单/操作栏产生的行级操作, 统一在最后处理。
        let mut actions: Vec<RowAction> = Vec::new();

        // 当前可见行统计(过滤后)与选中项信息, 供顶部栏与选中栏使用。
        let (folders, plain) = self.visible_rows();
        let visible_total = folders.len() + plain.len();
        let sel_meta = self.selected_names();
        let dl_candidates = self.selected_plain_files();
        let dl_has_folder = self.selected_has_folder();
        let clip_info = self
            .clipboard
            .as_ref()
            .map(|c| (c.label.clone(), c.ids.len()));

        // -------- 顶部: 面包屑 + 搜索 + 操作 --------
        egui::TopBottomPanel::top("file_head")
            .frame(Frame::new().fill(th.bg).inner_margin(Margin { left: 20, right: 20, top: 14, bottom: 10 }))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.set_min_height(Self::HEAD_H);

                    // 左侧区域限制在右侧控件之外, 面包屑/计数超长时截断, 避免溢出重叠。
                    let right_reserve = 360.0;
                    let left_w = (ui.available_width() - right_reserve).max(140.0);
                    ui.allocate_ui_with_layout(
                        vec2(left_w, Self::HEAD_H),
                        Layout::left_to_right(Align::Center),
                        |ui| {
                            let at_root = self.stack.len() <= 1;
                            let (r, rresp) = ui.allocate_exact_size(vec2(30.0, 30.0), egui::Sense::click());
                            ui.painter().rect_filled(r, th.cr(8), if rresp.hovered() && !at_root { th.hover } else { egui::Color32::TRANSPARENT });
                            icons::paint(ui.painter(), r.shrink(6.0), Glyph::Up, if at_root { th.text_faint } else { th.text_weak });
                            if rresp.clicked() && !at_root {
                                up = true;
                            }
                            rresp.clone().on_hover_text("返回上级");
                            ui.add_space(4.0);

                            let count_reserve = 76.0;
                            let clip_reserve = if clip_info.is_some() { 150.0 } else { 0.0 };
                            let crumbs_budget = (ui.available_width() - count_reserve - clip_reserve).max(48.0);
                            if let Some(i) = breadcrumbs(ui, th, &self.stack, crumbs_budget) {
                                jumped = Some(i);
                            }

                            if sel_meta.is_empty() {
                                ui.label(RichText::new(format!("· {visible_total} 项")).color(th.text_faint).size(12.5));
                            } else {
                                ui.label(
                                    RichText::new(format!("· 已选 {}/{} 项", sel_meta.len(), visible_total))
                                        .color(th.accent)
                                        .size(12.5),
                                );
                            }

                            // 剪贴板 chip(只显示数量, 避免文件名过长溢出)
                            if let Some((_label, n)) = &clip_info {
                                ui.add_space(8.0);
                                egui::Frame::new()
                                    .fill(th.accent_soft())
                                    .corner_radius(th.cr(7))
                                    .inner_margin(Margin::symmetric(8, 3))
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            ui.label(RichText::new(format!("剪贴板 · {n} 项")).color(th.accent).size(11.5));
                                            let (xr, xresp) = ui.allocate_exact_size(vec2(14.0, 14.0), egui::Sense::click());
                                            icons::paint(ui.painter(), xr.shrink(2.5), Glyph::Close, if xresp.hovered() { th.accent } else { th.text_faint });
                                            if xresp.clicked() {
                                                clear_clip = true;
                                            }
                                        });
                                    });
                            }
                        },
                    );

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if sel_meta.is_empty() {
                            // ---- 普通模式: 搜索 + 刷新 + 新建 + 视图 ----
                            let focused = ui.memory(|m| m.has_focus(egui::Id::new("file_search")));
                            egui::Frame::new()
                                .fill(th.card)
                                .stroke(Stroke::new(1.0, if focused { th.accent } else { th.border }))
                                .corner_radius(th.cr(9))
                                .inner_margin(Margin::symmetric(10, 5))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        let (ir, _) = ui.allocate_exact_size(vec2(15.0, 15.0), egui::Sense::hover());
                                        icons::paint(ui.painter(), ir, Glyph::Search, th.text_faint);
                                        ui.add_space(1.0);
                                        ui.add(
                                            egui::TextEdit::singleline(&mut self.filter)
                                                .id(egui::Id::new("file_search"))
                                                .frame(false)
                                                .desired_width(150.0)
                                                .hint_text("搜索当前目录")
                                                .font(FontId::proportional(13.5)),
                                        );
                                        if !self.filter.is_empty() {
                                            let (xr, xresp) = ui.allocate_exact_size(vec2(14.0, 14.0), egui::Sense::click());
                                            icons::paint(ui.painter(), xr.shrink(2.5), Glyph::Close, if xresp.hovered() { th.accent } else { th.text_faint });
                                            if xresp.clicked() {
                                                self.filter.clear();
                                            }
                                        }
                                    });
                                });
                            ui.add_space(6.0);

                            // 刷新
                            let (rr, rresp) = ui.allocate_exact_size(vec2(30.0, 30.0), egui::Sense::click());
                            ui.painter().rect_filled(rr, th.cr(8), if rresp.hovered() { th.hover } else { egui::Color32::TRANSPARENT });
                            icons::paint(ui.painter(), rr.shrink(7.0), Glyph::Refresh, th.text_weak);
                            if rresp.clicked() {
                                refresh = true;
                            }
                            rresp.on_hover_text("刷新 (F5)");

                            ui.add_space(4.0);
                            // 上传菜单(上传文件 / 上传文件夹)
                            ui.menu_button(
                                RichText::new("上传").size(13.0).color(th.text_weak),
                                |ui| {
                                    if ui.button("上传文件").clicked() {
                                        upload = true;
                                        ui.close_menu();
                                    }
                                    if ui.button("上传文件夹").clicked() {
                                        upload_dir = true;
                                        ui.close_menu();
                                    }
                                },
                            );

                            ui.add_space(4.0);
                            // 新建文件夹(矢量图标 + 悬浮提示)
                            let (pr, presp) =
                                ui.allocate_exact_size(vec2(30.0, 30.0), egui::Sense::click());
                            ui.painter().rect_filled(
                                pr,
                                th.cr(8),
                                if presp.hovered() {
                                    th.accent_soft()
                                } else {
                                    egui::Color32::TRANSPARENT
                                },
                            );
                            icons::paint(ui.painter(), pr.shrink(7.0), Glyph::Plus, th.accent);
                            if presp.clicked() {
                                mkdir = true;
                            }
                            presp.on_hover_text("新建文件夹");

                            ui.add_space(6.0);
                            // 视图切换
                            let icon_bg = |active: bool, hovered: bool| {
                                if active { th.accent_soft() } else if hovered { th.hover } else { egui::Color32::TRANSPARENT }
                            };
                            // 图标视图: 2x2 网格
                            let (ri, ri_resp) = ui.allocate_exact_size(vec2(28.0, 28.0), egui::Sense::click());
                            ui.painter().rect_filled(ri, th.cr(6), icon_bg(self.view_mode == ViewMode::Icon, ri_resp.hovered()));
                            let p = ui.painter();
                            let s = 3.0;
                            let gap = 2.0;
                            let cx = ri.center().x;
                            let cy = ri.center().y;
                            let color = if self.view_mode == ViewMode::Icon { th.accent } else { th.text_weak };
                            for dx in [-(s + gap / 2.0), s + gap / 2.0] {
                                for dy in [-(s + gap / 2.0), s + gap / 2.0] {
                                    p.rect_filled(
                                        Rect::from_center_size(Pos2::new(cx + dx, cy + dy), vec2(s, s)),
                                        th.cr(1),
                                        color,
                                    );
                                }
                            }
                            if ri_resp.clicked() { self.view_mode = ViewMode::Icon; }
                            ri_resp.on_hover_text("图标视图");

                            // 列表视图: 三条横线
                            let (li, li_resp) = ui.allocate_exact_size(vec2(28.0, 28.0), egui::Sense::click());
                            ui.painter().rect_filled(li, th.cr(6), icon_bg(self.view_mode == ViewMode::List, li_resp.hovered()));
                            let p = ui.painter();
                            let color = if self.view_mode == ViewMode::List { th.accent } else { th.text_weak };
                            let lx = li.center().x - 5.0;
                            let lw = 10.0;
                            for dy in [-3.5, 0.0, 3.5] {
                                p.line_segment(
                                    [Pos2::new(lx, li.center().y + dy), Pos2::new(lx + lw, li.center().y + dy)],
                                    Stroke::new(1.5, color),
                                );
                            }
                            if li_resp.clicked() { self.view_mode = ViewMode::List; }
                            li_resp.on_hover_text("列表视图");
                        } else {
                            // ---- 选中模式: 就地显示操作, 不新增行/不改变列表位置 ----
                            let single = sel_meta.len() == 1;
                            let show_open = single && !dl_candidates.is_empty();

                            // 取消(最右)
                            if ui
                                .add(egui::Button::new(RichText::new("取消").color(th.text_weak)))
                                .clicked()
                            {
                                self.selected.clear();
                            }
                            if ui
                                .add(
                                    egui::Button::new(RichText::new("移入回收站").color(th.danger))
                                        .stroke(Stroke::new(1.0, mix(th.danger, th.bg, 0.35)))
                                        .fill(egui::Color32::TRANSPARENT)
                                        .corner_radius(th.cr(8)),
                                )
                                .clicked()
                            {
                                ask_trash = true;
                            }
                            if self.clipboard.is_some()
                                && ui
                                    .add(egui::Button::new(RichText::new("粘贴").color(th.text_weak)))
                                    .clicked()
                            {
                                self.paste_clipboard();
                            }
                            if ui
                                .add(egui::Button::new(RichText::new("剪切").color(th.text_weak)))
                                .clicked()
                            {
                                self.clip_selection(ClipKind::Cut);
                            }
                            if ui
                                .add(egui::Button::new(RichText::new("复制").color(th.text_weak)))
                                .clicked()
                            {
                                self.clip_selection(ClipKind::Copy);
                            }
                            if ui
                                .add(egui::Button::new(RichText::new("分享").color(th.text_weak)))
                                .clicked()
                            {
                                ask_share = true;
                            }
                            if single
                                && ui
                                    .add(egui::Button::new(RichText::new("重命名").color(th.text_weak)))
                                    .clicked()
                            {
                                ask_rename = true;
                            }
                            if show_open {
                                let id = sel_meta[0].0.clone();
                                let name = sel_meta[0].1.clone();
                                if is_video_file(&name) {
                                    let quality = match self.quality_cache.get(&id) {
                                        Some(r) => QualityMenuState::Ready(r),
                                        None => QualityMenuState::Loading,
                                    };
                                    play_menu(ui, &id, &name, quality, &mut actions);
                                } else {
                                    let label = if is_media_file(&name) { "播放" } else { "打开" };
                                    if ui
                                        .add(egui::Button::new(RichText::new(label).color(th.text_weak)))
                                        .clicked()
                                    {
                                        ask_preview = true;
                                    }
                                }
                            }
                            if !dl_candidates.is_empty()
                                && ui
                                    .add(
                                        egui::Button::new(RichText::new("下载到本地").color(th.on_accent))
                                            .fill(th.accent)
                                            .stroke(Stroke::NONE)
                                            .corner_radius(th.cr(8)),
                                    )
                                    .clicked()
                            {
                                want_download = true;
                            }
                        }
                    });
                });
            });

        if up {
            self.stack.pop();
            self.show_dir();
        }
        if let Some(i) = jumped {
            self.stack.truncate(i + 1);
            self.show_dir();
        }
        if mkdir {
            self.mkdir_open = true;
            self.mkdir_name = String::new();
        }
        if refresh {
            self.refresh_dir();
        }
        if upload {
            self.upload_here();
        }
        if upload_dir {
            self.upload_dir_here();
        }

        // 处理顶部栏产生的操作
        if clear_clip {
            self.clipboard = None;
        }
        if ask_rename && sel_meta.len() == 1 {
            self.rename_id = Some(sel_meta[0].0.clone());
            self.rename_name = sel_meta[0].1.clone();
        }
        if ask_preview && sel_meta.len() == 1 {
            self.open_preview(sel_meta[0].0.clone(), sel_meta[0].1.clone());
        }
        if ask_trash {
            self.trash_confirm = Some(sel_meta.clone());
        }
        if ask_share {
            self.share_selection();
        }

        if want_download && !dl_candidates.is_empty() {
            let dir = if !self.download_dir.is_empty()
                && std::path::Path::new(&self.download_dir).is_dir()
            {
                std::path::PathBuf::from(&self.download_dir)
            } else {
                let Some(d) = self.choose_download_dir() else {
                    return;
                };
                d
            };
            let has_folder = dl_has_folder;
            self.enqueue_downloads(dl_candidates.clone(), dir);
            if has_folder {
                self.toast_warn("已跳过选中的文件夹(暂不支持整目录下载)");
            }
        }

        // -------- 文件列表 --------
        let mut open_folder: Option<(String, String)> = None;
        let mut sel_reqs: Vec<String> = Vec::new();

        egui::CentralPanel::default()
            .frame(Frame::new().fill(th.card).inner_margin(Margin { left: 20, right: 20, top: 4, bottom: 12 }))
            .show(ctx, |ui| {
                if self.dir_loading && self.files.is_empty() {
                    ui.vertical_centered(|ui| {
                        ui.add_space(90.0);
                        ui.spinner();
                        ui.add_space(8.0);
                        ui.label(RichText::new("加载中…").color(th.text_weak));
                    });
                    return;
                }

                // Breeze/Dolphin 风格: 视图区直接平铺, 不再套外层卡片与边框。
                let inner = ui.max_rect();
                let mut inner_ui = ui.new_child(UiBuilder::new().max_rect(inner));

                inner_ui.add_space(6.0);
                // 列表视图时显示表头
                if self.view_mode == ViewMode::List {
                    self.file_list_header(&mut inner_ui, th, inner.width());
                    let sep_y = inner_ui.cursor().min.y;
                    ui.painter().line_segment(
                        [
                            Pos2::new(inner.min.x + 6.0, sep_y + 3.0),
                            Pos2::new(inner.max.x - 6.0, sep_y + 3.0),
                        ],
                        Stroke::new(1.0, th.border),
                    );
                }

                let list_avail_h = inner.max.y - (inner_ui.cursor().min.y) - 4.0;

                egui::ScrollArea::vertical()
                    .id_salt("file_list_scroll")
                    .auto_shrink([false, false])
                    .max_height(list_avail_h.max(40.0))
                    .show(&mut inner_ui, |ui| {
                    ui.set_min_height(list_avail_h.max(40.0));
                    ui.set_width(inner.width());

                    // 目录空白处右键菜单(粘贴 / 新建文件夹 / 刷新)。先于行注册,
                    // 后续行的右键菜单优先级更高, 空白处才会落到这里。
                    let bg_resp = ui.interact(
                        ui.clip_rect(),
                        ui.id().with("file_list_bg"),
                        egui::Sense::click(),
                    );
                    let has_clip = self.clipboard.is_some();
                    bg_resp.context_menu(|ui| {
                        if has_clip && ui.button("粘贴").clicked() {
                            self.paste_clipboard();
                            ui.close_menu();
                        }
                        if ui.button("上传文件").clicked() {
                            self.upload_here();
                            ui.close_menu();
                        }
                        if ui.button("上传文件夹").clicked() {
                            self.upload_dir_here();
                            ui.close_menu();
                        }
                        if ui.button("新建文件夹").clicked() {
                            self.mkdir_open = true;
                            self.mkdir_name.clear();
                            ui.close_menu();
                        }
                        if ui.button("刷新").clicked() {
                            self.refresh_dir();
                            ui.close_menu();
                        }
                    });

                    let mut even = false;
                    let (cn_x, cs_x, ct_x) = self.col_layout(inner.width());
                    let all_files: Vec<&File> = folders.iter().chain(plain.iter()).collect();

                    if self.view_mode == ViewMode::List {
                        // 列表视图
                        for f in &all_files {
                            let is_sel = self.selected.contains(&f.id);
                            let quality = match self.quality_cache.get(&f.id) {
                                Some(r) => QualityMenuState::Ready(r),
                                None => QualityMenuState::Loading,
                            };
                            if let Some(id) = file_row(ui, th, f, is_sel, even, has_clip, quality, &mut actions, cn_x, cs_x, ct_x) {
                                sel_reqs.push(id);
                            }
                            even = !even;
                        }
                    } else {
                        // 图标视图
                        let card_w = 100.0;
                        let card_h = 100.0;
                        let gap = 8.0;
                        let avail_w = inner.width();
                        let cols = ((avail_w + gap) / (card_w + gap)).floor().max(1.0) as usize;
                        let total_rows = all_files.len().div_ceil(cols);
                        for row in 0..total_rows {
                            ui.horizontal(|ui| {
                                ui.add_space(4.0);
                                for col in 0..cols {
                                    let idx = row * cols + col;
                                    if idx >= all_files.len() { break; }
                                    let f = all_files[idx];
                                    let is_sel = self.selected.contains(&f.id);
                                    let quality = match self.quality_cache.get(&f.id) {
                                        Some(r) => QualityMenuState::Ready(r),
                                        None => QualityMenuState::Loading,
                                    };
                                    let (rect, resp) = ui.allocate_exact_size(vec2(card_w, card_h), egui::Sense::click());
                                    let painter = ui.painter().clone();

                                    // 背景
                                    let bg = if is_sel {
                                        if th.breeze {
                                            th.accent
                                        } else {
                                            mix(th.card, th.accent, if th.dark { 0.22 } else { 0.12 })
                                        }
                                    } else if resp.hovered() {
                                        mix(th.card, th.text, if th.dark { 0.07 } else { 0.045 })
                                    } else {
                                        egui::Color32::TRANSPARENT
                                    };
                                    painter.rect_filled(rect, th.cr(10), bg);
                                    if is_sel && !th.breeze {
                                        painter.rect_stroke(rect, th.cr(10), Stroke::new(2.0, th.accent), egui::StrokeKind::Inside);
                                    }

                                    // 左上角复选框
                                    let cb_size = 14.0;
                                    let cb_x = rect.min.x + 6.0;
                                    let cb_y = rect.min.y + 6.0;
                                    let cb_rect = Rect::from_min_max(Pos2::new(cb_x, cb_y), Pos2::new(cb_x + cb_size, cb_y + cb_size));
                                    let cb_resp = ui.interact(cb_rect, ui.id().with(("cb", f.id.clone())), egui::Sense::click());
                                    let cb_on_accent = is_sel && th.breeze;
                                    let cb_bg = if cb_on_accent {
                                        th.on_accent
                                    } else if is_sel {
                                        th.accent
                                    } else {
                                        mix(th.card, th.text, if th.dark { 0.3 } else { 0.2 })
                                    };
                                    let cb_stroke = if is_sel { Stroke::NONE } else { Stroke::new(1.0, th.text_faint) };
                                    painter.rect_filled(cb_rect, th.cr(3), cb_bg);
                                    painter.rect_stroke(cb_rect, th.cr(3), cb_stroke, egui::StrokeKind::Inside);
                                    if is_sel {
                                        let check_pts = [
                                            Pos2::new(cb_x + 3.0, cb_y + cb_size / 2.0),
                                            Pos2::new(cb_x + 5.5, cb_y + cb_size / 2.0 + 2.5),
                                            Pos2::new(cb_x + cb_size - 2.5, cb_y + 2.5),
                                        ];
                                        let tick = if cb_on_accent { th.accent } else { th.on_accent };
                                        painter.add(egui::Shape::line(check_pts.to_vec(), Stroke::new(1.8, tick)));
                                    }

                                    // 文件图标（居中偏上）
                                    let (glyph, color) = file_visual(f);
                                    let icon_size = 36.0;
                                    let icon_rect = Rect::from_center_size(
                                        Pos2::new(rect.center().x, rect.min.y + 34.0),
                                        vec2(icon_size, icon_size),
                                    );
                                    let tile_bg = if is_sel && th.breeze {
                                        mix(th.on_accent, color, if th.dark { 0.22 } else { 0.14 })
                                    } else {
                                        mix(th.card, color, if th.dark { 0.16 } else { 0.10 })
                                    };
                                    painter.rect_filled(icon_rect, th.cr(8), tile_bg);
                                    icons::paint(&painter, icon_rect.shrink(4.0), glyph, color);

                                    // 文件名（底部居中，最多 2 行）
                                    let name = &f.name;
                                    let max_w = card_w - 8.0;
                                    let name_color = if is_sel && th.breeze { th.on_accent } else { th.text };
                                    let name_g = truncate_text(&painter, name, max_w, FontId::proportional(11.0), name_color);
                                    let name_h = name_g.size().y.min(24.0);
                                    let name_y = rect.max.y - 8.0 - name_h;
                                    painter.galley(
                                        Pos2::new(rect.center().x - name_g.size().x / 2.0, name_y),
                                        name_g,
                                        name_color,
                                    );

                                    // 右键菜单
                                    let f_ctx = f.clone();
                                    let _menu = resp.context_menu(|ui| {
                                        if f_ctx.is_folder() {
                                            if ui.button("打开").clicked() {
                                                actions.push(RowAction::OpenFolder(f_ctx.id.clone(), f_ctx.name.clone()));
                                                ui.close_menu();
                                            }
                                        } else {
                                            if ui.button("下载到本地…").clicked() {
                                                actions.push(RowAction::DownloadFile(f_ctx.id.clone(), f_ctx.name.clone()));
                                                ui.close_menu();
                                            }
                                            if is_video_file(&f_ctx.name) {
                                                play_menu(ui, &f_ctx.id, &f_ctx.name, quality, &mut actions);
                                            } else {
                                                let label = if is_media_file(&f_ctx.name) { "播放" } else { "打开" };
                                                if ui.button(label).clicked() {
                                                    actions.push(RowAction::OpenFile(f_ctx.id.clone(), f_ctx.name.clone()));
                                                    ui.close_menu();
                                                }
                                            }
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

                                    // 点击处理
                                    let dbl = resp.double_clicked();
                                    // 只有点击复选框才勾选; 单击卡片不改变选择。
                                    if cb_resp.clicked() {
                                        sel_reqs.push(f.id.clone());
                                    } else if dbl {
                                        if f.is_folder() {
                                            actions.push(RowAction::OpenFolder(f.id.clone(), f.name.clone()));
                                        } else {
                                            actions.push(RowAction::OpenFile(f.id.clone(), f.name.clone()));
                                        }
                                    }
                                    ui.add_space(4.0);
                                }
                            });
                        }
                    }

                    if self.dir_next.is_some() {
                        ui.add_space(4.0);
                        ui.vertical_centered(|ui| {
                            if ui.button("加载更多").clicked() {
                                self.load_more();
                            }
                        });
                        ui.add_space(2.0);
                    }
                    if self.files.is_empty() && !self.dir_loading {
                        ui.add_space((list_avail_h * 0.3).max(20.0));
                        ui.vertical_centered(|ui| {
                            ui.label(RichText::new("此文件夹为空").color(th.text_weak));
                        });
                    } else if !self.filter.is_empty() && folders.is_empty() && plain.is_empty() {
                        ui.add_space((list_avail_h * 0.3).max(20.0));
                        ui.vertical_centered(|ui| {
                            ui.label(
                                RichText::new(format!("没有匹配「{}」的文件", self.filter))
                                    .color(th.text_weak),
                            );
                        });
                    }

                    // 处理复选框勾选请求
                    for id in sel_reqs {
                        if self.selected.contains(&id) {
                            self.selected.remove(&id);
                        } else {
                            self.selected.insert(id);
                        }
                    }
                });

                for action in actions {
                    match action {
                        RowAction::OpenFolder(id, name) => open_folder = Some((id, name)),
                        RowAction::OpenFile(id, name) => self.open_preview(id, name),
                        RowAction::FetchQualities(id, name) => self.fetch_qualities(id, name),
                        RowAction::PlayOption(id, opt) => self.play_option(id, opt),
                        RowAction::DownloadFile(id, name) => self.download_single(id, name),
                        RowAction::CopyName(name) => {
                            let ctx2 = ctx.clone();
                            ctx2.copy_text(name);
                            self.toast_ok("已复制名称");
                        }
                        RowAction::Rename(id, name) => {
                            self.rename_id = Some(id);
                            self.rename_name = name;
                        }
                        RowAction::CopyItem(id) => {
                            self.clip_item(ClipKind::Copy, id);
                        }
                        RowAction::CutItem(id) => {
                            self.clip_item(ClipKind::Cut, id);
                        }
                        RowAction::Share(id) => {
                            self.share_item(id);
                        }
                        RowAction::PasteInto(id) => {
                            self.paste_into(Some(id));
                        }
                        RowAction::Trash(id) => {
                            if let Some(name) = self
                                .files
                                .iter()
                                .find(|f| f.id == id)
                                .map(|f| f.name.clone())
                            {
                                self.trash_confirm = Some(vec![(id, name)]);
                            }
                        }
                    }
                }
            });

        if let Some((id, name)) = open_folder {
            self.goto_folder(&id, &name);
        }
    }

    fn file_list_header(&mut self, ui: &mut egui::Ui, th: &Theme, w: f32) {
        let (name_x, size_left, time_left) = self.col_layout(w);
        let h = 30.0;
        let (rect, _) = ui.allocate_exact_size(vec2(w, h), egui::Sense::hover());
        let painter = ui.painter().clone();
        let top = rect.min.y;
        let x0 = rect.min.x;

        // 表头全选复选框
        let (folders, plain) = self.visible_rows();
        let total_visible = folders.len() + plain.len();
        let all_selected = total_visible > 0 && self.selected.len() >= total_visible;
        let some_selected = !self.selected.is_empty() && !all_selected;

        let cb_size = 14.0;
        let cb_x = x0 + 6.0;
        let cb_y = top + (h - cb_size) / 2.0;
        let cb_rect = Rect::from_min_max(Pos2::new(cb_x, cb_y), Pos2::new(cb_x + cb_size, cb_y + cb_size));
        let cb_resp = ui.interact(cb_rect, ui.id().with("header_cb"), egui::Sense::click());

        // 复选框背景
        let cb_bg = if all_selected { th.accent } else if some_selected { mix(th.accent, th.bg, 0.5) } else { egui::Color32::TRANSPARENT };
        let cb_stroke = if all_selected || some_selected { Stroke::NONE } else { Stroke::new(1.0, th.text_faint) };
        painter.rect_filled(cb_rect, th.cr(3), cb_bg);
        painter.rect_stroke(cb_rect, th.cr(3), cb_stroke, egui::StrokeKind::Inside);

        // 全选时绘制勾号
        if all_selected {
            let check_pts = [
                Pos2::new(cb_x + 3.0, cb_y + cb_size / 2.0),
                Pos2::new(cb_x + 5.5, cb_y + cb_size / 2.0 + 2.5),
                Pos2::new(cb_x + cb_size - 2.5, cb_y + 2.5),
            ];
            painter.add(egui::Shape::line(check_pts.to_vec(), Stroke::new(1.8, th.on_accent)));
        }
        // 部分选中时绘制横线
        if some_selected {
            painter.line_segment(
                [Pos2::new(cb_x + 3.0, cb_y + cb_size / 2.0), Pos2::new(cb_x + cb_size - 3.0, cb_y + cb_size / 2.0)],
                Stroke::new(2.0, th.on_accent),
            );
        }

        // 悬停效果
        if cb_resp.hovered() {
            painter.rect_filled(cb_rect, th.cr(3), mix(th.accent, th.bg, 0.85));
        }

        // 点击切换全选/取消全选(仅当前可见/过滤后的行)
        if cb_resp.clicked() {
            if all_selected {
                self.selected.clear();
            } else {
                let (folders, plain) = self.visible_rows();
                for f in folders.iter().chain(plain.iter()) {
                    self.selected.insert(f.id.clone());
                }
            }
        }

        struct Col {
            x: f32,
            label: &'static str,
            by: SortBy,
            w: f32,
        }
        let cols = [
            Col { x: name_x, label: "名称", by: SortBy::Name, w: size_left - name_x },
            Col { x: size_left, label: "大小", by: SortBy::Size, w: time_left - size_left },
            Col { x: time_left, label: "修改时间", by: SortBy::Modified, w: w - time_left - Self::RIGHT_PAD },
        ];
        for col in &cols {
            let active = self.sort_by == col.by;
            let color = if active { th.accent } else { th.text_faint };
            let crect = Rect::from_min_max(
                Pos2::new(x0 + col.x - 8.0, top),
                Pos2::new((x0 + col.x + col.w).min(rect.right()), top + h),
            );
            let resp = ui.interact(crect, ui.id().with(("header", col.by)), egui::Sense::click());
            if resp.hovered() {
                painter.rect_filled(crect, th.cr(6), th.hover);
            }
            let g = painter.layout_no_wrap(
                col.label.to_string(),
                FontId::proportional(12.5),
                color,
            );
            let label_w = g.size().x;
            painter.galley(
                Pos2::new(x0 + col.x, top + (h - g.size().y) / 2.0),
                g,
                color,
            );
            if active {
                let yc = top + h / 2.0;
                let mx = x0 + col.x + label_w + 5.0;
                let s = 8.0;
                let pts: Vec<egui::Pos2> = if self.sort_desc {
                    vec![
                        Pos2::new(mx, yc - s / 2.0),
                        Pos2::new(mx + s, yc - s / 2.0),
                        Pos2::new(mx + s / 2.0, yc + s / 2.0 - 1.0),
                    ]
                } else {
                    vec![
                        Pos2::new(mx, yc + s / 2.0),
                        Pos2::new(mx + s, yc + s / 2.0),
                        Pos2::new(mx + s / 2.0, yc - s / 2.0 + 1.0),
                    ]
                };
                painter.add(egui::Shape::convex_polygon(pts, color, Stroke::NONE));
            }
            if resp.clicked() {
                if active {
                    self.sort_desc = !self.sort_desc;
                } else {
                    self.sort_by = col.by;
                    self.sort_desc = false;
                }
            }
            let _ = resp.on_hover_text("点击排序");
        }

        // ---- 拖拽分割线 ----
        let cursor_range = 6.0f32;
        let handle_w = 4.0f32;

        // Handle 1: 名称 | 大小
        let h1_x = x0 + size_left;
        let h1_rect = Rect::from_min_max(
            Pos2::new(h1_x - cursor_range, top),
            Pos2::new(h1_x + cursor_range, top + h),
        );
        let h1_resp = ui.interact(h1_rect, ui.id().with("col_drag_1"), egui::Sense::drag());
        let h1_active = h1_resp.hovered() || h1_resp.is_pointer_button_down_on()
            || self.col_dragging.as_ref().is_some_and(|d| d.handle == 1);
        if h1_active {
            painter.rect_filled(
                Rect::from_center_size(
                    Pos2::new(h1_x, top + h / 2.0),
                    vec2(handle_w, h - 8.0),
                ),
                th.cr(2),
                th.text_faint,
            );
        }
        if h1_resp.drag_started() {
            self.col_dragging = Some(ColDrag {
                handle: 1,
                start_x: h1_resp.interact_pointer_pos().map(|p| p.x).unwrap_or(h1_x),
                orig_size_w: self.col_size_w,
                orig_time_w: self.col_time_w,
            });
        }

        // Handle 2: 大小 | 修改时间
        let h2_x = x0 + time_left;
        let h2_rect = Rect::from_min_max(
            Pos2::new(h2_x - cursor_range, top),
            Pos2::new(h2_x + cursor_range, top + h),
        );
        let h2_resp = ui.interact(h2_rect, ui.id().with("col_drag_2"), egui::Sense::drag());
        let h2_active = h2_resp.hovered() || h2_resp.is_pointer_button_down_on()
            || self.col_dragging.as_ref().is_some_and(|d| d.handle == 2);
        if h2_active {
            painter.rect_filled(
                Rect::from_center_size(
                    Pos2::new(h2_x, top + h / 2.0),
                    vec2(handle_w, h - 8.0),
                ),
                th.cr(2),
                th.text_faint,
            );
        }
        if h2_resp.drag_started() {
            self.col_dragging = Some(ColDrag {
                handle: 2,
                start_x: h2_resp.interact_pointer_pos().map(|p| p.x).unwrap_or(h2_x),
                orig_size_w: self.col_size_w,
                orig_time_w: self.col_time_w,
            });
        }

        // 设置拖拽时的鼠标样式
        if h1_resp.hovered() || h2_resp.hovered() || self.col_dragging.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
    }
}

