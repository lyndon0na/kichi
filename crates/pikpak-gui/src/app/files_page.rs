use eframe::egui::{self, Align, CornerRadius, FontId, Frame, Key, Layout, Margin, Pos2, Rect, RichText, Stroke, vec2, UiBuilder};

use pikpak_core::types::File;

use crate::format;
use crate::icons::{self, Glyph};
use crate::theme::{mix, Theme};

use super::helpers::truncate_text;
use super::types::{ColDrag, RowAction, RowSel, SortBy};
use super::App;

/// 文件类型 -> 图标 / 颜色。
fn file_visual(f: &File) -> (Glyph, egui::Color32) {
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
    const NAME_X: f32 = 46.0;
    const RIGHT_PAD: f32 = 8.0;
    const MIN_SIZE_W: f32 = 60.0;
    const MIN_TIME_W: f32 = 80.0;

    fn col_layout(&self, w: f32) -> (f32, f32, f32) {
        let time_left = (w - self.col_time_w - Self::RIGHT_PAD).max(Self::NAME_X + 60.0);
        let size_left = (time_left - self.col_size_w - 16.0).max(Self::NAME_X + 60.0);
        (Self::NAME_X, size_left, time_left)
    }
}

/// 列表行; 返回选择变更。
fn file_row(
    ui: &mut egui::Ui,
    th: &Theme,
    f: &File,
    is_sel: bool,
    ctrl: bool,
    even: bool,
    actions: &mut Vec<RowAction>,
    name_x: f32,
    size_left: f32,
    time_left: f32,
) -> Option<RowSel> {
    let row_h = 40.0;
    let w = ui.available_width().max(320.0);
    let (rect, row_resp) = ui.allocate_exact_size(vec2(w, row_h), egui::Sense::click());
    let painter = ui.painter().clone();

    let hovered = row_resp.hovered();

    let bg = if is_sel {
        mix(th.card, th.accent, if th.dark { 0.22 } else { 0.12 })
    } else if hovered {
        mix(th.card, th.text, if th.dark { 0.07 } else { 0.045 })
    } else if even {
        mix(th.card, th.text, if th.dark { 0.018 } else { 0.012 })
    } else {
        egui::Color32::TRANSPARENT
    };
    painter.rect_filled(rect.shrink2(vec2(2.0, 2.0)), CornerRadius::same(8), bg);
    if is_sel {
        painter.rect_filled(
            Rect::from_min_max(
                Pos2::new(rect.min.x + 3.0, rect.min.y + 7.0),
                Pos2::new(rect.min.x + 5.0, rect.max.y - 7.0),
            ),
            CornerRadius::same(2),
            th.accent,
        );
    }

    let yc = rect.center().y;
    let (glyph, color) = file_visual(f);
    let icon_rect = Rect::from_center_size(Pos2::new(rect.min.x + 22.0, yc), vec2(22.0, 22.0));
    painter.rect_filled(icon_rect, CornerRadius::same(6), mix(th.card, color, if th.dark { 0.16 } else { 0.10 }));
    icons::paint(&painter, icon_rect.shrink(2.5), glyph, color);

    let x0 = rect.min.x;

    let name_w = (size_left - 12.0 - name_x).max(24.0);
    let name_g = truncate_text(&painter, &f.name, name_w, FontId::proportional(14.0), th.text);
    painter.galley(
        Pos2::new(x0 + name_x, yc - name_g.size().y / 2.0),
        name_g,
        th.text,
    );

    if !f.is_folder() {
        let g = painter.layout_no_wrap(
            format::fmt_bytes(f.size),
            FontId::proportional(12.5),
            th.text_weak,
        );
        painter.galley(
            Pos2::new(x0 + size_left, yc - g.size().y / 2.0),
            g,
            th.text_weak,
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
        th.text_weak,
    );
    painter.galley(
        Pos2::new(x0 + time_left, yc - g.size().y / 2.0),
        g,
        th.text_weak,
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
            if ui.button("打开(预览暂未支持)").clicked() {
                actions.push(RowAction::OpenFile);
                ui.close_menu();
            }
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

    let mut sel: Option<RowSel> = None;
    if dbl {
        if f.is_folder() {
            actions.push(RowAction::OpenFolder(f.id.clone(), f.name.clone()));
        }
    } else if row_resp.clicked() {
        if ctrl {
            sel = Some(RowSel::Toggle(f.id.clone()));
        } else {
            sel = Some(RowSel::Replace(f.id.clone()));
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
                    self.selected = self.files.iter().map(|f| f.id.clone()).collect();
                }
                if (i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace))
                    && !self.selected.is_empty()
                {
                    let sel = self.selected_names();
                    if !sel.is_empty() {
                        self.trash_confirm = Some(sel);
                    }
                }
            });
        }

        let mut up = false;
        let mut jumped: Option<usize> = None;
        let mut mkdir = false;
        let mut refresh = false;

        // -------- 顶部: 面包屑 + 操作 --------
        egui::TopBottomPanel::top("file_head")
            .frame(Frame::new().fill(th.bg).inner_margin(Margin { left: 20, right: 20, top: 14, bottom: 10 }))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let at_root = self.stack.len() <= 1;
                    let (r, rresp) = ui.allocate_exact_size(vec2(30.0, 30.0), egui::Sense::click());
                    ui.painter().rect_filled(r, CornerRadius::same(8), if rresp.hovered() && !at_root { th.hover } else { egui::Color32::TRANSPARENT });
                    icons::paint(ui.painter(), r.shrink(6.0), Glyph::Up, if at_root { th.text_faint } else { th.text_weak });
                    if rresp.clicked() && !at_root {
                        up = true;
                    }
                    rresp.clone().on_hover_text("返回上级");
                    ui.add_space(4.0);
                    for (i, crumb) in self.stack.iter().enumerate() {
                        if i > 0 {
                            ui.label(RichText::new("›").color(th.text_faint));
                        }
                        let last = i == self.stack.len() - 1;
                        let text = RichText::new(&crumb.label)
                            .size(15.0)
                            .color(if last { th.text } else { th.text_weak });
                        let resp = ui.add(egui::Button::new(text).frame(false));
                        if last {
                            let _ = resp;
                        }
                        if !last && resp.clicked() {
                            jumped = Some(i);
                        }
                    }

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add(
                                egui::Button::new(RichText::new("＋ 新建文件夹").color(th.accent))
                                    .fill(th.accent_soft())
                                    .stroke(Stroke::new(1.0, mix(th.accent, th.bg, 0.4)))
                                    .corner_radius(CornerRadius::same(8)),
                            )
                            .clicked()
                        {
                            mkdir = true;
                        }
                        ui.add_space(4.0);
                        let (rr, rresp) = ui.allocate_exact_size(vec2(30.0, 30.0), egui::Sense::click());
                        ui.painter().rect_filled(rr, CornerRadius::same(8), if rresp.hovered() { th.hover } else { egui::Color32::TRANSPARENT });
                        icons::paint(ui.painter(), rr.shrink(7.0), Glyph::Refresh, th.text_weak);
                        if rresp.clicked() {
                            refresh = true;
                        }
                        rresp.on_hover_text("刷新 (F5)");
                    });
                });
            });

        if up {
            self.stack.pop();
            self.refresh_dir();
        }
        if let Some(i) = jumped {
            self.stack.truncate(i + 1);
            self.refresh_dir();
        }
        if mkdir {
            self.mkdir_open = true;
            self.mkdir_name = String::new();
        }
        if refresh {
            self.refresh_dir();
        }

        // -------- 工具栏: 筛选 / 排序 --------
        let (folders, plain) = self.visible_rows();
        let visible_total = folders.len() + plain.len();

        egui::TopBottomPanel::top("file_toolbar")
            .frame(Frame::new().fill(th.bg).inner_margin(Margin { left: 20, right: 20, top: 4, bottom: 6 }))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("筛选").color(th.text_weak).size(12.5));
                    ui.add(
                        egui::TextEdit::singleline(&mut self.filter)
                            .desired_width(240.0)
                            .hint_text("按名称过滤当前目录")
                            .margin(egui::Margin::symmetric(8, 6)),
                    );
                    if !self.filter.is_empty()
                        && ui
                            .add(egui::Button::new(RichText::new("清除").color(th.text_weak)).frame(false))
                            .clicked()
                    {
                        self.filter.clear();
                    }

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if !self.selected.is_empty() {
                            if ui
                                .add(egui::Button::new(RichText::new("清除选择").color(th.text_weak)).frame(false))
                                .clicked()
                            {
                                self.selected.clear();
                            }
                            ui.label(RichText::new(format!("已选 {}", self.selected.len())).strong().color(th.accent));
                        }
                        ui.label(RichText::new(format!("{visible_total} 项")).color(th.text_faint).size(12.5));
                    });
                });
            });

        // -------- 底部多选操作条 --------
        let sel_meta = self.selected_names();
        let dl_candidates = self.selected_plain_files();
        let dl_has_folder = self.selected_has_folder();
        let mut want_download = false;

        if !sel_meta.is_empty() {
            egui::TopBottomPanel::bottom("file_ops")
                .frame(Frame::new().fill(th.card).inner_margin(Margin::symmetric(16, 10)))
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        let single = sel_meta.len() == 1;
                        if single {
                            ui.label(RichText::new(format!("已选择 · {}", sel_meta[0].1)).size(13.0).color(th.text));
                        } else {
                            ui.label(RichText::new(format!("已选择 {} 项", sel_meta.len())).size(13.0).color(th.text));
                        }
                        ui.add_space(10.0);
                        if single && ui.button("重命名").clicked() {
                            self.rename_id = Some(sel_meta[0].0.clone());
                            self.rename_name = sel_meta[0].1.clone();
                        }
                        if !dl_candidates.is_empty()
                            && ui
                                .add(
                                    egui::Button::new(RichText::new("下载到本地…").color(th.on_accent))
                                        .fill(th.accent)
                                        .stroke(Stroke::NONE)
                                        .corner_radius(CornerRadius::same(8)),
                                )
                                .clicked()
                        {
                            want_download = true;
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui
                                .add(
                                    egui::Button::new(RichText::new("移入回收站").color(th.danger))
                                        .stroke(Stroke::new(1.0, mix(th.danger, th.bg, 0.35)))
                                        .fill(egui::Color32::TRANSPARENT)
                                        .corner_radius(CornerRadius::same(8)),
                                )
                                .clicked()
                            {
                                self.trash_confirm = Some(sel_meta.clone());
                            }
                        });
                    });
                });
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
        let mut actions: Vec<RowAction> = Vec::new();
        let mut sel_reqs: Vec<RowSel> = Vec::new();

        egui::CentralPanel::default()
            .frame(Frame::new().fill(th.bg).inner_margin(Margin { left: 20, right: 20, top: 4, bottom: 12 }))
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

                let card_top = ui.cursor().min.y;
                let card_rect = Rect::from_min_max(
                    Pos2::new(ui.cursor().min.x, card_top),
                    Pos2::new(ui.max_rect().max.x, ui.max_rect().max.y),
                );
                let card_painter = ui.painter_at(card_rect);
                card_painter.rect_filled(card_rect, CornerRadius::same(12), th.card);
                card_painter.rect_stroke(card_rect, CornerRadius::same(12), Stroke::new(1.0, th.border), egui::StrokeKind::Inside);

                let inner = card_rect.shrink2(vec2(2.0, 8.0));
                let mut inner_ui = ui.new_child(UiBuilder::new().max_rect(inner));

                inner_ui.add_space(6.0);
                self.file_list_header(&mut inner_ui, th, inner.width());
                let sep_y = inner_ui.cursor().min.y;
                ui.painter().line_segment(
                    [
                        Pos2::new(inner.min.x + 6.0, sep_y + 3.0),
                        Pos2::new(inner.max.x - 6.0, sep_y + 3.0),
                    ],
                    Stroke::new(1.0, th.border),
                );

                let list_avail_h = inner.max.y - (sep_y + 4.0);
                egui::ScrollArea::vertical()
                    .id_salt("file_list_scroll")
                    .auto_shrink([false, false])
                    .max_height(list_avail_h.max(40.0))
                    .show(&mut inner_ui, |ui| {
                    ui.set_min_height(list_avail_h.max(40.0));
                    ui.set_width(inner.width());
                    let ctrl = ui.input(|i| i.modifiers.ctrl);
                    let mut even = false;
                    let (cn_x, cs_x, ct_x) = self.col_layout(inner.width());
                    for f in folders.iter().chain(plain.iter()) {
                        let is_sel = self.selected.contains(&f.id);
                        if let Some(sel) = file_row(ui, th, f, is_sel, ctrl, even, &mut actions, cn_x, cs_x, ct_x) {
                            sel_reqs.push(sel);
                        }
                        even = !even;
                    }

                    if self.dir_next.is_some() {
                        ui.add_space(4.0);
                        ui.centered_and_justified(|ui| {
                            if ui.button("加载更多").clicked() {
                                self.load_more();
                            }
                        });
                        ui.add_space(2.0);
                    }
                    if self.files.is_empty() && !self.dir_loading {
                        ui.add_space(20.0);
                        ui.centered_and_justified(|ui| {
                            ui.label(RichText::new("此文件夹为空").color(th.text_weak));
                        });
                    } else if !self.filter.is_empty() && folders.is_empty() && plain.is_empty() {
                        ui.add_space(20.0);
                        ui.centered_and_justified(|ui| {
                            ui.label(
                                RichText::new(format!("没有匹配「{}」的文件", self.filter))
                                    .color(th.text_weak),
                            );
                        });
                    }
                });

                for sel in sel_reqs {
                    match sel {
                        RowSel::Replace(id) => {
                            self.selected.clear();
                            self.selected.insert(id);
                        }
                        RowSel::Toggle(id) => {
                            if self.selected.contains(&id) {
                                self.selected.remove(&id);
                            } else {
                                self.selected.insert(id);
                            }
                        }
                    }
                }

                for action in actions {
                    match action {
                        RowAction::OpenFolder(id, name) => open_folder = Some((id, name)),
                        RowAction::OpenFile => {
                            self.toast_warn("文件预览将在后续版本支持");
                        }
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
                painter.rect_filled(crect, CornerRadius::same(6), th.hover);
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
            || self.col_dragging.as_ref().map_or(false, |d| d.handle == 1);
        if h1_active {
            painter.rect_filled(
                Rect::from_center_size(
                    Pos2::new(h1_x, top + h / 2.0),
                    vec2(handle_w, h - 8.0),
                ),
                CornerRadius::same(2),
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
            || self.col_dragging.as_ref().map_or(false, |d| d.handle == 2);
        if h2_active {
            painter.rect_filled(
                Rect::from_center_size(
                    Pos2::new(h2_x, top + h / 2.0),
                    vec2(handle_w, h - 8.0),
                ),
                CornerRadius::same(2),
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
