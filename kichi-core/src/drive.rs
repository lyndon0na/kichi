//! 网盘侧只读接口（S2 只用到容量 / 会员 / 当前用户）。
//!
//! 每个请求都带 `x-captcha-token`；官方以 `captcha_invalid` 拒绝时，
//! 作废并重取一次再试（rclone 同款兜底）。

use std::sync::Arc;

use serde::Deserialize;

use crate::captcha::CaptchaSource;
use crate::http::{Body, HttpClient};
use crate::types::Quota;
use crate::{Config, Error};

/// 已登录的网盘客户端（内部令牌由调用方的 `Session` 提供）。
pub struct DriveClient {
    config: Config,
    http: HttpClient,
    captcha: CaptchaSource,
    access_token: Arc<str>,
    user_id: String,
}

/// 当前用户摘要。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct UserInfo {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub email: String,
}

#[derive(Deserialize)]
struct AboutResponse {
    #[serde(default)]
    quota: Option<AboutQuota>,
}

#[derive(Deserialize)]
struct AboutQuota {
    #[serde(default, deserialize_with = "lenient_u64")]
    usage: u64,
    #[serde(default, deserialize_with = "lenient_u64")]
    limit: u64,
    #[serde(default)]
    is_unlimited: bool,
}

/// 官方把大数字编码为字符串（`"123"`），也兼容直接给数字。
fn lenient_u64<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error as _;
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::Number(n) => Ok(n.as_u64().unwrap_or(0)),
        serde_json::Value::String(s) => s.parse::<u64>().map_err(D::Error::custom),
        _ => Ok(0),
    }
}

impl DriveClient {
    fn with_http(config: Config, http: HttpClient, access_token: Arc<str>, user_id: &str) -> Self {
        let captcha = CaptchaSource::new(config.clone(), http.clone());
        Self {
            config,
            http,
            captcha,
            access_token,
            user_id: user_id.to_owned(),
        }
    }

    /// 已知令牌构造客户端（不做任何 I/O；登录/刷新由 `auth` 完成）。
    pub fn new(
        config: &Config,
        device_id: &str,
        access_token: &str,
        user_id: &str,
    ) -> Result<Self, Error> {
        let http = HttpClient::new(config.clone(), device_id)?;
        Ok(Self::with_http(
            config.clone(),
            http,
            Arc::from(access_token),
            user_id,
        ))
    }

    /// 带鉴权与 captcha 的 GET 请求（captcha 被拒时自动重试一次）。
    fn get<R: serde::de::DeserializeOwned>(&self, url: &str) -> Result<R, Error> {
        let action = action_for("GET", url);
        let token = self.captcha.acquire(&action, None, Some(&self.user_id))?;
        let first = self.http.call::<R>(
            reqwest::Method::GET,
            url,
            Body::Empty,
            Some(&self.access_token),
            Some(token.token.as_str()).filter(|t| !t.is_empty()),
        );
        match first {
            Err(err) if err.is_captcha() => {
                self.captcha.invalidate();
                let retry = self.captcha.acquire(&action, None, Some(&self.user_id))?;
                self.http.call(
                    reqwest::Method::GET,
                    url,
                    Body::Empty,
                    Some(&self.access_token),
                    Some(retry.token.as_str()).filter(|t| !t.is_empty()),
                )
            }
            other => other,
        }
    }

    /// 容量与用量。
    pub fn about(&self) -> Result<Quota, Error> {
        let response: AboutResponse = self.get(&self.config.about_url())?;
        let quota = response
            .quota
            .ok_or_else(|| Error::ApiChanged("about 响应缺少 quota".into()))?;
        // 会员状态只在成功时覆盖，失败按非会员处理（不让容量查询一起失败）
        let vip = self.is_vip().unwrap_or(false);
        Ok(Quota {
            usage: quota.usage,
            limit: (!quota.is_unlimited && quota.limit > 0).then_some(quota.limit),
            vip,
        })
    }

    /// 会员状态（失败不致命，`about` 里按 false 处理）。
    pub fn is_vip(&self) -> Result<bool, Error> {
        let vip: VipResponse = self.get(&self.config.vip_url())?;
        Ok(vip.data.is_some_and(|data| {
            data.status.eq_ignore_ascii_case("ok") && !data.vip_type.eq_ignore_ascii_case("novip")
        }))
    }

    /// 当前用户（name / email 用来在界面上显示身份）。
    pub fn me(&self) -> Result<UserInfo, Error> {
        self.get(&self.config.user_me_url())
    }
}

#[derive(Deserialize)]
struct VipResponse {
    #[serde(default)]
    data: Option<VipData>,
}

#[derive(Deserialize)]
struct VipData {
    #[serde(default)]
    status: String,
    #[serde(default, rename = "type")]
    vip_type: String,
}

/// 把 URL 还原成 rclone 风格的 action：`GET:/drive/v1/about`。
fn action_for(method: &str, url: &str) -> String {
    let path = url
        .split("://")
        .nth(1)
        .and_then(|rest| rest.find('/').map(|idx| &rest[idx..]))
        .unwrap_or(url);
    let path = path.split('?').next().unwrap_or(path);
    format!("{method}:{path}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_uses_path_only() {
        assert_eq!(
            action_for("GET", "https://api-drive.mypikpak.com/drive/v1/about"),
            "GET:/drive/v1/about"
        );
        assert_eq!(
            action_for("GET", "https://user.mypikpak.com/v1/user/me?x=1"),
            "GET:/v1/user/me"
        );
        assert_eq!(action_for("GET", "/drive/v1/about"), "GET:/drive/v1/about");
    }

    #[test]
    fn quota_numbers_may_be_strings() {
        let json = r#"{"quota":{"limit":"10995116277760","usage":"123","is_unlimited":false}}"#;
        let Ok(parsed) = serde_json::from_str::<AboutResponse>(json) else {
            panic!("quota 字符串形式应可解析");
        };
        let Some(quota) = parsed.quota else {
            panic!("缺少 quota");
        };
        assert_eq!(quota.usage, 123);
        assert_eq!(quota.limit, 10_995_116_277_760);
    }
}
