//! 预览域: 大文件确认 / 缓存下载进度与取消 / 清晰度解析 / 「系统打开」探针。
//!
//! 入口在 `App` 侧(需读当前目录的文件类型与同集字幕), 状态与消息处理在本域;
//! `pending_open` 探针同时服务预览与传输页的「打开文件 / 打开目录」。

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use eframe::egui::{self, vec2, Align2, RichText, Stroke};

use crate::icons::{self, Glyph};
use crate::msg::{Cmd, QualityOption};
use crate::theme::Theme;

use super::global::Global;
use super::helpers;

/// 已解析的可用清晰度与同集字幕。
pub(crate) struct QualityReady {
    pub options: Vec<crate::msg::QualityOption>,
    pub subs: Vec<PathBuf>,
}

/// 「播放」子菜单展示所需的清晰度状态。
#[derive(Clone, Copy)]
pub(crate) enum QualityMenuState<'a> {
    /// 尚未解析完成。
    Loading,
    /// 已解析(可能为空列表)。
    Ready(&'a QualityReady),
}

/// 待回传的「用系统程序打开」动作(后台探针结果)。
pub(crate) struct PendingOpen {
    pub rx: std::sync::mpsc::Receiver<super::helpers::OpenOutcome>,
    /// 提示里展示的目标名(文件名 / 目录路径)。
    pub label: String,
    /// 成功时不提示(打开目录保持安静, 只有失败才说话)。
    pub quiet_ok: bool,
}

/// 大文件预览确认弹窗的状态: 非媒体预览需先整份下载, 超过阈值时先问一次。
#[derive(Clone)]
pub(crate) struct PreviewConfirm {
    pub id: String,
    pub name: String,
    pub size: i64,
}

/// 非媒体预览的缓存下载进度, 驱动常驻进度条与取消按钮。
#[derive(Clone)]
pub(crate) struct PreviewProgress {
    pub req_id: u64,
    pub name: String,
    /// 总大小未知时为 0(进度条走不确定动画)。
    pub total: u64,
    pub done: u64,
}

/// 预览域状态。
#[derive(Default)]
pub(crate) struct PreviewPage {
    /// 正在准备中的预览任务 (req_id, 文件名); 用于给出加载反馈。
    pending: Option<(u64, String)>,
    /// 待确认的大文件预览(非媒体预览需先整份下载, 超过阈值先问一次)。
    confirm: Option<PreviewConfirm>,
    /// 非媒体预览的缓存下载进度(常驻进度条 + 取消)。
    progress: Option<PreviewProgress>,
    /// 待回传的「用系统程序打开」探针(避免 xdg-open 假成功)。
    open_probe: Option<PendingOpen>,
    /// 已解析的媒体文件清晰度缓存(file_id -> 清晰度 + 字幕)。
    quality_cache: HashMap<String, QualityReady>,
    /// 正在解析清晰度的文件 id。
    quality_inflight: HashSet<String>,
}

impl PreviewPage {
    /// 清空状态(退出登录 / 会话失效时调用)。
    pub(crate) fn clear(&mut self) {
        self.pending = None;
        self.confirm = None;
        self.progress = None;
        self.open_probe = None;
        self.quality_cache.clear();
        self.quality_inflight.clear();
    }

    // ---------- 供文件页 / 主循环查询 ----------

    /// 某文件已解析的清晰度与字幕。
    pub(crate) fn quality(&self, file_id: &str) -> Option<&QualityReady> {
        self.quality_cache.get(file_id)
    }

    /// 是否有非媒体预览正在下载缓存。
    pub(crate) fn is_progressing(&self) -> bool {
        self.progress.is_some()
    }

    /// 是否有清晰度解析在途。
    pub(crate) fn has_inflight_qualities(&self) -> bool {
        !self.quality_inflight.is_empty()
    }

    // ---------- 发起 ----------

    /// 交给系统默认程序打开某路径, 结果由后台探针回传(见 [`helpers::open_async`])。
    pub(crate) fn open_with_system(&mut self, path: PathBuf, label: String, quiet_ok: bool) {
        self.open_probe = Some(PendingOpen {
            rx: helpers::open_async(path),
            label,
            quiet_ok,
        });
    }

    /// 记下待确认的大文件预览(确认后由调用方复用 `App::start_preview`)。
    pub(crate) fn confirm_big(&mut self, id: String, name: String, size: i64) {
        self.confirm = Some(PreviewConfirm { id, name, size });
    }

