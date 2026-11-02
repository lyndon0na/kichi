//! 账号事件：读取播放历史 / 最近保存，并上报播放进度。
//!
//! 请求形状对照 PikPak 官方 Web 客户端与 pikpak-kotlin（MIT）的
//! `EventEndpoint.kt` / 响应记录：
//! - 播放历史是 `TYPE_PLAY` 事件，按最近播放排序，每个文件一条；
//! - 最近保存是 `TYPE_UPLOAD` / `TYPE_RESTORE` 事件，按更新时间排序；
//! - `play_seconds` / `play_duration` 是十进制字符串；
//! - 上报成功返回 `{}`。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer};
use serde_json::{json, Value};

use crate::drive::{lenient_time, DriveClient, FileDto};
use crate::http::Body;
use crate::types::FileEntry;
use crate::Error;

/// 播放历史单页最多条数（服务端当前上限）。
pub const PLAY_HISTORY_LIMIT: u32 = 100;
/// 最近保存单页最多条数（服务端当前上限）。
pub const RECENT_SAVED_LIMIT: u32 = 100;

const PLAY_FILTERS: &str = r#"{"type":{"in":"TYPE_PLAY"}}"#;
const EVENT_TYPE_PLAY: &str = "TYPE_PLAY";
const RECENT_SAVED_FILTERS: &str = r#"{"type":{"in":"TYPE_UPLOAD,TYPE_RESTORE"}}"#;
const EVENT_TYPE_UPLOAD: &str = "TYPE_UPLOAD";
const EVENT_TYPE_RESTORE: &str = "TYPE_RESTORE";

/// 官方播放进度（整数秒）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaybackProgress {
    pub seconds: u64,
    pub duration: u64,
}

impl PlaybackProgress {
    /// 规范化成非零、非满且不超过总时长的进度。
    ///
    /// 服务端会把 `play_seconds == play_duration` 的满进度保存成 0；上报前
    /// 必须保留至少一秒余量，完成状态由客户端显示层表达。
    pub fn normalized(self) -> Option<Self> {
        if self.duration == 0 {
            return None;
        }
        let seconds = self.seconds.min(self.duration.saturating_sub(1));
        (seconds > 0).then_some(Self {
            seconds,
            duration: self.duration,
        })
    }
}

/// 一条官方播放历史。
#[derive(Debug, Clone)]
pub struct PlaybackHistoryEntry {
    /// 事件 id；部分响应可能为空。
    pub id: String,
    pub file_id: String,
    /// 事件更新时间；用于资料库页排序。
    pub updated_at: Option<DateTime<Utc>>,
    /// 进度 0 秒仍保留历史并显示 `0%`；总时长缺失时页面显示「—」。
    pub progress: Option<PlaybackProgress>,
    /// 事件附带的完整文件；不完整时为 `None`，调用方再按需读取只读详情。
    pub reference_resource: Option<FileEntry>,
}

/// 一条最近保存事件。
///
/// `resource` 只在官方内嵌的参考资源足够完整时才有值；否则会话层按
/// `file_id` 补一次只读详情请求。
#[derive(Debug, Clone)]
pub struct RecentSavedEvent {
    pub event_id: Option<String>,
    pub file_id: String,
    pub occurred_at: Option<DateTime<Utc>>,
    pub resource: Option<FileEntry>,
}

impl DriveClient {
    /// 读取一页官方播放历史（`page_token = None` 表示第一页）。
    pub fn list_play_history(
        &self,
        page_token: Option<&str>,
    ) -> Result<(Vec<PlaybackHistoryEntry>, Option<String>), Error> {
        let query = play_history_query(page_token);
        let response: EventListResponse =
            self.get_with_query(&self.config().events_url(), &query)?;
        let events = response
            .events
            .into_iter()
            .filter_map(EventDto::into_history_entry)
            .collect();
        let next = response
            .next_page_token
            .map(|token| token.trim().to_owned())
            .filter(|token| !token.is_empty());
        Ok((events, next))
    }

