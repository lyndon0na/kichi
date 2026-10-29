//! 分享链接：创建 / 列表 / 撤销。
//!
//! 请求形态对照 rclone（MIT）与 pikpaktui 0.0.58（Apache-2.0）：
//! - 一个分享可以包含多个文件 / 文件夹；
//! - `expiration_days = -1` 表示永久，正数表示有效天数；
//! - 需要密码时由服务端生成密码，客户端不能自定义；
//! - 列表和批量撤销分别走 `/share/list` 与 `/share:batchDelete`。
//!
//! 列表的 `file_kind` 由 pikpak-kotlin（MIT）的真实响应记录补充。

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer};

use crate::drive::DriveClient;
use crate::types::EntryKind;
use crate::Error;

/// 分享列表单页请求条数。
pub const SHARE_LIST_LIMIT: u32 = 100;

const MAX_BATCH: usize = 100;
const ACTION_DELETE: &str = "batchDelete";

/// 创建分享时的选项。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CreateShareOptions {
    /// `-1` = 永久；正数 = 有效天数。
    pub expiration_days: i64,
    /// 是否让服务端生成访问密码。
    pub need_password: bool,
}

/// 刚创建的分享。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedShare {
    pub share_id: String,
    pub share_url: String,
    pub pass_code: String,
    pub share_text: String,
}

/// 分享列表中的一条记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MyShare {
    pub share_id: String,
    pub share_url: String,
    pub title: String,
    pub pass_code: String,
    pub share_to: String,
    pub created_at: Option<DateTime<Utc>>,
    pub expiration_days: i64,
    pub view_count: u64,
    pub restore_count: u64,
    pub file_count: u64,
    pub status: String,
    /// 分享首项是文件还是文件夹；旧响应没有该字段时为 `None`
    pub kind: Option<EntryKind>,
}

impl MyShare {
    pub fn has_password(&self) -> bool {
        !self.pass_code.is_empty() || self.share_to.to_ascii_lowercase().contains("encrypted")
    }
}

impl DriveClient {
    /// 创建一个分享链接。条目 id 去空、去重，最多 100 个。
    pub fn create_share(
        &self,
        file_ids: &[&str],
        options: CreateShareOptions,
    ) -> Result<CreatedShare, Error> {
        validate_expiration_days(options.expiration_days)?;
        let body = create_share_body(file_ids, options)?;
        let response: CreateShareResponse = self.request(
            reqwest::Method::POST,
            &self.config().share_url(),
            &[],
            crate::http::Body::Json(&body),
        )?;
        response.into_share()
    }

    /// 读取一页分享列表（`page_token = None` 表示第一页）。
    pub fn list_shares(
        &self,
        page_token: Option<&str>,
    ) -> Result<(Vec<MyShare>, Option<String>), Error> {
        let query = share_list_query(page_token);
        let response: ShareListResponse =
            self.get_with_query(&self.config().share_list_url(), &query)?;
        let shares = response
            .data
            .into_iter()
            .map(ShareDto::into_share)
            .collect();
        let next = response
            .next_page_token
            .map(|token| token.trim().to_owned())
            .filter(|token| !token.is_empty());
        Ok((shares, next))
    }

    /// 批量撤销分享记录。底层接口不会删除网盘里的源文件。
    pub fn delete_shares(&self, share_ids: &[&str]) -> Result<(), Error> {
        let ids = normalize_ids(share_ids, "分享")?;
        let body = serde_json::json!({ "ids": ids });
        self.request_no_response_body(
            reqwest::Method::POST,
            &self.config().share_action_url(ACTION_DELETE),
            &[],
            &body,
        )
    }
}

#[derive(Deserialize)]
struct CreateShareResponse {
    #[serde(default, deserialize_with = "lenient_string")]
    share_id: String,
    #[serde(default, deserialize_with = "lenient_string")]
    share_url: String,
    #[serde(default, deserialize_with = "lenient_string")]
    pass_code: String,
    #[serde(default, deserialize_with = "lenient_string")]
    share_text: String,
}

impl CreateShareResponse {
    fn into_share(self) -> Result<CreatedShare, Error> {
        if self.share_id.trim().is_empty() {
            return Err(Error::ApiChanged("创建分享响应里没有 share_id".into()));
        }
        if self.share_url.trim().is_empty() {
            return Err(Error::ApiChanged("创建分享响应里没有 share_url".into()));
        }
        Ok(CreatedShare {
            share_id: self.share_id,
            share_url: self.share_url,
            pass_code: self.pass_code,
            share_text: self.share_text,
        })
    }
}

