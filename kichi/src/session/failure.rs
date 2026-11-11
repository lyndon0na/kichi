//! 会话层错误：把 core 的错误转成界面能直接消费的类别。
//!
//! 从 `mod.rs` 拆出来只为守行数上限；语义没有变动。

use kichi_core::Error as CoreError;

use super::StoreError;

/// 一次操作的结果（给调用方转成界面提示）。
#[derive(Debug)]
pub enum Failure {
    Captcha {
        url: Option<String>,
    },
    Auth(String),
    /// 触发官方限制：配额、日任务数、限速等。不是客户端故障，要给解释而不是重试
    RateLimited {
        detail: String,
    },
    /// 别人的分享打不开：需要密码 / 密码错误 / 链接失效等。
    ShareUnavailable {
        status: String,
        message: String,
    },
    Other(String),
}

impl From<CoreError> for Failure {
    fn from(err: CoreError) -> Self {
        match err {
            CoreError::Captcha { url, .. } => Failure::Captcha { url },
            CoreError::Auth { reason } => Failure::Auth(reason),
            CoreError::RateLimited { reason } => Failure::RateLimited { detail: reason },
            CoreError::ShareUnavailable { status, message } => {
                Failure::ShareUnavailable { status, message }
            }
            other => Failure::Other(other.to_string()),
        }
    }
}

impl Failure {
    /// 是否属于「官方限制」类（界面上要解释原因，不该让用户以为坏了）。
    pub fn is_rate_limited(&self) -> bool {
        matches!(self, Failure::RateLimited { .. })
    }

    /// 登录态 / 验证 / 限流：继续发请求只会重复失败，批量操作应中止剩余请求。
    pub fn is_session_failure(&self) -> bool {
        matches!(
            self,
            Failure::Auth(_) | Failure::Captcha { .. } | Failure::RateLimited { .. }
        )
    }

    /// 分享打不开时的官方状态码。
    pub fn share_status(&self) -> Option<&str> {
        match self {
            Failure::ShareUnavailable { status, .. } => Some(status),
            _ => None,
        }
    }

    /// 这个分享需要访问密码（界面据此切到密码输入）。
    pub fn needs_pass_code(&self) -> bool {
        self.share_status() == Some("PASS_CODE_EMPTY")
    }
}

/// 配额 / 限速类错误的完整解释。
///
/// 官方超限一般是**软限速**（约 100KB/s）而不是报错，客户端探测不到；
/// 能收到的都是明确的配额拒绝（日任务数、空间不足、HTTP 429）。
/// 这段文案的目的就是告诉用户「这是官方规则，不是 kichi 出故障」。
pub fn rate_limit_text(failure: &Failure) -> String {
    let detail = match failure {
        Failure::RateLimited { detail } => detail.as_str(),
        _ => "",
    };
    let head = if detail.is_empty() {
        "PikPak 拒绝了这个操作：触发了官方配额或限速限制".to_owned()
    } else {
        format!("PikPak 拒绝了这个操作：{detail}")
    };
    format!(
        "{head}\n这是 PikPak 官方的限制，不是 kichi 故障。\
         免费账号每日总下行约 20GB（新加坡时间 0 点重置），第三方接入合计约 5GB/天；\
         超限后官方会降速到约 100KB/s 而不报错。会员免每日限额，但第三方月度限额仍在。\
         处理办法：等配额重置，或在官方客户端里确认账号状态。"
    )
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Failure::Captcha { url: Some(url) } => {
                write!(f, "需要完成人机验证，请在浏览器打开：{url}")
            }
            Failure::Captcha { url: None } => write!(f, "需要完成人机验证"),
            Failure::Auth(reason) => write!(f, "登录状态失效：{reason}"),
            Failure::RateLimited { .. } => write!(f, "{}", rate_limit_text(self)),
            Failure::ShareUnavailable { message, .. } => write!(f, "{message}"),
            Failure::Other(reason) => write!(f, "{reason}"),
        }
    }
}

impl From<StoreError> for Failure {
    fn from(err: StoreError) -> Self {
        Failure::Other(err.to_string())
    }
}

#[cfg(test)]
mod limit_tests {
    use super::*;

    #[test]
    fn rate_limit_text_explains_it_is_official_not_a_bug() {
        let failure = Failure::RateLimited {
            detail: "task_daily_create_limit_vip（code 11）".into(),
        };
        assert!(failure.is_rate_limited());
        let text = rate_limit_text(&failure);
        // 用户最需要知道的三件事：不是客户端坏了 / 官方规则是什么 / 怎么办
        assert!(text.contains("不是 kichi 故障"), "{text}");
        assert!(text.contains("20GB"), "要给出具体额度：{text}");
        assert!(text.contains("100KB/s"), "要说明限速表现：{text}");
        assert!(text.contains("code 11"), "保留原始错误码便于排查：{text}");
    }

    #[test]
    fn other_failures_are_not_rate_limited() {
        assert!(!Failure::Other("x".into()).is_rate_limited());
        assert!(!Failure::Auth("x".into()).is_rate_limited());
    }

    #[test]
    fn core_rate_limited_maps_to_the_explained_failure() {
        let failure = Failure::from(kichi_core::Error::RateLimited {
            reason: "file_space_not_enough（code 8）".into(),
        });
        assert!(failure.is_rate_limited());
        assert!(failure.to_string().contains("不是 kichi 故障"));
    }
}