    /// 读取一页最近保存事件（上传 + 保存 / 转存），按官方更新时间新到旧。
    pub fn list_recent_saved(
        &self,
        page_token: Option<&str>,
    ) -> Result<(Vec<RecentSavedEvent>, Option<String>), Error> {
        let query = recent_saved_query(page_token);
        let response: EventListResponse =
            self.get_with_query(&self.config().events_url(), &query)?;
        let events = response
            .events
            .into_iter()
            .filter_map(EventDto::into_recent_saved)
            .collect();
        let next = response
            .next_page_token
            .map(|token| token.trim().to_owned())
            .filter(|token| !token.is_empty());
        Ok((events, next))
    }

    /// 写入官方播放历史。位置或总时长为 0 时不发请求。
    pub fn report_play(&self, file_id: &str, progress: PlaybackProgress) -> Result<(), Error> {
        let file_id = file_id.trim();
        if file_id.is_empty() {
            return Err(Error::InvalidInput("播放记录缺少文件 id".into()));
        }
        let Some(progress) = progress.normalized() else {
            return Ok(());
        };
        let body = play_report_body(file_id, progress, EVENT_TYPE_PLAY);
        let _: Value = self.request(
            reqwest::Method::POST,
            &self.config().events_url(),
            &[],
            Body::Json(&body),
        )?;
        Ok(())
    }
}

fn play_history_query(page_token: Option<&str>) -> Vec<(String, String)> {
    let mut query = vec![
        ("limit".to_owned(), PLAY_HISTORY_LIMIT.to_string()),
        ("thumbnail_size".to_owned(), "SIZE_SMALL".to_owned()),
        ("filters".to_owned(), PLAY_FILTERS.to_owned()),
    ];
    if let Some(token) = page_token.map(str::trim).filter(|token| !token.is_empty()) {
        query.push(("page_token".to_owned(), token.to_owned()));
    }
    query
}

fn recent_saved_query(page_token: Option<&str>) -> Vec<(String, String)> {
    let mut query = vec![
        ("limit".to_owned(), RECENT_SAVED_LIMIT.to_string()),
        ("thumbnail_size".to_owned(), "SIZE_SMALL".to_owned()),
        ("filters".to_owned(), RECENT_SAVED_FILTERS.to_owned()),
    ];
    if let Some(token) = page_token.map(str::trim).filter(|token| !token.is_empty()) {
        query.push(("page_token".to_owned(), token.to_owned()));
    }
    query
}

fn play_report_body(file_id: &str, progress: PlaybackProgress, event_type: &str) -> Value {
    json!({
        "event": {
            "type": event_type,
            "file_id": file_id,
            "params": {
                "play_duration": progress.duration.to_string(),
                "play_seconds": progress.seconds.to_string(),
            }
        }
    })
}

#[derive(Deserialize)]
struct EventListResponse {
    #[serde(default)]
    events: Vec<EventDto>,
    #[serde(default)]
    next_page_token: Option<String>,
}

#[derive(Deserialize)]
struct EventDto {
    #[serde(default, deserialize_with = "lenient_string")]
    id: String,
    #[serde(default, alias = "fileId", deserialize_with = "lenient_string")]
    file_id: String,
    #[serde(default, rename = "type")]
    event_type: String,
    #[serde(default, alias = "updatedTime", deserialize_with = "lenient_time")]
    updated_time: Option<DateTime<Utc>>,
    #[serde(default, alias = "createdTime", deserialize_with = "lenient_time")]
    created_time: Option<DateTime<Utc>>,
    #[serde(default)]
    params: Option<PlayParamsDto>,
    #[serde(default, alias = "referenceResource")]
    reference_resource: Option<FileDto>,
}

