//! 播放：文件详情与画质档位选择。
//!
//! 选链规则借用 pikpaktui（Apache-2.0）的行为结论，代码自己写：
//! - `medias[]` 是转码档列表，`is_origin` 为原画；隐藏（`is_visible: false`）
//!   与需要额外配额（`need_more_quota: true`）的档位一律不可用；
//! - **不得绕过限制**：medias 存在但全部不可用时，不能回退到下载直链，
//!   否则等于用下载链接绕过官方的播放权限；
//! - 只有 medias 缺失或为空（非视频文件、老响应）时，才用
//!   `web_content_link` / `links["application/octet-stream"]` 兜底为「原画」。
//!
//! 直链是带签名的临时 URL：不缓存、不打印（最多打到域名）。

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::drive::DriveClient;
use crate::types::{PlayOption, StreamKind};
use crate::Error;

/// 档位不可用原因（界面层负责翻成中文）。
pub const REASON_HIDDEN: &str = "hidden";
pub const REASON_QUOTA: &str = "quota";
pub const REASON_NOT_READY: &str = "not_ready";

/// 文件详情里与播放相关的部分。
#[derive(Debug, Clone, Default)]
pub struct FileInfo {
    pub id: String,
    pub name: String,
    /// 原始下载直链（可能为空：非视频或官方未下发）
    pub web_content_link: String,
    /// `links["application/octet-stream"].url`
    pub octet_stream: String,
    /// `medias` 原始档位
    pub medias: Vec<MediaStream>,
    /// 该文件是否没有任何可用链接（session 层据此延迟重试）
    pub all_links_empty: bool,
}

/// `medias[]` 的一项。
#[derive(Debug, Clone, Default)]
pub struct MediaStream {
    pub name: String,
    pub url: String,
    pub expires_at: Option<DateTime<Utc>>,
    pub original: bool,
    pub visible: bool,
    pub need_more_quota: bool,
    pub height: Option<u32>,
}

impl FileInfo {
    /// 选出可播放的档位（原画在前，转码按清晰度降序）。
    pub fn play_options(&self) -> Vec<PlayOption> {
        let mut options: Vec<PlayOption> = self.medias.iter().map(MediaStream::to_option).collect();
        // 排序：原画优先，其余按高度降序（未知高度排后面）
        options.sort_by_key(|option| {
            (
                !option.original,
                std::cmp::Reverse(option.height.unwrap_or(0)),
            )
        });

        if options.is_empty() {
            if let Some(url) = self.fallback_url() {
                options.push(PlayOption {
                    label: "原画".to_owned(),
                    url,
                    kind: StreamKind::Original,
                    height: None,
                    original: true,
                    available: true,
                    reason: None,
                });
            }
        }
        options
    }

    /// 兜底下载直链（仅在没有任何 medias 时使用）。
    fn fallback_url(&self) -> Option<String> {
        [&self.web_content_link, &self.octet_stream]
            .into_iter()
            .map(|url| url.trim())
            .find(|url| !url.is_empty())
            .map(str::to_owned)
    }

    /// 还有没有档位能播。
    pub fn has_playable(&self) -> bool {
        self.play_options().iter().any(PlayOption::playable)
    }
}

impl MediaStream {
    fn to_option(&self) -> PlayOption {
        let reason = if self.need_more_quota {
            Some(REASON_QUOTA)
        } else if !self.visible {
            Some(REASON_HIDDEN)
        } else if self.url.trim().is_empty() {
            Some(REASON_NOT_READY)
        } else {
            None
        };
        let (label, kind) = self.label_and_kind();
        PlayOption {
            label,
            url: self.url.clone(),
            kind,
            height: self.height,
            original: self.original,
            available: reason.is_none(),
            reason,
        }
    }

    /// 显示名 + 链接类别（转码档走 m3u8，原画走文件直链）。
    fn label_and_kind(&self) -> (String, StreamKind) {
        if self.original {
            return ("原画".to_owned(), StreamKind::Original);
        }
        let label = if self.name.trim().is_empty() {
            "转码".to_owned()
        } else {
            self.name.trim().to_owned()
        };
        (label, StreamKind::Media)
    }
}

impl DriveClient {
    /// 取一个文件的详情（直链 + medias 列表）。
    pub fn file_detail(&self, id: &str) -> Result<FileInfo, Error> {
        let id = id.trim();
        if id.is_empty() {
            return Err(Error::InvalidInput("文件 id 为空".into()));
        }
        let dto: FileDto = self.get(&self.config().file_url(id))?;
        Ok(dto.into_info())
    }
}

