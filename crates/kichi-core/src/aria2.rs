//! aria2 JSON-RPC 客户端: 把云端文件的限时直链推送给外部下载器(aria2 / Motrix 等)。
//!
//! 只覆盖推送需要的三个方法: `aria2.getVersion`(测试连接)、`aria2.addUri`(加入下载)、
//! `aria2.getGlobalOption`(取下载目录兜底)。请求体构造与响应解析是纯函数, 便于单测;
//! 网络访问只经 [`Aria2Client`]。
//!
//! 实测结论(aria2 1.37, 2026-09-29):
//! - 逐任务的 `header` 数组会覆盖全局 User-Agent, 自定义头(X-Device-Id 等)原样透传
//!   —— 因此统一用 `header` 承载服务端要求的请求头, 不再单发 `user-agent` 选项;
//! - `dir` 指向不存在的目录时 aria2 会自行创建(含多级), 推送前无需本地建目录。

use std::time::Duration;

use serde_json::{json, Map, Value};

use crate::error::Error;

/// 单次 RPC 的超时(连接 + 响应); aria2 是本机服务, 正常在毫秒级返回。
const RPC_TIMEOUT: Duration = Duration::from_secs(10);

/// JSON-RPC 请求 id; aria2 不依赖 id 关联请求, 固定值即可。
const RPC_ID: &str = "kichi";

/// aria2 JSON-RPC 客户端(一次推送 / 一次测试连接持有一个)。
pub struct Aria2Client {
    http: reqwest::Client,
    rpc_url: String,
    secret: String,
}

impl Aria2Client {
    pub fn new(rpc_url: impl Into<String>, secret: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(RPC_TIMEOUT)
            .build()
            .expect("failed to build aria2 http client");
        Self {
            http,
            rpc_url: rpc_url.into(),
            secret: secret.into(),
        }
    }

    /// 调用一个 RPC 方法并返回 `result`。
    async fn call(&self, method: &str, params: Value) -> Result<Value, Error> {
        let body = rpc_body(method, &self.secret, params);
        let resp = self
            .http
            .post(&self.rpc_url)
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::msg(format!("无法连接 aria2: {e}")))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| Error::msg(format!("读取 aria2 响应失败: {e}")))?;
        // aria2 对 JSON-RPC 错误(如密钥不对)也返回 HTTP 400 + 标准错误体,
        // 因此先按 JSON-RPC 解析, 让「aria2 错误(1): Unauthorized」这类消息直达用户。
        if let Ok(value) = serde_json::from_str::<Value>(&text) {
            return parse_result(&value);
        }
        if !status.is_success() {
            return Err(Error::HttpStatus {
                status: status.as_u16(),
                body: body_snippet(&text),
            });
        }
        Err(Error::msg(format!(
            "aria2 响应不是有效 JSON: {}",
            body_snippet(&text)
        )))
    }

    /// `aria2.getVersion`; 返回版本号, 用于设置页的「测试连接」。
    pub async fn version(&self) -> Result<String, Error> {
        let result = self.call("aria2.getVersion", json!([])).await?;
        Ok(result
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("未知")
            .to_string())
    }

    /// `aria2.getGlobalOption`; 返回全局下载目录(可能为空串)。
    pub async fn global_dir(&self) -> Result<String, Error> {
        let result = self.call("aria2.getGlobalOption", json!([])).await?;
        Ok(result
            .get("dir")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string())
    }

    /// `aria2.addUri`; 返回任务 gid(推送成功即表示已入队)。
    pub async fn add_uri(&self, url: &str, opts: &AddUriOptions) -> Result<String, Error> {
        let result = self
            .call("aria2.addUri", json!([[url], opts.to_value()]))
            .await?;
        Ok(result.as_str().unwrap_or_default().to_string())
    }
}

/// `aria2.addUri` 的逐任务选项(空字段不下发, 交由 aria2 用全局设置)。
#[derive(Debug, Default, Clone, PartialEq)]
pub struct AddUriOptions {
    /// 保存文件名(aria2 的 `out`)。
    pub out: String,
    /// 保存目录(绝对路径); 空 = 不传, 用 aria2 自己的全局 `dir`。
    pub dir: String,
    /// 附加请求头("Name: value"); 覆盖 aria2 全局 User-Agent 等。
    pub headers: Vec<(String, String)>,
}

impl AddUriOptions {
    /// 转换成 aria2 的 options 对象。
    pub fn to_value(&self) -> Value {
        let mut o = Map::new();
        if !self.out.is_empty() {
            o.insert("out".to_string(), json!(self.out));
        }
        if !self.dir.is_empty() {
            o.insert("dir".to_string(), json!(self.dir));
        }
        if !self.headers.is_empty() {
            let headers: Vec<String> = self
                .headers
                .iter()
                .map(|(k, v)| format!("{k}: {v}"))
                .collect();
            o.insert("header".to_string(), json!(headers));
        }
        Value::Object(o)
    }
}

