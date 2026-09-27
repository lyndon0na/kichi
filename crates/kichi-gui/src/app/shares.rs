//! 「我的分享」与「转存分享」域: 状态、消息处理、页面渲染与相关弹窗。
//!
//! 我的分享: 列出已创建的分享, 支持复制链接 / 打开 / 批量取消 / 分页加载;
//! 转存分享: 解析他人分享链接, 选文件与目标目录后转存到自己的网盘。

use std::collections::HashSet;
use std::time::{Duration, Instant};

use eframe::egui::{
    self, vec2, Align, Align2, Color32, Frame, Key, Layout, Margin, RichText, Stroke,
};

use kichi_core::types::{File, Share, ShareList};

use crate::format;
use crate::icons::{self, Glyph};
use crate::msg::Cmd;
use crate::theme::{mix, Theme};

use super::files::Crumb;
use super::global::Global;
use super::helpers::{self, folder_row, input};
use super::types::ShareResult;

/// 「我的分享」列表新鲜期: 进入页面时命中则不发请求。
const SHARES_TTL: Duration = Duration::from_secs(60);

/// 分享页面状态(我的分享列表 + 创建分享弹窗 + 转存分享)。
pub(crate) struct SharesPage {
    // 我的分享
    list: Vec<Share>,
    next: Option<String>,
    /// 列表是否正在加载(刷新或加载更多)。
    pub(crate) loading: bool,
    /// 最近一次列表请求的 id, 用于丢弃乱序的旧响应。
    req: u64,
    /// 选中项 share_id。
    selected: HashSet<String>,
    /// 最近一次成功加载分享列表的时间, 用于 SWR 新鲜度判定。
    fetched_at: Option<Instant>,
    /// 待创建分享的选中项 (id, name); Some 表示「创建分享」设置框打开。
    create_dialog: Option<Vec<(String, String)>>,
    expiration_days: i64,
    need_password: bool,
    /// 分享创建成功后的结果框。
    result: Option<ShareResult>,
    /// 取消分享确认 (share_id, 标题)。
    delete_confirm: Option<Vec<(String, String)>>,

    // 转存分享
    /// 转存弹窗是否打开。
    save_open: bool,
    /// 用户输入的分享链接或分享 ID。
    save_input: String,
    /// 用户输入的提取码。
    save_pass_code: String,
    /// 是否正在解析分享链接。
    save_resolving: bool,
    /// 解析成功后的分享 ID。
    save_id: Option<String>,
    /// 解析成功后的分享标题。
    save_title: Option<String>,
    /// 解析成功后的 pass_code_token(转存时需要)。
    save_token: Option<String>,
    /// 解析出的文件列表。
    save_files: Vec<File>,
    /// 用户选中的文件 id。
    save_selected: HashSet<String>,
    /// 文件名搜索过滤。
    save_filter: String,
    /// 分享文件列表分页 token。
    save_next: Option<String>,
    /// 是否正在加载更多文件。
    save_loading_more: bool,
    /// 是否正在转存。
    save_saving: bool,
    /// 解析或转存的错误信息。
    save_error: Option<String>,
    /// 转存目标目录选择器是否打开。
    picker_open: bool,
    /// 转存目标目录选择器的面包屑导航。
    picker_stack: Vec<Crumb>,
    /// 转存目标目录选择器当前目录的子文件夹。
    picker_folders: Vec<File>,
    /// 转存目标目录选择器加载状态。
    picker_loading: bool,
    /// 转存目标目录选择器请求 ID。
    picker_req: u64,
    /// 用户选择的转存目标目录 (id, name); None 表示默认位置。
    save_dest: Option<(String, String)>,
    /// 自动移动失败时的目标目录信息, 用于重试。
    save_move_failed: Option<(String, String)>,
}

impl Default for SharesPage {
    fn default() -> Self {
        Self {
            list: Vec::new(),
            next: None,
            loading: false,
            req: 0,
            selected: HashSet::new(),
            fetched_at: None,
            create_dialog: None,
            // -1 = 永久有效。
            expiration_days: -1,
            need_password: false,
            result: None,
            delete_confirm: None,
            save_open: false,
            save_input: String::new(),
            save_pass_code: String::new(),
            save_resolving: false,
            save_id: None,
            save_title: None,
            save_token: None,
            save_files: Vec::new(),
            save_selected: HashSet::new(),
            save_filter: String::new(),
            save_next: None,
            save_loading_more: false,
            save_saving: false,
            save_error: None,
            picker_open: false,
            picker_stack: vec![Crumb {
                id: None,
                label: "我的云盘".to_string(),
            }],
            picker_folders: Vec::new(),
            picker_loading: false,
            picker_req: 0,
            save_dest: None,
            save_move_failed: None,
        }
    }
}

impl SharesPage {
    // ---------- 我的分享: 状态 ----------

    /// 清空我的分享相关状态(退出登录 / 会话失效时调用)。
    pub(crate) fn clear(&mut self) {
        self.list.clear();
        self.next = None;
        self.loading = false;
        self.req = 0;
        self.selected.clear();
        self.fetched_at = None;
        self.create_dialog = None;
        self.result = None;
        self.delete_confirm = None;
    }

    /// 进入「我的分享」页面: 缓存新鲜则零请求, 否则刷新。
    pub(crate) fn enter(&mut self, g: &mut Global) {
        let fresh = self.fetched_at.is_some_and(|t| t.elapsed() < SHARES_TTL);
        if !fresh {
            self.refresh(g);
        }
    }

