//! 领域类型：对 PikPak API 数据的语义化封装。
//! 与线上的 JSON DTO 解耦——DTO 转换放在各 API 模块里（S2 起）。

use chrono::{DateTime, Utc};

/// 登录后持有的令牌对。
#[derive(Debug, Clone)]
pub struct TokenPair {
    pub access_token: String,
    pub refresh_token: String,
    /// access_token 的过期时刻
    pub expires_at: DateTime<Utc>,
}

impl TokenPair {
    /// 令牌是否已过期（留 10 秒安全余量，避免边界上发出必败请求）。
    pub fn is_expired(&self) -> bool {
        Utc::now() + chrono::Duration::seconds(10) >= self.expires_at
    }

    /// 是否应在 `margin` 之内刷新（主动刷新用）。
    pub fn needs_refresh(&self, margin: chrono::Duration) -> bool {
        Utc::now() + margin >= self.expires_at
    }
}

/// 条目类别：文件夹或文件。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Folder,
    File,
}

/// 文件或文件夹（列表项）。
#[derive(Debug, Clone)]
pub struct FileEntry {
    pub id: String,
    pub name: String,
    pub kind: EntryKind,
    /// 字节数；文件夹为 0
    pub size: u64,
    pub modified_at: Option<DateTime<Utc>>,
    pub md5: Option<String>,
    /// 审核状态异常（官方标记违规等），UI 需提示
    pub audit_flagged: bool,
    /// 官方归类为媒体（视频/音频），界面上可显示播放入口
    pub media: bool,
}

/// 离线任务阶段。
///
/// 官方的 `PHASE_TYPE_*` 集合并不能穷举（实测还有 `PAUSED` 与拼写为
/// `UNKNOW` 的未知态），未识别的取值一律落到 [`TaskPhase::Unknown`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskPhase {
    Pending,
    Running,
    Paused,
    Completed,
    Error,
    Unknown,
}

impl TaskPhase {
    /// 是否已经不再变化（完成或失败），轮询可据此退避。
    pub fn is_settled(self) -> bool {
        matches!(self, TaskPhase::Completed | TaskPhase::Error)
    }
}

/// 离线下载任务。
#[derive(Debug, Clone)]
pub struct OfflineTask {
    pub id: String,
    pub name: String,
    pub phase: TaskPhase,
    /// 进度百分比 0-100
    pub progress: u8,
    /// 官方返回的状态说明（如 "Saving"）
    pub message: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    /// 完成后产物对应的文件 id
    pub file_id: Option<String>,
    /// 产物大小（字节）；官方给字符串，缺失按 0 处理
    pub file_size: u64,
}

/// 网盘配额与会员状态。
#[derive(Debug, Clone, Default)]
pub struct Quota {
    /// 已用字节
    pub usage: u64,
    /// 总量；`None` 表示不限量
    pub limit: Option<u64>,
    pub vip: bool,
    /// 离线下载额度：已用字节；`None` 表示官方没下发
    pub cloud_download_usage: Option<u64>,
    /// 离线下载额度：上限；`Some(None)` 表示不限量
    pub cloud_download_limit: Option<Option<u64>>,
}

/// 串流链接类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamKind {
    /// 原文件直链：稳，mpv 直接放
    Original,
    /// 转码媒体（m3u8）：快，但偶发失效
    Media,
}

/// 给播放器的串流目标（带签名的临时直链）。
#[derive(Debug, Clone)]
pub struct StreamTarget {
    pub url: String,
    pub kind: StreamKind,
    /// 链接过期时刻
    pub expires_at: Option<DateTime<Utc>>,
}

/// 一个可播放的画质档位（PikPak 对会转码的视频给出多个流）。
///
/// `available` 为 false 的档位照样保留：界面要显示它的存在并说明原因
/// （`reason`），而不是凭空消失。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayOption {
    /// 显示名，如 `原画`、`1080P`
    pub label: String,
    /// 带签名的临时直链（只在 Rust 侧流转，不进日志）
    pub url: String,
    pub kind: StreamKind,
    /// 分辨率高度；未知为 None
    pub height: Option<u32>,
    /// 是否为原文件（未转码）
    pub original: bool,
    pub available: bool,
    /// 不可用原因（英文短标记，界面层翻译成中文）
    pub reason: Option<&'static str>,
}

impl PlayOption {
    /// 档位能否播放。
    pub fn playable(&self) -> bool {
        self.available && !self.url.is_empty()
    }

    /// 直链是否已过期（留 10 秒余量，与 rclone 一致）。
    ///
    /// 优先读 URL 里的 `expire` 查询参数（unix 秒）；解析不出时退回给出的
    /// `fallback` 时间。两者都没有时按「未知、未过期」处理。
    pub fn is_expired(&self, fallback: Option<DateTime<Utc>>) -> bool {
        let margin = chrono::Duration::seconds(10);
        let now = Utc::now();
        if let Some(seconds) = expire_query(&self.url) {
            return now + margin >= DateTime::from_timestamp(seconds, 0).unwrap_or(now);
        }
        fallback.is_some_and(|at| now + margin >= at)
    }
}

/// 从直链查询串里取 `expire`（unix 秒）。
fn expire_query(url: &str) -> Option<i64> {
    let query = url.split_once('?')?.1;
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == "expire").then(|| value.parse::<i64>().ok())?
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn token_expiry_has_margin() {
        let now = Utc::now();
        let tok = |delta: Duration| TokenPair {
            access_token: String::new(),
            refresh_token: String::new(),
            expires_at: now + delta,
        };
        // 还有 5 秒才到期 -> 视为已过期（余量 10 秒）
        assert!(tok(Duration::seconds(5)).is_expired());
        // 还有 1 分钟 -> 未过期
        assert!(!tok(Duration::minutes(1)).is_expired());
    }
}
