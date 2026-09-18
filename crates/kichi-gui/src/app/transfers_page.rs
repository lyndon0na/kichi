use std::path::PathBuf;

use eframe::egui::{
    self, vec2, Align, FontId, Frame, Layout, Margin, Pos2, Rect, RichText, Stroke,
};

use crate::format;
use crate::icons::{self, Glyph};
use crate::msg::Cmd;
use crate::settings;
use crate::theme::{mix, Theme};

use super::helpers::{
    card_shell, icon_action, open_dir, open_path, paint_checkbox, truncate_text, CheckState,
};
use super::types::{
    Crumb, DlFilter, DlJob, DlNode, DlOp, DlRow, DlSel, DlStatus, Page, TransferTab, UlFilter,
    UlJob, UlOp, UlStatus,
};
use super::App;

/// 顶层任务卡片高度, 以及其后的间距。
const DL_CARD_H: f32 = 72.0;
const DL_CARD_GAP: f32 = 8.0;
/// 目录树子行(紧凑单行)的高度与间距: 子目录略矮、子文件稍高。
const NODE_DIR_H: f32 = 30.0;
const NODE_FILE_H: f32 = 38.0;
const NODE_GAP: f32 = 2.0;
/// 展开目录卡片底部的留白。
const TREE_PAD: f32 = 8.0;
/// 单个展开目录最多渲染的子行数与截断提示行高(超出部分只提示)。
const TREE_CHILD_CAP: usize = 1000;
const TREE_HINT_H: f32 = 26.0;

/// 左侧复选框列的宽度, 卡片内容统一从这里起排。
const CB_W: f32 = 26.0;
/// 右侧图标按钮组预留宽度(保证各卡片列对齐)。
const BTN_W: f32 = 92.0;
/// 子项的基准缩进(相对复选框列)与每层递增量。
const NODE_BASE: f32 = 22.0;
const NODE_STEP: f32 = 18.0;

/// 某个层级节点的名称左边界(相对行矩形)。父级目录标题在 CB_W+18 处,
/// 因此 depth=1 的子项会比父标题再右移一档, 层级才看得出区别。
fn node_left(inner_min_x: f32, depth: u32) -> f32 {
    inner_min_x + CB_W + NODE_BASE + depth as f32 * NODE_STEP
}

/// 画层级竖线: 每个祖先层级一条, 形成目录树的分组导轨。`extend` 用于跨行间距相接。
fn paint_rails(
    painter: &egui::Painter,
    th: &Theme,
    rect: Rect,
    inner_min_x: f32,
    depth: u32,
    extend: f32,
) {
    let col = mix(th.text_faint, th.bg, 0.45);
    for k in 1..depth {
        let x = node_left(inner_min_x, k) - 30.0;
        painter.line_segment(
            [
                Pos2::new(x, rect.min.y + 2.0),
                Pos2::new(x, rect.max.y + extend),
            ],
            Stroke::new(1.0, col),
        );
    }
}

/// 下载行状态文案(目录任务的「文件 k/N」由卡片自行追加)。
fn status_line(job: &DlJob) -> (egui::Color32, String) {
    if job.is_folder() {
        return match &job.status {
            DlStatus::Queued => (egui::Color32::from_gray(150), "扫描目录中…".into()),
            DlStatus::Running => (egui::Color32::from_rgb(60, 130, 200), "下载中".into()),
            DlStatus::Done => (egui::Color32::from_rgb(70, 150, 90), "已完成".into()),
            DlStatus::Failed(what) => (egui::Color32::from_rgb(217, 70, 60), what.clone()),
        };
    }
    match &job.status {
        DlStatus::Queued => (egui::Color32::from_gray(150), "排队中".into()),
        DlStatus::Running => (egui::Color32::from_rgb(60, 130, 200), "下载中".into()),
        DlStatus::Done => (egui::Color32::from_rgb(70, 150, 90), "已完成".into()),
        DlStatus::Failed(what) => (egui::Color32::from_rgb(217, 70, 60), what.clone()),
    }
}

/// 画目录行的展开/收起三角(展开朝下, 收起朝右)。
fn paint_disclosure(painter: &egui::Painter, rect: Rect, expanded: bool, color: egui::Color32) {
    let c = rect.center();
    let r = rect.width().min(rect.height()) * 0.45;
    let pts = if expanded {
        vec![
            Pos2::new(c.x - r, c.y - r * 0.5),
            Pos2::new(c.x + r, c.y - r * 0.5),
            Pos2::new(c.x, c.y + r * 0.7),
        ]
    } else {
        vec![
            Pos2::new(c.x - r * 0.5, c.y - r),
            Pos2::new(c.x + r * 0.7, c.y),
            Pos2::new(c.x - r * 0.5, c.y + r),
        ]
    };
    painter.add(egui::Shape::convex_polygon(pts, color, Stroke::NONE));
}

/// 上传行状态文案。
fn upload_status_line(job: &UlJob) -> (egui::Color32, String) {
    match &job.status {
        UlStatus::Queued => (egui::Color32::from_gray(150), "排队中".into()),
        UlStatus::Running => (egui::Color32::from_rgb(60, 130, 200), "上传中".into()),
        UlStatus::Done => (egui::Color32::from_rgb(70, 150, 90), "已完成".into()),
        UlStatus::Failed(what) => (egui::Color32::from_rgb(217, 70, 60), what.clone()),
    }
}

