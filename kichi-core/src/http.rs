//! HTTP 传输层：统一请求头、超时、错误负载解析。
//!
//! 只做“发请求 / 收 JSON”的机械工作，业务语义由 `auth` / `drive` 解释。

use std::io::Read;
use std::time::Duration;

use reqwest::blocking::{Client, RequestBuilder};
use reqwest::StatusCode;
use serde::de::DeserializeOwned;
use serde::Deserialize;

use crate::{Config, Error};

/// 请求体形态（S2 只用到 JSON 与表单）。
pub(crate) enum Body<'a> {
    Empty,
    Json(&'a serde_json::Value),
    Form(&'a [(&'a str, &'a str)]),
}

impl<'a> Body<'a> {
    /// 借出等价的请求体，使其能先后用于两次请求（captcha 重试）。
    pub(crate) fn reborrow(&self) -> Body<'_> {
        match self {
            Body::Empty => Body::Empty,
            Body::Json(value) => Body::Json(value),
            Body::Form(fields) => Body::Form(fields),
        }
    }
}

/// 公开图片的主机策略。头像允许用户批准过的 Google 资料图 CDN，
/// 缩略图仍然只能走 PikPak。
#[derive(Clone, Copy)]
pub(crate) enum PublicImagePolicy {
    PikPakOnly,
    Avatar,
}

impl PublicImagePolicy {
    fn allows(self, host: Option<&str>) -> bool {
        if public_asset_host(host) {
            return true;
        }
        matches!(self, Self::Avatar)
            && normalized_host(host).as_deref() == Some("lh3.googleusercontent.com")
    }
}

/// 带 PikPak 身份头的同步 HTTP 客户端（reqwest 内部连接池共享，克隆廉价）。
#[derive(Clone)]
pub(crate) struct HttpClient {
    client: Client,
    /// 公开图片专用：不跟随重定向，确保请求始终留在允许的 CDN 主机。
    public_client: Client,
    config: Config,
    device_id: String,
}

impl HttpClient {
    pub(crate) fn new(config: Config, device_id: &str) -> Result<Self, Error> {
        let client = Client::builder()
            .user_agent(&config.user_agent)
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| Error::Network(format!("无法创建 HTTP 客户端：{e}")))?;
        let public_client = Client::builder()
            .user_agent(&config.user_agent)
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| Error::Network(format!("无法创建公开图片 HTTP 客户端：{e}")))?;
        Ok(Self {
            client,
            public_client,
            config,
            device_id: device_id.to_owned(),
        })
    }

    /// 本客户端使用的设备 id。
    pub(crate) fn device_id(&self) -> &str {
        &self.device_id
    }

    /// 下载无需鉴权的公开图片（缩略图 / 头像）。
    ///
    /// 不携带 Bearer / captcha 头，也不把 URL 放进错误文案。主机由
    /// [`PublicImagePolicy`] 收窄，避免服务端字段异常时请求任意地址。
    pub(crate) fn fetch_public_bytes(
        &self,
        url: &str,
        max_bytes: u64,
        policy: PublicImagePolicy,
    ) -> Result<Vec<u8>, Error> {
        let parsed =
            reqwest::Url::parse(url).map_err(|_| Error::InvalidInput("图片地址无效".to_owned()))?;
        if parsed.scheme() != "https" || !policy.allows(parsed.host_str()) {
            return Err(Error::InvalidInput(
                "图片地址不在允许的主机范围内".to_owned(),
            ));
        }

        let response = self
            .public_client
            .get(parsed)
            .send()
            .map_err(|e| Error::Network(network_reason(&e)))?;
        if !policy.allows(response.url().host_str()) {
            return Err(Error::Other("图片响应来自不允许的主机".to_owned()));
        }
        let status = response.status();
        if !status.is_success() {
            return Err(Error::Other(format!("图片下载失败（HTTP {status}）")));
        }
        if response
            .content_length()
            .is_some_and(|length| length > max_bytes)
        {
            return Err(Error::Other("图片超过大小上限".to_owned()));
        }

        let mut bytes = Vec::new();
        response
            .take(max_bytes + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| Error::Network(format!("读取图片失败：{e}")))?;
        if bytes.is_empty() {
            return Err(Error::Other("图片响应为空".to_owned()));
        }
        if bytes.len() as u64 > max_bytes {
            return Err(Error::Other("图片超过大小上限".to_owned()));
        }
        Ok(bytes)
    }

    /// 发出请求并解析 JSON 响应；非 2xx 时按官方错误负载归类。
    ///
    /// `access` 为 Bearer 令牌；`captcha` 为 `x-captcha-token`（不需要时传 `None`）。
    pub(crate) fn call<R: DeserializeOwned>(
        &self,
        method: reqwest::Method,
        url: &str,
        body: Body<'_>,
        access: Option<&str>,
        captcha: Option<&str>,
    ) -> Result<R, Error> {
        let mut req = self
            .client
            .request(method, url)
            .header("Referer", "https://mypikpak.com/")
            .header("x-client-id", &self.config.client_id)
            .header("x-client-version", &self.config.client_version)
            .header("x-device-id", &self.device_id);

        if let Some(token) = access {
            req = req.bearer_auth(token);
        }
        if let Some(token) = captcha {
            req = req.header("x-captcha-token", token);
        }

        req = attach_body(req, body);

        let response = req.send().map_err(|e| Error::Network(network_reason(&e)))?;
        let status = response.status();
        let text = response
            .text()
            .map_err(|e| Error::Network(network_reason(&e)))?;

        if !status.is_success() {
            return Err(api_error(status, &text));
        }

        serde_json::from_str(&text)
            .map_err(|e| Error::ApiChanged(format!("响应不是预期的 JSON：{e}")))
    }

    /// 同 [`HttpClient::call`]，额外附带查询参数（值做完整百分号转义）。
    pub(crate) fn call_with_query<R: DeserializeOwned>(
        &self,
        method: reqwest::Method,
        url: &str,
        query: &[(String, String)],
        body: Body<'_>,
        access: Option<&str>,
        captcha: Option<&str>,
    ) -> Result<R, Error> {
        self.call(method, &append_query(url, query), body, access, captcha)
    }

    /// 发出请求但**不解析响应体**（官方部分端点成功时返回空体）。
    ///
    /// 非 2xx 时仍按官方错误负载归类。
    pub(crate) fn call_no_response(
        &self,
        method: reqwest::Method,
        url: &str,
        query: &[(String, String)],
        access: Option<&str>,
        captcha: Option<&str>,
    ) -> Result<(), Error> {
        self.call_with_query_no_response(method, url, query, Body::Empty, access, captcha)
    }

    /// 同 [`HttpClient::call_no_response`]，但可带请求体。
    pub(crate) fn call_with_query_no_response(
        &self,
        method: reqwest::Method,
        url: &str,
        query: &[(String, String)],
        body: Body<'_>,
        access: Option<&str>,
        captcha: Option<&str>,
    ) -> Result<(), Error> {
        let url = append_query(url, query);
        let mut req = self
            .client
            .request(method, &url)
            .header("Referer", "https://mypikpak.com/")
            .header("x-client-id", &self.config.client_id)
            .header("x-client-version", &self.config.client_version)
            .header("x-device-id", &self.device_id);
        if let Some(token) = access {
            req = req.bearer_auth(token);
        }
        if let Some(token) = captcha {
            req = req.header("x-captcha-token", token);
        }
        req = attach_body(req, body);

        let response = req.send().map_err(|e| Error::Network(network_reason(&e)))?;
        let status = response.status();
        if status.is_success() {
            return Ok(());
        }
        let text = response
            .text()
            .map_err(|e| Error::Network(network_reason(&e)))?;
        Err(api_error(status, &text))
    }
}