#[derive(Deserialize)]
struct ShareListResponse {
    #[serde(default)]
    data: Vec<ShareDto>,
    #[serde(default)]
    next_page_token: Option<String>,
}

#[derive(Deserialize)]
struct ShareDto {
    #[serde(default, deserialize_with = "lenient_string")]
    share_id: String,
    #[serde(default, deserialize_with = "lenient_string")]
    share_url: String,
    #[serde(default, deserialize_with = "lenient_string")]
    title: String,
    #[serde(default, deserialize_with = "lenient_string")]
    pass_code: String,
    #[serde(default, deserialize_with = "lenient_string")]
    share_to: String,
    #[serde(default, deserialize_with = "lenient_time")]
    create_time: Option<DateTime<Utc>>,
    #[serde(default, deserialize_with = "lenient_i64")]
    expiration_days: i64,
    #[serde(default, deserialize_with = "lenient_u64")]
    view_count: u64,
    #[serde(default, deserialize_with = "lenient_u64")]
    restore_count: u64,
    #[serde(default, deserialize_with = "lenient_u64")]
    file_num: u64,
    #[serde(default, deserialize_with = "lenient_string")]
    file_kind: String,
    #[serde(default, deserialize_with = "lenient_string")]
    share_status: String,
}

impl ShareDto {
    fn into_share(self) -> MyShare {
        MyShare {
            share_id: self.share_id,
            share_url: self.share_url,
            title: self.title,
            pass_code: self.pass_code,
            share_to: self.share_to,
            created_at: self.create_time,
            expiration_days: self.expiration_days,
            view_count: self.view_count,
            restore_count: self.restore_count,
            file_count: self.file_num,
            status: self.share_status,
            kind: EntryKind::from_api_kind(&self.file_kind),
        }
    }
}

fn create_share_body(
    file_ids: &[&str],
    options: CreateShareOptions,
) -> Result<serde_json::Value, Error> {
    validate_expiration_days(options.expiration_days)?;
    let ids = normalize_ids(file_ids, "文件")?;
    Ok(serde_json::json!({
        "file_ids": ids,
        "share_to": if options.need_password { "encryptedlink" } else { "publiclink" },
        "expiration_days": options.expiration_days,
        "pass_code_option": if options.need_password { "REQUIRED" } else { "NOT_REQUIRED" },
    }))
}

fn share_list_query(page_token: Option<&str>) -> Vec<(String, String)> {
    let mut query = vec![
        ("limit".to_owned(), SHARE_LIST_LIMIT.to_string()),
        ("thumbnail_size".to_owned(), "SIZE_SMALL".to_owned()),
    ];
    if let Some(token) = page_token.map(str::trim).filter(|token| !token.is_empty()) {
        query.push(("page_token".to_owned(), token.to_owned()));
    }
    query
}

fn validate_expiration_days(days: i64) -> Result<(), Error> {
    if days == -1 || days > 0 {
        return Ok(());
    }
    Err(Error::InvalidInput(
        "分享有效期必须是正数天数；永久请传 -1".into(),
    ))
}

fn normalize_ids(ids: &[&str], subject: &str) -> Result<Vec<String>, Error> {
    let set: BTreeSet<String> = ids
        .iter()
        .map(|id| id.trim())
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        .collect();
    if set.is_empty() {
        return Err(Error::InvalidInput(format!("没有要操作的{subject}")));
    }
    if set.len() > MAX_BATCH {
        return Err(Error::InvalidInput(format!(
            "一次最多操作 {MAX_BATCH} 个{subject}（当前 {} 个）",
            set.len()
        )));
    }
    Ok(set.into_iter().collect())
}

fn lenient_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(match value {
        Some(serde_json::Value::String(text)) => text,
        Some(serde_json::Value::Number(number)) => number.to_string(),
        Some(serde_json::Value::Bool(value)) => value.to_string(),
        _ => String::new(),
    })
}

fn lenient_i64<'de, D>(deserializer: D) -> Result<i64, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(match value {
        Some(serde_json::Value::Number(number)) => number.as_i64().unwrap_or_default(),
        Some(serde_json::Value::String(text)) => text.trim().parse().unwrap_or_default(),
        _ => 0,
    })
}

fn lenient_u64<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(match value {
        Some(serde_json::Value::Number(number)) => number.as_u64().unwrap_or_default(),
        Some(serde_json::Value::String(text)) => text.trim().parse().unwrap_or_default(),
        _ => 0,
    })
}

