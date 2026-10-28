//! 传输页的速度 / 剩余时间 / 卡住判定与说明存活：全是纯逻辑。
//!
//! 从 `transfers_view.rs` 拆出来守行数上限：那边管「字段叫什么、什么时候
//! 显示什么」，这里管「5 秒窗口里有多快、还剩多久、多久没收到字节」。
//! `Transfer` 只存 [`SpeedMeter`] / [`NoteState`] 两个小状态；判定时把
//! `now` 传进来（生产用 `Instant::now()`，测试注入时间，不 sleep）。
//!
//! 规则见 `docs/adr/0006-transfer-tasks-page.md`：
//!
//! - 进度只走 `transferLive`；**卡住秒数靠主线程 1Hz 心跳重算**，不能等
//!   进度回调——没有字节就没有回调，秒数会停在原地（`docs/pitfalls.md`）。
//! - 相同字节的重复回调不刷新卡住时钟；字节回退（续传被截断 / 从头下）
//!   清空速度窗口重新采样。
//! - 整批剩余排除失败 / 取消；任一未完成文件大小未知就不给 ETA（不编造数字）。

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use super::actions::format_size;
use super::transfers::{Transfer, TransferState};

/// 速度的滑动窗口。
const SPEED_WINDOW: Duration = Duration::from_secs(5);
/// 窗口里首尾采样跨度不足时先不给速度（首帧 / 刚恢复）。
const MIN_SPAN: Duration = Duration::from_millis(250);
/// 超过这么久没有新字节就算卡住。
const STALL_AFTER: Duration = Duration::from_secs(5);
/// 恢复后重试 / 续传说明再保留这么久。
const NOTE_HOLD: Duration = Duration::from_secs(5);
/// 主线程心跳间隔：卡住秒数每秒走一格。
pub(crate) const LIVE_HEARTBEAT: Duration = Duration::from_secs(1);
/// 100ms 内的多次回调合并成一个采样点。
const SAMPLE_MERGE: Duration = Duration::from_millis(100);
/// ETA 超过 5 小时只报「5 小时以上」。
const ETA_CAP_SECONDS: u64 = 5 * 3600;

/// 一条记录的速度窗口与卡住时钟（只在内存，不落盘）。
#[derive(Debug, Clone, Default)]
pub(crate) struct SpeedMeter {
    /// 5 秒窗口内的 (时刻, 已下字节)；100ms 内的回调合并成一点
    samples: VecDeque<(Instant, u64)>,
    /// 最后一次「字节增加」的时刻（`begin` 也算），卡住时钟从这里走
    last_event: Option<Instant>,
}

impl SpeedMeter {
    /// 行变「下载中」：清空窗口，卡住时钟从这一刻开始。
    ///
    /// 一个字节都没收到也算卡住，所以起点必须在这里落下，而不是等首次进度。
    pub(crate) fn begin(&mut self, at: Instant) {
        self.samples.clear();
        self.last_event = Some(at);
    }

    /// 一次进度回调。
    ///
    /// 首次采样只当窗口起点：续传时 offset 可能已经很大，把它算成速度会
    /// 得到几十 MB/s 的假数字。之后的字节增加才推进卡住时钟。
    pub(crate) fn observe(&mut self, at: Instant, bytes: u64) {
        let Some(previous) = self.samples.back().copied() else {
            self.samples.push_back((at, bytes));
            return;
        };
        if bytes < previous.1 {
            // 续传被截断 / 从头下：旧窗口作废，重新采样（这是一次真实的新事件，
            // 卡住时钟也跟着重来，否则重启后的头几秒会被误报成「已卡住」）
            self.samples.clear();
            self.samples.push_back((at, bytes));
            self.last_event = Some(at);
            return;
        }
        if bytes == previous.1 {
            // 相同字节的重复回调不刷新卡住时钟——否则断流时秒数永远走不动
            return;
        }
        self.last_event = Some(at);
        if at.saturating_duration_since(previous.0) < SAMPLE_MERGE {
            if let Some(back) = self.samples.back_mut() {
                back.1 = bytes;
            }
        } else {
            self.samples.push_back((at, bytes));
        }
        self.prune(at);
    }

