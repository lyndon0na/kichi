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
    summary_with(name, folder, done, failed, 0, failure)
}

fn summary_with<'a>(
    name: &'a str,
    folder: bool,
    done: usize,
    failed: usize,
    skipped: usize,
    failure: Option<&'a str>,
) -> BatchSummary<'a> {
    BatchSummary {
        name,
        folder,
        done,
        failed,
        cancelled: 0,
        skipped,
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
    assert!(!notice.body.contains("跳过"), "没有跳过时不写后缀");
    assert_eq!(notice.icon, "folder-download");
}

#[test]
fn folder_success_mentions_skipped_directories() {
    let notice = batch_finished(summary_with("剧集", true, 50, 0, 2, None));
    assert_eq!(notice.summary, "下载完成");
    assert!(
        notice.body.contains("50 个文件 · 跳过 2 个目录"),
        "跳过数是完成通知的一部分：{}",
        notice.body
    );
}

#[test]
fn folder_partial_failure_carries_the_skip_count() {
    let notice = batch_finished(summary_with("剧集", true, 48, 2, 1, Some("断网")));
    assert!(notice
        .body
        .contains("成功 48 个，失败 2 个 · 跳过 1 个目录"));
    assert!(notice.body.contains("详情见传输任务页"));
}

#[test]
fn folder_total_failure_does_not_add_the_skip_count() {
    let notice = batch_finished(summary_with("剧集", true, 0, 50, 3, Some("断网")));
    assert!(
        !notice.body.contains("跳过"),
        "全失败时数字已经够多，不塞跳过数：{}",
        notice.body
    );
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
fn folder_partial_cancellation_is_one_warning() {
    let mut notice_summary = summary("剧集", true, 3, 0, None);
    notice_summary.cancelled = 2;
    let notice = batch_finished(notice_summary);
    assert_eq!(notice.summary, "下载结束");
    assert!(notice.body.contains("成功 3 个，失败 0 个，取消 2 个"));
    assert_eq!(notice.icon, "dialog-warning");
}

#[test]
fn folder_all_cancelled_is_informational() {
    let mut notice_summary = summary("剧集", true, 0, 0, None);
    notice_summary.cancelled = 4;
    let notice = batch_finished(notice_summary);
    assert_eq!(notice.summary, "下载结束");
    assert!(notice.body.contains("取消 4 个文件"));
    assert_eq!(notice.icon, "dialog-information");
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
fn folder_failures_with_some_cancelled_do_not_claim_everything_failed() {
    let mut notice_summary = summary("剧集", true, 0, 3, Some("断网"));
    notice_summary.cancelled = 2;
    let notice = batch_finished(notice_summary);
    assert_eq!(notice.summary, "下载失败");
    assert!(notice.body.contains("失败 3 个，取消 2 个"));
    assert!(!notice.body.contains("全部失败"));
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
    let notice = batch_failed("剧集", true, "所有子目录都无法读取（跳过 2 个）");
    assert_eq!(notice.summary, "下载失败");
    assert_eq!(notice.body, "「剧集」\n所有子目录都无法读取（跳过 2 个）");
    assert_eq!(notice.icon, "dialog-error");
}

fn upload_summary<'a>(
    done: u32,
    failed: u32,
    cancelled: u32,
    interrupted: u32,
    failure: Option<&'a str>,
) -> UploadSummary<'a> {
    UploadSummary {
        done,
        failed,
        cancelled,
        interrupted,
        failure,
    }
}

#[test]
fn upload_success_is_one_summary() {
    let notice = upload_finished(upload_summary(3, 0, 0, 0, None)).expect("成功要通知");
    assert_eq!(notice.summary, "上传完成");
    assert_eq!(notice.body, "3 个文件已上传");
    assert_eq!(notice.icon, "folder-upload");
}

#[test]
fn upload_partial_failure_points_to_transfers_page() {
    let notice =
        upload_finished(upload_summary(2, 1, 0, 0, Some("连接中断"))).expect("部分失败要通知");
    assert_eq!(notice.summary, "上传结束");
    assert!(notice.body.contains("成功 2 个，失败 1 个"));
    assert!(notice.body.contains("详情见传输任务页"));
    assert_eq!(notice.icon, "dialog-warning");
}

#[test]
fn upload_total_failure_uses_the_last_reason() {
    let notice =
        upload_finished(upload_summary(0, 2, 0, 0, Some("上传连接中断"))).expect("全部失败要通知");
    assert_eq!(notice.summary, "上传失败");
    assert!(notice.body.contains("失败 2 个"));
    assert!(notice.body.contains("上传连接中断"));
    assert_eq!(notice.icon, "dialog-error");
}

#[test]
fn upload_cancel_and_interrupt_alone_stay_silent() {
    assert!(upload_finished(upload_summary(0, 0, 2, 0, None)).is_none());
    assert!(upload_finished(upload_summary(0, 0, 0, 1, None)).is_none());
}

#[test]
fn upload_mixed_cancel_still_reports_completed_files() {
    let notice = upload_finished(upload_summary(1, 0, 1, 0, None)).expect("有成功就通知");
    assert_eq!(notice.summary, "上传结束");
    assert!(notice.body.contains("成功 1 个，取消 1 个"));
}

#[test]
fn empty_batch_is_not_reported_as_success() {
    // 空文件夹已在 `queue_worker::end_notice` 直接返回 None，正常不会走到这里；
    // 这条只锁住 builder 自己的兜底文案，防止别处误用成「下载完成」。
    let notice = batch_finished(summary("剧集", true, 0, 0, None));
    assert_eq!(notice.summary, "下载结束");
    assert!(notice.body.contains("未下载任何文件"));
    assert_eq!(notice.icon, "dialog-warning");
}
