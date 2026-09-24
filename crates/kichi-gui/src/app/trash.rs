//! 「回收站」域: 状态、消息处理、页面渲染与确认弹窗。
//!
//! 页面列出已删除的文件/文件夹, 支持还原 / 彻底删除 / 清空。

use std::collections::HashSet;
use std::time::{Duration, Instant};

use eframe::egui::{
    self, vec2, Align, Align2, FontId, Frame, Layout, Margin, Pos2, Rect, RichText, Stroke,
};

use kichi_core::types::{File, FileList};

use crate::filetypes::file_visual;
use crate::format;
use crate::icons::{self, Glyph};
use crate::msg::Cmd;
use crate::theme::{mix, Theme};

use super::global::Global;
use super::helpers::truncate_text;

/// 回收站列表的缓存新鲜度: 超过该时限进入页面会重新拉取。
const TRASH_TTL: Duration = Duration::from_secs(60);

/// 回收站页面状态。
#[derive(Default)]
pub(crate) struct TrashPage {
    /// 已加载的回收站条目。
    pub(crate) items: Vec<File>,
    /// 下一页 token; None = 已到底。
    next: Option<String>,
    /// 是否正在加载(刷新或加载更多)。
    pub(crate) loading: bool,
    /// 最近一次列表请求的 id, 用于丢弃乱序的旧响应。
    req: u64,
    /// 选中项 id。
    selected: HashSet<String>,
    /// 最近一次成功加载的时间, 用于 SWR 新鲜度判定。
    fetched_at: Option<Instant>,
    /// 彻底删除确认 (id, name); 由确认弹窗消费。
    delete_confirm: Option<Vec<(String, String)>>,
    /// 清空回收站确认框是否打开。
    empty_confirm: bool,
}

impl TrashPage {
    /// 清空状态(退出登录 / 会话失效时调用)。
    pub(crate) fn clear(&mut self) {
        self.items.clear();
        self.next = None;
        self.loading = false;
        self.req = 0;
        self.selected.clear();
        self.fetched_at = None;
        self.delete_confirm = None;
        self.empty_confirm = false;
    }

    /// 有文件刚被删进回收站, 作废列表缓存。
    pub(crate) fn invalidate_cache(&mut self) {
        self.fetched_at = None;
    }

    /// 进入回收站页面: 缓存新鲜则零请求, 否则刷新。
    pub(crate) fn enter(&mut self, g: &mut Global) {
        let fresh = self.fetched_at.is_some_and(|t| t.elapsed() < TRASH_TTL);
        if !fresh {
            self.refresh(g);
        }
    }

    /// 请求刷新首页列表(保留旧数据, 仅置加载态)。
    pub(crate) fn refresh(&mut self, g: &mut Global) {
        self.loading = true;
        self.next = None;
        self.selected.clear();
        self.req += 1;
        let req_id = self.req;
        g.send(Cmd::ListTrash {
            token: None,
            append: false,
            req_id,
        });
    }

    /// 加载下一页。
    pub(crate) fn load_more(&mut self, g: &mut Global) {
        let Some(token) = self.next.clone() else {
            return;
        };
        self.loading = true;
        self.req += 1;
        let req_id = self.req;
        g.send(Cmd::ListTrash {
            token: Some(token),
            append: true,
            req_id,
        });
    }

    /// 选中项 (id, name)。
    fn selected_names(&self) -> Vec<(String, String)> {
        self.items
            .iter()
            .filter(|f| self.selected.contains(&f.id))
            .map(|f| (f.id.clone(), f.name.clone()))
            .collect()
    }

    // ---------- 消息处理 ----------

    /// 列表响应(乱序的旧响应直接丢弃)。
    pub(crate) fn on_list(&mut self, req_id: u64, append: bool, list: FileList) {
        if req_id != self.req {
            return;
        }
        self.loading = false;
        self.fetched_at = Some(Instant::now());
        self.next = list.next_page_token;
        if append {
            let known: HashSet<String> = self.items.iter().map(|f| f.id.clone()).collect();
            for f in list.files {
                if !known.contains(&f.id) {
                    self.items.push(f);
                }
            }
        } else {
            self.items = list.files;
        }
    }