fn normalized_host(host: Option<&str>) -> Option<String> {
    Some(host?.trim_end_matches('.').to_ascii_lowercase())
}

fn public_asset_host(host: Option<&str>) -> bool {
    let Some(host) = normalized_host(host) else {
        return false;
    };
    host == "mypikpak.com" || host.ends_with(".mypikpak.com")
}

/// 把查询参数拼到 URL 末尾；空查询返回原 URL。
fn append_query(url: &str, query: &[(String, String)]) -> String {
    if query.is_empty() {
        return url.to_owned();
    }
    let mut out = String::with_capacity(url.len() + 32 * query.len());
    out.push_str(url);
    out.push(if url.contains('?') { '&' } else { '?' });
    for (index, (key, value)) in query.iter().enumerate() {
        if index > 0 {
            out.push('&');
        }
        out.push_str(&percent_encode(key));
        out.push('=');
        out.push_str(&percent_encode(value));
    }
    out
}

fn attach_body(req: RequestBuilder, body: Body<'_>) -> RequestBuilder {
    match body {
        Body::Empty => req,
        Body::Json(value) => req.json(value),
        Body::Form(fields) => req
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .body(form_encode(fields)),
    }
}

/// 官方错误负载：`{error, error_code, error_description}`。
#[derive(Debug, Deserialize)]
struct ApiErrorPayload {
    #[serde(default)]
    error: String,
    #[serde(default)]
    error_code: i64,
    #[serde(default)]
    error_description: Option<String>,
}

fn api_error(status: StatusCode, body: &str) -> Error {
    if let Ok(payload) = serde_json::from_str::<ApiErrorPayload>(body) {
        if !payload.error.is_empty() {
            return Error::from_api(
                payload.error_code,
                &payload.error,
                payload.error_description.as_deref().unwrap_or(""),
            );
        }
    }
    match status.as_u16() {
        // 只有 401 是「令牌不认」。403 是「这个请求不被允许」（地区限制、
        // 风控、条目受限等），当成登录失效会把整条会话踢回登录页；官方真正
        // 的鉴权错误（invalid_grant / unauthenticated）在上面按 JSON 负载
        // 识别，不受这里影响。
        401 => Error::Auth {
            reason: format!("HTTP {status}"),
        },
        // 403 的文件体多是网关的 HTML，原样塞进界面很难看；给一句准确、
        // 不猜原因的说明（不再带 html / code）
        403 => Error::Other(format!("服务器拒绝了这个请求（HTTP {status}）")),
        429 => Error::RateLimited {
            reason: format!("HTTP {status}"),
        },
        _ => Error::Api {
            code: i64::from(status.as_u16()),
            reason: format!("http_{}", status.as_u16()),
            message: truncate(body, 200),
        },
    }
}

