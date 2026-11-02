//! `transfers_rate.rs` 的单元测试：速度窗口、卡住边界、ETA 分档与说明存活。
//!
//! 全部注入时间（`Instant + Duration`），不 sleep；记录用 `row_from` 建，
//! 时间字段直接调 `*_at` 版本，因此不碰进程级记录表。

use std::path::PathBuf;
use std::time::{Duration, Instant};

use cxx_qt_lib::QString;
use kichi_core::EntryKind;

use super::*;
use crate::cxxqt_object::transfers::{
    begin_record, note_record, progress_record, row_from, TransferDirection, TransferJob,
    UploadJob, UploadProgressState,
};
use crate::cxxqt_object::transfers_view::{live_map_at, QVariantMap};

fn job(name: &str, size: u64, kind: EntryKind) -> TransferJob {
    TransferJob {
        id: format!("id-{name}"),
        name: name.to_owned(),
        kind,
        size,
        audit_flagged: false,
        sub_dir: Vec::new(),
        dir: PathBuf::from("/tmp"),
        direction: TransferDirection::Download,
        upload: None,
    }
}

fn file_record(name: &str, size: u64) -> Transfer {
    row_from(
        1,
        0,
        "source",
        &job(name, size, EntryKind::File),
        1_790_000_000,
        1,
    )
}

/// 一个文件夹批次：父行 + 子行。
fn folder_records(children: &[(&str, u64)]) -> Vec<Transfer> {
    let mut records = vec![row_from(
        1,
        0,
        "source",
        &job("剧集", 0, EntryKind::Folder),
        1_790_000_000,
        1,
    )];
    for (index, (name, size)) in children.iter().enumerate() {
        let mut row = row_from(
            1,
            index,
            "source",
            &job(name, *size, EntryKind::File),
            1_790_000_000,
            1,
        );
        row.child = true;
        records.push(row);
    }
    records
}

/// 让一条记录进入下载中，卡住时钟从 `at` 开始走。
fn start(record: &mut Transfer, at: Instant) {
    begin_record(record, at);
}

/// 推一次进度（同时喂速度窗口与说明状态）。
fn feed(record: &mut Transfer, at: Instant, bytes: u64) {
    let total = record.total;
    progress_record(record, at, bytes, total);
}

/// 每秒 `mb` MB 匀速：从 `start` 起把 5 秒窗口填满。
fn steady(record: &mut Transfer, start: Instant, mb: u64) {
    for step in 0..=5u64 {
        feed(record, start + Duration::from_secs(step), step * mb);
    }
}

fn set_child_progress(records: &mut [Transfer], index: usize, at: Instant, bytes: u64) {
    let record = records
        .iter_mut()
        .find(|record| record.child && record.index == index)
        .expect("子行存在");
    let total = record.total;
    progress_record(record, at, bytes, total);
}

fn text_of(map: &QVariantMap, key: &str) -> String {
    map.get(&QString::from(key))
        .and_then(|value| value.value::<QString>())
        .map(|value| value.to_string())
        .unwrap_or_default()
}

// —— 速度窗口 ——

#[test]
fn speed_uses_the_five_second_window() {
    let base = Instant::now();
    let mut meter = SpeedMeter::default();
    meter.begin(base);
    // 5 秒里从 0 长到 5 MB = 1 MB/s
    for step in 0..=5u64 {
        meter.observe(base + Duration::from_secs(step), step * 1024 * 1024);
    }
    assert_eq!(
        meter.bytes_per_sec(base + Duration::from_secs(5)),
        Some(1024 * 1024)
    );

    // 又过了 6 秒没有新采样：窗口过期，速度跟着消失
    assert_eq!(meter.bytes_per_sec(base + Duration::from_secs(11)), None);
}

#[test]
fn first_sample_does_not_report_speed() {
    let base = Instant::now();
    let mut meter = SpeedMeter::default();
    meter.begin(base);
    meter.observe(base, 45 * 1024 * 1024);
    assert_eq!(
        meter.bytes_per_sec(base),
        None,
        "续传起点不能当速度（否则会报几十 MB/s 的假数字）"
    );

    // 200ms 就拿到 1 MB：跨度不足 250ms，先不给速度
    meter.observe(base + Duration::from_millis(200), 46 * 1024 * 1024);
    assert_eq!(meter.bytes_per_sec(base + Duration::from_millis(200)), None);

    // 满 250ms 才出速度：0.3 秒里长了 1.5 MB → 5 MB/s
    meter.observe(
        base + Duration::from_millis(300),
        46 * 1024 * 1024 + 512 * 1024,
    );
    assert_eq!(
        meter.bytes_per_sec(base + Duration::from_millis(300)),
        Some(5 * 1024 * 1024)
    );
}

