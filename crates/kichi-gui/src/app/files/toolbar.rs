//! 顶部栏的纯绘制片段: 面包屑导航。
//!
//! 搜索框 / 视图切换 / 列宽拖拽仍在入口(`files_page.rs`)的顶部面板里, 随 P2-11 后续步骤并入本模块。

use eframe::egui::{self, vec2, FontId, Pos2, Rect};

use crate::icons::{self, Glyph};
use crate::theme::Theme;

use super::super::helpers::truncate_text;
use super::super::types::Crumb;

/// 面包屑导航: 宽度不足时从左侧省略中间层级, 始终保留当前目录(必要时截断)。
/// 返回被点击的层级索引。
pub(crate) fn breadcrumbs(
    ui: &mut egui::Ui,
    th: &Theme,
    crumbs: &[Crumb],
    budget: f32,
) -> Option<usize> {
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
        icons::paint(
            &painter,
            Rect::from_center_size(sr.center(), vec2(11.0, 11.0)),
            Glyph::ChevronRight,
            th.text_faint,
        );
    }
    for i in start..n {
        if i > start {
            let (sr, _) = ui.allocate_exact_size(vec2(sep, row_h), egui::Sense::hover());
            icons::paint(
                &painter,
                Rect::from_center_size(sr.center(), vec2(11.0, 11.0)),
                Glyph::ChevronRight,
                th.text_faint,
            );
        }
        let last = i == n - 1;
        let (rect, resp) = ui.allocate_exact_size(vec2(widths[i], row_h), egui::Sense::click());
        if !last && resp.hovered() {
            painter.rect_filled(rect, th.cr(6), th.hover);
        }
        let color = if last { th.text } else { th.text_weak };
        let g = galleys[i].clone();
        painter.galley(
            Pos2::new(
                rect.center().x - g.size().x / 2.0,
                rect.center().y - g.size().y / 2.0,
            ),
            g,
            color,
        );
        if !last && resp.clicked() {
            clicked = Some(i);
        }
    }
    clicked
}