    /// 列表请求失败。
    pub(crate) fn on_failed(&mut self, g: &mut Global, what: String) {
        self.loading = false;
        g.toast_err(&what);
    }

    /// 还原成功。
    pub(crate) fn on_restored(&mut self, g: &mut Global, ids: &[String]) {
        self.items.retain(|f| !ids.contains(&f.id));
        self.selected.retain(|id| !ids.contains(id));
        self.delete_confirm = None;
        self.empty_confirm = false;
        g.send(Cmd::RefreshQuota);
        g.toast_ok(&format!("已还原 {} 项", ids.len()));
    }

    /// 彻底删除成功。
    pub(crate) fn on_deleted(&mut self, g: &mut Global, ids: &[String]) {
        self.items.retain(|f| !ids.contains(&f.id));
        self.selected.retain(|id| !ids.contains(id));
        self.delete_confirm = None;
        self.empty_confirm = false;
        g.send(Cmd::RefreshQuota);
        g.toast_ok(&format!("已彻底删除 {} 项", ids.len()));
    }

    /// 清空完成。
    pub(crate) fn on_emptied(&mut self, g: &mut Global) {
        self.items.clear();
        self.next = None;
        self.selected.clear();
        self.delete_confirm = None;
        self.empty_confirm = false;
        g.send(Cmd::RefreshQuota);
        g.toast_ok("回收站已清空");
    }

    // ---------- 渲染 ----------

