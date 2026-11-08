//! 容量 / 传输配额 / 会话状态的界面文案。

use crate::session::State;

pub(crate) struct QuotaDisplay {
    pub(crate) used: String,
    pub(crate) limit: String,
    /// < 0 = 不限量 / 未知，QML 据此隐藏容量条
    pub(crate) ratio: f64,
}

/// 容量显示：账号区用 TB 档，避免 10 TB 显示成「10240.0 GB」。
pub(crate) fn quota_display(quota: &kichi_core::Quota) -> QuotaDisplay {
    let used = crate::format::capacity(quota.usage);
    let limit = quota
        .limit
        .map(crate::format::capacity)
        .unwrap_or_else(|| "无限制".to_owned());
    // 不限量 / 上限为 0 时给 -1，QML 隐藏进度条，避免除零和「永远 0%」的假象
    let ratio = match quota.limit {
        None | Some(0) => -1.0,
        Some(limit) => (quota.usage as f64 / limit as f64).clamp(0.0, 1.0),
    };
    QuotaDisplay { used, limit, ratio }
}

pub(crate) struct TransferDisplay {
    pub(crate) cloud: String,
    pub(crate) download: String,
    pub(crate) upload: String,
    pub(crate) reset: String,
    /// 0..1；`-1` = 没有上限，界面不画进度条
    pub(crate) cloud_ratio: f64,
    pub(crate) download_ratio: f64,
    pub(crate) upload_ratio: f64,
}

/// 额度占用比例；没给上限时返回 -1，和底栏容量条同一约定。
fn allowance_ratio(allowance: kichi_core::TransferAllowance) -> f64 {
    if allowance.limit == 0 {
        return -1.0;
    }
    (allowance.used as f64 / allowance.limit as f64).clamp(0.0, 1.0)
}

/// 传输配额显示：默认按官方说明的下个自然月 1 日 00:00（SGT）重置。
pub(crate) fn transfer_display(
    quota: &kichi_core::TransferQuota,
    now: chrono::DateTime<chrono::Utc>,
) -> TransferDisplay {
    let text = |allowance: kichi_core::TransferAllowance| {
        let used = crate::format::capacity(allowance.used);
        if allowance.limit == 0 {
            // 官方没下发上限时只报已用，不编造一个 0 B 的上限
            format!("已用 {used}")
        } else {
            format!("已用 {used} / {}", crate::format::capacity(allowance.limit))
        }
    };
    TransferDisplay {
        cloud: text(quota.cloud_download),
        download: text(quota.download),
        upload: text(quota.upload),
        reset: next_reset_text(now),
        cloud_ratio: allowance_ratio(quota.cloud_download),
        download_ratio: allowance_ratio(quota.download),
        upload_ratio: allowance_ratio(quota.upload),
    }
}

/// 下个自然月的 1 日 00:00 SGT（UTC+8）。官方接口不返回重置时间。
fn next_reset_text(now: chrono::DateTime<chrono::Utc>) -> String {
    use chrono::{Datelike, FixedOffset};
    let Some(sgt) = FixedOffset::east_opt(8 * 3600) else {
        return String::new();
    };
    let now = now.with_timezone(&sgt);
    let (year, month) = if now.month() == 12 {
        (now.year() + 1, 1)
    } else {
        (now.year(), now.month() + 1)
    };
    format!("{year}/{month}/1 00:00 SGT")
}

/// 会话状态 → QML 用的字符串。
pub(crate) fn state_name(state: State) -> &'static str {
    match state {
        State::SignedIn => "signedIn",
        State::Expired => "expired",
        State::SignedOut => "signedOut",
    }
}

#[cfg(test)]
mod tests {
    use super::{next_reset_text, transfer_display};
    use chrono::{TimeZone, Utc};
    use kichi_core::{TransferAllowance, TransferQuota};

    #[test]
    fn transfer_display_formats_the_three_monthly_allowances() {
        let quota = TransferQuota {
            cloud_download: TransferAllowance {
                limit: 40 * 1024 * 1024 * 1024 * 1024,
                used: 1024 * 1024 * 1024,
            },
            download: TransferAllowance {
                limit: 4 * 1024 * 1024 * 1024 * 1024,
                used: 0,
            },
            upload: TransferAllowance {
                limit: 0,
                used: 512 * 1024 * 1024,
            },
            ..TransferQuota::default()
        };
        let now = Utc.with_ymd_and_hms(2026, 11, 6, 12, 0, 0).unwrap();
        let display = transfer_display(&quota, now);
        assert_eq!(display.cloud, "已用 1.0 GB / 40.0 TB");
        assert_eq!(display.download, "已用 0 B / 4.0 TB");
        assert_eq!(display.upload, "已用 512.0 MB", "没给上限时只报已用");
        assert_eq!(display.reset, "2026/12/1 00:00 SGT");
        assert!((display.cloud_ratio - 1.0 / 40960.0).abs() < 1e-9);
        assert_eq!(display.download_ratio, 0.0);
        assert_eq!(display.upload_ratio, -1.0, "没上限不画进度条");
    }

    #[test]
    fn reset_rolls_over_after_december_in_sgt() {
        // UTC 2026-12-31 17:00 已是 SGT 2027-01-01 01:00
        let now = Utc.with_ymd_and_hms(2026, 12, 31, 17, 0, 0).unwrap();
        assert_eq!(next_reset_text(now), "2027/2/1 00:00 SGT");
        let now = Utc.with_ymd_and_hms(2026, 11, 30, 15, 59, 0).unwrap();
        assert_eq!(next_reset_text(now), "2026/12/1 00:00 SGT");
    }
}