/// 渲染单个顶层下载任务卡片, 返回操作和选择请求。
/// `shell=false` 时不自绘卡片底(用于展开目录: 底由整块面板统一绘制)。
#[allow(clippy::too_many_arguments)]
fn dl_card(
    ui: &mut egui::Ui,
    th: &Theme,
    rid: u64,
    job: &DlJob,
    is_sel: bool,
    ctrl: bool,
    shift: bool,
    now: u64,
    shell: bool,
) -> (Option<DlOp>, Option<DlSel>) {
    let mut op: Option<DlOp> = None;
    let mut sel: Option<DlSel> = None;
    let w = ui.available_width().max(320.0);
    let h = DL_CARD_H;
    let (rect, resp) = ui.allocate_exact_size(vec2(w, h), egui::Sense::click());
    let painter = ui.painter().clone();

    let inner = rect.shrink2(vec2(12.0, 10.0));
    let is_folder = job.is_folder();

    // 背景 / 选中态
    if shell {
        card_shell(&painter, th, rect, resp.hovered(), is_sel);
    }

    // 复选框
    let cb_rect = Rect::from_center_size(
        Pos2::new(inner.min.x + 8.0, inner.center().y),
        vec2(16.0, 16.0),
    );
    let cb_resp = ui.interact(
        cb_rect,
        ui.id().with(("dl_check", rid)),
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

    // 布局: [复选框] [展开箭头] [名称/状态/目录] [进度] [按钮]
    let mut content_x = inner.min.x + CB_W;
    // 目录行: 展开/收起箭头
    if is_folder {
        let ch_rect = Rect::from_center_size(
            Pos2::new(content_x + 7.0, inner.center().y),
            vec2(16.0, 16.0),
        );
        let ch_resp = ui.interact(
            ch_rect,
            ui.id().with(("dl_expand", rid)),
            egui::Sense::click(),
        );
        if ch_resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let ch_col = if ch_resp.hovered() {
            th.text
        } else {
            th.text_weak
        };
        paint_disclosure(&painter, ch_rect.shrink(4.0), job.expanded, ch_col);
        if ch_resp.clicked() {
            op = Some(DlOp::Expand);
        }
        content_x += 18.0;
    }
    let right_start = inner.max.x - BTN_W;
    let left_w = ((right_start - content_x - 16.0) * 0.46).max(120.0);

    let name_g = truncate_text(
        &painter,
        &job.name,
        left_w,
        FontId::proportional(13.5),
        th.text,
    );
    painter.galley(Pos2::new(content_x, inner.min.y + 1.0), name_g, th.text);
    let (col, mut txt) = status_line(job);
    // 目录任务: 追加「文件 已完成/全部」。
    if is_folder && job.files_total > 0 {
        txt = format!("{txt} · 文件 {}/{}", job.files_done, job.files_total);
    }
    if let Some(at) = job.at {
        let rel = format::fmt_rel(now, at);
        if !rel.is_empty() {
            txt = format!("{txt} · {rel}");
        }
    }
    let status_g = truncate_text(&painter, &txt, left_w, FontId::proportional(11.5), col);
    painter.galley(Pos2::new(content_x, inner.min.y + 18.0), status_g, col);
    let dest = job.dir.to_string_lossy();
    if !dest.is_empty() {
        let dg = truncate_text(
            &painter,
            &format!("→ {dest}"),
            left_w,
            FontId::proportional(10.5),
            th.text_faint,
        );
        painter.galley(Pos2::new(content_x, inner.min.y + 35.0), dg, th.text_faint);
    }

    // 中间: 进度 / 速度 / ETA
    let mid_x = content_x + left_w + 14.0;
    let mid_w = (right_start - 14.0 - mid_x).max(0.0);
    if mid_w > 40.0 {
        match &job.status {
            DlStatus::Running if job.total > 0 => {
                let frac = (job.done as f32 / job.total as f32).clamp(0.0, 1.0);
                let bar_rect = Rect::from_min_max(
                    Pos2::new(mid_x, inner.center().y - 10.0),
                    Pos2::new(mid_x + mid_w, inner.center().y + 2.0),
                );
                painter.rect_filled(bar_rect, th.cr(3), mix(th.text_faint, th.bg, 0.7));
                let fill_w = mid_w * frac;
                if fill_w > 0.0 {
                    let fill_rect = Rect::from_min_max(
                        bar_rect.min,
                        Pos2::new(bar_rect.min.x + fill_w, bar_rect.max.y),
                    );
                    painter.rect_filled(fill_rect, th.cr(3), th.accent);
                }
                let mut info = format!(
                    "{} / {}",
                    format::fmt_bytes(job.done as i64),
                    format::fmt_bytes(job.total as i64)
                );
                if job.speed > 0 {
                    info.push_str(&format!("   {}/s", format::fmt_bytes(job.speed as i64)));
                    let eta = format::fmt_eta(job.total.saturating_sub(job.done), job.speed);
                    if !eta.is_empty() {
                        info.push_str(&format!("   剩余 {eta}"));
                    }
                }
                painter.text(
                    Pos2::new(mid_x, inner.center().y + 6.0),
                    egui::Align2::LEFT_TOP,
                    info,
                    FontId::proportional(10.5),
                    th.text_weak,
                );
            }
            DlStatus::Running => {
                let mut info = if job.done > 0 {
                    format!("已接收 {}", format::fmt_bytes(job.done as i64))
                } else {
                    "连接中…".to_string()
                };
                if job.speed > 0 {
                    info.push_str(&format!("   {}/s", format::fmt_bytes(job.speed as i64)));
                }
                painter.text(
                    Pos2::new(mid_x, inner.center().y - 7.0),
                    egui::Align2::LEFT_TOP,
                    info,
                    FontId::proportional(11.5),
                    th.text_weak,
                );
            }
            _ => {
                if job.done > 0 {
                    painter.text(
                        Pos2::new(mid_x, inner.center().y - 7.0),
                        egui::Align2::LEFT_TOP,
                        format!("已下载 {}", format::fmt_bytes(job.done as i64)),
                        FontId::proportional(11.5),
                        th.text_weak,
                    );
                }
            }
        }
    }

    // 右侧: 图标操作按钮(右对齐, 统一预留宽度保证各卡片列对齐)
    let mut btns: Vec<(Glyph, &str, DlOp)> = Vec::new();
    match &job.status {
        DlStatus::Queued | DlStatus::Running => btns.push((Glyph::Close, "取消下载", DlOp::Cancel)),
        DlStatus::Done => {
            // 目录没有单一文件可打开, 仅提供「打开所在目录」。
            if !is_folder {
                btns.push((Glyph::OpenExternal, "打开文件", DlOp::OpenFile));
            }
            btns.push((Glyph::Folder, "打开所在目录", DlOp::OpenDir));
            btns.push((Glyph::Trash, "从列表移除", DlOp::Remove));
        }
        DlStatus::Failed(_) => {
            if !job.file_id.is_empty() {
                btns.push((Glyph::Refresh, "重试下载", DlOp::Retry));
            }
            btns.push((Glyph::Folder, "打开所在目录", DlOp::OpenDir));
            btns.push((Glyph::Trash, "从列表移除", DlOp::Remove));
        }
    }

    let btn_sz = 28.0;
    let btn_gap = 4.0;
    let btn_y = inner.center().y - btn_sz / 2.0;
    let mut bx = inner.max.x;
    for (glyph, tip, dop) in btns.into_iter().rev() {
        let rect = Rect::from_min_max(Pos2::new(bx - btn_sz, btn_y), Pos2::new(bx, btn_y + btn_sz));
        bx -= btn_sz + btn_gap;
        let id = ui.id().with(("dl_btn", rid, tip));
        if icon_action(
            ui,
            &painter,
            th,
            rect,
            id,
            glyph,
            tip,
            glyph == Glyph::Trash,
        ) {
            op = Some(dop);
        }
    }

    // 点击处理(按钮已消费的操作不改变选中)
    if cb_clicked {
        sel = Some(DlSel::Toggle(rid));
    } else if op.is_none() && resp.clicked() {
        if shift {
            sel = Some(DlSel::Range(rid));
        } else if ctrl {
            sel = Some(DlSel::Toggle(rid));
        } else {
            sel = Some(DlSel::Replace(rid));
        }
    }

    (op, sel)
}

/// 渲染目录任务下的一个子目录行(紧凑单行: 缩进 + 箭头 + 文件夹图标 + 子树文件计数),
/// 返回是否点击了展开箭头。`h` 为该行高度。
fn dl_dir_node(
    ui: &mut egui::Ui,
    th: &Theme,
    node: &DlNode,
    folder: u64,
    idx: usize,
    h: f32,
) -> bool {
    let w = ui.available_width().max(320.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, h), egui::Sense::click());
    let painter = ui.painter().clone();
    let inner = rect.shrink2(vec2(12.0, 0.0));
    paint_rails(&painter, th, rect, inner.min.x, node.depth, NODE_GAP);

    // 子目录行用略深的底色作为分组标题。
    let base = mix(th.card, th.text_faint, 0.12);
    let bg = if resp.hovered() {
        mix(base, th.text, if th.dark { 0.05 } else { 0.03 })
    } else {
        base
    };
    let bar = Rect::from_min_max(
        Pos2::new(inner.min.x, rect.min.y),
        Pos2::new(inner.max.x, rect.max.y),
    );
    painter.rect_filled(bar, th.cr(6), bg);

    // 名称对齐到该层级的缩进位; 箭头与图标放在名称左侧的缩进区。
    let left = node_left(inner.min.x, node.depth);
    let ch_rect =
        Rect::from_center_size(Pos2::new(left - 32.0, inner.center().y), vec2(16.0, 16.0));
    let ch_resp = ui.interact(
        ch_rect,
        ui.id().with(("dl_dir_expand", folder, idx)),
        egui::Sense::click(),
    );
    if ch_resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let ch_col = if ch_resp.hovered() {
        th.text
    } else {
        th.text_weak
    };
    paint_disclosure(&painter, ch_rect.shrink(4.0), node.expanded, ch_col);

    let icon_rect =
        Rect::from_center_size(Pos2::new(left - 15.0, inner.center().y), vec2(14.0, 14.0));
    icons::paint(&painter, icon_rect, Glyph::Folder, th.text_weak);

    let right_start = inner.max.x - 10.0;
    let left_w = ((right_start - left - 16.0) * 0.55).max(100.0);
    let name_g = truncate_text(
        &painter,
        &node.name,
        left_w,
        FontId::proportional(12.5),
        th.text,
    );
    painter.galley(
        Pos2::new(left, inner.center().y - name_g.size().y / 2.0),
        name_g,
        th.text,
    );

    let info = if node.files_total == 0 {
        "空".to_string()
    } else {
        format!("{} / {} 个文件", node.files_done, node.files_total)
    };
    painter.text(
        Pos2::new(right_start, inner.center().y),
        egui::Align2::RIGHT_CENTER,
        info,
        FontId::proportional(11.0),
        th.text_faint,
    );

    // 点击整行也可切换展开。
    ch_resp.clicked() || resp.clicked()
}

