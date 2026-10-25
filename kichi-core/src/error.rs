//! 错误分类：GUI 按类别给出不同的提示与恢复建议。

/// kichi-core 的全部错误。
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// 登录态失效：令牌被拒或无法刷新、验证码校验失败。UI 应引导重新登录。
    #[error("认证失败：{reason}")]
    Auth { reason: String },

    /// 触发官方限制：限流、每日离线任务配额、流量/空间配额。
    /// 非客户端故障，UI 应解释原因而不是盲目重试。
    #[error("触发官方限制：{reason}")]
    RateLimited { reason: String },

    /// 网络层错误（DNS、连接失败、超时）。
    #[error("网络错误：{0}")]
    Network(String),

    /// API 响应结构与已知格式不符——官方接口可能已变动。
    #[error("接口响应无法识别（API 可能已变动）：{0}")]
    ApiChanged(String),

    /// 调用方传参无效。
    #[error("参数无效：{0}")]
    InvalidInput(String),

    /// 结构合法但未归类的业务错误，保留原始 code 与描述。
    #[error("接口错误 {code} {reason}：{message}")]
    Api {
        code: i64,
        reason: String,
        message: String,
    },
}

impl Error {
    /// 把 PikPak API 的错误负载（`error` + `error_code` + `error_description`）
    /// 映射为错误类别；已知码表来自 rclone 的实践经验（backend/pikpak）。
    pub fn from_api(code: i64, reason: &str, message: &str) -> Self {
        let desc = describe(code, reason, message);
        match reason {
            // 刷新令牌无效或过期（4126）
            "invalid_grant" => Error::Auth { reason: desc },
            // access_token 无效（16）
            "unauthenticated" => Error::Auth { reason: desc },
            // 验证码令牌被拒（登录时 4002，其他端点 9）
            "captcha_invalid" => Error::Auth { reason: desc },
            // 每日离线任务数已达上限（11）
            "task_daily_create_limit_vip" => Error::RateLimited { reason: desc },
            // 存储空间不足（8）
            "file_space_not_enough" => Error::RateLimited { reason: desc },
            _ => Error::Api {
                code,
                reason: reason.to_owned(),
                message: message.to_owned(),
            },
        }
    }
}

/// 拼装给人看的错误描述。
fn describe(code: i64, reason: &str, message: &str) -> String {
    if message.is_empty() {
        format!("{reason}（code {code}）")
    } else {
        format!("{reason}（code {code}）：{message}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_known_api_errors() {
        assert!(matches!(
            Error::from_api(4126, "invalid_grant", "refresh token expired"),
            Error::Auth { .. }
        ));
        assert!(matches!(
            Error::from_api(9, "captcha_invalid", ""),
            Error::Auth { .. }
        ));
        assert!(matches!(
            Error::from_api(11, "task_daily_create_limit_vip", "too many"),
            Error::RateLimited { .. }
        ));
        assert!(matches!(
            Error::from_api(8, "file_space_not_enough", ""),
            Error::RateLimited { .. }
        ));
    }

    #[test]
    fn unknown_api_errors_keep_raw_payload() {
        match Error::from_api(3, "file_duplicated_name", "name repeated") {
            Error::Api {
                code,
                reason,
                message,
            } => {
                assert_eq!(code, 3);
                assert_eq!(reason, "file_duplicated_name");
                assert_eq!(message, "name repeated");
            }
            other => panic!("应归类为 Api，实际是 {other:?}"),
        }
    }
}
