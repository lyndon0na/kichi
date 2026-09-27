//! 离线任务单卡渲染: 名称 / 大小 / 时间 + 按阶段变化的右侧操作。

use eframe::egui::{self, vec2, FontId, Pos2, Rect};

use kichi_core::types::{task_created, task_file_id, task_id, task_name, task_size, Task};

use crate::format;
use crate::icons::Glyph;
use crate::theme::Theme;

use super::super::helpers::{card_shell, icon_action, paint_checkbox, truncate_text, CheckState};
use super::super::types::{OfflineTab, TaskOp, TaskSel};

/// 离线任务卡片固定高度(虚拟滚动要求逐行等高)。
pub(super) const TASK_CARD_H: f32 = 64.0;

/// 渲染单张离线任务卡片, 返回行级操作与选择请求。
#[allow(clippy::too_many_arguments)]
pub(super) fn task_card(
    ui: &mut egui::Ui,
    th: &Theme,
    tab: OfflineTab,
    idx: usize,
    t: &Task,
    is_sel: bool,
    ctrl: bool,
    shift: bool,
) -> (Option<TaskOp>, Option<TaskSel>) {
    let mut op: Option<TaskOp> = None;
    let mut sel: Option<TaskSel> = None;
    let w = ui.available_width().max(320.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, TASK_CARD_H), egui::Sense::click());
    let painter = ui.painter().clone();

    card_shell(&painter, th, rect, resp.hovered(), is_sel);

    let inner = rect.shrink2(vec2(12.0, 10.0));

    // 复选框
    let cb_rect = Rect::from_center_size(
        Pos2::new(inner.min.x + 8.0, inner.center().y),
        vec2(16.0, 16.0),
    );
    let cb_resp = ui.interact(
        cb_rect,
        ui.id().with(("task_check", idx)),
        egui::Sense::click(),
    );
    if cb_resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    paint_checkbox(
        &painter,
        th,
        cb_rect,
        if is_sel {
            CheckState::Checked
        } else {
            CheckState::Unchecked
        },
        resp.hovered() || cb_resp.hovered(),
    );
    let cb_clicked = cb_resp.clicked();

    // 列宽: 右侧统一预留两个图标按钮位, 保证各卡片列对齐。
    const CB_W: f32 = 26.0;
    const BTN_SZ: f32 = 28.0;
    const BTN_GAP: f32 = 4.0;
    const BTN_RSV: f32 = BTN_SZ * 2.0 + BTN_GAP;
    let content_x = inner.min.x + CB_W;
    let right_start = inner.max.x - BTN_RSV - 4.0;
    let text_w = (right_start - content_x - 12.0).max(80.0);

    let name = task_name(t).unwrap_or_else(|| "未知任务".into());
    let name_g = truncate_text(&painter, &name, text_w, FontId::proportional(13.5), th.text);
    let truncated = name_g.text() != name.as_str();
    painter.galley(Pos2::new(content_x, inner.min.y + 2.0), name_g, th.text);

    // 次要行: 大小 · 创建时间
    let mut meta = String::new();
    if let Some(sz) = task_size(t) {
        if sz > 0 {
            meta.push_str(&format::fmt_bytes(sz));
        }
    }
    if let Some(created) = task_created(t) {
        let c = format::fmt_time(&created);
        if !c.is_empty() {
            if !meta.is_empty() {
                meta.push_str(" · ");
            }
            meta.push_str(&c);
        }
    }
    if !meta.is_empty() {
        let mg = truncate_text(
            &painter,
            &meta,
            text_w,
            FontId::proportional(11.5),
            th.text_weak,
        );
        painter.galley(Pos2::new(content_x, inner.min.y + 22.0), mg, th.text_weak);
    }

    // 右侧图标操作(按阶段决定)
    let tid = task_id(t);
    let fid = task_file_id(t).filter(|s| !s.is_empty());
    let mut btns: Vec<(Glyph, &str, TaskOp)> = Vec::new();
    match tab {
        OfflineTab::Complete => {
            if let Some(fid) = fid {
                btns.push((
                    Glyph::Download,
                    "下载到本地",
                    TaskOp::Download(fid, name.clone()),
                ));
            }
            if let Some(id) = tid.clone() {
                btns.push((Glyph::Trash, "删除任务记录", TaskOp::Delete(id)));
            }
        }
        OfflineTab::Error => {
            if let Some(id) = tid.clone() {
                btns.push((Glyph::Refresh, "重试", TaskOp::Retry(id)));
            }
            if let Some(id) = tid.clone() {
                btns.push((Glyph::Trash, "删除任务记录", TaskOp::Delete(id)));
            }
        }
        OfflineTab::Pending | OfflineTab::Running => {
            if let Some(id) = tid.clone() {
                btns.push((Glyph::Trash, "删除任务记录", TaskOp::Delete(id)));
            }
        }
    }
    let btn_y = inner.center().y - BTN_SZ / 2.0;
    let mut bx = inner.max.x;
    for (glyph, tip, dop) in btns.into_iter().rev() {
        let r = Rect::from_min_max(Pos2::new(bx - BTN_SZ, btn_y), Pos2::new(bx, btn_y + BTN_SZ));
        bx -= BTN_SZ + BTN_GAP;
        let id = ui.id().with(("task_btn", idx, tip));
        if icon_action(ui, &painter, th, r, id, glyph, tip, glyph == Glyph::Trash) {
            op = Some(dop);
        }
    }

    // 名称被截断时, hover 整行显示完整文件名。
    if truncated {
        resp.clone().on_hover_text(&name);
    }

    if cb_clicked {
        if let Some(id) = tid {
            sel = Some(TaskSel::Toggle(id));
        }
    } else if op.is_none() && resp.clicked() {
        if let Some(id) = tid {
            sel = Some(if shift {
                TaskSel::Range(id)
            } else if ctrl {
                TaskSel::Toggle(id)
            } else {
                TaskSel::Replace(id)
            });
        }
    }

    (op, sel)
}
