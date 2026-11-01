//! 多行离线任务提交：解析链接、顺序提交并汇总结果。
//!
//! 官方创建接口一次只接收一个链接。这里保持顺序提交，避免一次粘贴很多行时
//! 同时打出一串请求；某个链接失败后继续尝试后面的行，登录失效 / 官方限流则
//! 停止剩余请求。

use crate::session::{Failure, Session};

pub(super) struct SubmitOutcome {
    pub requested: usize,
    pub submitted: usize,
    pub errors: Vec<String>,
    pub stopped_early: bool,
    pub auth_failure: Option<Failure>,
}

/// 每行一个链接；去掉首尾空白和空行，保留输入顺序。
pub(super) fn parse_urls(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

pub(super) fn submit(session: &Session, urls: &[String], parent_id: Option<&str>) -> SubmitOutcome {
    let mut outcome = SubmitOutcome {
        requested: urls.len(),
        submitted: 0,
        errors: Vec::new(),
        stopped_early: false,
        auth_failure: None,
    };

    for url in urls {
        match session.add_offline_task(url, parent_id) {
            Ok(_) => outcome.submitted += 1,
            Err(Failure::Auth(reason)) => {
                outcome.auth_failure = Some(Failure::Auth(reason));
                outcome.stopped_early = true;
                break;
            }
            Err(err) => {
                let stop = err.is_rate_limited();
                outcome.errors.push(err.to_string());
                if stop {
                    outcome.stopped_early = true;
                    break;
                }
            }
        }
    }

    outcome
}

pub(super) fn summary(outcome: &SubmitOutcome) -> String {
    if outcome.errors.is_empty() {
        return format!("已提交 {} 条离线任务", outcome.submitted);
    }

    let first_error = outcome
        .errors
        .first()
        .map(String::as_str)
        .unwrap_or("未知错误");
    let remaining = outcome
        .requested
        .saturating_sub(outcome.submitted + outcome.errors.len());
    let mut text = if outcome.submitted == 0 {
        if outcome.requested == 1 {
            format!("提交失败：{first_error}")
        } else {
            format!("{} 条离线任务均未提交：{first_error}", outcome.requested)
        }
    } else {
        format!(
            "已提交 {} 条，{} 条失败",
            outcome.submitted,
            outcome.errors.len()
        )
    };
    if outcome.stopped_early && remaining > 0 {
        text.push_str(&format!("，{remaining} 条未提交"));
    }
    if outcome.submitted > 0 || outcome.requested > 1 {
        text.push_str(&format!("：{first_error}"));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_one_url_per_non_empty_line() {
        let urls = parse_urls(
            "  magnet:?xt=urn:btih:abc  \r\n\
             \n\
             https://example.com/file.zip\n",
        );
        assert_eq!(
            urls,
            vec![
                "magnet:?xt=urn:btih:abc".to_owned(),
                "https://example.com/file.zip".to_owned()
            ]
        );
    }

    #[test]
    fn summary_reports_success() {
        let outcome = SubmitOutcome {
            requested: 3,
            submitted: 3,
            errors: Vec::new(),
            stopped_early: false,
            auth_failure: None,
        };
        assert_eq!(summary(&outcome), "已提交 3 条离线任务");
    }

    #[test]
    fn summary_reports_partial_failure() {
        let outcome = SubmitOutcome {
            requested: 3,
            submitted: 2,
            errors: vec!["链接格式不正确".to_owned()],
            stopped_early: false,
            auth_failure: None,
        };
        assert_eq!(summary(&outcome), "已提交 2 条，1 条失败：链接格式不正确");
    }

    #[test]
    fn summary_reports_unsubmitted_lines_after_a_stop() {
        let outcome = SubmitOutcome {
            requested: 5,
            submitted: 1,
            errors: vec!["触发官方配额限制".to_owned()],
            stopped_early: true,
            auth_failure: None,
        };
        assert_eq!(
            summary(&outcome),
            "已提交 1 条，1 条失败，3 条未提交：触发官方配额限制"
        );
    }
}
