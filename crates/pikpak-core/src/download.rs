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
pub(crate) fn part_path(dest: &std::path::Path) -> std::path::PathBuf {
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
}
