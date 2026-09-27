//! 顶部栏: 面包屑导航、搜索框、视图切换与操作按钮(选中模式改为就地操作)。

use eframe::egui::{
    self, vec2, Align, FontId, Frame, Layout, Margin, Pos2, Rect, RichText, Stroke,
};

use crate::filetypes::{self, FileType, PreviewKind};
use crate::icons::{self, Glyph};
use crate::theme::{mix, Theme};

use super::super::helpers::truncate_text;
use super::super::preview::PreviewPage;
use super::super::preview::QualityMenuState;
use super::super::search::SearchPage;
use super::super::Global;
use super::row;
use super::FilesPage;
use super::{ClipKind, Crumb, RowAction, ViewMode};

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
    // 面包屑用 sep 手动控制层级间距, 这里关掉 egui 的 item_spacing(x):
    // `allocate_exact_size` 会在每个矩形后追加一次 item_spacing, 否则实际占用宽度
    // 会比上面的估算每项多出一个 item_spacing, 长目录名时就会顶到右侧控件。
    let prev_spacing = ui.spacing().item_spacing.x;
    ui.spacing_mut().item_spacing.x = 0.0;
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

    // 当前目录仍放不下时单独截断; 若前面还有被省略的层级, 需为「…」和分隔符留出宽度,
    // 否则面包屑会超出 budget, 把后面的计数挤进右侧控件。
    if start == n - 1 && widths[n - 1] > budget {
        let ell_reserve = if start > 0 { ell_w + sep } else { 0.0 };
        let w = (budget - pad * 2.0 - ell_reserve).max(24.0);
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
    ui.spacing_mut().item_spacing.x = prev_spacing;
    clicked
}

/// KDE 风格弹出菜单项列表: 常态无边框(避免「框里套框」), 仅悬停时整行填充高亮;
/// 宽度按最长项自适应并封顶 `cap`(超长文字截断)。返回被点击项的索引。
fn popup_menu(ui: &mut egui::Ui, th: &Theme, labels: &[String], cap: f32) -> Option<usize> {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let text_w = labels
        .iter()
        .map(|l| {
            ui.painter()
                .layout_no_wrap(l.clone(), font.clone(), egui::Color32::WHITE)
                .size()
                .x
        })
        .fold(0.0f32, f32::max);
    let pad = ui.spacing().button_padding.x;
    ui.set_width((text_w + pad * 2.0 + 2.0).min(cap));
    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
    ui.style_mut().visuals.widgets.hovered.weak_bg_fill = th.hover;
    ui.style_mut().visuals.widgets.hovered.bg_stroke = egui::Stroke::NONE;
    ui.style_mut().visuals.widgets.active.weak_bg_fill = th.accent_soft();
    ui.style_mut().visuals.widgets.active.bg_stroke = egui::Stroke::NONE;

    let mut clicked = None;
    for (i, label) in labels.iter().enumerate() {
        if ui.selectable_label(false, label.as_str()).clicked() {
            clicked = Some(i);
        }
    }
    clicked
}

/// 顶部栏渲染上下文: 本帧已算好的选中统计 + 各域句柄(域方法只收 `&mut Global`)。
pub(super) struct TopBar<'a> {
    pub th: &'a Theme,
    pub g: &'a mut Global,
    pub search: &'a mut SearchPage,
    pub preview: &'a PreviewPage,
    pub visible_total: usize,
    pub sel_meta: &'a [(String, String)],
    pub dl_files: &'a [(String, String)],
    pub dl_folders: &'a [(String, String)],
    pub clip_info: Option<(String, usize)>,
    pub actions: &'a mut Vec<RowAction>,
}

/// 顶部栏本帧的界面请求, 由 `show` 在渲染结束后统一处理。
#[derive(Default)]
pub(super) struct TopBarReq {
    pub up: bool,
    pub jumped: Option<usize>,
    pub upload: bool,
    pub upload_dir: bool,
    pub refresh: bool,
    pub clear_clip: bool,
    pub ask_rename: bool,
    pub ask_preview: bool,
    pub ask_trash: bool,
    pub ask_share: bool,
    pub want_download: bool,
}

