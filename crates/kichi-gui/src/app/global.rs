//! 各域共用的全局句柄: 后台命令通道、用户提示条与系统配色。
//!
//! 域结构只持有自己的状态; 需要发命令 / 给用户提示时由调用方传入 `&mut Global`,
//! 域之间不互相依赖。

use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use eframe::egui::Color32;

use crate::kde;
use crate::msg::Cmd;
use crate::theme::Theme;

/// 应用当前页面。
#[derive(PartialEq, Clone, Copy)]
pub(crate) enum Page {
    Files,
    Shares,
    Trash,
    Tasks,
    Transfers,
    Settings,
}

/// 磁盘缓存(预览 + 缩略图)的占用情况, 用于设置页展示。
#[derive(Clone, Copy, Default)]
pub(crate) struct CacheUsage {
    pub bytes: u64,
    pub entries: usize,
}

/// 用户提示条: 颜色 / 文案 / 出现时间 / 是否常驻。
#[derive(Clone)]
pub(crate) struct Toast {
    pub(crate) color: Color32,
    pub(crate) msg: String,
    pub(crate) since: Instant,
    /// 常驻提示: 不随超时消失, 直到被下一条提示替换(长等待的即时反馈)。
    pub(crate) sticky: bool,
}

/// 普通提示的停留时长; 常驻提示([`Toast::sticky`])不适用。
const TOAST_TTL: Duration = Duration::from_secs(6);

impl Toast {
    /// 是否该自动消失(常驻提示不超时)。
    pub(crate) fn expired(&self, elapsed: Duration) -> bool {
        !self.sticky && elapsed >= TOAST_TTL
    }
}

pub(crate) struct Global {
    /// 后台命令通道(worker 线程)。
    pub(crate) tx: Sender<Cmd>,
    /// 用户提示条; None = 不显示。
    pub(crate) toast: Option<Toast>,
    /// 系统(KDE)配色; 非 KDE 时为 None。
    pub(crate) kde_colors: Option<kde::KdeColors>,
    /// 上次轮询系统主题的时间。
    pub(crate) kde_checked: Instant,
    /// 请求 id 分配器; 各域共用同一命名空间 —— worker 的取消注册表按 req_id
    /// 索引下载 / 上传 / 预览三域, 重复的 id 会互相取消。启动起点由
    /// `App::restore_req_id` 按历史任务的最大 id 修正。
    pub(crate) req_id: u64,
}

impl Global {
    pub(crate) fn new(tx: Sender<Cmd>) -> Self {
        Self {
            tx,
            toast: None,
            kde_colors: kde::load(),
            kde_checked: Instant::now(),
            req_id: 0,
        }
    }

    pub(crate) fn send(&self, cmd: Cmd) {
        let _ = self.tx.send(cmd);
    }

    pub(crate) fn alloc_req_id(&mut self) -> u64 {
        self.req_id += 1;
        self.req_id
    }

    pub(crate) fn theme(&self) -> Theme {
        match &self.kde_colors {
            Some(k) => Theme::from_kde(k),
            None => Theme::fallback(),
        }
    }

    pub(crate) fn toast(&mut self, msg: &str, color: Color32) {
        self.toast = Some(Toast {
            color,
            msg: msg.to_string(),
            since: Instant::now(),
            sticky: false,
        });
    }

    /// 常驻提示: 不随超时消失, 直到被下一条提示替换(预览等待等长耗时反馈)。
    pub(crate) fn toast_sticky(&mut self, msg: &str, color: Color32) {
        self.toast = Some(Toast {
            color,
            msg: msg.to_string(),
            since: Instant::now(),
            sticky: true,
        });
    }

    /// 收回常驻提示(仅当当前提示为常驻时); 预览在途状态被整体作废时调用。
    pub(crate) fn clear_sticky_toast(&mut self) {
        if self.toast.as_ref().is_some_and(|t| t.sticky) {
            self.toast = None;
        }
    }

    pub(crate) fn toast_ok(&mut self, msg: &str) {
        self.toast(msg, self.theme().ok);
    }
    pub(crate) fn toast_warn(&mut self, msg: &str) {
        self.toast(msg, self.theme().warn);
    }
    pub(crate) fn toast_err(&mut self, msg: &str) {
        self.toast(msg, self.theme().danger);
    }

    /// 定期重读 kdeglobals, 让系统换主题后应用即时更新。
    pub(crate) fn poll_system_theme(&mut self) {
        if self.kde_checked.elapsed() < Duration::from_millis(1500) {
            return;
        }
        self.kde_checked = Instant::now();
        let fresh = kde::load();
        let changed = match (&self.kde_colors, &fresh) {
            (Some(a), Some(b)) => {
                a.accent != b.accent || a.dark != b.dark || a.view_bg != b.view_bg
            }
            (None, Some(_)) | (Some(_), None) => true,
            (None, None) => false,
        };
        if changed {
            self.kde_colors = fresh;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use eframe::egui::Color32;

    use super::{Toast, TOAST_TTL};

    fn toast(sticky: bool) -> Toast {
        Toast {
            color: Color32::WHITE,
            msg: "提示".into(),
            since: std::time::Instant::now(),
            sticky,
        }
    }

    #[test]
    fn sticky_toast_never_expires_plain_toast_times_out() {
        // 常驻提示只能被下一条提示替换, 不随超时消失。
        assert!(!toast(true).expired(TOAST_TTL * 10));
        let plain = toast(false);
        assert!(!plain.expired(TOAST_TTL - Duration::from_millis(1)));
        assert!(plain.expired(TOAST_TTL));
        assert!(plain.expired(TOAST_TTL + Duration::from_secs(1)));
    }
}