/// 网络错误文本（不含凭据）。
fn network_reason(err: &reqwest::Error) -> String {
    if err.is_timeout() {
        "请求超时".to_owned()
    } else if err.is_connect() {
        format!("无法连接：{err}")
    } else {
        format!("{err}")
    }
}

/// 表单编码（只需处理 token 这类无空格字符，仍做完整百分号转义以防万一）。
fn form_encode(fields: &[(&str, &str)]) -> String {
    let mut pairs = Vec::with_capacity(fields.len());
    for (key, value) in fields {
        pairs.push(format!("{}={}", percent_encode(key), percent_encode(value)));
    }
    pairs.join("&")
}

fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(char::from(byte));
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

fn truncate(text: &str, max_chars: usize) -> String {
    let mut out: String = text.chars().take(max_chars).collect();
    if text.chars().count() > max_chars {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_payload_maps_to_domain_error() {
        let body = r#"{"error":"invalid_grant","error_code":4126,"error_description":"expired"}"#;
        match api_error(StatusCode::BAD_REQUEST, body) {
            Error::Auth { reason } => assert!(reason.contains("4126")),
            other => panic!("应归为 Auth，实际 {other:?}"),
        }
    }

    #[test]
    fn non_json_error_keeps_status() {
        match api_error(StatusCode::BAD_GATEWAY, "<html>bad gateway</html>") {
            Error::Api { code, reason, .. } => {
                assert_eq!(code, 502);
                assert_eq!(reason, "http_502");
            }
            other => panic!("应归为 Api，实际 {other:?}"),
        }
    }

    #[test]
    fn only_401_means_the_session_is_gone() {
        // 服务器回 HTML / 空体时只能看状态码：401 是令牌不认
        assert!(matches!(
            api_error(StatusCode::UNAUTHORIZED, ""),
            Error::Auth { .. }
        ));
        // 403 只是「这次请求不允许」，不能把人踢回登录页
        match api_error(StatusCode::FORBIDDEN, "<html>forbidden</html>") {
            Error::Other(text) => {
                assert!(text.contains("HTTP 403"), "{text}");
                assert!(!text.contains("<html>"), "不要把网关的 HTML 塞进界面");
            }
            other => panic!("403 应归为普通失败，实际 {other:?}"),
        }
    }

    #[test]
    fn truncate_is_char_safe() {
        assert_eq!(truncate("一二三四", 2), "一二…");
        assert_eq!(truncate("ab", 2), "ab");
    }

    #[test]
    fn public_assets_stay_on_pikpak_hosts() {
        assert!(public_asset_host(Some("sg-thumbnail-drive.mypikpak.com")));
        assert!(public_asset_host(Some("mypikpak.com")));
        assert!(!public_asset_host(Some("mypikpak.com.example.org")));
        assert!(!public_asset_host(Some("evil-mypikpak.com")));
        assert!(!public_asset_host(None));
    }

    #[test]
    fn google_avatar_host_is_only_allowed_for_avatars() {
        assert!(PublicImagePolicy::Avatar.allows(Some("lh3.googleusercontent.com")));
        assert!(PublicImagePolicy::Avatar.allows(Some("lh3.googleusercontent.com.")));
        assert!(PublicImagePolicy::Avatar.allows(Some("user.mypikpak.com")));
        assert!(!PublicImagePolicy::PikPakOnly.allows(Some("lh3.googleusercontent.com")));
        assert!(!PublicImagePolicy::Avatar.allows(Some("googleusercontent.com")));
        assert!(!PublicImagePolicy::Avatar.allows(Some("lh4.googleusercontent.com")));
        assert!(!PublicImagePolicy::Avatar.allows(Some("lh3.googleusercontent.com.example")));
    }

    #[test]
    fn append_query_percent_encodes_filters() {
        let url = append_query(
            "https://api-drive.mypikpak.com/drive/v1/files",
            &[(
                "filters".to_owned(),
                r#"{"phase":{"eq":"PHASE_TYPE_COMPLETE"}}"#.to_owned(),
            )],
        );
        assert!(url.starts_with("https://api-drive.mypikpak.com/drive/v1/files?"));
        assert!(
            url.contains("filters=%7B%22phase%22%3A%7B%22eq%22%3A%22PHASE_TYPE_COMPLETE%22%7D%7D")
        );
        assert_eq!(
            append_query("https://example.test/a?x=1", &[]),
            "https://example.test/a?x=1"
        );
        assert_eq!(
            append_query("https://example.test/a?x=1", &[("y".into(), "a b".into())]),
            "https://example.test/a?x=1&y=a%20b"
        );
    }
}
