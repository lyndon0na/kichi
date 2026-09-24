//! 图标(网格)视图的纯计算: 缩略图预取行区间 / 纹理解码上限 / 卡片尺寸常量。
//!
//! 网格绘制本身仍是入口(`files_page.rs`)里的分支, 随 P2-11 后续步骤并入本模块。

use eframe::egui::Rect;

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