    pub(super) fn draw(&mut self, ctx: &egui::Context, g: &mut Global, th: &Theme) {
        let mut refresh = false;
        let mut load_more = false;
        let mut select_all: Option<bool> = None;
        let mut restore_selected = false;
        let mut delete_selected = false;
        let mut empty = false;
        let mut actions: Vec<TrashAction> = Vec::new();

        // 复制列表快照, 避免在渲染闭包里与 self 的可变借用冲突。
        let items = self.items.clone();
        let selected: HashSet<String> = self.selected.clone();
        let has_next = self.next.is_some();
        let loading = self.loading;
        let all_selected = !items.is_empty() && selected.len() >= items.len();
        let selected_count = selected.len();

        egui::CentralPanel::default()
            .frame(Frame::new().fill(th.bg).inner_margin(Margin {
                left: 20,
                right: 20,
                top: 16,
                bottom: 12,
            }))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(22.0, 22.0), egui::Sense::hover());
                    icons::paint(ui.painter(), r, Glyph::Trash, th.accent);
                    ui.label(RichText::new("回收站").size(19.0).strong().color(th.text));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if !items.is_empty() {
                            let mut sel = all_selected;
                            if ui.checkbox(&mut sel, "全选").changed() {
                                select_all = Some(sel);
                            }
                            ui.label(
                                RichText::new(format!("共 {} 个", items.len()))
                                    .color(th.text_faint)
                                    .size(12.0),
                            );
                        }
                        if !items.is_empty()
                            && ui
                                .add(
                                    egui::Button::new(RichText::new("清空回收站").color(th.danger))
                                        .fill(egui::Color32::TRANSPARENT)
                                        .stroke(Stroke::new(1.0, mix(th.danger, th.bg, 0.35)))
                                        .corner_radius(th.cr(8)),
                                )
                                .clicked()
                        {
                            empty = true;
                        }
                        let refreshing = loading && !items.is_empty();
                        let (rr, ico) =
                            ui.allocate_exact_size(vec2(22.0, 22.0), egui::Sense::click());
                        icons::paint(ui.painter(), rr, Glyph::Refresh, th.text_weak);
                        let ico_clicked = !refreshing && ico.clicked();
                        let btn = ui.add_enabled(
                            !refreshing,
                            egui::Button::new(
                                RichText::new(if refreshing {
                                    "正在刷新…"
                                } else {
                                    "刷新"
                                })
                                .color(th.text_weak),
                            )
                            .frame(false),
                        );
                        if btn.clicked() || ico_clicked {
                            refresh = true;
                        }
                        if refreshing {
                            ui.add(egui::Spinner::new().size(14.0).color(th.text_weak));
                        }
                    });
                });
                ui.add_space(4.0);
                ui.label(
                    RichText::new("回收站中的文件可还原到原目录, 或彻底删除(不可恢复)。")
                        .color(th.text_weak)
                        .size(12.5),
                );
                ui.add_space(12.0);

                if loading && items.is_empty() {
                    ui.vertical_centered(|ui| {
                        ui.add_space(60.0);
                        ui.spinner();
                        ui.add_space(8.0);
                        ui.label(RichText::new("正在获取回收站…").color(th.text_weak));
                    });
                    return;
                }

                if items.is_empty() {
                    ui.vertical_centered(|ui| {
                        ui.add_space((ui.available_height() * 0.24).max(50.0));
                        let (r, _) = ui.allocate_exact_size(vec2(56.0, 56.0), egui::Sense::hover());
                        icons::paint(ui.painter(), r, Glyph::Trash, th.text_faint);
                        ui.add_space(10.0);
                        ui.label(RichText::new("回收站为空").color(th.text_weak).size(13.5));
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new("在「我的文件」中删除的文件会出现在这里")
                                .color(th.text_faint)
                                .size(12.0),
                        );
                    });
                    return;
                }

                // 选中批量操作条
                if selected_count > 0 {
                    Frame::new()
                        .fill(th.accent_soft())
                        .corner_radius(th.cr(10))
                        .inner_margin(Margin::symmetric(12, 7))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(format!("已选 {selected_count} 项"))
                                        .color(th.accent)
                                        .size(12.5),
                                );
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new("彻底删除")
                                                    .color(egui::Color32::WHITE),
                                            )
                                            .fill(th.danger)
                                            .stroke(Stroke::NONE)
                                            .corner_radius(th.cr(8)),
                                        )
                                        .clicked()
                                    {
                                        delete_selected = true;
                                    }
                                    if ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new("还原").color(th.on_accent),
                                            )
                                            .fill(th.accent)
                                            .stroke(Stroke::NONE)
                                            .corner_radius(th.cr(8)),
                                        )
                                        .clicked()
                                    {
                                        restore_selected = true;
                                    }
                                    if ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new("取消选择").color(th.text_weak),
                                            )
                                            .frame(false),
                                        )
                                        .clicked()
                                    {
                                        select_all = Some(false);
                                    }
                                });
                            });
                        });
                    ui.add_space(8.0);
                }

                let scroll_h = (ui.available_height() - 44.0).max(80.0);
                egui::ScrollArea::vertical()
                    .id_salt("trash_scroll")
                    .auto_shrink([false, false])
                    .max_height(scroll_h)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        for f in &items {
                            trash_row(ui, th, f, selected.contains(&f.id), &mut actions);
                            ui.add_space(6.0);
                        }

                        if has_next {
                            ui.add_space(4.0);
                            ui.vertical_centered(|ui| {
                                if ui.button("加载更多").clicked() {
                                    load_more = true;
                                }
                            });
                            ui.add_space(2.0);
                        }
                    });
            });

        for a in actions {
            match a {
                TrashAction::Toggle(id) => {
                    if !self.selected.remove(&id) {
                        self.selected.insert(id);
                    }
                }
                TrashAction::Restore(id) => {
                    g.send(Cmd::Untrash { ids: vec![id] });
                }
                TrashAction::Delete(id, name) => {
                    self.delete_confirm = Some(vec![(id, name)]);
                }
            }
        }

        if let Some(true) = select_all {
            self.selected = items.iter().map(|f| f.id.clone()).collect();
        } else if let Some(false) = select_all {
            self.selected.clear();
        }
        if refresh {
            self.refresh(g);
        }
        if load_more {
            self.load_more(g);
        }
        if restore_selected {
            let ids: Vec<String> = items
                .iter()
                .filter(|f| self.selected.contains(&f.id))
                .map(|f| f.id.clone())
                .collect();
            if !ids.is_empty() {
                g.send(Cmd::Untrash { ids });
            }
        }
        if delete_selected {
            let names = self.selected_names();
            if !names.is_empty() {
                self.delete_confirm = Some(names);
            }
        }
        if empty {
            self.empty_confirm = true;
        }
    }

    /// 彻底删除 / 清空回收站的确认弹窗。
    pub(super) fn draw_confirms(&mut self, ctx: &egui::Context, g: &mut Global, th: &Theme) {
        if let Some(items) = self.delete_confirm.clone() {
            let ids: Vec<String> = items.iter().map(|(id, _)| id.clone()).collect();
            let mut confirmed = false;
            let mut close = false;
            egui::Window::new("彻底删除")
                .collapsible(false)
                .resizable(false)
                .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    if items.len() == 1 {
                        ui.label(format!("确定彻底删除「{}」吗?", items[0].1));
                    } else {
                        ui.label(format!("确定彻底删除这 {} 项吗?", items.len()));
                    }
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new("彻底删除后无法恢复。")
                            .color(th.danger)
                            .size(12.0),
                    );
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("彻底删除").color(egui::Color32::WHITE),
                                )
                                .fill(th.danger)
                                .stroke(Stroke::NONE),
                            )
                            .clicked()
                        {
                            confirmed = true;
                            close = true;
                        }
                        if ui.button("取消").clicked() {
                            close = true;
                        }
                    });
                });
            if confirmed {
                g.send(Cmd::DeleteTrash { ids });
                // 等待后台 TrashDeleted 回执后再关闭，避免请求失败时丢失确认框。
            } else if close {
                self.delete_confirm = None;
            }
        }

        if self.empty_confirm {
            let mut confirmed = false;
            let mut close = false;
            egui::Window::new("清空回收站")
                .collapsible(false)
                .resizable(false)
                .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label("确定清空回收站吗?");
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new("回收站中的全部文件将被彻底删除, 无法恢复。")
                            .color(th.danger)
                            .size(12.0),
                    );
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("清空").color(egui::Color32::WHITE),
                                )
                                .fill(th.danger)
                                .stroke(Stroke::NONE),
                            )
                            .clicked()
                        {
                            confirmed = true;
                            close = true;
                        }
                        if ui.button("取消").clicked() {
                            close = true;
                        }
                    });
                });
            if confirmed {
                g.send(Cmd::EmptyTrash);
                // 等待后台 TrashEmptied 回执后再关闭。
            } else if close {
                self.empty_confirm = false;
            }
        }
    }
}

