//! `queue_worker.rs` 的单元测试：进度节流与显示路径。
//!
//! 下载循环本身要真实会话，属于 `live_tests` 的范围。

use super::*;

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
fn display_path_keeps_folder_hierarchy() {
    let nested = Job {
        id: "1".into(),
        sub_dir: vec!["剧集".into(), "第一季".into()],
        name: "第 1 集.mkv".into(),
        size: 0,
        audit_flagged: false,
    };
    assert_eq!(display_path(&nested), "剧集/第一季/第 1 集.mkv");
    let single = Job {
        sub_dir: Vec::new(),
        ..nested
    };
    assert_eq!(display_path(&single), "第 1 集.mkv");
}
