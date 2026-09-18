//! 本地下载相关模型与直链解析。
//!
//! 机制(参考社区逆向 pikpakcli 等):
//! 1. `GET /drive/v1/files/{id}` 需先对该 action 做 captcha init;
//! 2. 响应里的 `web_content_link` 是带签名的限时直链, 裸 GET(可带 Range)即可流式下载;
//! 3. 下载先写 `<文件>.part`, 完成后 rename 为正式文件; `.part` 存在时用
//!    `Range bytes={已下载}-` 续传。

use serde_json::Value;

use crate::error::Error;

/// 文件详情解析出的下载信息。
#[derive(Debug, Clone)]
pub struct DownloadLink {
    pub file_id: String,
    /// 直链(可能限时, 失效后需重新解析)。
    pub url: String,
    /// 服务端给的文件名。
    pub name: String,
    /// 文件大小(可能未知为 0)。
    pub size: i64,
}

impl DownloadLink {
    /// 从 `GET /drive/v1/files/{id}` 的详情 JSON 中防御式提取下载信息。
    ///
    /// 返回 None 表示该响应不是可下载的文件详情(例如没有可用直链)。
    pub fn from_detail(file_id: &str, detail: &Value) -> Result<DownloadLink, Error> {
        let url = pick_download_url(detail)
            .ok_or_else(|| Error::msg("文件详情中未找到可用的下载直链"))?;
        let name = detail
            .get("name")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("download")
            .to_string();
        let size = detail.get("size").and_then(value_i64).unwrap_or(0);
        Ok(DownloadLink {
            file_id: file_id.to_string(),
            url,
            name,
            size,
        })
    }
}

/// 从文件详情解析出的一个可用清晰度 (含原画)。
#[derive(Debug, Clone, PartialEq)]
pub struct MediaVariant {
    /// 展示用标签, 如 "原画" / "1080P" / "720P h264"。
    pub label: String,
    /// 该清晰度的限时直链。
    pub url: String,
    /// 是否为原始文件(非转码)。
    pub is_origin: bool,
    /// 视频高度(像素); 未知为 0。
    pub height: i64,
}

impl MediaVariant {
    /// 从 `GET /drive/v1/files/{id}` 的详情 JSON 解析全部可播放清晰度。
    ///
    /// 解析顺序:
    /// 1. `medias[]` 各转码流: `is_origin=true` 视为原画, 否则按 `video.height` 命名;
    /// 2. 若 `medias` 中没有原画, 用顶层 `web_content_link` 补一条原画。
    ///
    /// 返回结果已排序(原画在前, 其余按分辨率从高到低)并去除重复标签。
    /// 无任何可用直链时返回空列表。
    pub fn from_detail(detail: &Value) -> Vec<MediaVariant> {
        let mut out: Vec<MediaVariant> = Vec::new();
        let mut origin_from_media = false;

        if let Some(medias) = detail.get("medias").and_then(|v| v.as_array()) {
            for m in medias {
                // 显式不可见/需额外配额才能播放的流跳过。
                if m.get("is_visible").and_then(|v| v.as_bool()) == Some(false) {
                    continue;
                }
                let Some(url) = pick_media_url(m) else {
                    continue;
                };
                let is_origin = m
                    .get("is_origin")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let height = m
                    .get("video")
                    .and_then(|v| v.get("height"))
                    .and_then(value_i64)
                    .unwrap_or(0);
                if is_origin {
                    origin_from_media = true;
                }
                out.push(MediaVariant {
                    label: media_label(m, is_origin, height),
                    url,
                    is_origin,
                    height,
                });
            }
        }

        if !origin_from_media {
            if let Some(url) = detail
                .get("web_content_link")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
            {
                out.push(MediaVariant {
                    label: "原画".to_string(),
                    url: url.to_string(),
                    is_origin: true,
                    height: 0,
                });
            }
        }

        // 原画优先, 其余分辨率从高到低; 同标签只保留第一条。
        out.sort_by(|a, b| {
            b.is_origin
                .cmp(&a.is_origin)
                .then(b.height.cmp(&a.height))
                .then(a.label.cmp(&b.label))
        });
        let mut seen = std::collections::HashSet::new();
        out.retain(|v| seen.insert(v.label.clone()));
        out
    }
}

/// 从单条 media 项里挑一个可用直链。
fn pick_media_url(m: &Value) -> Option<String> {
    if let Some(u) = m
        .get("url")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        return Some(u.to_string());
    }
    match m.get("link") {
        Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
        Some(link) => link
            .get("url")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string()),
        None => None,
    }
}