/// 渲染目录任务下的一个文件行(紧凑单行: 名称 + 状态 + 大小), 只读。
fn dl_file_node(ui: &mut egui::Ui, th: &Theme, node: &DlNode, job: &DlJob, h: f32) {
    let w = ui.available_width().max(320.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, h), egui::Sense::click());
    let painter = ui.painter().clone();
    let inner = rect.shrink2(vec2(12.0, 0.0));
    paint_rails(&painter, th, rect, inner.min.x, node.depth, NODE_GAP);
    if resp.hovered() {
        let hover = mix(th.card, th.text, if th.dark { 0.05 } else { 0.03 });
        painter.rect_filled(rect.shrink2(vec2(8.0, 0.0)), th.cr(6), hover);
    }

    let left = node_left(inner.min.x, node.depth);
    let right_start = inner.max.x - 10.0;
    let left_w = (right_start - left - 150.0).max(100.0);
    let name_g = truncate_text(
        &painter,
        &job.name,
        left_w,
        FontId::proportional(12.5),
        th.text,
    );
    painter.galley(
        Pos2::new(left, inner.center().y - name_g.size().y / 2.0),
        name_g,
        th.text,
    );

    let (col, txt) = file_node_status(job);
    painter.text(
        Pos2::new(right_start, inner.center().y),
        egui::Align2::RIGHT_CENTER,
        txt,
        FontId::proportional(11.0),
        col,
    );
}

/// 子文件行的右侧文案与颜色: 状态 + 大小/速率。
fn file_node_status(job: &DlJob) -> (egui::Color32, String) {
    let green = egui::Color32::from_rgb(70, 150, 90);
    let blue = egui::Color32::from_rgb(60, 130, 200);
    let gray = egui::Color32::from_gray(150);
    let red = egui::Color32::from_rgb(217, 70, 60);
    match &job.status {
        DlStatus::Done => (
            green,
            format!("已完成 · {}", format::fmt_bytes(job.done as i64)),
        ),
        DlStatus::Running => {
            let mut s = String::from("下载中");
            if job.total > 0 {
                let pct = (job.done as f32 / job.total as f32 * 100.0).round() as u32;
                s.push_str(&format!(" · {pct}%"));
            } else if job.done > 0 {
                s.push_str(&format!(" · {}", format::fmt_bytes(job.done as i64)));
            }
            if job.speed > 0 {
                s.push_str(&format!(" · {}/s", format::fmt_bytes(job.speed as i64)));
            }
            (blue, s)
        }
        DlStatus::Queued => (gray, "排队中".to_string()),
        DlStatus::Failed(_) => (red, "失败".to_string()),
    }
}

impl App {
    /// 目录树节点的行高。
    fn node_h(&self, folder: &u64, idx: usize) -> f32 {
        let is_dir = self
            .jobs
            .get(folder)
            .and_then(|j| j.nodes.get(idx))
            .map(|n| n.is_dir)
            .unwrap_or(false);
        if is_dir {
            NODE_DIR_H
        } else {
            NODE_FILE_H
        }
    }

    /// 展开目录整块面板的高度(标题行 + 子行 + 可能的截断提示 + 底部留白)。
    fn tree_block_height(&self, folder: &u64, nodes: &[usize]) -> f32 {
        let shown = nodes.len().min(TREE_CHILD_CAP);
        let mut h = DL_CARD_H + TREE_PAD;
        for idx in &nodes[..shown] {
            h += self.node_h(folder, *idx) + NODE_GAP;
        }
        if nodes.len() > shown {
            h += TREE_HINT_H;
        }
        h
    }
}

/// 渲染单个上传任务卡片, 返回操作和选择请求。
#[allow(clippy::too_many_arguments)]
fn ul_card(
    ui: &mut egui::Ui,
    th: &Theme,
    rid: u64,
    job: &UlJob,
    is_sel: bool,
    ctrl: bool,
    shift: bool,
    now: u64,
) -> (Option<UlOp>, Option<DlSel>) {
    let mut op: Option<UlOp> = None;
    let mut sel: Option<DlSel> = None;
    let w = ui.available_width().max(320.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, DL_CARD_H), egui::Sense::click());
    let painter = ui.painter().clone();

    // 背景 / 选中态
    card_shell(&painter, th, rect, resp.hovered(), is_sel);

    let inner = rect.shrink2(vec2(12.0, 10.0));

    // 复选框
    let cb_rect = Rect::from_center_size(
        Pos2::new(inner.min.x + 8.0, inner.center().y),
        vec2(16.0, 16.0),
    );
    let cb_resp = ui.interact(
        cb_rect,
        ui.id().with(("ul_check", rid)),
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

    const CB_W: f32 = 26.0;
    const BTN_W: f32 = 92.0;
    let content_x = inner.min.x + CB_W;
    let right_start = inner.max.x - BTN_W;
    let left_w = ((right_start - content_x - 16.0) * 0.46).max(120.0);

    // 左列: 名称 / 状态(含相对时间) / 目标路径
    let name_g = truncate_text(
        &painter,
        &job.name,
        left_w,
        FontId::proportional(13.5),
        th.text,
    );
    painter.galley(Pos2::new(content_x, inner.min.y + 1.0), name_g, th.text);
    let (col, mut txt) = upload_status_line(job);
    if job.is_dir && job.files_total > 0 {
        txt = format!("{txt} · 文件 {}/{}", job.files_done, job.files_total);
    }
    if let Some(at) = job.at {
        let rel = format::fmt_rel(now, at);
        if !rel.is_empty() {
            txt = format!("{txt} · {rel}");
        }
    }
    let st_g = truncate_text(&painter, &txt, left_w, FontId::proportional(11.5), col);
    painter.galley(Pos2::new(content_x, inner.min.y + 18.0), st_g, col);
    let dest = job.dest_label();
    if !dest.is_empty() {
        let dg = truncate_text(
            &painter,
            &format!("→ {dest}"),
            left_w,
            FontId::proportional(10.5),
            th.text_faint,
        );
        painter.galley(Pos2::new(content_x, inner.min.y + 35.0), dg, th.text_faint);
    }

    // 中间: 进度 / 速率 / 剩余时间
    let mid_x = content_x + left_w + 14.0;
    let mid_w = (right_start - 14.0 - mid_x).max(0.0);
    if mid_w > 40.0 {
        match &job.status {
            UlStatus::Running if job.total > 0 => {
                let frac = (job.done as f32 / job.total as f32).clamp(0.0, 1.0);
                let bar = Rect::from_min_max(
                    Pos2::new(mid_x, inner.center().y - 10.0),
                    Pos2::new(mid_x + mid_w, inner.center().y + 2.0),
                );
                painter.rect_filled(bar, th.cr(3), mix(th.text_faint, th.bg, 0.7));
                if frac > 0.0 {
                    painter.rect_filled(
                        Rect::from_min_max(bar.min, Pos2::new(bar.min.x + mid_w * frac, bar.max.y)),
                        th.cr(3),
                        th.accent,
                    );
                }
                let mut info = format!(
                    "{} / {}",
                    format::fmt_bytes(job.done as i64),
                    format::fmt_bytes(job.total as i64)
                );
                if job.speed > 0 {
                    info.push_str(&format!("   {}/s", format::fmt_bytes(job.speed as i64)));
                    let eta = format::fmt_eta(job.total.saturating_sub(job.done), job.speed);
                    if !eta.is_empty() {
                        info.push_str(&format!("   剩余 {eta}"));
                    }
                }
                painter.text(
                    Pos2::new(mid_x, inner.center().y + 6.0),
                    egui::Align2::LEFT_TOP,
                    info,
                    FontId::proportional(10.5),
                    th.text_weak,
                );
            }
            UlStatus::Running => {
                let info = if job.is_dir {
                    if job.current.is_empty() {
                        "准备中…".to_string()
                    } else {
                        job.current.clone()
                    }
                } else if job.done > 0 {
                    format!("已上传 {}", format::fmt_bytes(job.done as i64))
                } else {
                    "计算哈希 / 连接中…".to_string()
                };
                painter.text(
                    Pos2::new(mid_x, inner.center().y - 7.0),
                    egui::Align2::LEFT_TOP,
                    info,
                    FontId::proportional(11.5),
                    th.text_weak,
                );
            }
            _ => {
                if job.done > 0 {
                    painter.text(
                        Pos2::new(mid_x, inner.center().y - 7.0),
                        egui::Align2::LEFT_TOP,
                        format!("已上传 {}", format::fmt_bytes(job.done as i64)),
                        FontId::proportional(11.5),
                        th.text_weak,
                    );
                }
            }
        }
    }

    // 图标操作按钮(右对齐)
    let mut btns: Vec<(Glyph, &str, UlOp)> = Vec::new();
    match &job.status {
        UlStatus::Queued | UlStatus::Running => btns.push((Glyph::Close, "取消上传", UlOp::Cancel)),
        UlStatus::Failed(_) => {
            btns.push((Glyph::Refresh, "重试上传", UlOp::Retry));
            btns.push((Glyph::Trash, "从列表移除", UlOp::Remove));
        }
        UlStatus::Done => {
            btns.push((Glyph::Folder, "在网盘中打开", UlOp::OpenInDrive));
            btns.push((Glyph::Trash, "从列表移除", UlOp::Remove));
        }
    }
    let btn_sz = 28.0;
    let btn_gap = 4.0;
    let btn_y = inner.center().y - btn_sz / 2.0;
    let mut bx = inner.max.x;
    for (glyph, tip, dop) in btns.into_iter().rev() {
        let r = Rect::from_min_max(Pos2::new(bx - btn_sz, btn_y), Pos2::new(bx, btn_y + btn_sz));
        bx -= btn_sz + btn_gap;
        let id = ui.id().with(("ul_btn", rid, tip));
        if icon_action(ui, &painter, th, r, id, glyph, tip, glyph == Glyph::Trash) {
            op = Some(dop);
        }
    }

    // 失败原因完整展示
    if let UlStatus::Failed(what) = &job.status {
        if !what.is_empty() {
            resp.clone().on_hover_text(what);
        }
    }

    if cb_clicked {
        sel = Some(DlSel::Toggle(rid));
    } else if op.is_none() && resp.clicked() {
        if shift {
            sel = Some(DlSel::Range(rid));
        } else if ctrl {
            sel = Some(DlSel::Toggle(rid));
        } else {
            sel = Some(DlSel::Replace(rid));
        }
    }

    (op, sel)
}