impl FilesPage {
    /// 顶部栏: 面包屑 + 搜索 + 操作(或选中模式下的就地操作)。
    pub(super) fn top_bar(&mut self, ctx: &egui::Context, tb: TopBar<'_>) -> TopBarReq {
        let mut req = TopBarReq::default();
        // -------- 顶部: 面包屑 + 搜索 + 操作 --------
        egui::TopBottomPanel::top("file_head")
            .frame(Frame::new().fill(tb.th.bg).inner_margin(Margin {
                left: 20,
                right: 20,
                top: 14,
                bottom: 10,
            }))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.set_min_height(Self::HEAD_H);

                    // 布局顺序: 先在整行内从右往左排右侧控件, 再把左侧(返回 / 面包屑 /
                    // 计数 / 剪贴板)嵌在其后占用剩余宽度。右侧按实际宽度占位, 窄窗口下
                    // 不会像固定预留宽度那样反向溢出盖住面包屑。
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if tb.sel_meta.is_empty() {
                            // ---- 普通模式: 搜索 + 刷新 + 上传 + 视图 ----
                            let focused = ui.memory(|m| m.has_focus(egui::Id::new("file_search")));
                            egui::Frame::new()
                                .fill(tb.th.card)
                                .stroke(Stroke::new(
                                    1.0,
                                    if focused { tb.th.accent } else { tb.th.border },
                                ))
                                .corner_radius(tb.th.cr(9))
                                .inner_margin(Margin::symmetric(10, 5))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        let (ir, _) = ui.allocate_exact_size(
                                            vec2(15.0, 15.0),
                                            egui::Sense::hover(),
                                        );
                                        icons::paint(
                                            ui.painter(),
                                            ir,
                                            Glyph::Search,
                                            tb.th.text_faint,
                                        );
                                        ui.add_space(1.0);
                                        let _search_response = ui.add(
                                            egui::TextEdit::singleline(&mut self.filter)
                                                .id(egui::Id::new("file_search"))
                                                .frame(false)
                                                .desired_width(150.0)
                                                .hint_text(if tb.search.is_active() {
                                                    "搜索中..."
                                                } else {
                                                    "按 Enter 全局搜索"
                                                })
                                                .font(FontId::proportional(13.5)),
                                        );
                                        if !self.filter.is_empty() {
                                            let (xr, xresp) = ui.allocate_exact_size(
                                                vec2(14.0, 14.0),
                                                egui::Sense::click(),
                                            );
                                            icons::paint(
                                                ui.painter(),
                                                xr.shrink(2.5),
                                                Glyph::Close,
                                                if xresp.hovered() {
                                                    tb.th.accent
                                                } else {
                                                    tb.th.text_faint
                                                },
                                            );
                                            if xresp.clicked() {
                                                self.exit_search(tb.search);
                                            }
                                        }
                                    });
                                });
                            ui.add_space(6.0);

                            // 刷新
                            let (rr, rresp) =
                                ui.allocate_exact_size(vec2(30.0, 30.0), egui::Sense::click());
                            ui.painter().rect_filled(
                                rr,
                                tb.th.cr(8),
                                if rresp.hovered() {
                                    tb.th.hover
                                } else {
                                    egui::Color32::TRANSPARENT
                                },
                            );
                            icons::paint(
                                ui.painter(),
                                rr.shrink(7.0),
                                Glyph::Refresh,
                                tb.th.text_weak,
                            );
                            if rresp.clicked() {
                                req.refresh = true;
                            }
                            rresp.on_hover_text("刷新 (F5)");

                            ui.add_space(4.0);
                            // 上传(图标按钮 + 下拉菜单: 上传文件 / 上传文件夹)
                            let (ur, uresp) =
                                ui.allocate_exact_size(vec2(30.0, 30.0), egui::Sense::click());
                            ui.painter().rect_filled(
                                ur,
                                tb.th.cr(8),
                                if uresp.hovered() {
                                    tb.th.hover
                                } else {
                                    egui::Color32::TRANSPARENT
                                },
                            );
                            icons::paint(
                                ui.painter(),
                                ur.shrink(7.0),
                                Glyph::Upload,
                                tb.th.text_weak,
                            );
                            let upload_menu_id = ui.make_persistent_id("file_upload_menu");
                            if uresp.clicked() {
                                ui.memory_mut(|m| m.toggle_popup(upload_menu_id));
                            }
                            egui::popup_below_widget(
                                ui,
                                upload_menu_id,
                                &uresp,
                                egui::PopupCloseBehavior::CloseOnClick,
                                |ui| {
                                    let labels = ["上传文件".to_string(), "上传文件夹".to_string()];
                                    match popup_menu(ui, tb.th, &labels, 360.0) {
                                        Some(0) => req.upload = true,
                                        Some(_) => req.upload_dir = true,
                                        None => {}
                                    }
                                },
                            );
                            uresp.on_hover_text("上传");

                            ui.add_space(6.0);
                            // 视图切换
                            let icon_bg = |active: bool, hovered: bool| {
                                if active {
                                    tb.th.accent_soft()
                                } else if hovered {
                                    tb.th.hover
                                } else {
                                    egui::Color32::TRANSPARENT
                                }
                            };
                            // 图标视图: 2x2 网格
                            let (ri, ri_resp) =
                                ui.allocate_exact_size(vec2(28.0, 28.0), egui::Sense::click());
                            ui.painter().rect_filled(
                                ri,
                                tb.th.cr(6),
                                icon_bg(self.view_mode == ViewMode::Icon, ri_resp.hovered()),
                            );
                            let p = ui.painter();
                            let s = 3.0;
                            let gap = 2.0;
                            let cx = ri.center().x;
                            let cy = ri.center().y;
                            let color = if self.view_mode == ViewMode::Icon {
                                tb.th.accent
                            } else {
                                tb.th.text_weak
                            };
                            for dx in [-(s + gap / 2.0), s + gap / 2.0] {
                                for dy in [-(s + gap / 2.0), s + gap / 2.0] {
                                    p.rect_filled(
                                        Rect::from_center_size(
                                            Pos2::new(cx + dx, cy + dy),
                                            vec2(s, s),
                                        ),
                                        tb.th.cr(1),
                                        color,
                                    );
                                }
                            }
                            if ri_resp.clicked() {
                                self.view_mode = ViewMode::Icon;
                            }
                            ri_resp.on_hover_text("图标视图");

                            // 列表视图: 三条横线
                            let (li, li_resp) =
                                ui.allocate_exact_size(vec2(28.0, 28.0), egui::Sense::click());
                            ui.painter().rect_filled(
                                li,
                                tb.th.cr(6),
                                icon_bg(self.view_mode == ViewMode::List, li_resp.hovered()),
                            );
                            let p = ui.painter();
                            let color = if self.view_mode == ViewMode::List {
                                tb.th.accent
                            } else {
                                tb.th.text_weak
                            };
                            let lx = li.center().x - 5.0;
                            let lw = 10.0;
                            for dy in [-3.5, 0.0, 3.5] {
                                p.line_segment(
                                    [
                                        Pos2::new(lx, li.center().y + dy),
                                        Pos2::new(lx + lw, li.center().y + dy),
                                    ],
                                    Stroke::new(1.5, color),
                                );
                            }
                            if li_resp.clicked() {
                                self.view_mode = ViewMode::List;
                            }
                            li_resp.on_hover_text("列表视图");
                        } else {
                            // ---- 选中模式: 就地显示操作, 不新增行/不改变列表位置 ----
                            let single = tb.sel_meta.len() == 1;
                            let open_sel = if single && !tb.dl_files.is_empty() {
                                let (id, name) = tb.sel_meta[0].clone();
                                let ft = self.file_type(&id, &name);
                                // 只下载类不给入口(双击由 open_preview 兜底提示)。
                                (filetypes::preview_kind(ft) != PreviewKind::DownloadOnly)
                                    .then_some((ft, id, name))
                            } else {
                                None
                            };

                            // 取消(最右)
                            if ui
                                .add(egui::Button::new(
                                    RichText::new("取消").color(tb.th.text_weak),
                                ))
                                .clicked()
                            {
                                self.selected.clear();
                            }
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("移入回收站").color(tb.th.danger),
                                    )
                                    .stroke(Stroke::new(1.0, mix(tb.th.danger, tb.th.bg, 0.35)))
                                    .fill(egui::Color32::TRANSPARENT)
                                    .corner_radius(tb.th.cr(8)),
                                )
                                .clicked()
                            {
                                req.ask_trash = true;
                            }
                            if self.clipboard.is_some()
                                && ui
                                    .add(egui::Button::new(
                                        RichText::new("粘贴").color(tb.th.text_weak),
                                    ))
                                    .clicked()
                            {
                                self.paste_clipboard(tb.g);
                            }
                            if ui
                                .add(egui::Button::new(
                                    RichText::new("剪切").color(tb.th.text_weak),
                                ))
                                .clicked()
                            {
                                self.clip_selection(tb.g, ClipKind::Cut);
                            }
                            if ui
                                .add(egui::Button::new(
                                    RichText::new("复制").color(tb.th.text_weak),
                                ))
                                .clicked()
                            {
                                self.clip_selection(tb.g, ClipKind::Copy);
                            }
                            if ui
                                .add(egui::Button::new(
                                    RichText::new("分享").color(tb.th.text_weak),
                                ))
                                .clicked()
                            {
                                req.ask_share = true;
                            }
                            if single
                                && ui
                                    .add(egui::Button::new(
                                        RichText::new("重命名").color(tb.th.text_weak),
                                    ))
                                    .clicked()
                            {
                                req.ask_rename = true;
                            }
                            if let Some((ft, id, name)) = open_sel {
                                if ft == FileType::Video {
                                    let quality = match tb.preview.quality(&id) {
                                        Some(r) => QualityMenuState::Ready(r),
                                        None => QualityMenuState::Loading,
                                    };
                                    row::play_menu(ui, &id, &name, quality, tb.actions);
                                } else {
                                    let label = if ft == FileType::Audio {
                                        "播放"
                                    } else {
                                        "打开"
                                    };
                                    if ui
                                        .add(egui::Button::new(
                                            RichText::new(label).color(tb.th.text_weak),
                                        ))
                                        .clicked()
                                    {
                                        req.ask_preview = true;
                                    }
                                }
                            }
                            if (!tb.dl_files.is_empty() || !tb.dl_folders.is_empty())
                                && ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new("下载到本地").color(tb.th.on_accent),
                                        )
                                        .fill(tb.th.accent)
                                        .stroke(Stroke::NONE)
                                        .corner_radius(tb.th.cr(8)),
                                    )
                                    .clicked()
                            {
                                req.want_download = true;
                            }
                        }

                        // ---- 左侧: 返回上级 + 面包屑 + 计数 + 剪贴板 ----
                        // 嵌在右侧控件之后, 占用右侧之外的真实剩余宽度。
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                            let at_root = self.stack.len() <= 1;
                            let (r, rresp) =
                                ui.allocate_exact_size(vec2(30.0, 30.0), egui::Sense::click());
                            ui.painter().rect_filled(
                                r,
                                tb.th.cr(8),
                                if rresp.hovered() && !at_root {
                                    tb.th.hover
                                } else {
                                    egui::Color32::TRANSPARENT
                                },
                            );
                            icons::paint(
                                ui.painter(),
                                r.shrink(6.0),
                                Glyph::Up,
                                if at_root {
                                    tb.th.text_faint
                                } else {
                                    tb.th.text_weak
                                },
                            );
                            if rresp.clicked() && !at_root {
                                req.up = true;
                            }
                            rresp.clone().on_hover_text("返回上级");
                            ui.add_space(4.0);

                            // 计数文案宽度按实际测量预留(选中态文案更长), 避免长目录名
                            // 把计数挤到右侧控件上。
                            let count_text = if tb.sel_meta.is_empty() {
                                format!("· {} 项", tb.visible_total)
                            } else {
                                format!("· 已选 {}/{} 项", tb.sel_meta.len(), tb.visible_total)
                            };
                            let count_w = ui
                                .painter()
                                .layout_no_wrap(
                                    count_text.clone(),
                                    FontId::proportional(12.5),
                                    tb.th.text_faint,
                                )
                                .size()
                                .x;
                            let clip_reserve = if tb.clip_info.is_some() { 150.0 } else { 0.0 };
                            let crumbs_budget =
                                (ui.available_width() - count_w - 8.0 - clip_reserve).max(48.0);

                            // 搜索模式指示器
                            if tb.search.is_active() {
                                egui::Frame::new()
                                    .fill(tb.th.accent_soft())
                                    .corner_radius(tb.th.cr(7))
                                    .inner_margin(Margin::symmetric(8, 3))
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            ui.label(
                                                RichText::new(format!(
                                                    "搜索: {}",
                                                    tb.search.keyword()
                                                ))
                                                .color(tb.th.accent)
                                                .size(12.0),
                                            );
                                        });
                                    });
                                ui.add_space(6.0);
                            } else if let Some(i) =
                                breadcrumbs(ui, tb.th, &self.stack, crumbs_budget)
                            {
                                req.jumped = Some(i);
                            }

                            if tb.sel_meta.is_empty() {
                                ui.label(
                                    RichText::new(count_text).color(tb.th.text_faint).size(12.5),
                                );
                            } else {
                                ui.label(RichText::new(count_text).color(tb.th.accent).size(12.5));
                            }

                            // 剪贴板 chip(只显示数量, 避免文件名过长溢出)
                            if let Some((_label, n)) = &tb.clip_info {
                                ui.add_space(8.0);
                                egui::Frame::new()
                                    .fill(tb.th.accent_soft())
                                    .corner_radius(tb.th.cr(7))
                                    .inner_margin(Margin::symmetric(8, 3))
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            ui.label(
                                                RichText::new(format!("剪贴板 · {n} 项"))
                                                    .color(tb.th.accent)
                                                    .size(11.5),
                                            );
                                            let (xr, xresp) = ui.allocate_exact_size(
                                                vec2(14.0, 14.0),
                                                egui::Sense::click(),
                                            );
                                            icons::paint(
                                                ui.painter(),
                                                xr.shrink(2.5),
                                                Glyph::Close,
                                                if xresp.hovered() {
                                                    tb.th.accent
                                                } else {
                                                    tb.th.text_faint
                                                },
                                            );
                                            if xresp.clicked() {
                                                req.clear_clip = true;
                                            }
                                        });
                                    });
                            }
                        });
                    });
                });
            });
        req
    }
}
