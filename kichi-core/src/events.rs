//! 账号事件：读取官方播放历史，并上报播放进度。
//!
//! 请求形状对照 PikPak 官方 Web 客户端与 pikpak-kotlin（MIT）的
//! `EventEndpoint.kt` / 响应记录：
//! - 播放历史是 `TYPE_PLAY` 事件，按最近播放排序，每个文件一条；
//! - `play_seconds` / `play_duration` 是十进制字符串；
//! - 上报成功返回 `{}`。

use serde::{Deserialize, Deserializer};
use serde_json::{json, Value};

use crate::drive::DriveClient;
use crate::http::Body;
use crate::Error;

/// 播放历史单页最多条数（服务端当前上限）。
pub const PLAY_HISTORY_LIMIT: u32 = 100;

const PLAY_FILTERS: &str = r#"{"type":{"in":"TYPE_PLAY"}}"#;
const EVENT_TYPE_PLAY: &str = "TYPE_PLAY";

/// 官方播放进度（整数秒）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaybackProgress {
    pub seconds: u64,
    pub duration: u64,
}

impl PlaybackProgress {
    /// 规范化成非零且不超过总时长的进度。官方历史里偶尔会有空值或异常值。
    pub fn normalized(self) -> Option<Self> {
        if self.duration == 0 {
            return None;
        }
        let seconds = self.seconds.min(self.duration);
        (seconds > 0).then_some(Self {
            seconds,
            duration: self.duration,
        })
    }
}

/// 一条官方播放历史。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaybackHistoryEntry {
    pub file_id: String,
    pub progress: PlaybackProgress,
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
    #[serde(default, alias = "fileId", deserialize_with = "lenient_string")]
    file_id: String,
    #[serde(default, rename = "type")]
    event_type: String,
    #[serde(default)]
    params: Option<PlayParamsDto>,
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
        let params = self.params?;
        let file_id = [self.file_id, params.file_id]
            .into_iter()
            .map(|id| id.trim().to_owned())
            .find(|id| !id.is_empty())?;
        let progress = PlaybackProgress {
            seconds: params.play_seconds,
            duration: params.play_duration,
        }
        .normalized()?;
        Some(PlaybackHistoryEntry { file_id, progress })
    }
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
    fn history_parses_strings_numbers_and_skips_bad_rows() {
        let raw = r#"{
            "events": [
                {"type":"TYPE_PLAY","file_id":"f1",
                 "params":{"play_seconds":"115","play_duration":"162"}},
                {"type":"TYPE_PLAY","fileId":"f2",
                 "params":{"play_seconds":8,"play_duration":30}},
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
        assert_eq!(entries.len(), 4);
        assert_eq!(entries[0].file_id, "f1");
        assert_eq!(
            entries[0].progress,
            PlaybackProgress {
                seconds: 115,
                duration: 162
            }
        );
        assert_eq!(entries[1].file_id, "f2");
        assert_eq!(entries[1].progress.seconds, 8);
        assert_eq!(entries[2].file_id, "f3", "params 里的 file_id 也要兼容");
        assert_eq!(entries[3].file_id, "clamp");
        assert_eq!(entries[3].progress.seconds, 20, "位置不能超过总时长");
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
    fn zero_progress_is_not_reported() {
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
    }
}