#[derive(Deserialize)]
struct FileDto {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    web_content_link: String,
    #[serde(default)]
    links: Option<LinksDto>,
    #[serde(default)]
    medias: Vec<MediaDto>,
}

#[derive(Deserialize)]
struct LinksDto {
    #[serde(default, rename = "application/octet-stream")]
    octet_stream: Option<LinkDto>,
}

#[derive(Deserialize)]
struct LinkDto {
    #[serde(default)]
    url: String,
    #[serde(default)]
    expire: Option<String>,
}

#[derive(Deserialize)]
struct MediaDto {
    #[serde(default)]
    media_name: String,
    #[serde(default)]
    is_origin: bool,
    #[serde(default = "default_visible")]
    is_visible: bool,
    #[serde(default)]
    need_more_quota: bool,
    #[serde(default)]
    link: Option<LinkDto>,
    #[serde(default)]
    video: Option<VideoDto>,
}

fn default_visible() -> bool {
    true
}

#[derive(Deserialize)]
struct VideoDto {
    #[serde(default)]
    height: Option<u32>,
}

impl FileDto {
    fn into_info(self) -> FileInfo {
        let octet = self
            .links
            .and_then(|links| links.octet_stream)
            .map(|link| link.url)
            .unwrap_or_default();
        let medias = self
            .medias
            .into_iter()
            .map(|media| {
                let url = media
                    .link
                    .as_ref()
                    .map(|link| link.url.trim().to_owned())
                    .unwrap_or_default();
                let expires_at = media
                    .link
                    .as_ref()
                    .and_then(|link| link.expire.as_deref())
                    .and_then(parse_time);
                MediaStream {
                    name: media.media_name.clone(),
                    url,
                    expires_at,
                    original: media.is_origin,
                    visible: media.is_visible,
                    need_more_quota: media.need_more_quota,
                    height: media
                        .video
                        .and_then(|video| video.height)
                        .filter(|h| *h > 0)
                        .or_else(|| height_from_label(&media.media_name)),
                }
            })
            .collect::<Vec<_>>();
        let all_links_empty = medias.iter().all(|media| media.url.is_empty())
            && self.web_content_link.trim().is_empty()
            && octet.trim().is_empty();
        FileInfo {
            id: self.id,
            name: self.name,
            web_content_link: self.web_content_link,
            octet_stream: octet,
            medias,
            all_links_empty,
        }
    }
}

/// RFC3339 时间；空串 / 解析失败一律当作「没有」。
fn parse_time(text: &str) -> Option<DateTime<Utc>> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|time| time.with_timezone(&Utc))
}

