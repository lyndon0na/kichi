//! `notifications.rs` 的单元测试：下载通知文案。
//!
//! 只测拼文案的纯函数，不真的发系统通知。

use super::*;

fn summary<'a>(
    name: &'a str,
    folder: bool,
    done: usize,
    failed: usize,
    failure: Option<&'a str>,
) -> BatchSummary<'a> {
    BatchSummary {
        name,
        folder,
        done,
        failed,
        failure,
    }
}

#[test]
fn single_file_success_uses_name_only() {
    let notice = batch_finished(summary("电影.mkv", false, 1, 0, None));
    assert_eq!(notice.summary, "下载完成");
    assert_eq!(notice.body, "电影.mkv", "单文件成功只写文件名");
    assert!(
        !notice.body.contains("保存到") && !notice.body.contains('/'),
        "通知不写落盘路径：{}",
        notice.body
    );
    assert_eq!(notice.icon, "folder-download");
}

#[test]
fn folder_success_reports_count_without_path() {
    let notice = batch_finished(summary("剧集", true, 50, 0, None));
    assert_eq!(notice.summary, "下载完成");
    assert!(notice.body.contains("「剧集」"));
    assert!(notice.body.contains("50 个文件"));
    assert!(!notice.body.contains("保存到"));
    assert_eq!(notice.icon, "folder-download");
}

#[test]
fn folder_partial_failure_is_one_warning() {
    let notice = batch_finished(summary("剧集", true, 48, 2, Some("断网")));
    assert_eq!(notice.summary, "下载结束");
    assert!(notice.body.contains("成功 48 个，失败 2 个"));
    assert!(notice.body.contains("详情见传输任务页"));
    assert_eq!(notice.icon, "dialog-warning");
}

#[test]
fn folder_total_failure_is_error() {
    let notice = batch_finished(summary("剧集", true, 0, 50, Some("断网")));
    assert_eq!(notice.summary, "下载失败");
    assert_eq!(notice.icon, "dialog-error");
    assert!(notice.body.contains("50 个文件全部失败"));
    assert!(notice.body.contains("详情见传输任务页"));
}

#[test]
fn single_file_failure_includes_reason() {
    let notice = batch_finished(summary("电影.mkv", false, 0, 1, Some("网盘链接已过期")));
    assert_eq!(notice.summary, "下载失败");
    assert!(notice.body.contains("电影.mkv"));
    assert!(notice.body.contains("网盘链接已过期"));
    assert_eq!(notice.icon, "dialog-error");
}

#[test]
fn single_file_failure_without_reason_points_to_transfers() {
    let notice = batch_finished(summary("电影.mkv", false, 0, 1, None));
    assert!(notice.body.contains("下载失败，详情见传输任务页"));
}

#[test]
fn prepare_failure_includes_name_and_reason() {
    let notice = batch_failed("剧集", true, "这个位置没有可下载的文件");
    assert_eq!(notice.summary, "下载失败");
    assert_eq!(notice.body, "「剧集」\n这个位置没有可下载的文件");
    assert_eq!(notice.icon, "dialog-error");
}

#[test]
fn empty_batch_is_not_reported_as_success() {
    let notice = batch_finished(summary("剧集", true, 0, 0, None));
    assert_eq!(notice.summary, "下载结束");
    assert!(notice.body.contains("未下载任何文件"));
    assert_eq!(notice.icon, "dialog-warning");
}