/// 回收站行产生的操作。
enum TrashAction {
    Toggle(String),
    Restore(String),
    Delete(String, String),
}

/// 回收站列表行(复选框 + 类型图标 + 名称/信息 + 还原/彻底删除按钮)。
fn trash_row(
    ui: &mut egui::Ui,
    th: &Theme,
    f: &File,
    is_sel: bool,
    actions: &mut Vec<TrashAction>,
) {
    let row_h = 52.0;
    let w = ui.available_width().max(320.0);
    let (rect, row_resp) = ui.allocate_exact_size(vec2(w, row_h), egui::Sense::click());
    let painter = ui.painter().clone();

    if row_resp.hovered() {
        painter.rect_filled(rect.shrink2(vec2(2.0, 2.0)), th.cr(10), th.hover);
    }

    // 复选框
    let cb_size = 16.0;
    let cb_x = rect.min.x + 10.0;
    let cb_y = rect.center().y - cb_size / 2.0;
    let cb_rect = Rect::from_min_max(
        Pos2::new(cb_x, cb_y),
        Pos2::new(cb_x + cb_size, cb_y + cb_size),
    );
    let cb_resp = ui.interact(
        cb_rect,
        ui.id().with(("trash_cb", f.id.clone())),
        egui::Sense::click(),
    );
    painter.rect_filled(
        cb_rect,
        th.cr(3),
        if is_sel {
            th.accent
        } else {
            egui::Color32::TRANSPARENT
        },
    );
    painter.rect_stroke(
        cb_rect,
        th.cr(3),
        if is_sel {
            Stroke::NONE
        } else {
            Stroke::new(1.5, th.text_faint)
        },
        egui::StrokeKind::Inside,
    );
    if is_sel {
        let check_pts = [
            Pos2::new(cb_x + 3.5, cb_y + cb_size / 2.0),
            Pos2::new(cb_x + 6.5, cb_y + cb_size / 2.0 + 3.0),
            Pos2::new(cb_x + cb_size - 3.0, cb_y + 3.0),
        ];
        painter.add(egui::Shape::line(
            check_pts.to_vec(),
            Stroke::new(2.0, th.on_accent),
        ));
    }
    if cb_resp.clicked() {
        actions.push(TrashAction::Toggle(f.id.clone()));
    }

    // 类型图标
    let (glyph, color) = file_visual(f);
    let icon_rect = Rect::from_center_size(
        Pos2::new(rect.min.x + 46.0, rect.center().y),
        vec2(26.0, 26.0),
    );
    painter.rect_filled(
        icon_rect,
        th.cr(7),
        mix(th.card, color, if th.dark { 0.16 } else { 0.10 }),
    );
    icons::paint(&painter, icon_rect.shrink(4.0), glyph, color);

    // 右侧: 彻底删除(描边) + 还原(实底)
    let btn_h = 26.0;
    let del_label = "彻底删除";
    let del_g =
        painter.layout_no_wrap(del_label.to_string(), FontId::proportional(12.5), th.danger);
    let del_w = del_g.size().x + 24.0;
    let del_rect = Rect::from_min_size(
        Pos2::new(rect.max.x - del_w - 10.0, rect.center().y - btn_h / 2.0),
        vec2(del_w, btn_h),
    );
    let del_resp = ui.interact(
        del_rect,
        ui.id().with(("trash_del", f.id.clone())),
        egui::Sense::click(),
    );
    painter.rect_filled(
        del_rect,
        th.cr(8),
        if del_resp.hovered() {
            mix(th.bg, th.danger, if th.dark { 0.20 } else { 0.10 })
        } else {
            egui::Color32::TRANSPARENT
        },
    );
    painter.rect_stroke(
        del_rect,
        th.cr(8),
        Stroke::new(1.0, mix(th.danger, th.bg, 0.35)),
        egui::StrokeKind::Inside,
    );
    painter.galley(
        Pos2::new(
            del_rect.center().x - del_g.size().x / 2.0,
            del_rect.center().y - del_g.size().y / 2.0,
        ),
        del_g,
        th.danger,
    );
    if del_resp.clicked() {
        actions.push(TrashAction::Delete(f.id.clone(), f.name.clone()));
    }

    let res_label = "还原";
    let res_g = painter.layout_no_wrap(
        res_label.to_string(),
        FontId::proportional(12.5),
        th.on_accent,
    );
    let res_w = res_g.size().x + 28.0;
    let res_rect = Rect::from_min_size(
        Pos2::new(del_rect.min.x - res_w - 8.0, rect.center().y - btn_h / 2.0),
        vec2(res_w, btn_h),
    );
    let res_resp = ui.interact(
        res_rect,
        ui.id().with(("trash_res", f.id.clone())),
        egui::Sense::click(),
    );
    let res_bg = if res_resp.hovered() {
        mix(th.accent, th.text, 0.14)
    } else {
        th.accent
    };
    painter.rect_filled(res_rect, th.cr(8), res_bg);
    painter.galley(
        Pos2::new(
            res_rect.center().x - res_g.size().x / 2.0,
            res_rect.center().y - res_g.size().y / 2.0,
        ),
        res_g,
        th.on_accent,
    );
    if res_resp.clicked() {
        actions.push(TrashAction::Restore(f.id.clone()));
    }

    // 名称 + 元信息(图标与按钮之间)
    let text_x = rect.min.x + 66.0;
    let text_w = (res_rect.min.x - 14.0 - text_x).max(50.0);
    let name_g = truncate_text(
        &painter,
        &f.name,
        text_w,
        FontId::proportional(14.0),
        th.text,
    );
    painter.galley(
        Pos2::new(text_x, rect.center().y - name_g.size().y / 2.0 - 8.0),
        name_g,
        th.text,
    );

    let mut meta = if f.is_folder() {
        "文件夹".to_string()
    } else {
        format::fmt_bytes(f.size)
    };
    let t = f
        .delete_time
        .as_deref()
        .or(f.modified_time.as_deref())
        .or(f.created_time.as_deref());
    if let Some(t) = t {
        meta.push_str(&format!(" · {}", format::fmt_time(t)));
    }
    let meta_g = truncate_text(
        &painter,
        &meta,
        text_w,
        FontId::proportional(11.5),
        th.text_weak,
    );
    painter.galley(
        Pos2::new(text_x, rect.center().y + 4.0),
        meta_g,
        th.text_weak,
    );
}