#[test]
fn same_byte_callbacks_do_not_refresh_the_stall_clock() {
    let base = Instant::now();
    let mut meter = SpeedMeter::default();
    meter.begin(base);
    meter.observe(base, 0);
    meter.observe(base + Duration::from_secs(1), 1024);
    // 每 250ms 都来一次「1 KB 没变」：不是新字节，不能续命
    for step in 2..=8u64 {
        meter.observe(base + Duration::from_millis(step * 250), 1024);
    }
    let stalled = meter
        .stalled_for(base + Duration::from_secs(8))
        .expect("begin 已经落下卡住时钟");
    assert_eq!(
        stalled,
        Duration::from_secs(7),
        "卡住时钟停在最后一次字节增加"
    );
}

#[test]
fn bytes_going_backwards_restarts_the_window() {
    let base = Instant::now();
    let mut meter = SpeedMeter::default();
    meter.begin(base);
    for step in 0..=4u64 {
        meter.observe(base + Duration::from_secs(step), step * 1024 * 1024);
    }
    assert!(meter.bytes_per_sec(base + Duration::from_secs(4)).is_some());

    // 续传被截断 / 服务器忽略 Range：从 1 MB 重来
    meter.observe(base + Duration::from_secs(5), 1024 * 1024);
    assert_eq!(
        meter.bytes_per_sec(base + Duration::from_secs(5)),
        None,
        "回退后旧窗口作废，重新采样"
    );
    assert_eq!(
        meter.stalled_for(base + Duration::from_secs(5)),
        Some(Duration::ZERO),
        "回退是一次真实的下载事件，卡住时钟跟着重来"
    );
    meter.observe(base + Duration::from_secs(6), 2 * 1024 * 1024);
    assert_eq!(
        meter.bytes_per_sec(base + Duration::from_secs(6)),
        Some(1024 * 1024),
        "新窗口从回退后的字节重新算"
    );
}

// —— 卡住 / 速度 / ETA 文案 ——

#[test]
fn stall_boundary_is_five_seconds() {
    let base = Instant::now();
    let mut record = file_record("a.mkv", 100 * 1024 * 1024);
    start(&mut record, base);
    feed(&mut record, base, 10 * 1024 * 1024);

    let before = file_rate_info(&record, base + Duration::from_millis(4_900)).rate;
    assert!(
        !before.contains("已卡住"),
        "4.9 秒还不算卡住，实际：{before}"
    );
    let after = file_rate_info(&record, base + Duration::from_secs(5)).rate;
    assert_eq!(after, "已卡住 5 秒", "5.0 秒起报卡住");
}

#[test]
fn running_row_shows_speed_and_eta() {
    let base = Instant::now();
    // 100 MB 的文件，5 秒里下了 5 MB → 剩 95 MB / 1 MB/s = 95 秒
    let mut record = file_record("a.mkv", 100 * 1024 * 1024);
    start(&mut record, base);
    steady(&mut record, base, 1024 * 1024);

    let info = file_rate_info(&record, base + Duration::from_secs(5));
    assert_eq!(info.rate, "1.0 MB/s · 剩 1 分 35 秒");
    assert_eq!(info.note, "");
}

#[test]
fn unknown_size_keeps_speed_and_hides_eta() {
    let base = Instant::now();
    let mut record = file_record("a.mkv", 0);
    record.total = None;
    start(&mut record, base);
    steady(&mut record, base, 1024 * 1024);

    let info = file_rate_info(&record, base + Duration::from_secs(5));
    assert_eq!(info.rate, "1.0 MB/s", "大小未知只给速度");
}

#[test]
fn idle_rows_show_nothing() {
    let base = Instant::now();
    for state in [
        TransferState::Queued,
        TransferState::Preparing,
        TransferState::Done,
        TransferState::Failed,
        TransferState::Cancelled,
    ] {
        let mut record = file_record("a.mkv", 100);
        record.state = state;
        record.detail = "连接中断，3 秒后从 45 MB 继续（1/3）".to_owned();
        let info = file_rate_info(&record, base);
        assert_eq!(info.rate, "", "{state:?} 不显示速度 / 卡住");
        assert_eq!(info.note, "", "{state:?} 的说明仍走快照 detail");
    }
}

#[test]
fn eta_buckets() {
    assert_eq!(duration_text(1), "1 秒");
    assert_eq!(duration_text(59), "59 秒");
    assert_eq!(duration_text(60), "1 分");
    assert_eq!(duration_text(90), "1 分 30 秒");
    assert_eq!(duration_text(3599), "59 分 59 秒");
    assert_eq!(duration_text(3600), "1 小时");
    assert_eq!(duration_text(3660), "1 小时 1 分");
    assert_eq!(duration_text(18000), "5 小时");
    assert_eq!(duration_text(18001), "5 小时以上");
    assert_eq!(eta_text(0, 1024), None, "已经下满就没有剩余时间");
    assert_eq!(eta_text(1024, 0), None, "速度为 0 不编造 ETA");
}