/// 生成一条转码流的展示标签。
fn media_label(m: &Value, is_origin: bool, height: i64) -> String {
    if is_origin {
        return "原画".to_string();
    }
    let codec = m
        .get("video")
        .and_then(|v| v.get("video_codec"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());
    match (height, codec) {
        (h, Some(c)) if h > 0 => format!("{h}P {c}"),
        (h, None) if h > 0 => format!("{h}P"),
        _ => m
            .get("media_name")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("转码")
            .to_string(),
    }
}

/// 从文件详情 JSON 中挑一个可用直链。
///
/// 依次尝试:
/// 1. `web_content_link`(官方签名直链, 社区 CLI 广泛使用);
/// 2. `medias[].link.url`(流媒体直链, 支持 Range);
/// 3. `links.*.url`(其他直链)。
fn pick_download_url(detail: &Value) -> Option<String> {
    if let Some(u) = detail
        .get("web_content_link")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        return Some(u.to_string());
    }
    if let Some(medias) = detail.get("medias").and_then(|v| v.as_array()) {
        for m in medias {
            if let Some(u) = m
                .get("link")
                .and_then(|l| l.get("url"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
            {
                return Some(u.to_string());
            }
        }
    }
    if let Some(links) = detail.get("links").and_then(|v| v.as_object()) {
        for item in links.values() {
            if let Some(u) = item
                .get("url")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
            {
                return Some(u.to_string());
            }
        }
    }
    None
}

/// 兼容 number / string 的整数取值。
pub(crate) fn value_i64(v: &Value) -> Option<i64> {
    v.as_i64()
        .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
}

/// 给定正式目标路径, 返回其伴随的 `.part` 临时路径。
pub fn part_path(dest: &std::path::Path) -> std::path::PathBuf {
    let mut s = dest.as_os_str().to_owned();
    s.push(".part");
    std::path::PathBuf::from(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn detail() -> Value {
        json!({
            "kind": "drive#file",
            "name": "movie.mkv",
            "size": "1048576000",
            "web_content_link": "https://f.mypikpak.com/download/abc?sig=1",
            "medias": [ { "link": { "url": "https://api-drive.mypikpak.com/media/1" } } ],
            "links": { "application/octet-stream": { "url": "https://f.mypikpak.com/octet?sig=2" } }
        })
    }

    #[test]
    fn parses_prefers_web_content_link() {
        let dl = DownloadLink::from_detail("f1", &detail()).unwrap();
        assert_eq!(dl.name, "movie.mkv");
        assert_eq!(dl.size, 1048576000);
        assert_eq!(dl.file_id, "f1");
        assert!(dl.url.contains("/download/abc"));
    }

    #[test]
    fn falls_back_to_medias_then_links() {
        let mut d = detail();
        d["web_content_link"] = json!("");
        let dl = DownloadLink::from_detail("f1", &d).unwrap();
        assert_eq!(dl.url, "https://api-drive.mypikpak.com/media/1");

        d["medias"] = json!([]);
        let dl = DownloadLink::from_detail("f1", &d).unwrap();
        assert_eq!(dl.url, "https://f.mypikpak.com/octet?sig=2");
    }

    #[test]
    fn missing_url_is_error() {
        let d = json!({ "name": "x", "size": 1 });
        assert!(DownloadLink::from_detail("f1", &d).is_err());
    }

    #[test]
    fn numeric_and_string_sizes() {
        let dl = DownloadLink::from_detail(
            "f",
            &json!({
                "name": "n", "size": 12345,
                "web_content_link": "u"
            }),
        )
        .unwrap();
        assert_eq!(dl.size, 12345);

        let dl = DownloadLink::from_detail(
            "f",
            &json!({
                "name": "n", "size": "9",
                "web_content_link": "u"
            }),
        )
        .unwrap();
        assert_eq!(dl.size, 9);
    }

    #[test]
    fn part_path_appends_suffix() {
        assert_eq!(
            part_path(std::path::Path::new("/a/b/x.mkv")),
            std::path::Path::new("/a/b/x.mkv.part")
        );
        assert_eq!(
            part_path(std::path::Path::new("/a/无扩展")),
            std::path::Path::new("/a/无扩展.part")
        );
    }

    #[test]
    fn variants_origin_first_then_by_height() {
        let d = json!({
            "name": "movie.mkv",
            "web_content_link": "https://f.mypikpak.com/orig",
            "medias": [
                { "link": { "url": "https://cdn/720" }, "video": { "height": 720, "video_codec": "h264" } },
                { "link": { "url": "https://cdn/orig" }, "is_origin": true },
                { "link": { "url": "https://cdn/1080" }, "video": { "height": 1080 } }
            ]
        });
        let v = MediaVariant::from_detail(&d);
        assert_eq!(v.len(), 3);
        assert!(v[0].is_origin);
        assert_eq!(v[0].label, "原画");
        assert_eq!(v[0].url, "https://cdn/orig");
        assert_eq!(v[1].label, "1080P");
        assert_eq!(v[2].label, "720P h264");
    }

    #[test]
    fn variants_fall_back_to_web_content_link() {
        let d = json!({
            "name": "clip.mp4",
            "web_content_link": "https://f.mypikpak.com/orig"
        });
        let v = MediaVariant::from_detail(&d);
        assert_eq!(v.len(), 1);
        assert!(v[0].is_origin);
        assert_eq!(v[0].url, "https://f.mypikpak.com/orig");
    }

    #[test]
    fn variants_skip_invisible_and_dedup_labels() {
        let d = json!({
            "name": "x.mkv",
            "medias": [
                { "link": { "url": "https://cdn/a" }, "is_visible": false, "video": { "height": 1080 } },
                { "link": { "url": "https://cdn/b" }, "video": { "height": 720 } },
                { "link": { "url": "https://cdn/c" }, "video": { "height": 720 } }
            ]
        });
        let v = MediaVariant::from_detail(&d);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].label, "720P");
        assert_eq!(v[0].url, "https://cdn/b");
    }

    #[test]
    fn variants_empty_without_links() {
        assert!(MediaVariant::from_detail(&json!({ "name": "x" })).is_empty());
    }
}
