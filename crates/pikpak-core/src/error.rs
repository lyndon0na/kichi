use crate::consts::CLIENT_ID;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("http 请求失败: {0}")]
    Http(#[from] reqwest::Error),

    #[error("json 解析失败: {0}")]
    Json(#[from] serde_json::Error),

    #[error("io 错误: {0}")]
    Io(#[from] std::io::Error),

    #[error("API 错误(error_code={error_code}): {description}")]
    Api {
        error: String,
        error_code: i64,
        description: String,
    },

    /// 直链下载等场景的原始 HTTP 状态错误(带响应体片段)。
    #[error("HTTP {status}: {body}")]
    HttpStatus { status: u16, body: String },

    /// 值得退避重试的临时状态(如传输不完整, 可凭 `.part` 续传)。
    #[error("{0}")]
    Transient(String),

    #[error("{0}")]
    Msg(String),
}

impl Error {
    pub fn msg(s: impl Into<String>) -> Self {
        Error::Msg(s.into())
    }

    /// 构造一个值得退避重试的临时错误。
    pub fn transient(s: impl Into<String>) -> Self {
        Error::Transient(s.into())
    }

    /// 是否为值得自动重试的错误: 瞬时网络(超时/断连/请求或响应体读取失败)、
    /// 直链下载的 5xx / 429 / 408, 以及显式标记的临时状态。
    /// 用于本地下载的有限次退避重试判断。
    pub fn is_transient(&self) -> bool {
        match self {
            Error::Http(e) => e.is_timeout() || e.is_connect() || e.is_request() || e.is_body(),
            Error::HttpStatus { status, .. } => {
                matches!(*status, 408 | 429) || (500..=599).contains(status)
            }
            Error::Transient(_) => true,
            _ => false,
        }
    }
}

impl From<std::convert::Infallible> for Error {
    fn from(_: std::convert::Infallible) -> Self {
        unreachable!()
    }
}

/// 服务端错误体，也用于判断 access token 是否过期。
#[derive(Debug, serde::Deserialize, Default)]
pub struct ApiErrorBody {
    #[serde(default)]
    pub error: String,
    #[serde(default)]
    pub error_code: i64,
    #[serde(default)]
    pub error_description: String,
}

/// token 失效(需要 refresh)时返回的错误码。
pub const ERROR_CODE_TOKEN_EXPIRED: i64 = 16;
/// captcha token 无效/不匹配。
pub const ERROR_CODE_CAPTCHA_INVALID: i64 = 9;
pub const ERROR_INVALID_ACCOUNT: &str = "invalid_account_or_password";

pub fn api_err_body() -> ApiErrorBody {
    ApiErrorBody::default()
}

/// 根据响应 body 构造 Api 错误。
pub fn api_error(body: &ApiErrorBody) -> Error {
    let description = if !body.error_description.is_empty() {
        body.error_description.clone()
    } else if !body.error.is_empty() {
        body.error.clone()
    } else {
        "未知错误".to_string()
    };
    Error::Api {
        error: body.error.clone(),
        error_code: body.error_code,
        description,
    }
}

/// 完整 URL(带 client_id 的回调等场景使用)。
#[allow(dead_code)]
pub fn url_with_client_id(path: &str) -> String {
    format!("{}?client_id={CLIENT_ID}", path)
}