impl App {
    /// 传输任务页顶部的「上传 / 下载」分栏按钮。
    fn transfer_tab_button(
        &mut self,
        ui: &mut egui::Ui,
        th: &Theme,
        tab: TransferTab,
        label: &str,
    ) {
        let selected = self.transfer_tab == tab;
        let text = RichText::new(label).size(13.0).color(if selected {
            th.on_accent
        } else {
            th.text_weak
        });
        let btn = egui::Button::new(text)
            .fill(if selected {
                th.accent
            } else {
                egui::Color32::TRANSPARENT
            })
            .stroke(Stroke::new(
                1.0,
                if selected { th.accent } else { th.border },
            ))
            .corner_radius(th.cr(8));
        if ui.add(btn).clicked() {
            self.transfer_tab = tab;
        }
    }

    /// 下载列表的状态筛选按钮。
    fn dl_filter_button(&mut self, ui: &mut egui::Ui, th: &Theme, filter: DlFilter, label: &str) {
        let selected = self.dl_filter == filter;
        let text = RichText::new(label).size(12.0).color(if selected {
            th.on_accent
        } else {
            th.text_weak
        });
        let btn = egui::Button::new(text)
            .fill(if selected {
                th.accent
            } else {
                egui::Color32::TRANSPARENT
            })
            .stroke(Stroke::new(
                1.0,
                if selected { th.accent } else { th.border },
            ))
            .corner_radius(th.cr(8));
        if ui.add(btn).clicked() && !selected {
            self.dl_filter = filter;
            // 切换筛选时清空选择, 避免被筛掉的项仍处于选中状态
            self.selected_dl.clear();
            self.last_clicked_dl = None;
        }
    }

    /// 从列表移除一个下载任务(运行中的普通文件先取消; 目录任务连同子文件一起移除)。
    fn remove_download_job(&mut self, rid: u64) {
        let Some((is_folder, kids, rec, running, parent)) = self.jobs.get(&rid).map(|j| {
            (
                j.is_folder(),
                j.file_rids().collect::<Vec<_>>(),
                (
                    j.record_id.clone(),
                    j.file_id.clone(),
                    j.name.clone(),
                    j.dir.clone(),
                ),
                matches!(j.status, DlStatus::Queued | DlStatus::Running),
                j.parent,
            )
        }) else {
            return;
        };
        if is_folder {
            // 目录: 停止运行中的子任务并连同子行一起移除, 同时删除目录历史记录。
            for cid in kids {
                let c_running = self
                    .jobs
                    .get(&cid)
                    .map(|j| matches!(j.status, DlStatus::Queued | DlStatus::Running))
                    .unwrap_or(false);
                if c_running {
                    self.send(Cmd::CancelDownload { req_id: cid });
                }
                self.jobs.remove(&cid);
                self.selected_dl.remove(&cid);
            }
            settings::remove_download_record(&rec.0, &rec.1, &rec.2, &rec.3);
            self.jobs.remove(&rid);
            self.selected_dl.remove(&rid);
            if self.last_clicked_dl == Some(rid) {
                self.last_clicked_dl = None;
            }
            return;
        }
        if let Some(p) = parent {
            // 子文件行(兜底路径): 从父目录的目录树中摘除后刷新聚合。
            if let Some(pj) = self.jobs.get_mut(&p) {
                pj.nodes.retain(|n| n.rid != Some(rid));
            }
            self.jobs.remove(&rid);
            self.selected_dl.remove(&rid);
            if self.last_clicked_dl == Some(rid) {
                self.last_clicked_dl = None;
            }
            self.recompute_folder(p);
            return;
        }
        if running {
            self.send(Cmd::CancelDownload { req_id: rid });
            return;
        }
        settings::remove_download_record(&rec.0, &rec.1, &rec.2, &rec.3);
        self.jobs.remove(&rid);
        self.selected_dl.remove(&rid);
        if self.last_clicked_dl == Some(rid) {
            self.last_clicked_dl = None;
        }
    }

    /// 上传列表的状态筛选按钮。
    fn ul_filter_button(&mut self, ui: &mut egui::Ui, th: &Theme, filter: UlFilter, label: &str) {
        let selected = self.ul_filter == filter;
        let text = RichText::new(label).size(12.0).color(if selected {
            th.on_accent
        } else {
            th.text_weak
        });
        let btn = egui::Button::new(text)
            .fill(if selected {
                th.accent
            } else {
                egui::Color32::TRANSPARENT
            })
            .stroke(Stroke::new(
                1.0,
                if selected { th.accent } else { th.border },
            ))
            .corner_radius(th.cr(8));
        if ui.add(btn).clicked() && !selected {
            self.ul_filter = filter;
            self.selected_ul.clear();
            self.ul_last_clicked = None;
        }
    }

    /// 重试一个上传任务(移除旧行与历史后重新入队)。
    fn retry_upload_job(&mut self, rid: u64) {
        let info = self.ul_jobs.get(&rid).map(|j| {
            (
                j.local_path.clone(),
                j.parent.clone(),
                j.record_id.clone(),
                j.name.clone(),
                j.is_dir,
                j.dest_stack.clone(),
            )
        });
        if let Some((path, parent, rec_id, name, is_dir, stack)) = info {
            settings::remove_upload_record(&rec_id, &path, &name);
            self.ul_jobs.remove(&rid);
            self.selected_ul.remove(&rid);
            if self.ul_last_clicked == Some(rid) {
                self.ul_last_clicked = None;
            }
            if is_dir {
                self.enqueue_upload_dir(path, parent, stack);
            } else {
                self.enqueue_upload(vec![path], parent, stack);
            }
        }
    }

