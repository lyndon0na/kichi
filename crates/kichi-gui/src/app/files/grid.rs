//! 图标(网格)视图: 缩略图预取行区间 / 纹理解码上限 / 卡片绘制。

use std::time::Instant;

use eframe::egui::{self, pos2, vec2, Color32, FontId, Pos2, Rect, RichText, Stroke};

use crate::filetypes::file_visual;
use crate::icons;
use crate::msg::Cmd;
use crate::theme::{mix, Theme};

use super::super::helpers::truncate_text;
use super::super::preview::PreviewPage;
use super::super::preview::QualityMenuState;
use super::super::thumbs::ThumbsPage;
use super::super::Global;
use super::row;
use super::FilesPage;
use super::RowAction;
use kichi_core::types::File;

/// 图标视图卡片大小的可调范围(Ctrl + 滚轮)。
pub(crate) const GRID_CARD_MIN: f32 = 80.0;
pub(crate) const GRID_CARD_MAX: f32 = 160.0;
/// 缩略图在卡片内的最大占宽比(与网格绘制处一致)。
pub(crate) const THUMB_MAX_CARD_RATIO: f32 = 0.85;

/// 图标视图里需要请求缩略图的行区间: 可见行上下各扩一屏预取。
///
/// `top` 是首行在屏幕坐标里的 y(内容滚动后为负), `clip` 是滚动视口。
/// 算错的后果是静默的 —— 区间偏小则网格长期留白, 偏大则等于整目录入队。
pub(crate) fn thumb_row_range(
    clip: Rect,
    top: f32,
    row_h: f32,
    total_rows: usize,
) -> std::ops::Range<usize> {
    let row_h = row_h.max(1.0);
    let margin = clip.height();
    let first = ((clip.min.y - margin - top) / row_h).floor().max(0.0) as usize;
    let last = ((clip.max.y + margin - top) / row_h).ceil().max(0.0) as usize;
    first.min(total_rows)..last.min(total_rows)
}

/// 单张缩略图纹理最长边的上限(物理像素): 卡片最大显示尺寸 × 屏幕像素密度。
///
/// 服务端下发的缩略图(实测 720×405)远大于卡片所需, 不降采样就直接上传纹理
/// 会让显存按原始尺寸记账。按此上限解码, 卡片放到最大、屏幕是 HiDPI 时也够清。
pub(crate) fn thumb_max_edge(pixels_per_point: f32) -> u32 {
    let px = GRID_CARD_MAX * THUMB_MAX_CARD_RATIO * pixels_per_point;
    (px.ceil() as u32).clamp(128, 512)
}

/// 网格视图上下文(与 `ListCtx` 同构, 多一条缩略图域句柄)。
pub(super) struct GridCtx<'a> {
    pub th: &'a Theme,
    pub g: &'a mut Global,
    pub thumbs: &'a mut ThumbsPage,
    pub preview: &'a PreviewPage,
    pub files: &'a [&'a File],
    pub avail_w: f32,
    pub has_clip: bool,
    /// 是否启用 aria2 推送(决定右键菜单是否出现「发送到 aria2」)。
    pub aria2: bool,
    pub actions: &'a mut Vec<RowAction>,
    pub sel_reqs: &'a mut Vec<String>,
}