/// 从档位名解析清晰度：`"1080P"` → 1080，`"4K"` → 2160。
///
/// 官方一般会给 `video.height`，这里只做兜底（解析不出返回 None）。
fn height_from_label(name: &str) -> Option<u32> {
    let label = name.trim().to_ascii_lowercase();
    if label == "4k" {
        return Some(2160);
    }
    let digits: String = label.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse::<u32>().ok().filter(|h| *h > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 解码官方结构的示例响应（字段形状取自 rclone 的 api.File）。
    fn info(json: &str) -> FileInfo {
        let dto: FileDto = serde_json::from_str(json).unwrap_or_else(|err| panic!("{err}"));
        dto.into_info()
    }

    const MIXED: &str = r#"{
        "id": "f1", "name": "movie.mkv",
        "web_content_link": "https://download.invalid/slow",
        "links": {"application/octet-stream": {"url": "https://download.invalid/octet"}},
        "medias": [
            {"media_name": "720P", "is_visible": true, "link": {"url": "https://media.invalid/720"}},
            {"media_name": "1080P", "is_origin": true, "is_visible": true,
             "link": {"url": "https://media.invalid/original?expire=1", "expire": "2030-01-01T00:00:00+08:00"}},
            {"media_name": "Hidden", "is_visible": false, "link": {"url": "https://media.invalid/hidden"}},
            {"media_name": "2160P", "is_visible": true, "need_more_quota": true, "link": {"url": "https://media.invalid/2160"}},
            {"media_name": "480P", "is_visible": true, "link": {"url": ""}}
        ]
    }"#;

    #[test]
    fn original_comes_first_then_transcodes_by_height() {
        let info = info(MIXED);
        let options = info.play_options();
        assert_eq!(options.len(), 5);
        assert_eq!(options[0].label, "原画");
        assert!(options[0].original);
        assert_eq!(options[0].kind, StreamKind::Original);
        // 剩余按高度降序：2160P(不可用) → 720P → 480P(未就绪) → Hidden
        assert_eq!(
            options[1..]
                .iter()
                .map(|o| o.label.as_str())
                .collect::<Vec<_>>(),
            vec!["2160P", "720P", "480P", "Hidden"]
        );
        assert_eq!(options[1].kind, StreamKind::Media, "转码档按 m3u8 处理");
        assert_eq!(options[1].reason, Some(REASON_QUOTA));
        assert!(!options[1].available);
        assert!(options[2].playable());
        assert_eq!(options[3].reason, Some(REASON_NOT_READY));
        assert_eq!(options[4].reason, Some(REASON_HIDDEN));
    }

    #[test]
    fn restricted_media_never_falls_back_to_download_link() {
        // 原画被隐藏：官方不允许播放，也不许拿下载链接绕过
        let info = info(
            r#"{"name": "x", "web_content_link": "https://download.invalid/file",
                "medias": [{"is_origin": true, "is_visible": false, "link": {"url": "https://media.invalid/original"}}]}"#,
        );
        let options = info.play_options();
        assert_eq!(options.len(), 1);
        assert!(!options[0].available);
        assert!(
            options[0].url.starts_with("https://media.invalid"),
            "不得替换成下载直链"
        );
        assert!(!info.has_playable());
    }

    #[test]
    fn legacy_response_without_medias_uses_download_link() {
        let info =
            info(r#"{"name": "a.mp3", "web_content_link": "https://download.invalid/file"}"#);
        let options = info.play_options();
        assert_eq!(options.len(), 1);
        assert!(options[0].original && options[0].playable());
        assert_eq!(options[0].url, "https://download.invalid/file");
    }

    #[test]
    fn empty_web_content_link_falls_through_to_octet_stream() {
        let info = info(
            r#"{"name": "a.mp3", "web_content_link": "  ",
                "links": {"application/octet-stream": {"url": "https://download.invalid/fallback"}}}"#,
        );
        assert_eq!(
            info.play_options()[0].url,
            "https://download.invalid/fallback"
        );
    }

    #[test]
    fn height_parses_from_metadata_only_and_label_is_readable() {
        // 官方给的高度优先
        let with_meta = info(
            r#"{"medias": [{"media_name": "2160P", "video": {"height": 1440}, "link": {"url": "u1"}}]}"#,
        );
        assert_eq!(with_meta.play_options()[0].height, Some(1440));
        // 缺字段时从档位名兜底
        let from_label = info(
            r#"{"medias": [
                {"media_name": "1080P", "link": {"url": "u1"}},
                {"media_name": "4K", "link": {"url": "u2"}},
                {"media_name": "", "link": {"url": "u3"}}
            ]}"#,
        );
        let options = from_label.play_options();
        assert_eq!(
            options
                .iter()
                .map(|o| (o.label.as_str(), o.height))
                .collect::<Vec<_>>(),
            vec![("4K", Some(2160)), ("1080P", Some(1080)), ("转码", None)]
        );
        // 没有名字也不至于让界面空着
        let unnamed = info(r#"{"medias": [{"link": {"url": "u"}}]}"#);
        assert_eq!(unnamed.play_options()[0].label, "转码");
        assert!(unnamed.play_options()[0].playable());
    }

    #[test]
    fn all_links_empty_is_reported_for_retry() {
        let empty = info(r#"{"name": "x", "medias": [{"link": {"url": "  "}}]}"#);
        assert!(empty.all_links_empty);
        let ready = info(MIXED);
        assert!(!ready.all_links_empty);
    }

    #[test]
    fn expire_query_wins_over_link_expire() {
        use chrono::Duration;
        let past = Utc::now() - Duration::hours(1);
        let mut option = PlayOption {
            label: "原画".into(),
            url: "https://cdn.invalid/v?expire=1&x=2".into(),
            kind: StreamKind::Original,
            height: None,
            original: true,
            available: true,
            reason: None,
        };
        // URL 里的 expire=1（1970 年）已过期，即便 link.expire 在未来
        assert!(option.is_expired(Some(Utc::now() + Duration::hours(1))));
        let future = (Utc::now() + Duration::hours(1)).timestamp();
        option.url = format!("https://cdn.invalid/v?a=1&expire={future}");
        assert!(!option.is_expired(Some(past)));
        // 没有 expire 参数时退回 link.expire
        option.url = "https://cdn.invalid/v".into();
        assert!(option.is_expired(Some(past)));
        assert!(!option.is_expired(None), "时间未知时不当作过期");
    }
}