    /// 请求刷新「我的分享」首页列表(保留旧数据, 仅置加载态)。
    pub(crate) fn refresh(&mut self, g: &mut Global) {
        self.loading = true;
        self.next = None;
        self.selected.clear();
        self.req += 1;
        let req_id = self.req;
        g.send(Cmd::ListShares {
            token: None,
            append: false,
            req_id,
        });
    }

    /// 加载分享列表下一页。
    pub(crate) fn load_more(&mut self, g: &mut Global) {
        let Some(token) = self.next.clone() else {
            return;
        };
        self.loading = true;
        self.req += 1;
        let req_id = self.req;
        g.send(Cmd::ListShares {
            token: Some(token),
            append: true,
            req_id,
        });
    }

    /// 打开「创建分享」设置框, targets 为 (id, name)。
    pub(crate) fn open_dialog(&mut self, targets: Vec<(String, String)>) {
        if targets.is_empty() {
            return;
        }
        self.create_dialog = Some(targets);
    }

    // ---------- 我的分享: 消息 ----------

    /// 分享创建成功: 记下结果并作废列表缓存(是否立即刷新由调用方按页面判断)。
    pub(crate) fn on_created(
        &mut self,
        url: String,
        pass_code: String,
        share_text: String,
        label: String,
    ) {
        self.result = Some(ShareResult {
            url,
            pass_code,
            share_text,
            label,
        });
        self.fetched_at = None;
    }

    /// 列表响应(乱序的旧响应直接丢弃)。
    pub(crate) fn on_list(&mut self, req_id: u64, append: bool, list: ShareList) {
        if req_id != self.req {
            return;
        }
        self.loading = false;
        self.fetched_at = Some(Instant::now());
        self.next = list.next_page_token;
        if append {
            let known: HashSet<String> = self.list.iter().map(|s| s.share_id.clone()).collect();
            for s in list.shares {
                if !known.contains(&s.share_id) {
                    self.list.push(s);
                }
            }
        } else {
            self.list = list.shares;
        }
    }

    /// 列表请求失败。
    pub(crate) fn on_list_failed(&mut self, g: &mut Global, what: String) {
        self.loading = false;
        g.toast_err(&what);
    }

    /// 取消分享成功。
    pub(crate) fn on_deleted(&mut self, g: &mut Global, ids: &[String]) {
        self.list.retain(|s| !ids.contains(&s.share_id));
        self.selected.retain(|id| !ids.contains(id));
        self.delete_confirm = None;
        g.toast_ok(&format!("已取消 {} 个分享", ids.len()));
    }

    // ---------- 转存分享: 状态 ----------

    /// 清空转存分享弹窗状态。
    pub(crate) fn clear_save(&mut self) {
        self.save_input.clear();
        self.save_pass_code.clear();
        self.save_resolving = false;
        self.save_id = None;
        self.save_title = None;
        self.save_token = None;
        self.save_files.clear();
        self.save_selected.clear();
        self.save_filter.clear();
        self.save_next = None;
        self.save_loading_more = false;
        self.save_saving = false;
        self.save_error = None;
        self.picker_open = false;
        self.picker_stack = vec![Crumb {
            id: None,
            label: "我的云盘".to_string(),
        }];
        self.picker_folders.clear();
        self.picker_loading = false;
        self.save_dest = None;
    }

    /// 转存分享目录选择器当前所在目录。
    pub(crate) fn picker_parent(&self) -> Option<String> {
        self.picker_stack.last().and_then(|c| c.id.clone())
    }

    /// 打开转存分享「保存到」网盘目录选择器, 从根目录开始。
    pub(crate) fn open_picker(&mut self, g: &mut Global) {
        self.picker_open = true;
        self.picker_stack = vec![Crumb {
            id: None,
            label: "我的云盘".into(),
        }];
        self.picker_list(g);
    }

    /// 请求转存分享选择器当前目录的子文件夹列表。
    pub(crate) fn picker_list(&mut self, g: &mut Global) {
        self.picker_loading = true;
        self.picker_folders.clear();
        self.picker_req += 1;
        let req_id = self.picker_req;
        g.send(Cmd::ListFolders {
            parent: self.picker_parent(),
            req_id,
        });
    }

    /// 该 req_id 是否属于转存分享目录选择器。
    pub(crate) fn handles_picker(&self, req_id: u64) -> bool {
        self.picker_open && req_id == self.picker_req
    }

    /// 目录选择器响应(父目录已不是当前所在目录时丢弃)。
    pub(crate) fn on_folders(&mut self, parent: Option<String>, files: Vec<File>) {
        if parent != self.picker_parent() {
            return;
        }
        self.picker_loading = false;
        self.picker_folders = files;
    }

    // ---------- 转存分享: 消息 ----------

    /// 分享链接解析成功。
    pub(crate) fn on_resolved(
        &mut self,
        share_id: String,
        title: String,
        pass_code_token: String,
        files: Vec<File>,
        next_page_token: Option<String>,
    ) {
        self.save_resolving = false;
        self.save_id = Some(share_id);
        self.save_title = Some(title);
        self.save_token = Some(pass_code_token);
        self.save_files = files;
        self.save_next = next_page_token;
        self.save_selected = HashSet::new();
    }

    /// 分享文件列表加载更多成功。
    pub(crate) fn on_files_loaded(&mut self, files: Vec<File>, next_page_token: Option<String>) {
        self.save_loading_more = false;
        self.save_files.extend(files);
        self.save_next = next_page_token;
    }

    /// 分享文件列表加载更多失败。
    pub(crate) fn on_files_failed(&mut self, what: String) {
        self.save_loading_more = false;
        self.save_error = Some(what);
    }

    /// 分享链接解析失败。
    pub(crate) fn on_resolve_failed(&mut self, what: String) {
        self.save_resolving = false;
        self.save_error = Some(what);
    }