#[test]
fn stall_wins_over_speed() {
    let base = Instant::now();
    let mut record = file_record("a.mkv", 100 * 1024 * 1024);
    start(&mut record, base);
    steady(&mut record, base, 1024 * 1024);
    let info = file_rate_info(&record, base + Duration::from_secs(9));
    assert!(
        info.rate.contains("MB/s") && !info.rate.contains("已卡住"),
        "4 秒还在报速度，实际：{}",
        info.rate
    );
    let info = file_rate_info(&record, base + Duration::from_secs(12));
    assert_eq!(info.rate, "已卡住 7 秒", "5 秒后只报卡住");
}

// —— 整批 ——

#[test]
fn batch_remaining_skips_failed_and_cancelled() {
    let base = Instant::now();
    let mut records = folder_records(&[("a.mkv", 10 * 1024 * 1024), ("b.mkv", 20 * 1024 * 1024)]);
    records[0].state = TransferState::Running;
    records[1].state = TransferState::Done;
    records[1].bytes = 10 * 1024 * 1024;
    begin_record(&mut records[2], base);
    // c 失败、d 取消：残量都不再算进整批
    let mut failed = row_from(
        1,
        2,
        "source",
        &job("c.mkv", 30 * 1024 * 1024, EntryKind::File),
        1_790_000_000,
        1,
    );
    failed.child = true;
    failed.state = TransferState::Failed;
    records.push(failed);
    let mut cancelled = row_from(
        1,
        3,
        "source",
        &job("d.mkv", 50 * 1024 * 1024, EntryKind::File),
        1_790_000_000,
        1,
    );
    cancelled.child = true;
    cancelled.state = TransferState::Cancelled;
    cancelled.bytes = 40 * 1024 * 1024;
    records.push(cancelled);

    // 5 秒里 b 下了 5 MB → 1 MB/s；整批剩余 = 20 MB - 5 MB = 15 MB
    for step in 0..=5u64 {
        set_child_progress(
            &mut records,
            1,
            base + Duration::from_secs(step),
            step * 1024 * 1024,
        );
    }
    let info = batch_rate_info(&records, 1, base + Duration::from_secs(5));
    assert_eq!(info.rate, "1.0 MB/s · 剩 15 秒");
}

#[test]
fn batch_unknown_size_hides_eta_but_a_failed_row_does_not() {
    let base = Instant::now();
    let mut records = folder_records(&[
        ("done.mkv", 10 * 1024 * 1024),
        ("current.mkv", 10 * 1024 * 1024),
        ("unknown.mkv", 0),
    ]);
    records[1].state = TransferState::Done;
    records[1].bytes = 10 * 1024 * 1024;
    begin_record(&mut records[2], base);
    // 还没轮到、大小未知：整批不给 ETA
    records[3].total = None;
    // 失败行的大小未知不该阻塞 ETA
    let mut failed = row_from(
        1,
        3,
        "source",
        &job("failed.mkv", 0, EntryKind::File),
        1_790_000_000,
        1,
    );
    failed.child = true;
    failed.state = TransferState::Failed;
    failed.total = None;
    records.push(failed);
    for step in 0..=5u64 {
        set_child_progress(
            &mut records,
            1,
            base + Duration::from_secs(step),
            step * 1024 * 1024,
        );
    }
    let info = batch_rate_info(&records, 1, base + Duration::from_secs(5));
    assert_eq!(
        info.rate, "1.0 MB/s",
        "有未下载文件大小未知就不给 ETA（失败行不算）"
    );

    // 未知大小的那条改成失败：剩下都已知，ETA 回来
    records[3].state = TransferState::Failed;
    let info = batch_rate_info(&records, 1, base + Duration::from_secs(5));
    assert_eq!(
        info.rate, "1.0 MB/s · 剩 5 秒",
        "失败行不占剩余，实际：{}",
        info.rate
    );
}

#[test]
fn batch_speed_comes_from_the_running_child() {
    let base = Instant::now();
    let mut records = folder_records(&[
        ("done.mkv", 10 * 1024 * 1024),
        ("current.mkv", 10 * 1024 * 1024),
        ("later.mkv", 10 * 1024 * 1024),
    ]);
    records[1].state = TransferState::Done;
    records[1].bytes = 10 * 1024 * 1024;
    begin_record(&mut records[2], base);
    for step in 0..=5u64 {
        set_child_progress(
            &mut records,
            1,
            base + Duration::from_secs(step),
            step * 512 * 1024,
        );
    }
    // 当前文件 512 KB/s；整批剩余 = 10 MB - 2.5 MB + 10 MB = 17.5 MB → 35 秒
    let info = batch_rate_info(&records, 1, base + Duration::from_secs(5));
    assert_eq!(info.rate, "512 KB/s · 剩 35 秒");
}

