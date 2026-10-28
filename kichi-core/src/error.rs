//! 错误分类：GUI 按类别给出不同的提示与恢复建议。

/// kichi-core 的全部错误。
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// 登录态失效：令牌被拒或无法刷新、验证码校验失败。UI 应引导重新登录。
    #[error("认证失败：{reason}")]
    Auth { reason: String },

    /// 需要用户完成人机验证。`url` 为官方验证页面；令牌可能已随响应下发。
    #[error("需要完成人机验证，请打开：{url:?}")]
    Captcha {
        url: Option<String>,
        /// 验证流程中上一次拿到的 captcha token（可能为空），重试时沿用
        token: Option<String>,
    },

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

    /// 下载等响应头超时（不是连接失败；文案已经写好，不再加前缀）。
    #[error("{0}")]
    DownloadTimeout(String),

    /// 服务器拒绝从请求的偏移继续（HTTP 416，或 206 的区间信息对不上）。
    /// 调用方应把 `.kichi-part` 截断后从 0 重来。
    #[error("服务器不接受从这个位置继续下载")]
    RangeNotSatisfiable {
        /// 服务器在 `Content-Range: bytes */<total>` 里给的总大小
        total: Option<u64>,
    },

    /// 本地/业务侧的普通失败（不是网络错误，也不属于官方限制）。
    #[error("{0}")]
    Other(String),
}

impl Error {
    /// 是否属于“需要人机验证”的失败（登录流程据此回到验证分支）。
    pub fn is_captcha(&self) -> bool {
        matches!(self, Error::Captcha { .. })
    }

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
            "captcha_invalid" => Error::Captcha {
                url: None,
                token: None,
            },
            // 每日离线任务数已达上限（11）
            "task_daily_create_limit_vip" => Error::RateLimited { reason: desc },
            // 存储空间不足（8）
            "file_space_not_enough" => Error::RateLimited { reason: desc },
            // 同目录已有同名文件/文件夹（3）：改名或新建时最常见
            "file_duplicated_name" => {
                Error::InvalidInput("这个名字已被同目录下的其他条目占用，请换一个".into())
            }
            // 条目已被别处删除 / 移动（owner / 多端同时操作）
            "file_not_found" => {
                Error::InvalidInput("这个条目已不存在（可能在其它设备上被删除或移动了）".into())
            }
            // 移动 / 复制到当前目录或自己的子目录（9）
            "file_move_or_copy_to_cur" => Error::InvalidInput(
                "不能移动/复制到它当前所在的文件夹，也不能移进自己的子文件夹".into(),
            ),
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
            Error::Captcha { .. }
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
        match Error::from_api(4, "some_future_error", "detail") {
            Error::Api {
                code,
                reason,
                message,
            } => {
                assert_eq!(code, 4);
                assert_eq!(reason, "some_future_error");
                assert_eq!(message, "detail");
            }
            other => panic!("应归类为 Api，实际是 {other:?}"),
        }
    }

    #[test]
    fn duplicate_name_reads_as_plain_chinese() {
        // 改名 / 新建文件夹最常见的失败：不能把错误码原样丢给用户
        let err = Error::from_api(3, "file_duplicated_name", "name repeated");
        assert!(matches!(err, Error::InvalidInput(_)));
        let text = err.to_string();
        assert!(text.contains("已被同目录"), "要说明是重名：{text}");
        assert!(!text.contains("code"), "不该暴露错误码：{text}");
    }

    #[test]
    fn missing_file_reads_as_plain_chinese() {
        let err = Error::from_api(1, "file_not_found", "");
        assert!(matches!(err, Error::InvalidInput(_)));
        assert!(err.to_string().contains("已不存在"));
    }

    #[test]
    fn same_folder_move_reads_as_plain_chinese() {
        let err = Error::from_api(9, "file_move_or_copy_to_cur", "");
        assert!(matches!(err, Error::InvalidInput(_)));
        let text = err.to_string();
        assert!(text.contains("当前所在的文件夹"), "要说明目标非法：{text}");
    }
}