    /// 转存成功: 关闭弹窗; auto_move_failed 时保留目标目录以便重试移动。
    pub(crate) fn on_saved(&mut self, g: &mut Global, auto_move_failed: bool) {
        self.save_saving = false;
        self.save_open = false;
        let dest = self.save_dest.clone();
        if auto_move_failed {
            // 保存失败信息以便重试
            self.save_move_failed = dest.clone();
        }
        self.clear_save();
        if auto_move_failed {
            if let Some((_, name)) = dest {
                g.toast_warn(&format!(
                    "转存成功, 但自动移动到「{name}」失败。文件仍在「转存自分享」中"
                ));
            } else {
                g.toast_warn("转存成功, 但自动移动失败, 请在「转存自分享」中查看");
            }
        } else if dest.is_some() {
            g.toast_ok("转存成功, 文件已移动到目标目录");
        } else {
            g.toast_ok("转存成功, 文件已保存到「转存自分享」");
        }
    }

    /// 转存失败。
    pub(crate) fn on_save_failed(&mut self, what: String) {
        self.save_saving = false;
        self.save_error = Some(what);
    }

    /// 自动移动重试成功(目录重列由调用方负责)。
    pub(crate) fn on_move_retried(&mut self, g: &mut Global) {
        self.save_move_failed = None;
        g.toast_ok("移动成功");
    }

    /// 自动移动重试失败。
    pub(crate) fn on_move_retry_failed(&mut self, g: &mut Global, what: String) {
        g.toast_err(&what);
    }

    // ---------- 渲染 ----------