#[test]
fn batch_without_running_child_reports_nothing() {
    let base = Instant::now();
    let mut records = folder_records(&[("a.mkv", 10)]);
    records[1].state = TransferState::Done;
    assert_eq!(batch_rate_info(&records, 1, base).rate, "");
}

#[test]
fn upload_rate_is_hidden_outside_the_actual_transfer_stage() {
    let base = Instant::now();
    let mut record = file_record("电影.mkv", 10 * 1024 * 1024);
    record.job.direction = TransferDirection::Upload;
    record.job.upload = Some(UploadJob {
        local_path: PathBuf::from("/home/me/电影.mkv"),
        parent_id: "cloud-folder".to_owned(),
        parent_name: "动画".to_owned(),
    });
    start(&mut record, base);
    record.upload_progress = Some(UploadProgressState {
        stage: kichi_core::UploadStage::Hashing,
        bytes: 0,
        total: 10 * 1024 * 1024,
    });
    for step in 0..=5u64 {
        feed(
            &mut record,
            base + Duration::from_secs(step),
            step * 1024 * 1024,
        );
    }
    assert_eq!(
        file_rate_info(&record, base + Duration::from_secs(5)),
        RateInfo::default(),
        "计算校验阶段不把本地读取速度冒充上传速度"
    );

    record.upload_progress = Some(UploadProgressState {
        stage: kichi_core::UploadStage::Uploading,
        bytes: 5 * 1024 * 1024,
        total: 10 * 1024 * 1024,
    });
    assert_eq!(
        file_rate_info(&record, base + Duration::from_secs(5)).rate,
        "1.0 MB/s · 剩 5 秒"
    );
}

#[test]
fn batch_note_comes_from_the_running_child() {
    let base = Instant::now();
    let mut records = folder_records(&[("a.mkv", 10 * 1024 * 1024)]);
    begin_record(&mut records[1], base);
    note_record(
        &mut records[1],
        base,
        "连接中断，3 秒后从 45 MB 继续（1/3）",
    );
    let info = batch_rate_info(&records, 1, base + Duration::from_secs(1));
    assert_eq!(info.note, "连接中断，3 秒后从 45 MB 继续（1/3）");
}

// —— 说明存活 ——

#[test]
fn note_stays_until_resume_and_fades_after_five_seconds() {
    let base = Instant::now();
    let mut note = NoteState::default();
    assert!(!note.visible(base));
    note.write(base, 45 * 1024 * 1024);
    assert!(
        note.visible(base + Duration::from_secs(60)),
        "未恢复前一直在"
    );

    note.observe(base + Duration::from_secs(60), 45 * 1024 * 1024);
    assert!(
        note.visible(base + Duration::from_secs(300)),
        "没动字节不算恢复"
    );

    note.observe(base + Duration::from_secs(61), 45 * 1024 * 1024 + 1024);
    assert!(
        note.visible(base + Duration::from_secs(65)),
        "恢复后 4 秒还在"
    );
    assert!(
        !note.visible(base + Duration::from_secs(66)),
        "恢复后 5 秒消失"
    );
}

#[test]
fn new_note_resets_the_timer() {
    let base = Instant::now();
    let mut note = NoteState::default();
    note.write(base, 0);
    note.observe(base + Duration::from_secs(1), 1024);
    assert!(note.visible(base + Duration::from_secs(4)));
    // 又是一次断流：新说明重新计时，上次的「恢复」作废
    note.write(base + Duration::from_secs(4), 1024);
    assert!(note.visible(base + Duration::from_secs(60)));
}

#[test]
fn note_appears_in_the_file_live_map() {
    let base = Instant::now();
    let mut record = file_record("a.mkv", 10 * 1024 * 1024);
    start(&mut record, base);
    note_record(&mut record, base, "连接中断，3 秒后从 4 MB 继续（1/3）");
    let live = live_map_at(&record, base + Duration::from_secs(1));
    assert_eq!(
        text_of(&live, "noteText"),
        "连接中断，3 秒后从 4 MB 继续（1/3）"
    );
    assert_eq!(text_of(&live, "rateText"), "", "还没采样，速度列空着");
}

#[test]
fn heartbeat_boundary() {
    let base = Instant::now();
    assert!(heartbeat_due(None, base), "从没推过就该推");
    assert!(!heartbeat_due(
        Some(base),
        base + Duration::from_millis(999)
    ));
    assert!(heartbeat_due(Some(base), base + LIVE_HEARTBEAT));
}