    /// 只保留窗口内的采样点（避免一次长下载攒下成千上万个点）。
    fn prune(&mut self, now: Instant) {
        while self
            .samples
            .front()
            .is_some_and(|(at, _)| now.saturating_duration_since(*at) > SPEED_WINDOW)
        {
            self.samples.pop_front();
        }
    }

    /// 5 秒窗口内的平均速度；采样跨度不足 / 窗口过期时 `None`。
    pub(crate) fn bytes_per_sec(&self, now: Instant) -> Option<u64> {
        let mut window = self
            .samples
            .iter()
            .filter(|(at, _)| now.saturating_duration_since(*at) <= SPEED_WINDOW);
        let (first_at, first_bytes) = *window.next()?;
        let mut last = (first_at, first_bytes);
        for sample in window {
            last = *sample;
        }
        let span = last.0.saturating_duration_since(first_at);
        if span < MIN_SPAN {
            return None;
        }
        let grown = last.1.saturating_sub(first_bytes);
        if grown == 0 {
            return None;
        }
        let rate = grown as f64 / span.as_secs_f64();
        Some(rate.round() as u64)
    }

    /// 距最后一次字节增加多久（`begin` 也算）；还没开始过就是 `None`。
    pub(crate) fn stalled_for(&self, now: Instant) -> Option<Duration> {
        self.last_event.map(|at| now.saturating_duration_since(at))
    }
}

/// 重试 / 续传说明的存活状态（说明文字本身仍存在 `Transfer::detail`）。
#[derive(Debug, Clone, Default)]
pub(crate) struct NoteState {
    /// 写说明的时刻；`None` = 这条记录还没写过说明
    at: Option<Instant>,
    /// 写说明时的已下字节
    bytes: u64,
    /// 之后第一次字节变化
    resumed_at: Option<Instant>,
}

impl NoteState {
    /// 写一条重试 / 续传说明：重新计时。
    pub(crate) fn write(&mut self, at: Instant, bytes: u64) {
        self.at = Some(at);
        self.bytes = bytes;
        self.resumed_at = None;
    }

    /// 进度回调：第一次字节变化记为「恢复」，此后说明再保留 5 秒。
    pub(crate) fn observe(&mut self, at: Instant, bytes: u64) {
        let Some(note_at) = self.at else {
            return;
        };
        if at < note_at {
            return;
        }
        if bytes < self.bytes {
            // 从头重下：说明的基线跟着退回去，否则要等重新下满旧位置才消失
            self.bytes = bytes;
            self.resumed_at = None;
        } else if bytes > self.bytes && self.resumed_at.is_none() {
            self.resumed_at = Some(at);
        }
    }

    /// 说明现在还算数吗（未恢复前一直显示；恢复后保留 [`NOTE_HOLD`]）。
    pub(crate) fn visible(&self, now: Instant) -> bool {
        if self.at.is_none() {
            return false;
        }
        match self.resumed_at {
            Some(resumed) => now.saturating_duration_since(resumed) < NOTE_HOLD,
            None => true,
        }
    }
}

/// 一行在 `transferLive` 里多出来的两段文案（其余字段仍是进度 / 大小）。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct RateInfo {
    /// 速度 + 剩余时间，或「已卡住 N 秒」；没有可显示的内容时为空串
    pub(crate) rate: String,
    /// 重试 / 续传说明（过期为空串）
    pub(crate) note: String,
}

/// 单文件行：自己的速度与剩余时间。
pub(crate) fn file_rate_info(record: &Transfer, now: Instant) -> RateInfo {
    if record.state != TransferState::Running {
        return RateInfo::default();
    }
    RateInfo {
        rate: running_rate_text(&record.speed, remaining_bytes(record), now),
        note: note_text(record, now),
    }
}