    pub(super) fn draw(&mut self, ctx: &egui::Context, g: &mut Global, th: &Theme) {
        let mut refresh = false;
        let mut load_more = false;
        let mut copy: Option<(String, &'static str)> = None;
        let mut open: Option<String> = None;
        let mut delete_single: Option<(String, String)> = None;
        let mut delete_selected = false;
        let mut toggle: Option<(String, bool)> = None;
        let mut select_all: Option<bool> = None;

        // 复制列表快照, 避免在渲染闭包里与 self 的可变借用冲突。
        let shares = self.list.clone();
        let selected: HashSet<String> = self.selected.clone();
        let has_next = self.next.is_some();
        let loading = self.loading;
        let all_selected = !shares.is_empty() && selected.len() >= shares.len();
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
                    icons::paint(ui.painter(), r, Glyph::Share, th.accent);
                    ui.label(RichText::new("我的分享").size(19.0).strong().color(th.text));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        // 转存分享按钮
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("转存分享").color(th.accent),
                                )
                                .frame(true),
                            )
                            .clicked()
                        {
                            self.save_open = true;
                            self.clear_save();
                        }
                        let refreshing = loading && !shares.is_empty();
                        let (rr, ico) =
                            ui.allocate_exact_size(vec2(22.0, 22.0), egui::Sense::click());
                        icons::paint(ui.painter(), rr, Glyph::Refresh, th.text_weak);
                        let ico_clicked = !refreshing && ico.clicked();
                        let btn = ui.add_enabled(
                            !refreshing,
                            egui::Button::new(
                                RichText::new(if refreshing { "正在刷新…" } else { "刷新" })
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
                        if !shares.is_empty() {
                            let mut sel = all_selected;
                            if ui.checkbox(&mut sel, "全选").changed() {
                                select_all = Some(sel);
                            }
                            ui.label(
                                RichText::new(format!("共 {} 个", shares.len()))
                                    .color(th.text_faint)
                                    .size(12.0),
                            );
                        }
                    });
                });
                ui.add_space(4.0);
                ui.label(
                    RichText::new(
                        "在「我的文件」中选中文件后点击「分享」即可创建链接; 在此管理已创建的分享。",
                    )
                    .color(th.text_weak)
                    .size(12.5),
                );
                ui.add_space(12.0);

                if loading && shares.is_empty() {
                    ui.vertical_centered(|ui| {
                        ui.add_space(60.0);
                        ui.spinner();
                        ui.add_space(8.0);
                        ui.label(RichText::new("正在获取分享…").color(th.text_weak));
                    });
                    return;
                }

                if shares.is_empty() {
                    ui.vertical_centered(|ui| {
                        ui.add_space((ui.available_height() * 0.24).max(50.0));
                        let (r, _) = ui.allocate_exact_size(vec2(56.0, 56.0), egui::Sense::hover());
                        icons::paint(ui.painter(), r, Glyph::Share, th.text_faint);
                        ui.add_space(10.0);
                        ui.label(RichText::new("暂无分享内容").color(th.text_weak).size(13.5));
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new("在文件页选中文件后点击「分享」创建")
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
                                                RichText::new("取消选中分享")
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
                    .id_salt("shares_scroll")
                    .auto_shrink([false, false])
                    .max_height(scroll_h)
                    .show(ui, |ui| {
                        for s in &shares {
                            Frame::new()
                                .fill(th.card)
                                .stroke(Stroke::new(1.0, th.border))
                                .corner_radius(th.cr(14))
                                .inner_margin(Margin::symmetric(16, 12))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        let mut checked = selected.contains(&s.share_id);
                                        if ui.checkbox(&mut checked, "").changed() {
                                            toggle = Some((s.share_id.clone(), checked));
                                        }
                                        ui.add_space(2.0);
                                        ui.vertical(|ui| {
                                            ui.horizontal(|ui| {
                                                let title = if s.title.is_empty() {
                                                    "未命名分享"
                                                } else {
                                                    s.title.as_str()
                                                };
                                                ui.label(
                                                    RichText::new(title)
                                                        .size(14.0)
                                                        .strong()
                                                        .color(th.text),
                                                );
                                                let (badge, color) = if s.is_unavailable() {
                                                    ("已失效", th.danger)
                                                } else if s.needs_pass_code() {
                                                    ("私密", th.warn)
                                                } else {
                                                    ("公开", th.ok)
                                                };
                                                Frame::new()
                                                    .fill(mix(
                                                        th.card,
                                                        color,
                                                        if th.dark { 0.22 } else { 0.14 },
                                                    ))
                                                    .corner_radius(th.cr(6))
                                                    .inner_margin(Margin::symmetric(6, 1))
                                                    .show(ui, |ui| {
                                                        ui.label(
                                                            RichText::new(badge)
                                                                .color(color)
                                                                .size(11.0),
                                                        );
                                                    });
                                            });
                                            ui.add_space(3.0);
                                            let views = if s.view_count.trim().is_empty() {
                                                "0"
                                            } else {
                                                s.view_count.trim()
                                            };
                                            let restores = if s.restore_count.trim().is_empty() {
                                                "0"
                                            } else {
                                                s.restore_count.trim()
                                            };
                                            let mut meta = format!(
                                                "{} 个文件 · 有效期 {} · 浏览 {} · 转存 {}",
                                                s.file_count(),
                                                s.expiry_label(),
                                                views,
                                                restores,
                                            );
                                            if !s.create_time.is_empty() {
                                                meta.push_str(&format!(
                                                    " · {}",
                                                    format::fmt_time(&s.create_time)
                                                ));
                                            }
                                            ui.label(
                                                RichText::new(meta).color(th.text_weak).size(12.0),
                                            );
                                            if !s.pass_code.is_empty() {
                                                ui.add_space(2.0);
                                                ui.label(
                                                    RichText::new(format!(
                                                        "提取码 {}",
                                                        s.pass_code
                                                    ))
                                                    .color(th.accent)
                                                    .size(12.0),
                                                );
                                            }
                                        });
                                        ui.with_layout(
                                            Layout::right_to_left(Align::Center),
                                            |ui| {
                                                if ui
                                                    .add(
                                                        egui::Button::new(
                                                            RichText::new("取消分享")
                                                                .color(th.danger),
                                                        )
                                                        .fill(egui::Color32::TRANSPARENT)
                                                        .stroke(Stroke::new(
                                                            1.0,
                                                            mix(th.danger, th.bg, 0.35),
                                                        ))
                                                        .corner_radius(th.cr(8)),
                                                    )
                                                    .clicked()
                                                {
                                                    delete_single = Some((
                                                        s.share_id.clone(),
                                                        s.title.clone(),
                                                    ));
                                                }
                                                if ui
                                                    .add(
                                                        egui::Button::new(
                                                            RichText::new("复制链接")
                                                                .color(th.accent),
                                                        )
                                                        .fill(th.accent_soft())
                                                        .stroke(Stroke::NONE)
                                                        .corner_radius(th.cr(8)),
                                                    )
                                                    .clicked()
                                                {
                                                    copy = Some((
                                                        s.share_url.clone(),
                                                        "已复制分享链接",
                                                    ));
                                                }
                                                if !s.pass_code.is_empty()
                                                    && ui
                                                        .add(
                                                            egui::Button::new(
                                                                RichText::new("复制链接和提取码")
                                                                    .color(th.text_weak),
                                                            )
                                                            .corner_radius(th.cr(8)),
                                                        )
                                                        .clicked()
                                                {
                                                    copy = Some((
                                                        format!(
                                                            "{} 提取码: {}",
                                                            s.share_url, s.pass_code
                                                        ),
                                                        "已复制分享链接和提取码",
                                                    ));
                                                }
                                                let (or, ores) = ui.allocate_exact_size(
                                                    vec2(26.0, 26.0),
                                                    egui::Sense::click(),
                                                );
                                                if ores.hovered() {
                                                    ui.painter()
                                                        .rect_filled(or, th.cr(6), th.hover);
                                                }
                                                icons::paint(
                                                    ui.painter(),
                                                    or.shrink(5.0),
                                                    Glyph::OpenExternal,
                                                    if ores.hovered() {
                                                        th.accent
                                                    } else {
                                                        th.text_weak
                                                    },
                                                );
                                                let ores = ores.on_hover_text("在浏览器打开");
                                                if ores.clicked() {
                                                    open = Some(s.share_url.clone());
                                                }
                                            },
                                        );
                                    });
                                    ui.add_space(4.0);
                                    ui.label(
                                        RichText::new(s.share_url.as_str())
                                            .color(th.text_faint)
                                            .size(11.5),
                                    );
                                });
                            ui.add_space(8.0);
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

        if let Some((id, on)) = toggle {
            if on {
                self.selected.insert(id);
            } else {
                self.selected.remove(&id);
            }
        }
        if let Some(true) = select_all {
            self.selected = shares.iter().map(|s| s.share_id.clone()).collect();
        } else if let Some(false) = select_all {
            self.selected.clear();
        }
        if refresh {
            self.refresh(g);
        }
        if load_more {
            self.load_more(g);
        }
        if let Some((text, msg)) = copy {
            ctx.copy_text(text);
            g.toast_ok(msg);
        }
        if let Some(url) = open {
            if let Err(e) = helpers::open_url(&url) {
                g.toast_err(&format!("打开链接失败: {e}"));
            }
        }
        if delete_selected {
            let items: Vec<(String, String)> = shares
                .iter()
                .filter(|s| self.selected.contains(&s.share_id))
                .map(|s| (s.share_id.clone(), s.title.clone()))
                .collect();
            if !items.is_empty() {
                self.delete_confirm = Some(items);
            }
        }
        if let Some(item) = delete_single {
            self.delete_confirm = Some(vec![item]);
        }
    }

    /// 「创建分享」设置框 / 分享结果 / 取消分享确认。
    pub(super) fn draw_dialogs(&mut self, ctx: &egui::Context, g: &mut Global, th: &Theme) {
        if let Some(targets) = self.create_dialog.clone() {
            let n = targets.len();
            let name = if n == 1 {
                targets[0].1.clone()
            } else {
                format!("选中的 {n} 项")
            };
            let mut confirm = false;
            let mut close = false;
            egui::Window::new("创建分享")
                .collapsible(false)
                .resizable(false)
                .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.add_space(4.0);
                    ui.label(RichText::new(format!("将分享: {name}")).color(th.text));
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("有效期").color(th.text_weak));
                        let label = match self.expiration_days {
                            1 => "1 天",
                            7 => "7 天",
                            30 => "30 天",
                            _ => "永久",
                        };
                        egui::ComboBox::from_id_salt("share_expiry")
                            .selected_text(label)
                            .show_ui(ui, |ui| {
                                for (d, l) in
                                    [(-1, "永久"), (1, "1 天"), (7, "7 天"), (30, "30 天")]
                                {
                                    ui.selectable_value(&mut self.expiration_days, d, l);
                                }
                            });
                    });
                    ui.add_space(6.0);
                    ui.checkbox(&mut self.need_password, "需要提取码");
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new("带提取码的分享更安全, 访问者需输入提取码。")
                            .color(th.text_faint)
                            .size(11.5),
                    );
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add(
                                egui::Button::new(RichText::new("创建分享").color(th.on_accent))
                                    .fill(th.accent)
                                    .stroke(Stroke::NONE)
                                    .corner_radius(th.cr(8)),
                            )
                            .clicked()
                        {
                            confirm = true;
                            close = true;
                        }
                        if ui.button("取消").clicked() {
                            close = true;
                        }
                    });
                });
            if confirm {
                let ids: Vec<String> = targets.iter().map(|(id, _)| id.clone()).collect();
                let label = if n == 1 {
                    targets[0].1.clone()
                } else {
                    format!("{n} 项")
                };
                g.send(Cmd::CreateShare {
                    file_ids: ids,
                    expiration_days: self.expiration_days,
                    need_password: self.need_password,
                    label,
                });
            }
            if close {
                self.create_dialog = None;
            }
        }

        if let Some(r) = self.result.clone() {
            let mut close = false;
            let mut copied: Option<&'static str> = None;
            let mut open = false;
            egui::Window::new("分享已创建")
                .collapsible(false)
                .resizable(false)
                .default_width(420.0)
                .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(format!("「{}」的分享链接", r.label))
                            .color(th.text_weak)
                            .size(12.5),
                    );
                    ui.add_space(6.0);
                    // 用可选中标签展示(只读文本域在 egui 中无法拖选)。
                    ui.label(
                        RichText::new(r.url.as_str())
                            .monospace()
                            .size(12.5)
                            .color(th.text),
                    );
                    if !r.pass_code.is_empty() {
                        ui.add_space(8.0);
                        ui.label(RichText::new("提取码").color(th.text_weak).size(12.5));
                        ui.label(
                            RichText::new(r.pass_code.as_str())
                                .monospace()
                                .size(12.5)
                                .color(th.text),
                        );
                    }
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add(
                                egui::Button::new(RichText::new("复制链接").color(th.on_accent))
                                    .fill(th.accent)
                                    .stroke(Stroke::NONE)
                                    .corner_radius(th.cr(8)),
                            )
                            .clicked()
                        {
                            ctx.copy_text(r.url.clone());
                            copied = Some("已复制分享链接");
                        }
                        if !r.pass_code.is_empty()
                            && ui
                                .add(egui::Button::new(
                                    RichText::new("复制链接和提取码").color(th.text_weak),
                                ))
                                .clicked()
                        {
                            // 服务端返回的 share_text 未必带提取码, 只有确认包含时才用,
                            // 否则自行拼出「链接 + 提取码」, 保证与按钮文案一致。
                            let text = if !r.share_text.is_empty()
                                && r.share_text.contains(r.pass_code.as_str())
                            {
                                r.share_text.clone()
                            } else {
                                format!("{} 提取码: {}", r.url, r.pass_code)
                            };
                            ctx.copy_text(text);
                            copied = Some("已复制分享链接和提取码");
                        }
                        if ui.button("在浏览器打开").clicked() {
                            open = true;
                        }
                        if ui.button("完成").clicked() {
                            close = true;
                        }
                    });
                    ui.add_space(4.0);
                });
            if let Some(msg) = copied {
                g.toast_ok(msg);
            }
            if open {
                if let Err(e) = helpers::open_url(&r.url) {
                    g.toast_err(&format!("打开链接失败: {e}"));
                }
            }
            if close {
                self.result = None;
            }
        }

        if let Some(items) = self.delete_confirm.clone() {
            let ids: Vec<String> = items.iter().map(|(id, _)| id.clone()).collect();
            let mut confirmed = false;
            let mut close = false;
            egui::Window::new("取消分享")
                .collapsible(false)
                .resizable(false)
                .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    if items.len() == 1 {
                        let title = if items[0].1.is_empty() {
                            "该分享"
                        } else {
                            items[0].1.as_str()
                        };
                        ui.label(format!("确定取消分享「{title}」吗? 分享链接将立即失效。"));
                    } else {
                        ui.label(format!("确定取消这 {} 个分享吗?", items.len()));
                    }
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add(
                                egui::Button::new(RichText::new("取消分享").color(Color32::WHITE))
                                    .fill(th.danger)
                                    .stroke(Stroke::NONE),
                            )
                            .clicked()
                        {
                            confirmed = true;
                            close = true;
                        }
                        if ui.button("关闭").clicked() {
                            close = true;
                        }
                    });
                });
            if confirmed {
                g.send(Cmd::DeleteShares { ids });
                // 等待后台 SharesDeleted 回执后再关闭，避免请求失败时丢失确认框。
            } else if close {
                self.delete_confirm = None;
            }
        }
    }

    /// 转存分享弹窗 / 目标目录选择器 / 自动移动失败重试。
    pub(super) fn draw_save_dialogs(&mut self, ctx: &egui::Context, g: &mut Global, th: &Theme) {
        // 转存分享弹窗
        if self.save_open {
            let mut resolve = false;
            let mut save = false;
            let mut close = false;

            egui::Window::new("转存分享")
                .collapsible(false)
                .resizable(false)
                .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.set_min_width(380.0);
                    ui.add_space(4.0);

                    // 错误提示
                    if let Some(err) = &self.save_error {
                        ui.label(RichText::new(err).color(th.danger).size(12.0));
                        ui.add_space(6.0);
                    }

                    // 未解析: 显示输入框
                    if self.save_id.is_none() {
                        ui.label(RichText::new("分享链接或 ID").color(th.text_weak));
                        let resp = ui.add(
                            input(&mut self.save_input)
                                .desired_width(360.0)
                                .hint_text("https://mypikpak.com/s/xxx 或直接输入 ID"),
                        );
                        let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                        ui.add_space(6.0);
                        ui.label(RichText::new("提取码 (可选)").color(th.text_weak));
                        ui.add(
                            input(&mut self.save_pass_code)
                                .desired_width(360.0)
                                .hint_text("公开分享无需填写"),
                        );
                        ui.add_space(12.0);
                        ui.horizontal(|ui| {
                            let can_resolve =
                                !self.save_input.trim().is_empty() && !self.save_resolving;
                            if ui
                                .add_enabled(can_resolve, egui::Button::new("解析"))
                                .clicked()
                                || enter
                            {
                                resolve = true;
                            }
                            if ui.button("取消").clicked() {
                                close = true;
                            }
                            if self.save_resolving {
                                ui.add_space(8.0);
                                ui.spinner();
                                ui.label(RichText::new("正在解析…").color(th.text_weak));
                            }
                        });
                    } else {
                        // 已解析: 显示文件列表
                        let title = self.save_title.clone().unwrap_or_default();
                        if !title.is_empty() {
                            ui.label(
                                RichText::new(format!("分享: {title}"))
                                    .strong()
                                    .color(th.text),
                            );
                            ui.add_space(6.0);
                        }

                        let files = self.save_files.clone();
                        let filter = self.save_filter.clone();
                        let filtered: Vec<&File> = if filter.is_empty() {
                            files.iter().collect()
                        } else {
                            let lower = filter.to_lowercase();
                            files
                                .iter()
                                .filter(|f| f.name.to_lowercase().contains(&lower))
                                .collect()
                        };
                        let selected = self.save_selected.clone();
                        let filtered_selected =
                            filtered.iter().filter(|f| selected.contains(&f.id)).count();
                        let all_filtered_selected =
                            !filtered.is_empty() && filtered_selected >= filtered.len();

                        ui.label(
                            RichText::new(format!("共 {} 个文件", files.len()))
                                .color(th.text_weak)
                                .size(12.0),
                        );
                        ui.add_space(4.0);

                        // 搜索框
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("搜索").color(th.text_weak).size(12.0));
                            ui.add(
                                input(&mut self.save_filter)
                                    .desired_width(200.0)
                                    .hint_text("按文件名过滤"),
                            );
                            if !filter.is_empty() && ui.button("清除").clicked() {
                                self.save_filter.clear();
                            }
                        });
                        ui.add_space(4.0);

                        // 全选(仅对过滤后的文件生效)
                        let mut sel = all_filtered_selected;
                        if ui.checkbox(&mut sel, "全选").changed() {
                            if sel {
                                for f in &filtered {
                                    self.save_selected.insert(f.id.clone());
                                }
                            } else {
                                for f in &filtered {
                                    self.save_selected.remove(&f.id);
                                }
                            }
                        }
                        if !filter.is_empty() {
                            ui.label(
                                RichText::new(format!("(匹配 {} 个)", filtered.len()))
                                    .color(th.text_faint)
                                    .size(11.0),
                            );
                        }
                        ui.add_space(4.0);

                        // 文件列表
                        egui::ScrollArea::vertical()
                            .max_height(240.0)
                            .show(ui, |ui| {
                                if filtered.is_empty() && !files.is_empty() {
                                    ui.label(
                                        RichText::new("没有匹配的文件")
                                            .color(th.text_weak)
                                            .size(12.0),
                                    );
                                }
                                for file in &filtered {
                                    let is_sel = self.save_selected.contains(&file.id);
                                    let mut checked = is_sel;
                                    let icon = if file.is_folder() {
                                        Glyph::Folder
                                    } else {
                                        Glyph::File
                                    };
                                    ui.horizontal(|ui| {
                                        if ui.checkbox(&mut checked, "").changed() {
                                            if checked {
                                                self.save_selected.insert(file.id.clone());
                                            } else {
                                                self.save_selected.remove(&file.id);
                                            }
                                        }
                                        let (r, _) = ui.allocate_exact_size(
                                            vec2(16.0, 16.0),
                                            egui::Sense::hover(),
                                        );
                                        let color = if file.is_folder() {
                                            Color32::from_rgb(232, 178, 84)
                                        } else {
                                            th.text_weak
                                        };
                                        icons::paint(ui.painter(), r, icon, color);
                                        ui.label(
                                            RichText::new(&file.name).color(th.text).size(13.0),
                                        );
                                    });
                                }
                            });

                        // 加载更多按钮
                        if let Some(next_token) = self.save_next.clone() {
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                if self.save_loading_more {
                                    ui.spinner();
                                    ui.label(
                                        RichText::new("正在加载…").color(th.text_weak).size(12.0),
                                    );
                                } else if ui.button("加载更多文件").clicked() {
                                    if let (Some(share_id), Some(token)) =
                                        (&self.save_id, &self.save_token)
                                    {
                                        self.save_loading_more = true;
                                        self.save_error = None;
                                        g.send(Cmd::LoadMoreShareFiles {
                                            share_id: share_id.clone(),
                                            pass_code_token: token.clone(),
                                            page_token: next_token,
                                        });
                                    }
                                }
                            });
                        }

                        ui.add_space(12.0);

                        ui.horizontal(|ui| {
                            let can_save = !self.save_selected.is_empty() && !self.save_saving;

                            // 左侧: 目录选择按钮, 显示当前目标目录名或"默认位置"
                            let dest_label = self
                                .save_dest
                                .as_ref()
                                .map(|(_, name)| name.as_str())
                                .unwrap_or("默认位置");
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new(dest_label).color(Color32::WHITE),
                                    )
                                    .fill(Color32::from_gray(45))
                                    .stroke(Stroke::new(1.0, Color32::from_gray(70)))
                                    .min_size(vec2(120.0, 0.0)),
                                )
                                .on_hover_text("点击选择保存目录")
                                .clicked()
                            {
                                self.open_picker(g);
                            }

                            ui.add_space(8.0);

                            // 右侧: 保存按钮
                            if ui
                                .add_enabled(
                                    can_save,
                                    egui::Button::new(RichText::new("保存").color(Color32::WHITE))
                                        .fill(th.accent)
                                        .stroke(Stroke::NONE),
                                )
                                .clicked()
                            {
                                save = true;
                            }

                            if ui.button("返回").clicked() {
                                // 返回输入状态
                                self.save_id = None;
                                self.save_title = None;
                                self.save_token = None;
                                self.save_files.clear();
                                self.save_selected.clear();
                                self.save_error = None;
                            }
                            if ui.button("取消").clicked() {
                                close = true;
                            }
                            if self.save_saving {
                                ui.add_space(8.0);
                                ui.spinner();
                                ui.label(RichText::new("正在转存…").color(th.text_weak));
                            }
                        });
                    }
                });

            if resolve {
                self.save_error = None;
                match parse_share_input(&self.save_input) {
                    Some((share_id, extracted_pass)) => {
                        // 链接里带了提取码且用户未填写时自动回填。
                        if let Some(p) = extracted_pass {
                            if self.save_pass_code.trim().is_empty() {
                                self.save_pass_code = p;
                            }
                        }
                        self.save_resolving = true;
                        let pass_code = self.save_pass_code.clone();
                        g.send(Cmd::ResolveShare {
                            share_id,
                            pass_code,
                        });
                    }
                    None => {
                        self.save_error = Some(
                            "无法识别分享链接, 请检查格式 (例如 https://mypikpak.com/s/xxx 或直接输入 ID)"
                                .to_string(),
                        );
                    }
                }
            }
            if save {
                if let (Some(share_id), Some(token)) = (&self.save_id, &self.save_token) {
                    self.save_saving = true;
                    self.save_error = None;
                    let file_ids: Vec<String> = self.save_selected.iter().cloned().collect();
                    let dest = self.save_dest.as_ref().and_then(|(id, _)| {
                        if id.is_empty() {
                            None
                        } else {
                            Some(id.clone())
                        }
                    });
                    g.send(Cmd::SaveShare {
                        share_id: share_id.clone(),
                        pass_code_token: token.clone(),
                        file_ids,
                        dest,
                    });
                }
            }
            if close {
                self.save_open = false;
                self.clear_save();
            }
        }

        // 转存分享目录选择器
        if self.picker_open {
            let crumbs = self.picker_stack.clone();
            let folders = self.picker_folders.clone();
            let loading = self.picker_loading;
            let cur_dest = self.save_dest.clone();
            let mut nav_to: Option<usize> = None;
            let mut enter: Option<(String, String)> = None;
            let mut confirm = false;
            let mut close_picker = false;

            egui::Window::new("选择转存到（网盘目录）")
                .collapsible(false)
                .resizable(false)
                .default_width(420.0)
                .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.add_space(4.0);
                    ui.horizontal_wrapped(|ui| {
                        let last = crumbs.len().saturating_sub(1);
                        for (i, c) in crumbs.iter().enumerate() {
                            if i > 0 {
                                ui.label(RichText::new("/").color(th.text_faint));
                            }
                            if ui.selectable_label(i == last, &c.label).clicked() && i != last {
                                nav_to = Some(i);
                            }
                        }
                    });
                    ui.add_space(6.0);
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .id_salt("save_share_picker_scroll")
                        .auto_shrink([false, false])
                        .max_height(260.0)
                        .show(ui, |ui| {
                            ui.set_min_width(380.0);
                            if loading {
                                ui.vertical_centered(|ui| {
                                    ui.add_space(24.0);
                                    ui.spinner();
                                    ui.add_space(24.0);
                                });
                            } else if folders.is_empty() {
                                ui.vertical_centered(|ui| {
                                    ui.add_space(24.0);
                                    ui.label(RichText::new("此目录下没有子文件夹").weak());
                                    ui.add_space(24.0);
                                });
                            } else {
                                for f in &folders {
                                    if folder_row(ui, th, &f.name) {
                                        enter = Some((f.id.clone(), f.name.clone()));
                                    }
                                    ui.add_space(2.0);
                                }
                            }
                        });
                    ui.separator();
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("选择此目录").color(Color32::WHITE),
                                )
                                .fill(th.accent)
                                .stroke(Stroke::NONE),
                            )
                            .clicked()
                        {
                            confirm = true;
                        }
                        if ui.button("取消").clicked() {
                            close_picker = true;
                        }
                        if let Some((_, name)) = cur_dest.as_ref() {
                            ui.label(RichText::new(format!("当前: {name}")).color(th.text_faint));
                        }
                    });
                });

            if let Some(i) = nav_to {
                self.picker_stack.truncate(i + 1);
                self.picker_list(g);
            }
            if let Some((id, name)) = enter {
                self.picker_stack.push(Crumb {
                    id: Some(id.clone()),
                    label: name,
                });
                self.picker_list(g);
            }
            if confirm {
                // 获取当前目录作为目标
                let parent = self.picker_parent();
                let label = self
                    .picker_stack
                    .last()
                    .map(|c| c.label.clone())
                    .unwrap_or_else(|| "我的云盘".to_string());
                // 选择根目录时, 用空字符串标记, 以便 UI 显示"我的云盘"而非"默认位置"
                self.save_dest = Some((parent.unwrap_or_default(), label));
                self.picker_open = false;
            }
            if close_picker {
                self.picker_open = false;
            }
        }

        // 自动移动失败重试对话框
        if let Some((dest_id, dest_name)) = self.save_move_failed.clone() {
            let mut retry = false;
            let mut dismiss = false;

            egui::Window::new("移动失败")
                .collapsible(false)
                .resizable(false)
                .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.set_min_width(320.0);
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(format!(
                            "文件已转存到「转存自分享」, 但自动移动到「{dest_name}」失败。"
                        ))
                        .color(th.text),
                    );
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new("您可以重试移动, 或稍后手动处理。")
                            .color(th.text_weak)
                            .size(12.0),
                    );
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add(
                                egui::Button::new(RichText::new("重试移动").color(Color32::WHITE))
                                    .fill(th.accent)
                                    .stroke(Stroke::NONE),
                            )
                            .clicked()
                        {
                            retry = true;
                        }
                        if ui.button("稍后处理").clicked() {
                            dismiss = true;
                        }
                    });
                });

            if retry {
                g.send(Cmd::RetryMoveShare { dest: dest_id });
                self.save_move_failed = None;
            }
            if dismiss {
                self.save_move_failed = None;
            }
        }
    }
}

