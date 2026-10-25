//! 人机验证（captcha）令牌：申请、签名、缓存。
//!
//! 算法与元数据字段照 rclone `backend/pikpak/helper.go`（MIT）移植：
//! 每个业务请求都要带 `x-captcha-token`，登录用的 action 是
//! `POST:/v1/auth/signin`，其 meta 只带用户名；其他 action 带设备签名。

use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Duration, Utc};
use md5::{Digest, Md5};
use rand::Rng;
use serde::{Deserialize, Serialize};

use crate::http::{Body, HttpClient};
use crate::{Config, Error};

/// 登录 action。
pub(crate) const ACTION_SIGNIN: &str = "POST:/v1/auth/signin";

/// 设备 id 模板（小写 x 为随机十六进制，y 为 (rand & 3) | 8）。
const DEVICE_ID_TEMPLATE: &str = "xxxxxxxxxxxx4xxxyxxxxxxxxxxxxxxx";

/// captcha 签名盐链（顺序不可变）。
const MD5_SALTS: [&str; 15] = [
    "C9qPpZLN8ucRTaTiUMWYS9cQvWOE",
    "+r6CQVxjzJV6LCV",
    "F",
    "pFJRC",
    "9WXYIDGrwTCz2OiVlgZa90qpECPD6olt",
    "/750aCr4lm/Sly/c",
    "RB+DT/gZCrbV",
    "",
    "CyLsf7hdkIRxRm215hl",
    "7xHvLi2tOYP0Y92b",
    "ZGTXXxu8E/MIWaEDB+Sm/",
    "1UI3",
    "E7fP5Pfijd+7K+t6Tg/NhuLq0eEUVChpJSkrKxpO",
    "ihtqpG6FMt65+Xk+tWUH2",
    "NhXXU9rg4XXdzo7u5o",
];

/// 生成 32 位十六进制设备 id（与官方 Web 客户端同构）。
pub fn generate_device_id() -> String {
    let mut rng = rand::rng();
    let mut id = String::with_capacity(32);
    for ch in DEVICE_ID_TEMPLATE.chars() {
        let nibble: u8 = rng.random_range(0..16);
        id.push(match ch {
            'x' => hex_digit(nibble),
            'y' => hex_digit((nibble & 3) | 8),
            other => other,
        });
    }
    id
}

fn hex_digit(value: u8) -> char {
    char::from_digit(u32::from(value), 16).unwrap_or('0')
}

/// 计算 captcha 签名：`1.` + 15 轮加盐 md5。
///
/// `timestamp_ms` 为毫秒时间戳；返回 `(timestamp, sign)` 便于调用方填充 meta。
pub fn captcha_sign(config: &Config, device_id: &str, timestamp_ms: u64) -> String {
    let mut text = format!(
        "{}{}{}{}{}",
        config.client_id, config.client_version, config.package_name, device_id, timestamp_ms
    );
    for salt in MD5_SALTS {
        text = md5_hex(&format!("{text}{salt}"));
    }
    format!("1.{text}")
}

fn md5_hex(text: &str) -> String {
    let digest = Md5::digest(text.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// 一次 captcha init 的结果。
#[derive(Debug, Clone)]
pub struct CaptchaToken {
    /// 可能为空：此时需要用户去 `url` 完成验证
    pub token: String,
    /// 官方验证页面（部分账号需要）
    pub url: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
}

impl CaptchaToken {
    fn is_usable(&self) -> bool {
        if self.token.is_empty() {
            return false;
        }
        match self.expires_at {
            // 与 rclone 一致：到期前 10 秒即视为失效
            Some(expiry) => expiry > Utc::now() + Duration::seconds(10),
            None => true,
        }
    }
}

#[derive(Serialize)]
struct InitRequest<'a> {
    action: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    captcha_token: Option<&'a str>,
    client_id: &'a str,
    device_id: &'a str,
    meta: InitMeta<'a>,
}

#[derive(Serialize)]
struct InitMeta<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    username: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    user_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    captcha_sign: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    timestamp: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    client_version: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    package_name: Option<&'a str>,
}

#[derive(Deserialize)]
struct InitResponse {
    #[serde(default)]
    captcha_token: Option<String>,
    #[serde(default)]
    expires_in: i64,
    #[serde(default)]
    url: Option<String>,
}