#[derive(Default, Deserialize)]
struct PlayParamsDto {
    #[serde(default, deserialize_with = "lenient_seconds")]
    play_seconds: u64,
    #[serde(default, deserialize_with = "lenient_seconds")]
    play_duration: u64,
    #[serde(default, deserialize_with = "lenient_string")]
    file_id: String,
}

impl EventDto {
    fn into_history_entry(self) -> Option<PlaybackHistoryEntry> {
        if !self.event_type.is_empty() && self.event_type != EVENT_TYPE_PLAY {
            return None;
        }
        let params = self.params.unwrap_or_default();
        let resource_id = self
            .reference_resource
            .as_ref()
            .map(FileDto::event_resource_id)
            .unwrap_or_default()
            .to_owned();
        let file_id = [self.file_id, params.file_id, resource_id]
            .into_iter()
            .map(|id| id.trim().to_owned())
            .find(|id| !id.is_empty())?;
        let progress = playback_progress(params.play_seconds, params.play_duration);
        let reference_resource = self
            .reference_resource
            .and_then(|resource| resource.into_complete_entry(&file_id));
        Some(PlaybackHistoryEntry {
            id: self.id.trim().to_owned(),
            file_id,
            updated_at: self.updated_time,
            progress,
            reference_resource,
        })
    }

    fn into_recent_saved(self) -> Option<RecentSavedEvent> {
        if !matches!(
            self.event_type.as_str(),
            EVENT_TYPE_UPLOAD | EVENT_TYPE_RESTORE
        ) {
            return None;
        }
        let resource_id = self
            .reference_resource
            .as_ref()
            .map(FileDto::event_resource_id)
            .unwrap_or_default()
            .to_owned();
        let params_file_id = self
            .params
            .as_ref()
            .map(|params| params.file_id.trim().to_owned())
            .unwrap_or_default();
        let file_id = [self.file_id, params_file_id, resource_id]
            .into_iter()
            .map(|id| id.trim().to_owned())
            .find(|id| !id.is_empty())?;
        let event_id = (!self.id.trim().is_empty()).then(|| self.id.trim().to_owned());
        let occurred_at = self.updated_time.or(self.created_time);
        let resource = self
            .reference_resource
            .and_then(|resource| resource.into_complete_entry(&file_id));
        Some(RecentSavedEvent {
            event_id,
            file_id,
            occurred_at,
            resource,
        })
    }
}

/// 播放历史保留 0 秒事件（界面显示 `0%`），但总时长缺失时没有可展示进度。
/// 0 秒不是看完标记；服务端会把满进度归零，所以完成记录保留在片尾前一秒，
/// 由界面显示为 `100%`。位置超过总时长时仍保留读取到的实际值，续播路径还会
/// 再走一次 `normalized()`，因而 0 秒不会被误当成可恢复位置。
fn playback_progress(seconds: u64, duration: u64) -> Option<PlaybackProgress> {
    (duration > 0).then_some(PlaybackProgress {
        seconds: seconds.min(duration),
        duration,
    })
}

/// 数字字段可能是字符串、数字、null 或空串；坏值按 0 处理，不让整页失败。
fn lenient_seconds<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    Ok(match value {
        Value::Number(number) => number.as_u64().unwrap_or(0),
        Value::String(text) => text.trim().parse::<u64>().unwrap_or(0),
        _ => 0,
    })
}

