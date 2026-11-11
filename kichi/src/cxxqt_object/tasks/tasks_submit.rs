//! 多行离线任务提交：解析链接、顺序提交并汇总结果。
//!
//! 官方创建接口一次只接收一个链接。这里保持顺序提交，避免一次粘贴很多行时
//! 同时打出一串请求；某个链接失败后继续尝试后面的行，登录失效 / 官方限流则
//! 停止剩余请求。

use std::path::{Path, PathBuf};

use kichi_core::torrent::parse_torrent_file;
use kichi_core::{is_supported_task_url, normalize_task_url, OfflineTask};

use crate::session::{Failure, Session};

pub(super) struct SubmitOutcome {
    pub requested: usize,
    pub submitted: usize,
    pub created: Vec<OfflineTask>,
    pub errors: Vec<String>,
    pub stopped_early: bool,
    pub auth_failure: Option<Failure>,
}

/// 多行输入拆出来的可用链接与无法识别的行号（1 起）。
pub(super) struct ParsedInput {
    pub urls: Vec<String>,
    pub invalid_lines: Vec<usize>,
}

/// 每行一个链接；去掉首尾空白和空行，保留输入顺序。
///
/// 只把形态上像磁力 / 纯 infohash / ed2k / HTTP(S) 的行交给网络层，其余行
/// 记录行号，既不发请求，也不静默吞掉。纯 infohash 在这里补成标准磁力链接。
pub(super) fn parse_input(text: &str) -> ParsedInput {
    let mut urls = Vec::new();
    let mut invalid_lines = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if is_supported_task_url(line) {
            urls.push(normalize_task_url(line));
        } else {
            invalid_lines.push(index + 1);
        }
    }
    ParsedInput {
        urls,
        invalid_lines,
    }
}

/// 从任务记录里取出可重新提交的来源链接。
///
/// 只接受「恰好一条能识别的链接」：`params.url` 缺失、为空、或塞了多行 /
/// 无法识别的文本都返回 `None`，绝不凭空提交或提交空磁力。
pub(super) fn retry_url(source_url: Option<&str>) -> Option<String> {
    let mut parsed = parse_input(source_url?);
    if !parsed.invalid_lines.is_empty() || parsed.urls.len() != 1 {
        return None;
    }
    parsed.urls.pop()
}