    /// 从列表移除一个上传任务(运行中的取消; 其余删除行与历史记录)。
    fn remove_upload_job(&mut self, rid: u64) {
        let running = self
            .ul_jobs
            .get(&rid)
            .map(|j| matches!(j.status, UlStatus::Queued | UlStatus::Running))
            .unwrap_or(false);
        if running {
            self.send(Cmd::CancelUpload { req_id: rid });
            return;
        }
        if let Some(job) = self.ul_jobs.get(&rid) {
            settings::remove_upload_record(&job.record_id, &job.local_path, &job.name);
        }
        self.ul_jobs.remove(&rid);
        self.selected_ul.remove(&rid);
        if self.ul_last_clicked == Some(rid) {
            self.ul_last_clicked = None;
        }
    }

    /// 在「我的文件」中打开指定层级(stack 为 (id, label) 列表)。
    fn navigate_to_stack(&mut self, stack: Vec<(Option<String>, String)>) {
        if stack.is_empty() {
            return;
        }
        self.stack = stack
            .into_iter()
            .map(|(id, label)| Crumb { id, label })
            .collect();
        self.page = Page::Files;
        self.show_dir();
    }

    /// 传输任务页「上传」分栏。
    fn upload_tab(&mut self, ui: &mut egui::Ui, th: &Theme) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("上传本地文件到当前网盘目录。")
                    .color(th.text_weak)
                    .size(12.5),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add(
                        egui::Button::new(RichText::new("上传文件").color(th.on_accent))
                            .fill(th.accent)
                            .stroke(Stroke::NONE)
                            .corner_radius(th.cr(8)),
                    )
                    .clicked()
                {
                    self.upload_here();
                }
                if ui
                    .add(
                        egui::Button::new(RichText::new("上传文件夹").color(th.text_weak))
                            .stroke(Stroke::new(1.0, th.border))
                            .fill(egui::Color32::TRANSPARENT)
                            .corner_radius(th.cr(8)),
                    )
                    .clicked()
                {
                    self.upload_dir_here();
                }
            });
        });
        ui.add_space(8.0);

        if self.ul_jobs.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.add_space(60.0);
                let (r, _) = ui.allocate_exact_size(vec2(64.0, 64.0), egui::Sense::hover());
                icons::paint(ui.painter(), r, Glyph::Upload, th.text_faint);
                ui.add_space(10.0);
                ui.label(RichText::new("暂无上传任务").color(th.text_weak).size(14.0));
                ui.add_space(4.0);
                ui.label(
                    RichText::new("在「我的文件」中选择文件后上传, 或点右上角「上传文件」")
                        .color(th.text_faint)
                        .size(12.0),
                );
                ui.add_space(60.0);
            });
            return;
        }

        // 筛选 + 汇总 + 全选 + 清除已完成
        let now = format::now_unix();
        let mut clear_done = false;
        ui.horizontal(|ui| {
            let total = self.ul_jobs.len();
            let active = self
                .ul_jobs
                .values()
                .filter(|j| matches!(j.status, UlStatus::Queued | UlStatus::Running))
                .count();
            let done_c = self
                .ul_jobs
                .values()
                .filter(|j| j.status == UlStatus::Done)
                .count();
            let failed = self
                .ul_jobs
                .values()
                .filter(|j| matches!(j.status, UlStatus::Failed(_)))
                .count();
            self.ul_filter_button(ui, th, UlFilter::All, &format!("全部 {total}"));
            self.ul_filter_button(ui, th, UlFilter::Active, &format!("进行中 {active}"));
            self.ul_filter_button(ui, th, UlFilter::Done, &format!("已完成 {done_c}"));
            self.ul_filter_button(ui, th, UlFilter::Failed, &format!("失败 {failed}"));

            let ids = self.visible_ul_ids();
            self.selected_ul.retain(|id| ids.contains(id));
            let vis_total = ids.len();
            let vis_selected = ids
                .iter()
                .filter(|id| self.selected_ul.contains(id))
                .count();
            let master = if vis_total == 0 || vis_selected == 0 {
                CheckState::Unchecked
            } else if vis_selected >= vis_total {
                CheckState::Checked
            } else {
                CheckState::Partial
            };

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if done_c > 0
                    && ui
                        .add(
                            egui::Button::new(RichText::new("清除已完成").color(th.text_weak))
                                .frame(false),
                        )
                        .clicked()
                {
                    clear_done = true;
                }
                ui.add_space(10.0);
                let (cb_rect, cb_resp) =
                    ui.allocate_exact_size(vec2(16.0, 16.0), egui::Sense::click());
                paint_checkbox(ui.painter(), th, cb_rect, master, cb_resp.hovered());
                if cb_resp.clicked() {
                    if master == CheckState::Checked {
                        for id in &ids {
                            self.selected_ul.remove(id);
                        }
                    } else {
                        for id in &ids {
                            self.selected_ul.insert(*id);
                        }
                    }
                }
                ui.add_space(12.0);
                ui.label(RichText::new("全选").color(th.text_weak).size(12.5));
                ui.add_space(12.0);
                if vis_selected > 0 {
                    ui.label(
                        RichText::new(format!("已选 {vis_selected}/{vis_total}"))
                            .color(th.accent)
                            .size(12.5),
                    );
                } else {
                    ui.label(
                        RichText::new(format!("共 {vis_total} 个"))
                            .color(th.text_faint)
                            .size(12.5),
                    );
                }
                ui.add_space(12.0);
                let (sum_done, sum_total, speed) =
                    self.ul_jobs
                        .values()
                        .fold((0u64, 0u64, 0u64), |(d, t, s), j| {
                            let sp = if matches!(j.status, UlStatus::Running) {
                                j.speed
                            } else {
                                0
                            };
                            (d + j.done, t + j.total, s + sp)
                        });
                if sum_total > 0 {
                    let pct = (sum_done as f32 / sum_total as f32 * 100.0).round() as u32;
                    let mut agg = format!(
                        "整体 {pct}% · {}/{}",
                        format::fmt_bytes(sum_done as i64),
                        format::fmt_bytes(sum_total as i64)
                    );
                    if speed > 0 {
                        agg.push_str(&format!(" · {}/s", format::fmt_bytes(speed as i64)));
                    }
                    ui.label(RichText::new(agg).color(th.text_faint).size(12.0));
                }
            });
        });
        ui.add_space(4.0);

        if clear_done {
            let done_ids: Vec<u64> = self
                .ul_jobs
                .iter()
                .filter(|(_, j)| j.status == UlStatus::Done)
                .map(|(id, _)| *id)
                .collect();
            for rid in done_ids {
                self.remove_upload_job(rid);
            }
        }

        let ids = self.visible_ul_ids();
        if ids.is_empty() {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new("该筛选下暂无任务")
                        .color(th.text_weak)
                        .size(13.0),
                );
            });
            return;
        }

        let mut ops: Vec<(u64, UlOp)> = Vec::new();
        let mut sel_reqs: Vec<DlSel> = Vec::new();
        let ctrl = ui.input(|i| i.modifiers.ctrl);
        let shift = ui.input(|i| i.modifiers.shift);
        let scroll_h = ui.available_height();
        egui::ScrollArea::vertical()
            .id_salt("uploads_scroll")
            .auto_shrink([false, false])
            .max_height(scroll_h.max(60.0))
            .show_rows(ui, DL_CARD_H, ids.len(), |ui, range| {
                for i in range {
                    let rid = ids[i];
                    let Some(job) = self.ul_jobs.get(&rid) else {
                        continue;
                    };
                    let is_sel = self.selected_ul.contains(&rid);
                    let (op, sel) = ul_card(ui, th, rid, job, is_sel, ctrl, shift, now);
                    if let Some(op) = op {
                        ops.push((rid, op));
                    }
                    if let Some(sel) = sel {
                        sel_reqs.push(sel);
                    }
                }
            });

        for sel in sel_reqs {
            match sel {
                DlSel::Replace(rid) => {
                    self.selected_ul.clear();
                    self.selected_ul.insert(rid);
                    self.ul_last_clicked = Some(rid);
                }
                DlSel::Toggle(rid) => {
                    if self.selected_ul.contains(&rid) {
                        self.selected_ul.remove(&rid);
                    } else {
                        self.selected_ul.insert(rid);
                    }
                    self.ul_last_clicked = Some(rid);
                }
                DlSel::Range(rid) => {
                    if let Some(anchor) = self.ul_last_clicked {
                        let start = ids.iter().position(|x| *x == anchor).unwrap_or(0);
                        let end = ids.iter().position(|x| *x == rid).unwrap_or(0);
                        let (from, to) = if start <= end {
                            (start, end)
                        } else {
                            (end, start)
                        };
                        if !ctrl {
                            self.selected_ul.clear();
                        }
                        for id in &ids[from..=to] {
                            self.selected_ul.insert(*id);
                        }
                    } else {
                        self.selected_ul.clear();
                        self.selected_ul.insert(rid);
                    }
                    self.ul_last_clicked = Some(rid);
                }
            }
        }

        for (rid, op) in ops {
            match op {
                UlOp::Cancel => self.send(Cmd::CancelUpload { req_id: rid }),
                UlOp::Remove => self.remove_upload_job(rid),
                UlOp::Retry => self.retry_upload_job(rid),
                UlOp::OpenInDrive => {
                    let stack = self.ul_jobs.get(&rid).map(|j| j.dest_stack.clone());
                    if let Some(stack) = stack {
                        self.navigate_to_stack(stack);
                    }
                }
            }
        }
    }

    pub(super) fn transfers_page(&mut self, ctx: &egui::Context, th: &Theme) {
        let mut ops: Vec<(u64, DlOp)> = Vec::new();
        let mut sel_reqs: Vec<DlSel> = Vec::new();
        let mut clear_done = false;

        // 底部批量操作条: 底部面板保证布局高度正确; 用页面同色铺底避免露出窗口
        // 底色, 顶部加一条分隔线, 整体是单层扁平工具条, 不再有嵌套盒子。
        if self.transfer_tab == TransferTab::Download
            && !self.jobs.is_empty()
            && !self.selected_dl.is_empty()
        {
            egui::TopBottomPanel::bottom("dl_action_bar")
                .frame(Frame::new().fill(th.bg).inner_margin(Margin {
                    left: 20,
                    right: 20,
                    top: 0,
                    bottom: 0,
                }))
                .show(ctx, |ui| {
                    let top = ui.max_rect().min.y;
                    ui.painter()
                        .hline(ui.max_rect().x_range(), top, Stroke::new(1.0, th.border));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("已选择 {} 项", self.selected_dl.len()))
                                .size(13.0)
                                .color(th.text),
                        );
                        // 选中项里可重试(已知云端 id 的取消/失败任务)的数量
                        let retryable: Vec<u64> = self
                            .selected_dl
                            .iter()
                            .copied()
                            .filter(|rid| {
                                self.jobs
                                    .get(rid)
                                    .map(|j| {
                                        !j.file_id.is_empty()
                                            && matches!(j.status, DlStatus::Failed(_))
                                    })
                                    .unwrap_or(false)
                            })
                            .collect();
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui
                                .add(
                                    egui::Button::new(RichText::new("移除选中").color(th.danger))
                                        .stroke(Stroke::new(1.0, mix(th.danger, th.bg, 0.35)))
                                        .fill(egui::Color32::TRANSPARENT)
                                        .corner_radius(th.cr(8)),
                                )
                                .clicked()
                            {
                                for rid in &self.selected_dl {
                                    let running = self
                                        .jobs
                                        .get(rid)
                                        .map(|j| {
                                            matches!(j.status, DlStatus::Queued | DlStatus::Running)
                                        })
                                        .unwrap_or(false);
                                    ops.push((
                                        *rid,
                                        if running { DlOp::Cancel } else { DlOp::Remove },
                                    ));
                                }
                            }
                            if !retryable.is_empty()
                                && ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new("重试选中").color(th.on_accent),
                                        )
                                        .fill(th.accent)
                                        .stroke(Stroke::NONE)
                                        .corner_radius(th.cr(8)),
                                    )
                                    .clicked()
                            {
                                for rid in &retryable {
                                    ops.push((*rid, DlOp::Retry));
                                }
                            }
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("取消选中").color(th.text_weak),
                                    )
                                    .frame(false),
                                )
                                .clicked()
                            {
                                self.selected_dl.clear();
                            }
                        });
                    });
                    ui.add_space(8.0);
                });
        }

        // 上传页底部批量操作条(与下载页一致)
        if self.transfer_tab == TransferTab::Upload && !self.selected_ul.is_empty() {
            egui::TopBottomPanel::bottom("ul_action_bar")
                .frame(Frame::new().fill(th.bg).inner_margin(Margin {
                    left: 20,
                    right: 20,
                    top: 0,
                    bottom: 0,
                }))
                .show(ctx, |ui| {
                    let top = ui.max_rect().min.y;
                    ui.painter()
                        .hline(ui.max_rect().x_range(), top, Stroke::new(1.0, th.border));
                    ui.add_space(8.0);
                    let retryable: Vec<u64> = self
                        .selected_ul
                        .iter()
                        .copied()
                        .filter(|rid| {
                            self.ul_jobs
                                .get(rid)
                                .map(|j| matches!(j.status, UlStatus::Failed(_)))
                                .unwrap_or(false)
                        })
                        .collect();
                    let selected: Vec<u64> = self.selected_ul.iter().copied().collect();
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("已选择 {} 项", selected.len()))
                                .size(13.0)
                                .color(th.text),
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui
                                .add(
                                    egui::Button::new(RichText::new("移除选中").color(th.danger))
                                        .stroke(Stroke::new(1.0, mix(th.danger, th.bg, 0.35)))
                                        .fill(egui::Color32::TRANSPARENT)
                                        .corner_radius(th.cr(8)),
                                )
                                .clicked()
                            {
                                for rid in selected.iter().copied() {
                                    self.remove_upload_job(rid);
                                }
                            }
                            if !retryable.is_empty()
                                && ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new("重试选中").color(th.on_accent),
                                        )
                                        .fill(th.accent)
                                        .stroke(Stroke::NONE)
                                        .corner_radius(th.cr(8)),
                                    )
                                    .clicked()
                            {
                                for rid in retryable.iter().copied() {
                                    self.retry_upload_job(rid);
                                }
                            }
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("取消选中").color(th.text_weak),
                                    )
                                    .frame(false),
                                )
                                .clicked()
                            {
                                self.selected_ul.clear();
                                self.ul_last_clicked = None;
                            }
                        });
                    });
                    ui.add_space(8.0);
                });
        }

        egui::CentralPanel::default()
            .frame(Frame::new().fill(th.bg).inner_margin(Margin {
                left: 20,
                right: 20,
                top: 16,
                bottom: 12,
            }))
            .show(ctx, |ui| {
                // 标题行
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(22.0, 22.0), egui::Sense::hover());
                    icons::paint(ui.painter(), r, Glyph::Transfer, th.accent);
                    ui.label(RichText::new("传输任务").size(19.0).strong().color(th.text));
                });
                ui.add_space(10.0);

                // 上传 / 下载 分栏
                ui.horizontal(|ui| {
                    self.transfer_tab_button(ui, th, TransferTab::Upload, "上传");
                    self.transfer_tab_button(ui, th, TransferTab::Download, "下载");
                });
                ui.add_space(10.0);

                // -------- 上传 --------
                if self.transfer_tab == TransferTab::Upload {
                    self.upload_tab(ui, th);
                    return;
                }

                // -------- 下载 --------
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("文件保存在本地下载目录, 可前往「设置」修改。")
                            .color(th.text_weak)
                            .size(12.5),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("打开下载目录").color(th.text_weak),
                                )
                                .stroke(Stroke::new(1.0, th.border))
                                .fill(egui::Color32::TRANSPARENT)
                                .corner_radius(th.cr(8)),
                            )
                            .clicked()
                        {
                            let dir = if !self.download_dir.is_empty()
                                && std::path::Path::new(&self.download_dir).is_dir()
                            {
                                std::path::PathBuf::from(&self.download_dir)
                            } else {
                                dirs::download_dir().unwrap_or_default()
                            };
                            open_dir(&dir);
                        }
                    });
                });
                ui.add_space(8.0);

                // 筛选栏: 状态分段 + 主复选框(全选/全不选) + 计数 (仅有任务时显示)
                let mut dl_ids: Vec<u64> = Vec::new();
                if !self.jobs.is_empty() {
                    ui.horizontal(|ui| {
                        // 仅统计顶层任务(目录已聚合其子文件, 避免重复计数)。
                        let top = self.jobs.values().filter(|j| j.parent.is_none());
                        let total = top.clone().count();
                        let active = top
                            .clone()
                            .filter(|j| matches!(j.status, DlStatus::Queued | DlStatus::Running))
                            .count();
                        let done_c = top.clone().filter(|j| j.status == DlStatus::Done).count();
                        let failed = top
                            .filter(|j| matches!(j.status, DlStatus::Failed(_)))
                            .count();

                        self.dl_filter_button(ui, th, DlFilter::All, &format!("全部 {total}"));
                        self.dl_filter_button(
                            ui,
                            th,
                            DlFilter::Active,
                            &format!("进行中 {active}"),
                        );
                        self.dl_filter_button(ui, th, DlFilter::Done, &format!("已完成 {done_c}"));
                        self.dl_filter_button(ui, th, DlFilter::Failed, &format!("失败 {failed}"));

                        // 当前筛选下可见项
                        dl_ids = self.visible_dl_ids();
                        // 剔除已不在当前筛选中的选中项(如进行中任务完成后被筛掉)
                        self.selected_dl.retain(|id| dl_ids.contains(id));
                        let vis_total = dl_ids.len();
                        let vis_selected = dl_ids
                            .iter()
                            .filter(|id| self.selected_dl.contains(id))
                            .count();
                        let master = if vis_total == 0 || vis_selected == 0 {
                            CheckState::Unchecked
                        } else if vis_selected >= vis_total {
                            CheckState::Checked
                        } else {
                            CheckState::Partial
                        };

                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if done_c > 0
                                && ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new("清除已完成").color(th.text_weak),
                                        )
                                        .frame(false),
                                    )
                                    .clicked()
                            {
                                clear_done = true;
                            }
                            ui.add_space(10.0);
                            let (cb_rect, cb_resp) =
                                ui.allocate_exact_size(vec2(16.0, 16.0), egui::Sense::click());
                            paint_checkbox(ui.painter(), th, cb_rect, master, cb_resp.hovered());
                            if cb_resp.clicked() {
                                if master == CheckState::Checked {
                                    for id in &dl_ids {
                                        self.selected_dl.remove(id);
                                    }
                                } else {
                                    for id in &dl_ids {
                                        self.selected_dl.insert(*id);
                                    }
                                }
                            }
                            ui.add_space(6.0);
                            ui.label(RichText::new("全选").color(th.text_weak).size(12.5));
                            ui.add_space(12.0);
                            if vis_selected > 0 {
                                ui.label(
                                    RichText::new(format!("已选 {vis_selected}/{vis_total}"))
                                        .color(th.accent)
                                        .size(12.5),
                                );
                            } else {
                                ui.label(
                                    RichText::new(format!("共 {vis_total} 个"))
                                        .color(th.text_faint)
                                        .size(12.5),
                                );
                            }
                            ui.add_space(12.0);
                            let (sum_done, sum_total, speed) = self
                                .jobs
                                .values()
                                .filter(|j| j.parent.is_none())
                                .fold((0u64, 0u64, 0u64), |(d, t, s), j| {
                                    let sp = if matches!(j.status, DlStatus::Running) {
                                        j.speed
                                    } else {
                                        0
                                    };
                                    (d + j.done, t + j.total, s + sp)
                                });
                            if sum_total > 0 {
                                let pct =
                                    (sum_done as f32 / sum_total as f32 * 100.0).round() as u32;
                                let mut agg = format!(
                                    "整体 {pct}% · {}/{}",
                                    format::fmt_bytes(sum_done as i64),
                                    format::fmt_bytes(sum_total as i64)
                                );
                                if speed > 0 {
                                    agg.push_str(&format!(
                                        " · {}/s",
                                        format::fmt_bytes(speed as i64)
                                    ));
                                }
                                ui.label(RichText::new(agg).color(th.text_faint).size(12.0));
                            }
                        });
                    });
                    ui.add_space(4.0);
                }

                if self.jobs.is_empty() {
                    ui.centered_and_justified(|ui| {
                        ui.add_space(60.0);
                        let (r, _) = ui.allocate_exact_size(vec2(64.0, 64.0), egui::Sense::hover());
                        icons::paint(ui.painter(), r, Glyph::Download, th.text_faint);
                        ui.add_space(10.0);
                        ui.label(RichText::new("暂无下载任务").color(th.text_weak).size(14.0));
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new("在「我的文件」中选择文件后下载到本地")
                                .color(th.text_faint)
                                .size(12.0),
                        );
                        ui.add_space(60.0);
                    });
                    return;
                }

                // 当前筛选下没有任务
                if dl_ids.is_empty() {
                    ui.add_space(40.0);
                    ui.vertical_centered(|ui| {
                        ui.label(
                            RichText::new("该筛选下暂无任务")
                                .color(th.text_weak)
                                .size(13.0),
                        );
                    });
                    return;
                }

                let ctrl = ui.input(|i| i.modifiers.ctrl);
                let shift = ui.input(|i| i.modifiers.shift);
                let now = format::now_unix();

                // 顺序布局: 由 egui 负责滚动范围与排版(不再手写虚拟滚动, 避免坐标/裁剪问题)。
                // 展开目录把子树包在同一块面板里; 单个目录最多渲染 TREE_CHILD_CAP 个子行。
                let dl_rows = self.dl_rows();
                let scroll_h = ui.available_height();
                egui::ScrollArea::vertical()
                    .id_salt("downloads_scroll")
                    .auto_shrink([false, false])
                    .max_height(scroll_h.max(60.0))
                    .show(ui, |ui| {
                        // 行间距完全由下面的显式 gap 控制。
                        ui.spacing_mut().item_spacing.y = 0.0;
                        let width = ui.available_width().max(320.0);
                        for row in &dl_rows {
                            match row {
                                DlRow::Job(id) => {
                                    if let Some(job) = self.jobs.get(id) {
                                        let is_sel = self.selected_dl.contains(id);
                                        let (op, sel) = dl_card(
                                            ui, th, *id, job, is_sel, ctrl, shift, now, true,
                                        );
                                        if let Some(op) = op {
                                            ops.push((*id, op));
                                        }
                                        if let Some(sel) = sel {
                                            sel_reqs.push(sel);
                                        }
                                    }
                                    ui.add_space(DL_CARD_GAP);
                                }
                                DlRow::Tree(folder, nodes) => {
                                    let Some(job) = self.jobs.get(folder) else {
                                        continue;
                                    };
                                    let shown = nodes.len().min(TREE_CHILD_CAP);
                                    let truncated = nodes.len() > shown;
                                    // 面板高度已知, 先在当前光标处铺底, 再顺序画内容。
                                    let block_h = self.tree_block_height(folder, nodes);
                                    let top = ui.cursor().min;
                                    let block = Rect::from_min_size(
                                        Pos2::new(top.x, top.y),
                                        vec2(width, block_h),
                                    );
                                    let header =
                                        Rect::from_min_size(block.min, vec2(width, DL_CARD_H));
                                    let hovered = ui.rect_contains_pointer(header);
                                    card_shell(
                                        ui.painter(),
                                        th,
                                        block,
                                        hovered,
                                        self.selected_dl.contains(folder),
                                    );
                                    let is_sel = self.selected_dl.contains(folder);
                                    let (op, sel) = dl_card(
                                        ui, th, *folder, job, is_sel, ctrl, shift, now, false,
                                    );
                                    if let Some(op) = op {
                                        ops.push((*folder, op));
                                    }
                                    if let Some(sel) = sel {
                                        sel_reqs.push(sel);
                                    }
                                    for idx in &nodes[..shown] {
                                        let idx = *idx;
                                        let h = self.node_h(folder, idx);
                                        let Some(node) =
                                            self.jobs.get(folder).and_then(|j| j.nodes.get(idx))
                                        else {
                                            ui.add_space(h + NODE_GAP);
                                            continue;
                                        };
                                        if node.is_dir {
                                            if dl_dir_node(ui, th, node, *folder, idx, h) {
                                                ops.push((*folder, DlOp::ToggleDir(idx)));
                                            }
                                        } else if let Some(cid) = node.rid {
                                            if let Some(cjob) = self.jobs.get(&cid) {
                                                dl_file_node(ui, th, node, cjob, h);
                                            }
                                        }
                                        ui.add_space(NODE_GAP);
                                    }
                                    if truncated {
                                        let (r, _) = ui.allocate_exact_size(
                                            vec2(width, TREE_HINT_H),
                                            egui::Sense::hover(),
                                        );
                                        ui.painter().text(
                                            Pos2::new(r.min.x + CB_W + NODE_BASE, r.center().y),
                                            egui::Align2::LEFT_CENTER,
                                            format!("… 仅显示前 {shown} 项(共 {} 项)", nodes.len()),
                                            FontId::proportional(11.0),
                                            th.text_faint,
                                        );
                                    }
                                    ui.add_space(TREE_PAD);
                                    ui.add_space(DL_CARD_GAP);
                                }
                            }
                        }
                    });

                // 处理选择请求（在 CentralPanel 闭包内，确保 dl_ids 和 ctrl 可见）
                for sel in sel_reqs {
                    match sel {
                        DlSel::Replace(rid) => {
                            self.selected_dl.clear();
                            self.selected_dl.insert(rid);
                            self.last_clicked_dl = Some(rid);
                        }
                        DlSel::Toggle(rid) => {
                            if self.selected_dl.contains(&rid) {
                                self.selected_dl.remove(&rid);
                            } else {
                                self.selected_dl.insert(rid);
                            }
                            self.last_clicked_dl = Some(rid);
                        }
                        DlSel::Range(rid) => {
                            if let Some(anchor) = self.last_clicked_dl {
                                let start_idx =
                                    dl_ids.iter().position(|x| *x == anchor).unwrap_or(0);
                                let end_idx = dl_ids.iter().position(|x| *x == rid).unwrap_or(0);
                                let (from, to) = if start_idx <= end_idx {
                                    (start_idx, end_idx)
                                } else {
                                    (end_idx, start_idx)
                                };
                                if !ctrl {
                                    self.selected_dl.clear();
                                }
                                for id in &dl_ids[from..=to] {
                                    self.selected_dl.insert(*id);
                                }
                            } else {
                                self.selected_dl.clear();
                                self.selected_dl.insert(rid);
                            }
                            self.last_clicked_dl = Some(rid);
                        }
                    }
                }
            });

        // 处理操作
        if clear_done {
            let done_ids: Vec<u64> = self
                .jobs
                .iter()
                .filter(|(_, j)| j.parent.is_none() && j.status == DlStatus::Done)
                .map(|(id, _)| *id)
                .collect();
            for rid in done_ids {
                self.remove_download_job(rid);
            }
        }
        for (rid, op) in ops {
            let is_folder = self.jobs.get(&rid).map(|j| j.is_folder()).unwrap_or(false);
            match op {
                DlOp::Expand => {
                    if let Some(j) = self.jobs.get_mut(&rid) {
                        j.expanded = !j.expanded;
                    }
                }
                DlOp::ToggleDir(idx) => {
                    if let Some(j) = self.jobs.get_mut(&rid) {
                        if let Some(n) = j.nodes.get_mut(idx) {
                            n.expanded = !n.expanded;
                        }
                    }
                }
                DlOp::Cancel => {
                    if is_folder {
                        // 目录: 通知子任务停止并连同目录一起移除(不记历史)。
                        self.remove_download_job(rid);
                    } else {
                        self.send(Cmd::CancelDownload { req_id: rid });
                    }
                }
                DlOp::OpenDir => {
                    if let Some(job) = self.jobs.get(&rid) {
                        open_dir(&job.dir);
                    }
                }
                DlOp::OpenFile => {
                    if let Some(job) = self.jobs.get(&rid) {
                        let path = job.dir.join(&job.name);
                        if let Err(e) = open_path(&path) {
                            let msg = format!("打开文件失败: {e}");
                            self.toast_err(&msg);
                        }
                    }
                }
                DlOp::Retry => {
                    if is_folder {
                        self.retry_folder(rid);
                    } else {
                        // 取出旧条目信息后移除旧行, 再重新入队, 避免同一文件被重复重试。
                        let info = self.jobs.get(&rid).and_then(|j| {
                            if j.file_id.is_empty() {
                                None
                            } else {
                                Some((
                                    j.file_id.clone(),
                                    j.name.clone(),
                                    j.dir.clone(),
                                    j.record_id.clone(),
                                ))
                            }
                        });
                        if let Some((file_id, name, dir, rec_id)) = info {
                            self.enqueue_downloads(
                                vec![(file_id.clone(), name.clone())],
                                dir.clone(),
                            );
                            self.remove_download_job(rid);
                            settings::remove_download_record(&rec_id, &file_id, &name, &dir);
                        }
                    }
                }
                DlOp::Remove => {
                    // 仅从列表/历史记录中移除, 不删除本地已下载的文件。
                    self.remove_download_job(rid);
                }
            }
        }
    }

    /// 重试目录下载: 仅重下失败的子文件; 若尚未扫描出子文件(扫描阶段失败)则重新扫描。
    fn retry_folder(&mut self, rid: u64) {
        let Some((folder_id, name, dir, rec_id, file_id, has_files)) =
            self.jobs.get(&rid).map(|j| {
                (
                    j.folder_id.clone(),
                    j.name.clone(),
                    j.dir.clone(),
                    j.record_id.clone(),
                    j.file_id.clone(),
                    j.nodes.iter().any(|n| !n.is_dir),
                )
            })
        else {
            return;
        };
        if !has_files {
            // 扫描阶段失败(尚无文件节点): 重新扫描, 沿用同一 req_id。
            settings::remove_download_record(&rec_id, &file_id, &name, &dir);
            if let Some(j) = self.jobs.get_mut(&rid) {
                j.record_id.clear();
                j.at = None;
                j.status = DlStatus::Queued;
                j.expanded = true;
            }
            if let Some(fid) = folder_id {
                self.send(Cmd::StartDownloadFolder {
                    req_id: rid,
                    folder_id: fid,
                    name,
                    dest_dir: dir,
                });
                self.toast_ok("正在重新扫描目录…");
            }
            return;
        }
        // 收集失败文件所在的节点下标, 只重下这些文件。
        let failed: Vec<(usize, String, String, PathBuf)> = self
            .jobs
            .get(&rid)
            .map(|j| {
                j.nodes
                    .iter()
                    .enumerate()
                    .filter_map(|(i, n)| {
                        let c = self.jobs.get(&n.rid?)?;
                        match &c.status {
                            DlStatus::Failed(_) => {
                                Some((i, c.file_id.clone(), c.name.clone(), c.dir.clone()))
                            }
                            _ => None,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        if failed.is_empty() {
            return;
        }
        for (idx, c_file_id, c_name, c_dir) in failed {
            let new_cid = self.enqueue_download_item(c_file_id, c_name, c_dir, Some(rid));
            let old = {
                let Some(j) = self.jobs.get_mut(&rid) else {
                    continue;
                };
                j.nodes.get_mut(idx).and_then(|n| n.rid.replace(new_cid))
            };
            if let Some(old) = old {
                self.jobs.remove(&old);
                self.selected_dl.remove(&old);
            }
        }
        settings::remove_download_record(&rec_id, &file_id, &name, &dir);
        if let Some(j) = self.jobs.get_mut(&rid) {
            j.record_id.clear();
            j.at = None;
            j.expanded = true;
        }
        self.recompute_folder(rid);
    }
}