/// 文件夹父行：速度取当前子行，剩余时间取整批。
pub(crate) fn batch_rate_info(records: &[Transfer], batch: u64, now: Instant) -> RateInfo {
    let mut running: Option<&Transfer> = None;
    let mut remaining: Option<u64> = Some(0);
    for record in records
        .iter()
        .filter(|record| record.batch == batch && record.child)
    {
        match record.state {
            // 已完成不占剩余；失败 / 取消的残量不再算进整批；已中断的行不参与 ETA
            TransferState::Done
            | TransferState::Failed
            | TransferState::Cancelled
            | TransferState::Interrupted => {}
            TransferState::Running => {
                if running.is_none() {
                    running = Some(record);
                }
                remaining = remaining
                    .zip(remaining_bytes(record))
                    .map(|(sum, bytes)| sum.saturating_add(bytes));
            }
            TransferState::Queued | TransferState::Preparing => {
                // 未开始的按整个大小计入；大小未知（含 0 字节排队）就整个不给 ETA
                remaining = remaining
                    .zip(record.total)
                    .map(|(sum, total)| sum.saturating_add(total));
            }
        }
    }
    let Some(running) = running else {
        // 两个文件之间：没有正在跑的子行，不猜速度
        return RateInfo::default();
    };
    RateInfo {
        rate: running_rate_text(&running.speed, remaining, now),
        note: note_text(running, now),
    }
}

/// 这个文件还剩多少字节；总大小未知时 `None`（ETA 跟着未知）。
fn remaining_bytes(record: &Transfer) -> Option<u64> {
    record.total.map(|total| total.saturating_sub(record.bytes))
}

fn note_text(record: &Transfer, now: Instant) -> String {
    if record.note.visible(now) {
        record.detail.clone()
    } else {
        String::new()
    }
}

/// 卡住优先于速度：≥5 秒没有新字节就只报「已卡住 N 秒」。
fn running_rate_text(speed: &SpeedMeter, remaining: Option<u64>, now: Instant) -> String {
    if let Some(stalled) = speed.stalled_for(now) {
        if stalled >= STALL_AFTER {
            return format!("已卡住 {} 秒", stalled.as_secs());
        }
    }
    let Some(rate) = speed.bytes_per_sec(now) else {
        return String::new();
    };
    let speed_text = format!("{}/s", format_size(rate));
    match remaining.and_then(|remaining| eta_text(remaining, rate)) {
        Some(eta) => format!("{speed_text} · {eta}"),
        None => speed_text,
    }
}

/// 剩余时间文案；不编造数字：速度为 0 / 已经下满 / 大小未知都给 `None`。
fn eta_text(remaining: u64, rate: u64) -> Option<String> {
    if rate == 0 || remaining == 0 {
        return None;
    }
    let seconds = remaining.div_ceil(rate);
    Some(format!("剩 {}", duration_text(seconds)))
}

/// 秒数 → 分档文案（卡住秒数不走这里，一直用秒）。
fn duration_text(seconds: u64) -> String {
    if seconds > ETA_CAP_SECONDS {
        return "5 小时以上".to_owned();
    }
    if seconds >= 3600 {
        let hours = seconds / 3600;
        let minutes = (seconds % 3600) / 60;
        return if minutes == 0 {
            format!("{hours} 小时")
        } else {
            format!("{hours} 小时 {minutes} 分")
        };
    }
    if seconds >= 60 {
        let minutes = seconds / 60;
        let rest = seconds % 60;
        return if rest == 0 {
            format!("{minutes} 分")
        } else {
            format!("{minutes} 分 {rest} 秒")
        };
    }
    format!("{seconds} 秒")
}

/// 主线程心跳是否该重算 `transferLive`（距上次推送 ≥ 1 秒）。
pub(crate) fn heartbeat_due(last: Option<Instant>, now: Instant) -> bool {
    last.is_none_or(|last| now.saturating_duration_since(last) >= LIVE_HEARTBEAT)
}

#[cfg(test)]
#[path = "transfers_rate_tests.rs"]
mod tests;