/// 按 action 缓存 captcha 令牌；过期或失效后重新申请。
pub(crate) struct CaptchaSource {
    http: HttpClient,
    config: Config,
    cached: Mutex<Option<(String, CaptchaToken)>>,
}

impl CaptchaSource {
    pub(crate) fn new(config: Config, http: HttpClient) -> Self {
        Self {
            http,
            config,
            cached: Mutex::new(None),
        }
    }

    /// 取当前 action 可用的令牌；必要时向官方申请。
    ///
    /// 登录场景的 `username` 与业务场景的 `user_id` 同时给出也可，meta 按 action 取舍。
    pub(crate) fn acquire(
        &self,
        action: &str,
        username: Option<&str>,
        user_id: Option<&str>,
    ) -> Result<CaptchaToken, Error> {
        let previous = {
            let mut cache = self.cached.lock().unwrap_or_else(|e| e.into_inner());
            if let Some((cached_action, token)) = cache.as_ref() {
                if cached_action == action && token.is_usable() {
                    return Ok(token.clone());
                }
            }
            cache
                .take()
                .map(|(_, token)| token)
                .filter(|token| !token.token.is_empty())
        };

        let token = self.init(action, username, user_id, previous.as_ref())?;
        let mut cache = self.cached.lock().unwrap_or_else(|e| e.into_inner());
        *cache = Some((action.to_owned(), token.clone()));
        Ok(token)
    }

    /// 使缓存失效（官方拒绝令牌后重试前调用）。
    pub(crate) fn invalidate(&self) {
        let mut cache = self.cached.lock().unwrap_or_else(|e| e.into_inner());
        *cache = None;
    }

    fn init(
        &self,
        action: &str,
        username: Option<&str>,
        user_id: Option<&str>,
        previous: Option<&CaptchaToken>,
    ) -> Result<CaptchaToken, Error> {
        let timestamp_ms = timestamp_millis();
        let timestamp = timestamp_ms.to_string();
        let device_id = self.http.device_id().to_owned();
        let sign = if action == ACTION_SIGNIN {
            None
        } else {
            Some(captcha_sign(&self.config, &device_id, timestamp_ms))
        };

        let meta = if action == ACTION_SIGNIN {
            InitMeta {
                username,
                user_id: None,
                captcha_sign: None,
                timestamp: None,
                client_version: None,
                package_name: None,
            }
        } else {
            InitMeta {
                username: None,
                user_id,
                captcha_sign: sign.as_deref(),
                timestamp: Some(&timestamp),
                client_version: Some(&self.config.client_version),
                package_name: Some(&self.config.package_name),
            }
        };

        let request = InitRequest {
            action,
            captcha_token: previous
                .map(|token| token.token.as_str())
                .filter(|token| !token.is_empty()),
            client_id: &self.config.client_id,
            device_id: &device_id,
            meta,
        };

        let body = serde_json::to_value(&request)
            .map_err(|e| Error::ApiChanged(format!("无法编码验证请求：{e}")))?;
        let response: InitResponse = self.http.call(
            reqwest::Method::POST,
            &self.config.captcha_init_url(),
            Body::Json(&body),
            None,
            None,
        )?;

        Ok(CaptchaToken {
            token: response.captcha_token.unwrap_or_default(),
            url: response.url.filter(|url| !url.is_empty()),
            expires_at: (response.expires_in > 0)
                .then(|| Utc::now() + Duration::seconds(response.expires_in)),
        })
    }
}
fn timestamp_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_id_has_expected_shape() {
        let id = generate_device_id();
        assert_eq!(id.len(), 32);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
        // 模板第 13 位固定为 '4'
        assert_eq!(&id[12..13], "4");
        // y 位（模板下标 14）最高位恒为 1（8-f）
        let y: u32 = u32::from_str_radix(&id[14..15], 16).unwrap_or(0);
        assert!((8..=15).contains(&y));
    }

    #[test]
    fn sign_is_deterministic_and_prefixed() {
        let config = Config::default();
        let a = captcha_sign(
            &config,
            "0123456789abcdef0123456789abcdef",
            1_700_000_000_000,
        );
        let b = captcha_sign(
            &config,
            "0123456789abcdef0123456789abcdef",
            1_700_000_000_000,
        );
        assert_eq!(a, b);
        assert!(a.starts_with("1."));
        // "1." + 32 位 md5 十六进制
        assert_eq!(a.len(), 34);
    }
}