    /// 发起预览(是否媒体与同集字幕后缀由调用方按当前目录解析)。
    pub(crate) fn start(
        &mut self,
        g: &mut Global,
        req_id: u64,
        id: String,
        name: String,
        media: bool,
        subtitles: Vec<(String, String)>,
    ) {
        self.pending = Some((req_id, name.clone()));
        let hint = if media {
            "正在解析播放地址…"
        } else {
            "正在准备预览文件…"
        };
        g.toast(hint, g.theme().accent);
        g.send(Cmd::Preview {
            req_id,
            file_id: id,
            name,
            media,
            subtitles,
        });
    }

    /// 取消正在准备的非媒体预览(丢弃未完成的缓存下载)。
    pub(crate) fn cancel(&mut self, g: &mut Global) {
        let Some(p) = self.progress.take() else {
            return;
        };
        g.send(Cmd::CancelPreview { req_id: p.req_id });
        if self.pending.as_ref().is_some_and(|(id, _)| *id == p.req_id) {
            self.pending = None;
        }
        g.toast("已取消预览", g.theme().text_weak);
    }

    /// 确保某媒体文件的可用清晰度已解析(供「播放」子菜单展示)。
    pub(crate) fn fetch_qualities(
        &mut self,
        g: &mut Global,
        id: String,
        subtitles: Vec<(String, String)>,
    ) {
        if self.quality_cache.contains_key(&id) || self.quality_inflight.contains(&id) {
            return;
        }
        self.quality_inflight.insert(id.clone());
        g.send(Cmd::PreviewQualities {
            file_id: id,
            subtitles,
        });
    }

    /// 用某个已解析出的清晰度播放(挂载同集字幕)。
    pub(crate) fn play_option(
        &mut self,
        g: &mut Global,
        id: String,
        name: String,
        opt: QualityOption,
    ) {
        let subs = self
            .quality_cache
            .get(&id)
            .map(|r| r.subs.clone())
            .unwrap_or_default();
        match helpers::play_with_mpv(&name, &opt.url, &opt.headers, &subs) {
            Ok(()) => g.toast_ok(&format!("正在用 mpv 播放「{name}」({})", opt.label)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                g.toast_warn("播放音视频需要 mpv, 请先安装 mpv");
            }
            Err(e) => g.toast_err(&format!("启动 mpv 失败: {e}")),
        }
    }

    // ---------- 消息 ----------

    /// 音视频播放地址已就绪: 交给 mpv 流式播放。
    pub(crate) fn on_stream(
        &mut self,
        g: &mut Global,
        req_id: u64,
        name: String,
        url: String,
        headers: Vec<(String, String)>,
        subs: Vec<PathBuf>,
    ) {
        if self.pending.as_ref().is_some_and(|(id, _)| *id == req_id) {
            self.pending = None;
        }
        if self.progress.as_ref().is_some_and(|p| p.req_id == req_id) {
            self.progress = None;
        }
        match helpers::play_with_mpv(&name, &url, &headers, &subs) {
            Ok(()) => g.toast_ok(&format!("正在用 mpv 播放「{name}」")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // 音视频流式播放依赖 mpv, 缺失时只提示, 不下载回退。
                g.toast_warn("播放音视频需要 mpv, 请先安装 mpv");
            }
            Err(e) => g.toast_err(&format!("启动 mpv 失败: {e}")),
        }
    }

    /// 非媒体预览的缓存下载进度。
    pub(crate) fn on_progress(&mut self, req_id: u64, total: u64, done: u64) {
        // 只认当前在途的预览任务, 避免过期消息把进度条拉回来。
        let name = self
            .pending
            .as_ref()
            .filter(|(id, _)| *id == req_id)
            .map(|(_, name)| name.clone());
        if let Some(name) = name {
            self.progress = Some(PreviewProgress {
                req_id,
                name,
                total,
                done,
            });
        }
    }

    /// 非媒体预览已下载到本地缓存: 交给系统查看器打开。
    pub(crate) fn on_ready(&mut self, req_id: u64, name: String, path: PathBuf) {
        if self.pending.as_ref().is_some_and(|(id, _)| *id == req_id) {
            self.pending = None;
        }
        if self.progress.as_ref().is_some_and(|p| p.req_id == req_id) {
            self.progress = None;
        }
        // 结果由后台探针回传(xdg-open 失败不再是假成功)。
        self.open_with_system(path, name, false);
    }

    /// 媒体清晰度列表已解析。
    pub(crate) fn on_qualities(
        &mut self,
        file_id: String,
        qualities: Vec<QualityOption>,
        subs: Vec<PathBuf>,
    ) {
        self.quality_inflight.remove(&file_id);
        self.quality_cache.insert(
            file_id,
            QualityReady {
                options: qualities,
                subs,
            },
        );
    }

