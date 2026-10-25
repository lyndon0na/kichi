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
}

/// 离线任务阶段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskPhase {
    Pending,
    Running,
    Completed,
    Error,
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
}

/// 网盘配额与会员状态。
#[derive(Debug, Clone, Default)]
pub struct Quota {
    /// 已用字节
    pub usage: u64,
    /// 总量；`None` 表示不限量
    pub limit: Option<u64>,
    pub vip: bool,
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
