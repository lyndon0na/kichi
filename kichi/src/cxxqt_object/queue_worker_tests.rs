//! `queue_worker.rs` 的单元测试：进度节流与批次结束通知。
//!
//! 下载循环本身要真实会话，属于 `live_tests` 的范围；显示路径的测试在
//! `transfers_tests.rs`（由 `TransferJob::display_name` 负责）。

use super::*;

fn report(
    name: &str,
    folder: bool,
    done: usize,
    failed: usize,
    failure: Option<&str>,
) -> BatchReport {
    BatchReport {
        name: name.to_owned(),
        folder,
        done,
        failed,
        failure: failure.map(str::to_owned),
    }
}

#[test]
fn throttle_sends_first_then_waits_within_same_percent() {
    let throttle = Throttle::default();
    assert!(throttle.should_send(0, Some(10_000)), "第一次一定发");
    assert!(
        !throttle.should_send(50, Some(10_000)),
        "仍是 0%、未到 250ms：不发"
    );
    assert!(throttle.should_send(500, Some(10_000)), "进到 5% 就发");
}

#[test]
fn throttle_without_total_uses_time_only() {
    let throttle = Throttle::default();
    assert!(throttle.should_send(1, None));
    assert!(!throttle.should_send(2, None), "没有总大小时按时间节流");
}

#[test]
fn cancelled_batch_has_no_notice() {
    let report = report("剧集", true, 3, 0, None);
    assert!(
        end_notice(&report, BatchEnd::Cancelled).is_none(),
        "用户主动取消不弹系统通知"
    );
}

#[test]
fn interrupted_batch_has_one_error_notice() {
    let notice =
        end_notice(&report("剧集", true, 3, 0, None), BatchEnd::Interrupted).expect("中断要弹一条");
    assert_eq!(notice.summary, "下载失败");
    assert_eq!(notice.icon, "dialog-error");
    assert!(notice.body.contains("下载线程异常中断"));
}

#[test]
fn completed_batch_uses_summary_builder() {
    let notice = end_notice(&report("a.mkv", false, 1, 0, None), BatchEnd::Completed)
        .expect("正常完成要弹一条");
    assert_eq!(notice.summary, "下载完成");
    assert_eq!(notice.body, "a.mkv");
    assert!(!notice.body.contains('/'), "通知不写保存路径");
}
