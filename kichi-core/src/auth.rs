//! 登录与令牌刷新。
//!
//! 流程照 rclone `backend/pikpak`（MIT）实测行为：
//! 1. 向 `/v1/shield/captcha/init` 申请登录用 captcha token（meta 只带用户名）；
//! 2. `POST /v1/auth/signin`（JSON：username / password / client_id）拿 access + refresh；
//! 3. 若被官方以 `captcha_invalid`(4002) 拒绝，作废令牌重取一次再试；
//! 4. 令牌刷新走 `/v1/auth/token` 的 oauth2 表单流程。

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use chrono::{Duration, Utc};
use serde::Deserialize;

use crate::captcha::{CaptchaSource, CaptchaToken, ACTION_SIGNIN};
use crate::http::{Body, HttpClient};
use crate::types::TokenPair;
use crate::{Config, Error};

/// 刷新令牌的 action（用于申请 captcha token）。
const ACTION_TOKEN: &str = "POST:/v1/auth/token";

/// 刷新令牌时提前量：过期前 5 分钟主动刷新，避免请求打到边界上。
pub const REFRESH_MARGIN: Duration = Duration::minutes(5);

/// 登录结果。
#[derive(Debug, Clone)]
pub struct LoginOutcome {
    pub tokens: TokenPair,
    /// 用户 id（JWT `sub` 或登录响应 `sub`）；可能为空
    pub user_id: String,
}

#[derive(Deserialize)]
struct TokenResponse {
    #[serde(default)]
    access_token: String,
    #[serde(default)]
    refresh_token: String,
    #[serde(default)]
    expires_in: i64,
    #[serde(default)]
    sub: String,
}

/// 登录：用户名 + 密码，必要时走人机验证分支。
///
/// `manual_captcha` 是用户在验证页面完成后粘贴回来的 token；给出时直接使用，
/// 不再向官方申请。返回 `Error::Captcha` 表示需要用户完成验证。
pub fn sign_in(
    config: &Config,
    device_id: &str,
    username: &str,
    password: &str,
    manual_captcha: Option<&str>,
) -> Result<LoginOutcome, Error> {
    if username.trim().is_empty() || password.is_empty() {
        return Err(Error::InvalidInput("用户名或密码为空".into()));
    }

    let http = HttpClient::new(config.clone(), device_id)?;
    let source = CaptchaSource::new(config.clone(), http.clone());

    let captcha = match manual_captcha.map(str::trim).filter(|t| !t.is_empty()) {
        Some(manual) => CaptchaToken {
            token: manual.to_owned(),
            url: None,
            expires_at: None,
        },
        None => source.acquire(ACTION_SIGNIN, Some(username.trim()), None)?,
    };

    if captcha.token.is_empty() {
        // 官方要求先在浏览器完成验证，令牌随后才会下发
        return Err(Error::Captcha {
            url: captcha.url,
            token: None,
        });
    }

    let payload = serde_json::json!({
        "username": username.trim(),
        "password": password,
        "client_id": config.client_id,
    });

    let send = |token: &str| -> Result<TokenResponse, Error> {
        http.call(
            reqwest::Method::POST,
            &config.signin_url(),
            Body::Json(&payload),
            None,
            Some(token),
        )
    };

    let first = send(&captcha.token);
    let response = match first {
        Ok(response) => response,
        // 官方拒绝缓存/过期的 token：作废后重取一次再试（rclone 同款兜底）
        Err(err) if err.is_captcha() => {
            source.invalidate();
            let retry = source.acquire(ACTION_SIGNIN, Some(username.trim()), None)?;
            if retry.token.is_empty() {
                return Err(Error::Captcha {
                    url: retry.url,
                    token: None,
                });
            }
            send(&retry.token)?
        }
        Err(err) => return Err(err),
    };

    if response.access_token.is_empty() || response.refresh_token.is_empty() {
        return Err(Error::ApiChanged("登录响应缺少令牌字段".into()));
    }

    let user_id = if response.sub.is_empty() {
        user_id_from_jwt(&response.access_token).unwrap_or_default()
    } else {
        response.sub
    };

    Ok(LoginOutcome {
        tokens: TokenPair {
            access_token: response.access_token,
            refresh_token: response.refresh_token,
            // 官方通常给 3600s；缺省时保守按 1 小时计
            expires_at: Utc::now() + Duration::seconds(response.expires_in.max(60)),
        },
        user_id,
    })
}

/// 用 refresh token 换取新的令牌对。
pub fn refresh(config: &Config, device_id: &str, refresh_token: &str) -> Result<TokenPair, Error> {
    if refresh_token.is_empty() {
        return Err(Error::Auth {
            reason: "没有刷新令牌".into(),
        });
    }

    let http = HttpClient::new(config.clone(), device_id)?;
    let form = [
        ("client_id", config.client_id.as_str()),
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
    ];

    let captcha = CaptchaSource::new(config.clone(), http.clone())
        .acquire(ACTION_TOKEN, None, None)
        .ok()
        .map(|token| token.token)
        .filter(|token| !token.is_empty());
    let response: TokenResponse = http.call(
        reqwest::Method::POST,
        &config.token_url(),
        Body::Form(&form),
        None,
        captcha.as_deref(),
    )?;

    if response.access_token.is_empty() {
        return Err(Error::ApiChanged("刷新响应缺少 access_token".into()));
    }

    Ok(TokenPair {
        access_token: response.access_token,
        // 某些响应不带新 refresh_token：沿用旧的
        refresh_token: if response.refresh_token.is_empty() {
            refresh_token.to_owned()
        } else {
            response.refresh_token
        },
        expires_at: Utc::now() + Duration::seconds(response.expires_in.max(60)),
    })
}

/// 从 JWT payload 解出 `sub`（用户 id）。解析失败返回 `None`，不视为错误。
pub fn user_id_from_jwt(access_token: &str) -> Option<String> {
    #[derive(Deserialize)]
    struct Claims {
        #[serde(default)]
        sub: String,
    }

    let payload = access_token.split('.').nth(1)?;
    let bytes = BASE64.decode(payload).ok()?;
    let claims: Claims = serde_json::from_slice(&bytes).ok()?;
    (!claims.sub.is_empty()).then_some(claims.sub)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_user_id_from_jwt_payload() {
        let claims = serde_json::json!({ "sub": "u-123", "exp": 1 });
        let encoded = BASE64.encode(claims.to_string().as_bytes());
        let token = format!("header.{encoded}.signature");
        assert_eq!(user_id_from_jwt(&token).as_deref(), Some("u-123"));
    }

    #[test]
    fn malformed_jwt_is_not_fatal() {
        assert_eq!(user_id_from_jwt(""), None);
        assert_eq!(user_id_from_jwt("a.b.c"), None);
        assert_eq!(user_id_from_jwt("no-dots"), None);
    }
}
