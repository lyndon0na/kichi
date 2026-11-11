//! 领域类型：对 PikPak API 数据的语义化封装。
//! 与线上的 JSON DTO 解耦——DTO 转换放在各 API 模块里（S2 起）。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntryKind {
    Folder,
    File,
}

impl EntryKind {
    /// API 的 `drive#folder` / `drive#file`；未知或缺失值留给调用方兜底。
    pub(crate) fn from_api_kind(kind: &str) -> Option<Self> {
        match kind.trim() {
            "drive#folder" => Some(Self::Folder),
            "drive#file" => Some(Self::File),
            _ => None,
        }
    }
}

/// 文件或文件夹（列表项）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    pub id: String,
    pub name: String,
    pub kind: EntryKind,
    /// 字节数；文件夹为 0
    pub size: u64,
    /// 官方创建时间，可能缺失
    pub created_at: Option<DateTime<Utc>>,
    pub modified_at: Option<DateTime<Utc>>,
    /// 直接父目录 id；根目录条目为空串
    pub parent_id: String,
    pub md5: Option<String>,
    /// 审核状态异常（官方标记违规等），UI 需提示
    pub audit_flagged: bool,
    /// 官方系统标签里带 `STAR`；详情接口的同名字段不可靠
    pub starred: bool,
    /// 官方归类为媒体（视频/音频），界面上可显示播放入口
    pub media: bool,
    /// 官方明确归类为视频（缩略图角标用）
    pub video: bool,
    /// 列表下发的缩略图地址。只在 Rust 侧用于下载到本地缓存，不映射给 QML。
    ///
    /// 签名地址会过期，页面快照不持久化它：反序列化后为空，等后台校准补回。
    #[serde(skip)]
    pub thumbnail_url: String,
}

/// 回收站条目：普通文件字段 + 原父目录与删除时间。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrashEntry {
    pub entry: FileEntry,
    /// 删除前所在目录；根目录为空串。仅作原始元数据保留，
    /// `batchUntrash` 的实际还原位置由 PikPak 决定，不能据此向用户保证原目录。
    pub parent_id: String,
    /// 官方返回的删除时间；缺失时界面显示空值。
    pub deleted_at: Option<DateTime<Utc>>,
}

/// 离线任务阶段。
///
/// 官方的 `PHASE_TYPE_*` 集合并不能穷举（实测还有 `PAUSED` 与拼写为
/// `UNKNOW` 的未知态），未识别的取值一律落到 [`TaskPhase::Unknown`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskPhase {
    Pending,
    Running,
    Paused,
    Completed,
    Error,
    Unknown,
}

impl TaskPhase {
    /// 官方任务阶段是否已经不再变化（完成或失败）。
    ///
    /// 列表带了 `with=reference_resource` 时，产物删除会覆盖展示阶段；界面判断
    /// 是否结束应使用 `OfflineTask::status()` 返回的 [`crate::TaskStatus`]。
    pub fn is_settled(self) -> bool {
        matches!(self, TaskPhase::Completed | TaskPhase::Error)
    }
}

/// 离线下载任务。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OfflineTask {
    pub id: String,
    pub name: String,
    /// 创建任务时提交的原始链接（任务响应里的 `params.url`）。
    ///
    /// 只用于「重新提交」；旧快照没有该字段时为 `None`，空串按未下发处理。
    #[serde(default)]
    pub source_url: Option<String>,
    pub phase: TaskPhase,
    /// 进度百分比 0-100
    pub progress: u8,
    /// 官方返回的状态说明（如 "Saving"）
    pub message: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    /// 完成后产物对应的文件 id
    pub file_id: Option<String>,
    /// 完成产物所在目录；资源未下发父目录时为 `None`，根目录为 `Some("")`。
    pub output_parent_id: Option<String>,
    /// 产物大小（字节）；官方给字符串，缺失按 0 处理
    pub file_size: u64,
    /// 产物类别；列表未带 `reference_resource` 时为 `None`
    pub kind: Option<EntryKind>,
    /// 产物缩略图地址。只在 Rust 侧用于下载到本地缓存，不映射给 QML。
    #[serde(skip)]
    pub thumbnail_url: String,
    /// 产物是否可由播放器打开（视频 / 音频）
    pub media: bool,
    /// 产物是否明确归类为视频（缩略图播放角标用）
    pub video: bool,
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

/// 一项月度传输额度（字节）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TransferAllowance {
    /// 上限为 0 表示官方没下发这一项
    pub limit: u64,
    pub used: u64,
}

/// 账号自己的月度传输额度，对应官方「传输配额」弹窗。
///
/// 与 [`Quota`] 分开统计：秒传 / 离线保存可能不改变网盘容量，但仍消耗传输额度。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TransferQuota {
    /// 云下载（离线保存成功文件的当月流量）
    pub cloud_download: TransferAllowance,
    /// 下行（播放 / 查看 / 下载的当月流量）
    pub download: TransferAllowance,
    /// 上传（当月上传文件的流量）
    pub upload: TransferAllowance,
    /// 非会员每日下行上限；会员为 0
    pub download_daily: TransferAllowance,
    pub premium: bool,
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
    /// 归到哪个可选档位（[`TIERS`] 里的某一项）。菜单固定按这几个档位列出。
    pub tier: Option<Tier>,
    pub available: bool,
    /// 不可用原因（英文短标记，界面层翻译成中文）
    pub reason: Option<&'static str>,
}

/// 可选清晰度档位。全部可选项见 [`TIERS`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Original,
    Transcode(&'static str),
}