/// 构造 JSON-RPC 2.0 请求体。密钥非空时按 aria2 约定作为 `token:<secret>` 前置参数。
pub fn rpc_body(method: &str, secret: &str, params: Value) -> Value {
    let mut arr: Vec<Value> = Vec::new();
    if !secret.is_empty() {
        arr.push(Value::String(format!("token:{secret}")));
    }
    if let Value::Array(items) = params {
        arr.extend(items);
    }
    json!({
        "jsonrpc": "2.0",
        "id": RPC_ID,
        "method": method,
        "params": arr,
    })
}

/// 解析 JSON-RPC 响应: 有 `error` 对象则转为错误(错误码 + 消息), 否则取 `result`。
pub fn parse_result(body: &Value) -> Result<Value, Error> {
    if let Some(err) = body.get("error") {
        let code = err.get("code").and_then(|v| v.as_i64()).unwrap_or(0);
        let message = err
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("未知错误");
        return Err(Error::msg(format!("aria2 错误({code}): {message}")));
    }
    body.get("result")
        .cloned()
        .ok_or_else(|| Error::msg("aria2 响应缺少 result 字段"))
}

/// 日志 / 错误文案里夹带的响应体片段(截断, 避免把整页 HTML 灌进提示)。
fn body_snippet(body: &str) -> String {
    const MAX: usize = 120;
    let mut s: String = body.chars().take(MAX).collect();
    if body.chars().count() > MAX {
        s.push('…');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rpc_body_prepends_token_only_when_secret_set() {
        let with = rpc_body("aria2.getVersion", "sec", json!([]));
        assert_eq!(with["method"], "aria2.getVersion");
        assert_eq!(with["params"], json!(["token:sec"]));

        let without = rpc_body("aria2.getVersion", "", json!([]));
        assert_eq!(without["params"], json!([]));
    }

    #[test]
    fn rpc_body_keeps_regular_params_after_token() {
        let body = rpc_body(
            "aria2.addUri",
            "sec",
            json!([["https://example.com/f"], {"out": "f"}]),
        );
        assert_eq!(
            body["params"],
            json!(["token:sec", ["https://example.com/f"], {"out": "f"}])
        );
    }

    #[test]
    fn add_uri_options_skip_empty_fields() {
        let empty = AddUriOptions::default().to_value();
        assert_eq!(empty, json!({}));

        // 只有 out 时不带 dir / header。
        let out_only = AddUriOptions {
            out: "a.mkv".into(),
            ..Default::default()
        }
        .to_value();
        assert_eq!(out_only, json!({"out": "a.mkv"}));
    }

    #[test]
    fn add_uri_options_encode_dir_and_headers() {
        let opts = AddUriOptions {
            out: "a.mkv".into(),
            dir: "/dl/剧集/S1".into(),
            headers: vec![
                ("User-Agent".into(), "UA".into()),
                ("X-Device-Id".into(), "dev".into()),
            ],
        };
        assert_eq!(
            opts.to_value(),
            json!({
                "out": "a.mkv",
                "dir": "/dl/剧集/S1",
                "header": ["User-Agent: UA", "X-Device-Id: dev"],
            })
        );
    }

    #[test]
    fn parse_result_maps_error_object() {
        let err = parse_result(&json!({
            "id": "kichi",
            "error": {"code": 1, "message": "Unauthorized"}
        }))
        .unwrap_err();
        let text = err.to_string();
        assert!(
            text.contains("Unauthorized"),
            "错误应带上 aria2 消息: {text}"
        );
        assert!(text.contains('1'), "错误应带上错误码: {text}");
    }

    #[test]
    fn parse_result_returns_result_and_rejects_missing() {
        assert_eq!(
            parse_result(&json!({"result": "gid-1"})).unwrap(),
            json!("gid-1")
        );
        assert!(parse_result(&json!({"id": "kichi"})).is_err());
    }

    #[test]
    fn snippet_truncates_long_body() {
        let long = "x".repeat(500);
        let s = body_snippet(&long);
        assert!(s.chars().count() <= 121);
        assert!(s.ends_with('…'));
        assert_eq!(body_snippet("short"), "short");
    }

    /// 服务不可达(连接被拒)时给出可读错误, 而不是 panic 或悬挂。
    #[tokio::test]
    async fn unreachable_server_is_error() {
        let client = Aria2Client::new("http://127.0.0.1:1/jsonrpc", "sec");
        let err = client.version().await.unwrap_err().to_string();
        assert!(err.contains("无法连接 aria2"), "实际错误: {err}");
    }
}