fn lenient_time<'de, D>(deserializer: D) -> Result<Option<DateTime<Utc>>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<String>::deserialize(deserializer)?;
    let Some(text) = value else {
        return Ok(None);
    };
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    Ok(DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|time| time.with_timezone(&Utc)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_body_uses_multi_file_and_generated_password_flags() {
        let body = create_share_body(
            &[" b ", "a", "b"],
            CreateShareOptions {
                expiration_days: 7,
                need_password: true,
            },
        )
        .expect("请求体应合法");
        assert_eq!(body["file_ids"], serde_json::json!(["a", "b"]));
        assert_eq!(body["share_to"], "encryptedlink");
        assert_eq!(body["expiration_days"], 7);
        assert_eq!(body["pass_code_option"], "REQUIRED");

        let public = create_share_body(
            &["a"],
            CreateShareOptions {
                expiration_days: -1,
                need_password: false,
            },
        )
        .expect("永久公开分享应合法");
        assert_eq!(public["share_to"], "publiclink");
        assert_eq!(public["expiration_days"], -1);
        assert_eq!(public["pass_code_option"], "NOT_REQUIRED");
    }

    #[test]
    fn expiration_and_ids_are_validated() {
        assert!(validate_expiration_days(-1).is_ok());
        assert!(validate_expiration_days(1).is_ok());
        assert!(validate_expiration_days(0).is_err());
        assert!(validate_expiration_days(-2).is_err());
        assert!(create_share_body(
            &[],
            CreateShareOptions {
                expiration_days: 7,
                need_password: false,
            }
        )
        .is_err());
    }

    #[test]
    fn list_query_matches_known_parameters() {
        let query = share_list_query(None);
        assert!(query.contains(&("limit".to_owned(), "100".to_owned())));
        assert!(query.contains(&("thumbnail_size".to_owned(), "SIZE_SMALL".to_owned())));
        assert!(!query.iter().any(|(key, _)| key == "page_token"));

        let paged = share_list_query(Some(" tok "));
        assert!(paged.contains(&("page_token".to_owned(), "tok".to_owned())));
    }

    #[test]
    fn list_parses_string_counts_and_optional_time() {
        let json = r#"{
            "data": [{
                "share_id": "S1",
                "share_url": "https://mypikpak.com/s/S1",
                "title": "电影合集",
                "pass_code": "a1b2",
                "share_to": "encryptedlink",
                "create_time": "2026-10-29T10:20:30+08:00",
                "expiration_days": "7",
                "view_count": "12",
                "restore_count": 3,
                "file_num": "5",
                "file_kind": "drive#folder",
                "share_status": "OK"
            }, {
                "share_id": "S2",
                "share_url": "https://mypikpak.com/s/S2",
                "create_time": "",
                "expiration_days": -1,
                "view_count": null,
                "file_kind": "drive#file"
            }, {
                "share_id": "S3",
                "share_url": "https://mypikpak.com/s/S3",
                "expiration_days": 30
            }],
            "next_page_token": "next"
        }"#;
        let parsed: ShareListResponse = serde_json::from_str(json).expect("分享列表应可解析");
        assert_eq!(parsed.next_page_token.as_deref(), Some("next"));
        let shares: Vec<MyShare> = parsed.data.into_iter().map(ShareDto::into_share).collect();
        assert_eq!(shares[0].expiration_days, 7);
        assert_eq!(shares[0].view_count, 12);
        assert_eq!(shares[0].restore_count, 3);
        assert_eq!(shares[0].file_count, 5);
        assert_eq!(shares[0].kind, Some(EntryKind::Folder));
        assert!(shares[0].has_password());
        assert_eq!(
            shares[0].created_at.map(|time| time.timestamp()),
            Some(1_793_240_430)
        );
        assert_eq!(shares[1].expiration_days, -1);
        assert_eq!(shares[1].created_at, None);
        assert_eq!(shares[1].kind, Some(EntryKind::File));
        assert!(!shares[1].has_password());
        assert_eq!(shares[2].kind, None, "缺 file_kind 时应交给后缀兜底");
    }

    #[test]
    fn create_response_requires_link_fields() {
        let missing: CreateShareResponse =
            serde_json::from_str(r#"{"share_id":"S1"}"#).expect("可先解析");
        assert!(missing.into_share().is_err());

        let parsed: CreateShareResponse = serde_json::from_str(
            r#"{"share_id":"S1","share_url":"https://mypikpak.com/s/S1","pass_code":null}"#,
        )
        .expect("null 可选字段应兼容");
        let share = parsed.into_share().expect("应可用");
        assert_eq!(share.pass_code, "");
    }
}