/// 解析分享输入, 返回 `(share_id, 链接里携带的提取码)`。
///
/// 支持的形态:
/// - 完整链接: `https://mypikpak.com/s/<id>`
/// - 带查询参数 / 片段: `.../s/<id>?password=abcd#frag`(顺带提取 `password`/`pass_code`)
/// - 复制链接时附带的前后缀文字: 只取 `/s/` 之后的一段
/// - 裸 ID: `<id>`
///
/// 无法识别(如缺少 `/s/` 的其它域名链接、含非法字符)时返回 `None`, 由调用方给出提示。
fn parse_share_input(input: &str) -> Option<(String, Option<String>)> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }

    // 截出 `/s/` 之后的一段: ID 到第一个非法字符为止, 之后按查询串解析提取码。
    let (id_candidate, query) = if let Some(idx) = input.rfind("/s/") {
        let rest = &input[idx + 3..];
        let id: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        let query = rest.find('?').map(|q| {
            let after = &rest[q + 1..];
            let end = after
                .find(|c: char| c == '#' || c.is_whitespace())
                .unwrap_or(after.len());
            &after[..end]
        });
        (id, query)
    } else if input.contains("://") {
        // 是一个链接但不含 `/s/` 段, 无法定位分享 ID。
        return None;
    } else {
        // 视为裸 ID。
        (input.to_string(), None)
    };

    if !is_valid_share_id(&id_candidate) {
        return None;
    }
    let pass_code = query.and_then(pass_code_from_query);
    Some((id_candidate, pass_code))
}

