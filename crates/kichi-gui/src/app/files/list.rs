//! 文件页列表视图: 列宽布局、表头、行渲染。

use eframe::egui::{self, vec2, FontId, Pos2, Rect, Stroke};

use crate::theme::{mix, Theme};

use super::super::preview::PreviewPage;
use super::super::preview::QualityMenuState;
use super::super::search::SearchPage;
use super::{row, FilesPage};
use super::{ColDrag, RowAction, SortBy};
use kichi_core::types::File;

/// 列表视图上下文: 本帧可见行 + 各域句柄。
pub(super) struct ListCtx<'a> {
    pub th: &'a Theme,
    pub preview: &'a PreviewPage,
    pub files: &'a [&'a File],
    pub has_clip: bool,
    /// 是否启用 aria2 推送(决定右键菜单是否出现「发送到 aria2」)。
    pub aria2: bool,
    pub col_w: f32,
    pub actions: &'a mut Vec<RowAction>,
    pub sel_reqs: &'a mut Vec<String>,
}

impl FilesPage {
    /// 布局坐标。返回 (name_x, size_left, time_left)。
    /// 名称列占据剩余空间; size_w / time_w 来自自身状态。
    pub(super) fn col_layout(&self, w: f32) -> (f32, f32, f32) {
        let time_left = (w - self.col_time_w - Self::RIGHT_PAD).max(Self::NAME_X + 60.0);
        let size_left = (time_left - self.col_size_w - 16.0).max(Self::NAME_X + 60.0);
        (Self::NAME_X, size_left, time_left)
    }

    /// 列表视图的行渲染(文件夹与文件混排, 行底色隔行交替)。
    pub(super) fn list_rows(&self, ui: &mut egui::Ui, cx: ListCtx<'_>) {
        let mut even = false;
        let (cn_x, cs_x, ct_x) = self.col_layout(cx.col_w);

        // 列表视图
        for f in cx.files {
            let is_sel = self.selected.contains(&f.id);
            let quality = match cx.preview.quality(&f.id) {
                Some(r) => QualityMenuState::Ready(r),
                None => QualityMenuState::Loading,
            };
            if let Some(id) = row::file_row(
                ui,
                cx.th,
                f,
                is_sel,
                even,
                cx.has_clip,
                cx.aria2,
                quality,
                cx.actions,
                cn_x,
                cs_x,
                ct_x,
            ) {
                cx.sel_reqs.push(id);
            }
            even = !even;
        }
    }

    pub(super) fn file_list_header(
        &mut self,
        ui: &mut egui::Ui,
        th: &Theme,
        w: f32,
        search: &SearchPage,
    ) {
        let (name_x, size_left, time_left) = self.col_layout(w);
        let h = 30.0;
        let (rect, _) = ui.allocate_exact_size(vec2(w, h), egui::Sense::hover());
        let painter = ui.painter().clone();
        let top = rect.min.y;
        let x0 = rect.min.x;

        // 表头全选复选框
        let (folders, plain) = self.visible_rows(search);
        let total_visible = folders.len() + plain.len();
        let all_selected = total_visible > 0 && self.selected.len() >= total_visible;
        let some_selected = !self.selected.is_empty() && !all_selected;

        let cb_size = 14.0;
        let cb_x = x0 + 6.0;
        let cb_y = top + (h - cb_size) / 2.0;
        let cb_rect = Rect::from_min_max(
            Pos2::new(cb_x, cb_y),
            Pos2::new(cb_x + cb_size, cb_y + cb_size),
        );
        let cb_resp = ui.interact(cb_rect, ui.id().with("header_cb"), egui::Sense::click());

        // 复选框背景
        let cb_bg = if all_selected {
            th.accent
        } else if some_selected {
            mix(th.accent, th.bg, 0.5)
        } else {
            egui::Color32::TRANSPARENT
        };
        let cb_stroke = if all_selected || some_selected {
            Stroke::NONE
        } else {
            Stroke::new(1.0_f32, th.text_faint)
        };
        painter.rect_filled(cb_rect, th.cr(3), cb_bg);
        painter.rect_stroke(cb_rect, th.cr(3), cb_stroke, egui::StrokeKind::Inside);

        // 全选时绘制勾号
        if all_selected {
            let check_pts = [
                Pos2::new(cb_x + 3.0, cb_y + cb_size / 2.0),
                Pos2::new(cb_x + 5.5, cb_y + cb_size / 2.0 + 2.5),
                Pos2::new(cb_x + cb_size - 2.5, cb_y + 2.5),
            ];
            painter.add(egui::Shape::line(
                check_pts.to_vec(),
                Stroke::new(1.8_f32, th.on_accent),
            ));
        }
        // 部分选中时绘制横线
        if some_selected {
            painter.line_segment(
                [
                    Pos2::new(cb_x + 3.0, cb_y + cb_size / 2.0),
                    Pos2::new(cb_x + cb_size - 3.0, cb_y + cb_size / 2.0),
                ],
                Stroke::new(2.0_f32, th.on_accent),
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
                let (folders, plain) = self.visible_rows(search);
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
            Col {
                x: name_x,
                label: "名称",
                by: SortBy::Name,
                w: size_left - name_x,
            },
            Col {
                x: size_left,
                label: "大小",
                by: SortBy::Size,
                w: time_left - size_left,
            },
            Col {
                x: time_left,
                label: "修改时间",
                by: SortBy::Modified,
                w: w - time_left - Self::RIGHT_PAD,
            },
        ];
        for col in &cols {
            let active = self.sort_by == col.by;
            let color = if active { th.accent } else { th.text_faint };
            let crect = Rect::from_min_max(
                Pos2::new(x0 + col.x - 8.0, top),
                Pos2::new((x0 + col.x + col.w).min(rect.right()), top + h),
            );
            let resp = ui.interact(
                crect,
                ui.id().with(("header", col.by)),
                egui::Sense::click(),
            );
            if resp.hovered() {
                painter.rect_filled(crect, th.cr(6), th.hover);
            }
            let g =
                painter.layout_no_wrap(col.label.to_string(), FontId::proportional(12.5), color);
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
        let h1_active = h1_resp.hovered()
            || h1_resp.is_pointer_button_down_on()
            || self.col_dragging.as_ref().is_some_and(|d| d.handle == 1);
        if h1_active {
            painter.rect_filled(
                Rect::from_center_size(Pos2::new(h1_x, top + h / 2.0), vec2(handle_w, h - 8.0)),
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
        let h2_active = h2_resp.hovered()
            || h2_resp.is_pointer_button_down_on()
            || self.col_dragging.as_ref().is_some_and(|d| d.handle == 2);
        if h2_active {
            painter.rect_filled(
                Rect::from_center_size(Pos2::new(h2_x, top + h / 2.0), vec2(handle_w, h - 8.0)),
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