pub(super) fn submit(session: &Session, urls: &[String], parent_id: Option<&str>) -> SubmitOutcome {
    let mut outcome = SubmitOutcome {
        requested: urls.len(),
        submitted: 0,
        created: Vec::new(),
        errors: Vec::new(),
        stopped_early: false,
        auth_failure: None,
    };

    for url in urls {
        match session.add_offline_task(url, parent_id) {
            Ok(task) => {
                outcome.submitted += 1;
                outcome.created.push(task);
            }
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

/// 按输入顺序解析并提交一批本地 `.torrent` 文件。每个文件提交一条磁力任务。
pub(super) fn submit_torrents(
    session: &Session,
    paths: &[PathBuf],
    parent_id: Option<&str>,
) -> SubmitOutcome {
    let mut outcome = SubmitOutcome {
        requested: paths.len(),
        submitted: 0,
        created: Vec::new(),
        errors: Vec::new(),
        stopped_early: false,
        auth_failure: None,
    };

    for path in paths {
        let info = match parse_torrent_file(path) {
            Ok(info) => info,
            Err(err) => {
                outcome
                    .errors
                    .push(format!("{}：{err}", torrent_file_label(path)));
                continue;
            }
        };
        match session.add_offline_task(&info.magnet(), parent_id) {
            Ok(task) => {
                outcome.submitted += 1;
                outcome.created.push(task);
            }
            Err(Failure::Auth(reason)) => {
                outcome.auth_failure = Some(Failure::Auth(reason));
                outcome.stopped_early = true;
                break;
            }
            Err(err) => {
                let stop = err.is_rate_limited();
                outcome
                    .errors
                    .push(format!("{}：{err}", torrent_file_label(path)));
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

pub(super) fn torrent_summary(outcome: &SubmitOutcome) -> String {
    if outcome.errors.is_empty() && !outcome.stopped_early {
        return format!("已解析并提交 {} 个 .torrent 文件", outcome.submitted);
    }
    summary(outcome)
}

fn torrent_file_label(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or(".torrent 文件")
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_one_url_per_non_empty_line_and_keeps_input_order() {
        let parsed = parse_input(
            "  magnet:?xt=urn:btih:abc  \r\n\
             \n\
             https://example.com/file.zip\n",
        );
        assert_eq!(
            parsed.urls,
            vec![
                "magnet:?xt=urn:btih:abc".to_owned(),
                "https://example.com/file.zip".to_owned()
            ]
        );
        assert!(parsed.invalid_lines.is_empty());
    }

    #[test]
    fn unknown_lines_are_reported_and_never_submitted() {
        let parsed = parse_input(
            "随便写点什么\n\
             magnet:?xt=urn:btih:abc\n\
             ed2k://|file|a.mkv|1|HASH|/\n\
             www.example.com/file.zip\n",
        );
        assert_eq!(
            parsed.urls,
            vec![
                "magnet:?xt=urn:btih:abc".to_owned(),
                "ed2k://|file|a.mkv|1|HASH|/".to_owned()
            ]
        );
        assert_eq!(parsed.invalid_lines, vec![1, 4]);
    }

    #[test]
    fn bare_infohash_is_normalized_before_submission() {
        let parsed = parse_input(
            "0123456789ABCDEF0123456789ABCDEF01234567\n\
             AERUKZ4JVPG66AJDIVTYTK6N54ASGRLH\n",
        );
        assert_eq!(
            parsed.urls,
            vec![
                "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567".to_owned(),
                "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567".to_owned(),
            ]
        );
        assert!(parsed.invalid_lines.is_empty());
    }

    #[test]
    fn retry_url_accepts_exactly_one_recognized_link() {
        assert_eq!(
            retry_url(Some("  magnet:?xt=urn:btih:abc  ")).as_deref(),
            Some("magnet:?xt=urn:btih:abc")
        );
        assert_eq!(
            retry_url(Some("0123456789ABCDEF0123456789ABCDEF01234567")).as_deref(),
            Some("magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567"),
            "裸 infohash 沿用提交链路的规范化"
        );
        assert_eq!(
            retry_url(Some("https://example.com/file.zip")).as_deref(),
            Some("https://example.com/file.zip")
        );
    }

    #[test]
    fn retry_url_rejects_missing_blank_or_multiple_links() {
        assert_eq!(retry_url(None), None);
        assert_eq!(retry_url(Some("")), None);
        assert_eq!(retry_url(Some("   ")), None);
        assert_eq!(retry_url(Some("不是链接")), None);
        assert_eq!(retry_url(Some("magnet:?dn=no-hash")), None);
        assert_eq!(
            retry_url(Some("magnet:?xt=urn:btih:abc\nmagnet:?xt=urn:btih:def")),
            None,
            "响应里若混进多行，不能一次提交多条"
        );
        assert_eq!(
            retry_url(Some("magnet:?xt=urn:btih:abc\n垃圾行")),
            None,
            "任何无法识别的行都拒绝整条链接"
        );
    }

    #[test]
    fn summary_reports_success() {
        let outcome = SubmitOutcome {
            requested: 3,
            submitted: 3,
            created: Vec::new(),
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
            created: Vec::new(),
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
            created: Vec::new(),
            errors: vec!["触发官方配额限制".to_owned()],
            stopped_early: true,
            auth_failure: None,
        };
        assert_eq!(
            summary(&outcome),
            "已提交 1 条，1 条失败，3 条未提交：触发官方配额限制"
        );
    }

    #[test]
    fn torrent_summary_names_the_source_files() {
        let outcome = SubmitOutcome {
            requested: 2,
            submitted: 1,
            created: Vec::new(),
            errors: vec!["坏种子.torrent：无法解析 .torrent：缺少 info 字典".to_owned()],
            stopped_early: false,
            auth_failure: None,
        };
        assert_eq!(
            torrent_summary(&outcome),
            "已提交 1 条，1 条失败：坏种子.torrent：无法解析 .torrent：缺少 info 字典"
        );
    }
}