    /// 清晰度解析失败。
    pub(crate) fn on_qualities_failed(&mut self, g: &mut Global, file_id: String, what: String) {
        self.quality_inflight.remove(&file_id);
        g.toast_err(&what);
    }

    /// 预览准备失败。
    pub(crate) fn on_failed(&mut self, g: &mut Global, req_id: u64, what: String) {
        if self.pending.as_ref().is_some_and(|(id, _)| *id == req_id) {
            self.pending = None;
        }
        if self.progress.as_ref().is_some_and(|p| p.req_id == req_id) {
            self.progress = None;
        }
        g.toast_err(&what);
    }

    /// 回收「用系统程序打开」的探针结果, 给出诚实提示。
    pub(crate) fn poll_open_probe(&mut self, ctx: &egui::Context, g: &mut Global) {
        let Some(pending) = self.open_probe.take() else {
            return;
        };
        match pending.rx.try_recv() {
            Ok(helpers::OpenOutcome::Launched) => {
                if !pending.quiet_ok {
                    g.toast_ok(&format!("已打开「{}」", pending.label));
                }
            }
            Ok(helpers::OpenOutcome::NoHandler) => {
                g.toast_warn(&format!("系统未关联打开「{}」的程序", pending.label));
            }
            Ok(helpers::OpenOutcome::Failed(e)) => {
                g.toast_err(&format!("打开「{}」失败: {e}", pending.label));
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                // 探针还在等 xdg-open 退出(最多 1s), 保留结果下次再收。
                self.open_probe = Some(pending);
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {}
        }
    }

    // ---------- 渲染 ----------

    /// 大文件预览确认框; 返回确认项 (id, name), 由调用方发起预览。
    pub(super) fn draw_confirm(
        &mut self,
        ctx: &egui::Context,
        th: &Theme,
    ) -> Option<(String, String)> {
        let pc = self.confirm.clone()?;
        let mut confirmed = false;
        let mut close = false;
        egui::Window::new("预览大文件")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_max_width(360.0);
                ui.label(format!(
                    "「{}」大小为 {}, 预览前需要先完整下载到本地缓存。",
                    pc.name,
                    crate::format::fmt_bytes(pc.size)
                ));
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Button::new(RichText::new("继续预览").color(th.on_accent))
                                .fill(th.accent)
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
        if close {
            self.confirm = None;
        }
        if confirmed {
            Some((pc.id, pc.name))
        } else {
            None
        }
    }

    /// 非媒体预览的缓存下载状态条(常驻于 toast 上方, 带进度与取消)。
    pub(super) fn draw_status(&mut self, ctx: &egui::Context, g: &mut Global, th: &Theme) {
        let Some(p) = self.progress.clone() else {
            return;
        };
        let cr = th.cr(10);
        let shown: String = if p.name.chars().count() > 22 {
            format!("{}…", p.name.chars().take(20).collect::<String>())
        } else {
            p.name.clone()
        };
        let frac = if p.total > 0 {
            (p.done as f64 / p.total as f64).clamp(0.0, 1.0) as f32
        } else {
            0.0
        };
        let text = if p.total > 0 {
            format!(
                "{} / {}",
                crate::format::fmt_bytes(p.done as i64),
                crate::format::fmt_bytes(p.total as i64)
            )
        } else {
            crate::format::fmt_bytes(p.done as i64)
        };
        let mut cancel = false;
        egui::Area::new(egui::Id::new("preview-status"))
            .anchor(Align2::RIGHT_BOTTOM, [-16.0, -64.0])
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style())
                    .corner_radius(cr)
                    .show(ui, |ui| {
                        ui.set_max_width(300.0);
                        ui.add_space(2.0);
                        ui.label(
                            RichText::new(format!("预览下载中「{shown}」"))
                                .color(th.text)
                                .size(12.5),
                        );
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::ProgressBar::new(frac)
                                    .desired_width(240.0)
                                    .corner_radius(th.cr(4))
                                    .fill(th.accent)
                                    .animate(p.total == 0)
                                    .text(RichText::new(text).size(11.0)),
                            );
                            ui.add_space(2.0);
                            let (r, resp) =
                                ui.allocate_exact_size(vec2(20.0, 20.0), egui::Sense::click());
                            icons::paint(ui.painter(), r, Glyph::Close, th.text_weak);
                            if resp.on_hover_text("取消预览").clicked() {
                                cancel = true;
                            }
                        });
                        ui.add_space(2.0);
                    });
            });
        if cancel {
            self.cancel(g);
        }
    }
}