/// id / token 等字符串字段可能给 null；统一降级成空串。
fn lenient_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    Ok(match value {
        Value::String(text) => text,
        _ => String::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_query_matches_the_web_client() {
        let query = play_history_query(None);
        let get = |key: &str| {
            query
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, value)| value.as_str())
        };
        assert_eq!(get("limit"), Some("100"));
        assert_eq!(get("thumbnail_size"), Some("SIZE_SMALL"));
        assert_eq!(get("filters"), Some(r#"{"type":{"in":"TYPE_PLAY"}}"#));
        assert_eq!(get("page_token"), None);

        let paged = play_history_query(Some("next"));
        assert!(paged.contains(&("page_token".to_owned(), "next".to_owned())));

        let encoded = play_history_query(Some("%2Fnext%2Btoken"));
        assert!(encoded.contains(&("page_token".to_owned(), "%2Fnext%2Btoken".to_owned())));
    }

    #[test]
    fn recent_saved_query_matches_upload_and_restore_events() {
        let query = recent_saved_query(Some("next"));
        let get = |key: &str| {
            query
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, value)| value.as_str())
        };
        assert_eq!(get("limit"), Some("100"));
        assert_eq!(get("thumbnail_size"), Some("SIZE_SMALL"));
        assert_eq!(
            get("filters"),
            Some(r#"{"type":{"in":"TYPE_UPLOAD,TYPE_RESTORE"}}"#)
        );
        assert_eq!(get("page_token"), Some("next"));
    }

    #[test]
    fn history_parses_resource_time_and_lenient_progress() {
        let raw = r#"{
            "events": [
                {"id":"event-1","type":"TYPE_PLAY","file_id":"f1",
                 "updated_time":"2026-10-31T08:00:00Z",
                 "params":{"play_seconds":"115","play_duration":"162"},
                 "reference_resource":{"name":"movie.mkv","kind":"drive#file",
                   "size":"2048","file_extension":"mkv"}},
                {"type":"TYPE_PLAY","fileId":"f2",
                 "updatedTime":"2026-10-30T08:00:00Z",
                 "params":{"play_seconds":8,"play_duration":30},
                 "referenceResource":{"id":"f2","name":"incomplete.mkv"}},
                {"type":"TYPE_PLAY","file_id":"",
                 "params":{"file_id":"f3","play_seconds":"9","play_duration":"30"}},
                {"type":"TYPE_PLAY","file_id":"zero",
                 "params":{"play_seconds":"0","play_duration":"30"}},
                {"type":"TYPE_PLAY","file_id":"clamp",
                 "params":{"play_seconds":"99","play_duration":"20"}},
                {"type":"TYPE_PLAY","file_id":"bad",
                 "params":{"play_seconds":"oops","play_duration":null}},
                {"type":"TYPE_DOWNLOAD","file_id":"other",
                 "params":{"play_seconds":"2","play_duration":"3"}}
            ],
            "next_page_token": "tok"
        }"#;
        let parsed: EventListResponse = serde_json::from_str(raw).expect("播放历史应可解析");
        let entries: Vec<_> = parsed
            .events
            .into_iter()
            .filter_map(EventDto::into_history_entry)
            .collect();
        assert_eq!(entries.len(), 6, "坏进度不应丢掉整条播放历史");
        assert_eq!(entries[0].id, "event-1");
        assert_eq!(entries[0].file_id, "f1");
        assert_eq!(
            entries[0].progress,
            Some(PlaybackProgress {
                seconds: 115,
                duration: 162
            })
        );
        assert_eq!(
            entries[0].updated_at.map(|time| time.to_rfc3339()),
            Some("2026-10-31T08:00:00+00:00".to_owned())
        );
        let resource = entries[0]
            .reference_resource
            .as_ref()
            .expect("完整 reference_resource 应直接保留");
        assert_eq!(resource.id, "f1", "资源缺 id 时应使用事件文件 id");
        assert_eq!(resource.name, "movie.mkv");
        assert!(resource.video);

        assert_eq!(entries[1].file_id, "f2");
        assert_eq!(
            entries[1].progress.map(|progress| progress.seconds),
            Some(8)
        );
        assert!(
            entries[1].reference_resource.is_none(),
            "缺类型字段的资源应留给会话层补详情"
        );
        assert_eq!(entries[2].file_id, "f3", "params 里的 file_id 也要兼容");
        assert_eq!(
            entries[2].progress.map(|progress| progress.seconds),
            Some(9)
        );
        assert_eq!(entries[3].file_id, "zero");
        assert_eq!(
            entries[3].progress.map(|progress| progress.seconds),
            Some(0)
        );
        assert_eq!(entries[4].file_id, "clamp");
        assert_eq!(
            entries[4].progress.map(|progress| progress.seconds),
            Some(20),
            "位置不能超过总时长"
        );
        assert_eq!(entries[5].file_id, "bad");
        assert_eq!(entries[5].progress, None, "无总时长时页面显示短横线");
        assert_eq!(parsed.next_page_token.as_deref(), Some("tok"));
    }

    #[test]
    fn report_body_uses_decimal_strings() {
        let body = play_report_body(
            "file-1",
            PlaybackProgress {
                seconds: 115,
                duration: 162,
            },
            EVENT_TYPE_PLAY,
        );
        assert_eq!(body["event"]["type"], "TYPE_PLAY");
        assert_eq!(body["event"]["file_id"], "file-1");
        assert_eq!(body["event"]["params"]["play_seconds"], "115");
        assert_eq!(body["event"]["params"]["play_duration"], "162");
        assert!(body["event"]["params"]["play_seconds"].is_string());
        assert!(body["event"]["params"]["play_duration"].is_string());
    }

    #[test]
    fn recent_saved_reuses_event_and_file_parsing() {
        let raw = r#"{
            "events": [
                {"kind":"drive#event","type":"TYPE_UPLOAD","id":"E1",
                 "created_time":"2026-10-30T10:00:00+08:00",
                 "updated_time":"2026-10-31T09:30:00+08:00",
                 "file_id":"F1",
                 "reference_resource":{"kind":"drive#file","id":"F1",
                   "parent_id":"P1","name":"movie.mkv","size":"42",
                   "thumbnail_link":"https://thumb.mypikpak.com/F1"}},
                {"type":"TYPE_RESTORE","id":"E2","file_id":"F2",
                 "updated_time":"2026-10-31T08:00:00+08:00",
                 "reference_resource":{"kind":"drive#folder","name":"目录"}},
                {"type":"TYPE_PLAY","id":"E3","file_id":"ignored",
                 "updated_time":"2026-10-31T12:00:00+08:00"}
            ]
        }"#;
        let parsed: EventListResponse = serde_json::from_str(raw).expect("事件列表应可解析");
        let entries: Vec<_> = parsed
            .events
            .into_iter()
            .filter_map(EventDto::into_recent_saved)
            .collect();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].event_id.as_deref(), Some("E1"));
        assert_eq!(
            entries[0].occurred_at.map(|time| time.to_rfc3339()),
            Some("2026-10-31T01:30:00+00:00".to_owned())
        );
        let resource = entries[0].resource.as_ref().expect("完整资源应直接复用");
        assert_eq!(resource.name, "movie.mkv");
        assert_eq!(resource.parent_id, "P1");
        assert_eq!(resource.size, 42);
        assert_eq!(resource.thumbnail_url, "https://thumb.mypikpak.com/F1");
        let resource = entries[1]
            .resource
            .as_ref()
            .expect("资源缺 id 时应使用事件文件 id");
        assert_eq!(resource.id, "F2");
        assert_eq!(resource.kind, crate::EntryKind::Folder);
    }

    #[test]
    fn zero_and_full_progress_are_never_reported() {
        assert_eq!(
            PlaybackProgress {
                seconds: 0,
                duration: 100
            }
            .normalized(),
            None
        );
        assert_eq!(
            PlaybackProgress {
                seconds: 10,
                duration: 0
            }
            .normalized(),
            None
        );
        assert_eq!(
            PlaybackProgress {
                seconds: 100,
                duration: 100
            }
            .normalized(),
            Some(PlaybackProgress {
                seconds: 99,
                duration: 100
            })
        );
        assert_eq!(
            PlaybackProgress {
                seconds: 1,
                duration: 1
            }
            .normalized(),
            None
        );
    }
}