impl FilesPage {
    /// 网格视图: 按「可见行 ± 一屏」请求缩略图, 并绘制卡片。
    pub(super) fn grid_cards(&mut self, ui: &mut egui::Ui, cx: GridCtx<'_>) {
        // 图标视图
        let card_w = self.grid_card_size;
        let card_h = self.grid_card_size * 1.15;
        let gap = 8.0;
        let avail_w = cx.avail_w;
        let cols = ((avail_w + gap) / (card_w + gap)).floor().max(1.0) as usize;
        let total_rows = cx.files.len().div_ceil(cols);

        // 只请求「可见行 ± 一屏」的缩略图: 整目录一次性入队会让
        // worker 长时间啃已经滚出视野的图, 新滚到的位置反而排在后面。
        let clip = ui.clip_rect();
        let row_h = card_h + ui.spacing().item_spacing.y;
        let row_range = thumb_row_range(clip, ui.cursor().min.y, row_h, total_rows);
        let max_edge = thumb_max_edge(ui.ctx().pixels_per_point());
        let now = Instant::now();
        for row in row_range {
            for col in 0..cols {
                let idx = row * cols + col;
                if idx >= cx.files.len() {
                    break;
                }
                let f = cx.files[idx];
                if f.is_folder() {
                    continue;
                }
                // 纹理在 = 图正被看着: 刷新 LRU, 使其免于本轮淘汰。
                if cx.thumbs.textures.contains(&f.id) {
                    cx.thumbs.textures.mark_used(&f.id, now);
                    continue;
                }
                let Some(url) = &f.thumbnail_link else {
                    continue;
                };
                if !cx.thumbs.needs_request(&f.id) {
                    continue;
                }
                cx.thumbs.mark_inflight(f.id.clone());
                cx.g.send(Cmd::LoadThumbnail {
                    file_id: f.id.clone(),
                    url: url.clone(),
                    max_edge,
                });
            }
        }

        for row in 0..total_rows {
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                for col in 0..cols {
                    let idx = row * cols + col;
                    if idx >= cx.files.len() {
                        break;
                    }
                    let f = cx.files[idx];
                    let is_sel = self.selected.contains(&f.id);
                    let quality = match cx.preview.quality(&f.id) {
                        Some(r) => QualityMenuState::Ready(r),
                        None => QualityMenuState::Loading,
                    };
                    let (rect, resp) =
                        ui.allocate_exact_size(vec2(card_w, card_h), egui::Sense::click());
                    let painter = ui.painter().clone();

                    // 背景
                    let bg = if is_sel {
                        if cx.th.breeze {
                            cx.th.accent
                        } else {
                            mix(
                                cx.th.card,
                                cx.th.accent,
                                if cx.th.dark { 0.22 } else { 0.12 },
                            )
                        }
                    } else if resp.hovered() {
                        mix(
                            cx.th.card,
                            cx.th.text,
                            if cx.th.dark { 0.07 } else { 0.045 },
                        )
                    } else {
                        egui::Color32::TRANSPARENT
                    };
                    painter.rect_filled(rect, cx.th.cr(10), bg);
                    if is_sel && !cx.th.breeze {
                        painter.rect_stroke(
                            rect,
                            cx.th.cr(10),
                            Stroke::new(2.0_f32, cx.th.accent),
                            egui::StrokeKind::Inside,
                        );
                    }

                    // 左上角复选框
                    let cb_size = 14.0;
                    let cb_x = rect.min.x + 6.0;
                    let cb_y = rect.min.y + 6.0;
                    let cb_rect = Rect::from_min_max(
                        Pos2::new(cb_x, cb_y),
                        Pos2::new(cb_x + cb_size, cb_y + cb_size),
                    );
                    let cb_resp = ui.interact(
                        cb_rect,
                        ui.id().with(("cb", f.id.clone())),
                        egui::Sense::click(),
                    );
                    let cb_on_accent = is_sel && cx.th.breeze;
                    let cb_bg = if cb_on_accent {
                        cx.th.on_accent
                    } else if is_sel {
                        cx.th.accent
                    } else {
                        mix(cx.th.card, cx.th.text, if cx.th.dark { 0.3 } else { 0.2 })
                    };
                    let cb_stroke = if is_sel {
                        Stroke::NONE
                    } else {
                        Stroke::new(1.0_f32, cx.th.text_faint)
                    };
                    painter.rect_filled(cb_rect, cx.th.cr(3), cb_bg);
                    painter.rect_stroke(cb_rect, cx.th.cr(3), cb_stroke, egui::StrokeKind::Inside);
                    if is_sel {
                        let check_pts = [
                            Pos2::new(cb_x + 3.0, cb_y + cb_size / 2.0),
                            Pos2::new(cb_x + 5.5, cb_y + cb_size / 2.0 + 2.5),
                            Pos2::new(cb_x + cb_size - 2.5, cb_y + 2.5),
                        ];
                        let tick = if cb_on_accent {
                            cx.th.accent
                        } else {
                            cx.th.on_accent
                        };
                        painter.add(egui::Shape::line(
                            check_pts.to_vec(),
                            Stroke::new(1.8_f32, tick),
                        ));
                    }

                    // 文件图标或缩略图（居中偏上）
                    let icon_size = card_w * 0.45;
                    let icon_rect = Rect::from_center_size(
                        Pos2::new(rect.center().x, rect.min.y + card_h * 0.35),
                        vec2(icon_size, icon_size),
                    );

                    if let Some(texture) = cx.thumbs.textures.get(&f.id) {
                        // 渲染缩略图（保持宽高比）
                        let max_size = card_w * THUMB_MAX_CARD_RATIO;
                        let tex_size = texture.size_vec2();
                        let aspect = tex_size.x / tex_size.y;

                        let (w, h) = if aspect > 1.0 {
                            // 横向图片
                            (max_size, max_size / aspect)
                        } else {
                            // 纵向图片
                            (max_size * aspect, max_size)
                        };

                        let thumb_rect = Rect::from_center_size(
                            Pos2::new(rect.center().x, rect.min.y + card_h * 0.4),
                            vec2(w, h),
                        );
                        painter.image(
                            texture.id(),
                            thumb_rect,
                            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    } else {
                        // 回退到类型图标
                        let (glyph, color) = file_visual(f);
                        let tile_bg = if is_sel && cx.th.breeze {
                            mix(cx.th.on_accent, color, if cx.th.dark { 0.22 } else { 0.14 })
                        } else {
                            mix(cx.th.card, color, if cx.th.dark { 0.16 } else { 0.10 })
                        };
                        painter.rect_filled(icon_rect, cx.th.cr(8), tile_bg);
                        icons::paint(&painter, icon_rect.shrink(4.0), glyph, color);
                    }

                    // 文件名（底部居中，最多 2 行）
                    let name = &f.name;
                    let max_w = card_w - 8.0;
                    let name_color = if is_sel && cx.th.breeze {
                        cx.th.on_accent
                    } else {
                        cx.th.text
                    };
                    let font_size = (card_w * 0.11).clamp(9.0, 14.0);
                    let name_g = truncate_text(
                        &painter,
                        name,
                        max_w,
                        FontId::proportional(font_size),
                        name_color,
                    );
                    let name_h = name_g.size().y.min(card_h * 0.25);
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
                                cx.actions.push(RowAction::OpenFolder(
                                    f_ctx.id.clone(),
                                    f_ctx.name.clone(),
                                ));
                                ui.close_menu();
                            }
                            if ui.button("下载到本地…").clicked() {
                                cx.actions.push(RowAction::DownloadFolder(
                                    f_ctx.id.clone(),
                                    f_ctx.name.clone(),
                                ));
                                ui.close_menu();
                            }
                            if cx.aria2 && ui.button("发送到 aria2").clicked() {
                                cx.actions.push(RowAction::Aria2Folder(
                                    f_ctx.id.clone(),
                                    f_ctx.name.clone(),
                                ));
                                ui.close_menu();
                            }
                        } else {
                            if ui.button("下载到本地…").clicked() {
                                cx.actions.push(RowAction::DownloadFile(
                                    f_ctx.id.clone(),
                                    f_ctx.name.clone(),
                                ));
                                ui.close_menu();
                            }
                            if cx.aria2 && ui.button("发送到 aria2").clicked() {
                                cx.actions.push(RowAction::Aria2File(
                                    f_ctx.id.clone(),
                                    f_ctx.name.clone(),
                                ));
                                ui.close_menu();
                            }
                            row::preview_menu_items(ui, &f_ctx, quality, cx.actions);
                        }
                        ui.separator();
                        if ui.button("分享").clicked() {
                            cx.actions.push(RowAction::Share(f_ctx.id.clone()));
                            ui.close_menu();
                        }
                        if ui.button("复制").clicked() {
                            cx.actions.push(RowAction::CopyItem(f_ctx.id.clone()));
                            ui.close_menu();
                        }
                        if ui.button("剪切").clicked() {
                            cx.actions.push(RowAction::CutItem(f_ctx.id.clone()));
                            ui.close_menu();
                        }
                        if cx.has_clip && f_ctx.is_folder() && ui.button("粘贴到此处").clicked()
                        {
                            cx.actions.push(RowAction::PasteInto(f_ctx.id.clone()));
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui.button("重命名").clicked() {
                            cx.actions
                                .push(RowAction::Rename(f_ctx.id.clone(), f_ctx.name.clone()));
                            ui.close_menu();
                        }
                        if ui.button("复制名称").clicked() {
                            cx.actions.push(RowAction::CopyName(f_ctx.name.clone()));
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui
                            .button(RichText::new("移入回收站").color(cx.th.danger))
                            .clicked()
                        {
                            cx.actions.push(RowAction::Trash(f_ctx.id.clone()));
                            ui.close_menu();
                        }
                    });

                    // 点击处理
                    let dbl = resp.double_clicked();
                    // 只有点击复选框才勾选; 单击卡片不改变选择。
                    if cb_resp.clicked() {
                        cx.sel_reqs.push(f.id.clone());
                    } else if dbl {
                        if f.is_folder() {
                            cx.actions
                                .push(RowAction::OpenFolder(f.id.clone(), f.name.clone()));
                        } else {
                            cx.actions
                                .push(RowAction::OpenFile(f.id.clone(), f.name.clone()));
                        }
                    }
                    ui.add_space(4.0);
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::Pos2;

    #[test]
    fn thumb_rows_cover_visible_plus_one_screen() {
        let clip = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(600.0, 400.0));
        // 未滚动: 可见 0..4 行, 下侧预取一屏 => 0..8。
        assert_eq!(thumb_row_range(clip, 0.0, 100.0, 50), 0..8);
    }

    #[test]
    fn thumb_rows_follow_scroll_position() {
        let clip = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(600.0, 400.0));
        // 内容上移 1000px: 可见 10..14 行, 上下各预取一屏 => 6..18。
        assert_eq!(thumb_row_range(clip, -1000.0, 100.0, 50), 6..18);
    }

    #[test]
    fn thumb_rows_clamp_to_total_and_stay_empty_when_past_end() {
        let clip = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(600.0, 400.0));
        assert_eq!(thumb_row_range(clip, 0.0, 100.0, 3), 0..3);
        // 滚过列表末尾(理论上不会发生)也不能越界。
        assert_eq!(thumb_row_range(clip, -20_000.0, 100.0, 50), 50..50);
    }

    #[test]
    fn thumb_max_edge_covers_max_card_at_any_pixel_ratio() {
        // 卡片放到最大仍要够清: 160 × 0.85 = 136 逻辑像素。
        assert_eq!(thumb_max_edge(1.0), 136);
        assert_eq!(thumb_max_edge(1.25), 170);
        assert_eq!(thumb_max_edge(2.0), 272);
        // 极端缩放不失控。
        assert_eq!(thumb_max_edge(4.0), 512);
    }
}