/// 固定可选档位，顺序即菜单顺序。`kichi-core` 不依赖设置层，所以这里是唯一出处，
/// `settings::TRANSCODE_TIERS` 从这里派生。
pub const TIERS: [Tier; 4] = [
    Tier::Original,
    Tier::Transcode("1080P"),
    Tier::Transcode("720P"),
    Tier::Transcode("480P"),
];

impl Tier {
    /// 界面上的名字。
    pub fn label(self) -> &'static str {
        match self {
            Self::Original => "原画",
            Self::Transcode(name) => name,
        }
    }

    /// 设置里存的字符串（与 `Quality::as_str` 同一个词表）。
    pub fn setting_value(self) -> &'static str {
        match self {
            Self::Original => "original",
            Self::Transcode("1080P") => "transcode:1080P",
            Self::Transcode("720P") => "transcode:720P",
            Self::Transcode("480P") => "transcode:480P",
            Self::Transcode(_) => "ask",
        }
    }

    /// 档位名归到哪个可选档位（大小写不敏感）；归不进去返回 None。
    pub fn from_stream_name(name: &str) -> Option<Self> {
        let name = name.trim();
        TIERS
            .iter()
            .copied()
            .find(|tier| tier.label().eq_ignore_ascii_case(name))
    }
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

/// 按「用户点的那一档」挑出实际要播的档位，并给出一句回退说明。
///
/// 规则（2026-10-26 已定）：
/// - 点了原画：有可用原画就播；没有就用能用的最高转码档。
/// - 点了 1080P / 720P / 480P：有这一档就播；**没有就回退原画**，不自动降到别的转码档
///   （要 1080P 时给 720P 属于偷换）；原画也拿不到时，用能用的最高转码档。
/// - 什么都拿不到：`None`。
///
/// 第二个返回值只在**回退**时是 `Some(用户点的那一档的名字)`，用来拼
/// 「没有 1080P，已播放原画」这类提示。
pub fn resolve(options: &[PlayOption], want: Tier) -> Option<(PlayOption, Option<&'static str>)> {
    let playable: Vec<&PlayOption> = options.iter().filter(|option| option.playable()).collect();
    if playable.is_empty() {
        return None;
    }
    let original = || playable.iter().copied().find(|option| option.original);
    let best_transcode = || {
        playable
            .iter()
            .copied()
            .filter(|option| !option.original)
            .max_by_key(|option| option.height.unwrap_or(0))
    };
    match want {
        Tier::Original => original()
            .map(|option| (option.clone(), None))
            .or_else(|| best_transcode().map(|option| (option.clone(), Some(want.label())))),
        Tier::Transcode(_) => {
            let exact = playable
                .iter()
                .copied()
                .find(|option| option.tier == Some(want) && !option.original);
            if let Some(option) = exact {
                return Some((option.clone(), None));
            }
            original()
                .map(|option| (option.clone(), Some(want.label())))
                .or_else(|| best_transcode().map(|option| (option.clone(), Some(want.label()))))
        }
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

#[cfg(test)]
mod resolve_tests {
    use super::*;

    fn option(label: &str, height: Option<u32>, original: bool, available: bool) -> PlayOption {
        PlayOption {
            label: label.into(),
            url: "https://cdn.invalid/x?sign=S".into(),
            kind: StreamKind::Original,
            height,
            original,
            tier: Tier::from_stream_name(label),
            available,
            reason: (!available).then_some("quota"),
        }
    }

    #[test]
    fn requested_tier_is_played_as_is() {
        let all = vec![
            option("原画", None, true, true),
            option("1080P", Some(1080), false, true),
            option("720P", Some(720), false, true),
        ];
        let (played, note) = resolve(&all, Tier::Transcode("1080P")).expect("能播");
        assert_eq!(played.label, "1080P");
        assert_eq!(note, None, "命中时不需要回退说明");
    }

    #[test]
    fn missing_tier_falls_back_to_original_and_says_so() {
        let all = vec![
            option("原画", None, true, true),
            option("720P", Some(720), false, true),
        ];
        let (played, note) = resolve(&all, Tier::Transcode("1080P")).expect("能播");
        assert_eq!(played.label, "原画");
        assert_eq!(note, Some("1080P"), "要告诉用户点的档不在，播的是原画");
    }

    #[test]
    fn tier_match_ignores_case() {
        let all = vec![option("1080p", Some(1080), false, true)];
        let (played, _) = resolve(&all, Tier::Transcode("1080P")).expect("能播");
        assert_eq!(played.label, "1080p");
    }

    #[test]
    fn unavailable_original_uses_best_transcode() {
        let all = vec![
            option("原画", None, true, false),
            option("720P", Some(720), false, true),
            option("480P", Some(480), false, true),
        ];
        let (played, note) = resolve(&all, Tier::Transcode("1080P")).expect("能播");
        assert_eq!(played.label, "720P", "回退原画失败后用能用的最高档");
        assert_eq!(note, Some("1080P"));
        // 直接点原画、但原画不可用：同样回退到最高转码档
        let (played, note) = resolve(&all, Tier::Original).expect("能播");
        assert_eq!(played.label, "720P");
        assert_eq!(note, Some("原画"));
    }

    #[test]
    fn nothing_playable_returns_none() {
        let all = vec![option("原画", None, true, false)];
        assert!(resolve(&all, Tier::Original).is_none());
        assert!(resolve(&all, Tier::Transcode("1080P")).is_none());
        assert!(resolve(&[], Tier::Original).is_none());
    }
}