/// share_id 只由字母、数字、`-`、`_` 组成且非空。
fn is_valid_share_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// 从 URL 查询串里取出提取码 (`password` / `pass_code` / `passcode`)。
fn pass_code_from_query(query: &str) -> Option<String> {
    for pair in query.split('&') {
        let mut kv = pair.splitn(2, '=');
        let key = kv.next().unwrap_or("");
        if matches!(key, "password" | "pass_code" | "passcode") {
            if let Some(v) = kv.next() {
                let v = v.trim();
                if !v.is_empty() {
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_share_id_from_url_forms() {
        let cases = [
            ("https://mypikpak.com/s/VO8B-abc", Some(("VO8B-abc", None))),
            (
                "  https://mypikpak.com/s/VO8B-abc?password=abcd  ",
                Some(("VO8B-abc", Some("abcd"))),
            ),
            (
                "https://mypikpak.com/s/VO8B-abc#frag",
                Some(("VO8B-abc", None)),
            ),
            // 复制链接时附带的前后缀文字。
            (
                "打开链接 https://mypikpak.com/s/AbC_123 查看",
                Some(("AbC_123", None)),
            ),
            ("xyz-1", Some(("xyz-1", None))),
            // 无法识别的形态。
            ("https://example.com/download/abc", None),
            ("https://mypikpak.com/s/", None),
            ("not a valid id!", None),
            ("", None),
        ];
        for (input, want) in cases {
            let got = parse_share_input(input);
            let got = got.as_ref().map(|(id, p)| (id.as_str(), p.as_deref()));
            assert_eq!(got, want, "input: {input:?}");
        }
    }
}
