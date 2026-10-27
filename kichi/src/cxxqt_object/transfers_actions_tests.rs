//! `transfers_actions.rs` 的单元测试：重试目标 / 删旧行 / 取落盘路径。
//!
//! 全部用本地 `Vec<Transfer>` 调 `*_in` 纯函数，不碰进程级记录表，
//! 因此不需要像 `queue_tests` 那样串行加锁。

use std::path::PathBuf;

use kichi_core::EntryKind;

use super::*;
use crate::cxxqt_object::transfers::row_from;

fn job(id: &str, name: &str, kind: EntryKind, sub_dir: &[&str]) -> TransferJob {
    TransferJob {
        id: id.to_owned(),
        name: name.to_owned(),
        kind,
        size: 1234,
        audit_flagged: false,
        sub_dir: sub_dir.iter().map(|part| (*part).to_owned()).collect(),
        dir: PathBuf::from("/home/me/Downloads"),
    }
}

fn record(id: &str, kind: EntryKind, sub_dir: &[&str], state: TransferState) -> Transfer {
    let seed = job(id, "第 1 集.mkv", kind, sub_dir);
    let mut record = row_from(7, 0, "folder-id", &seed);
    record.state = state;
    record
}

/// 文件夹批次：父行 + 若干子行（index 0..）。
fn folder_batch(children: &[(&str, TransferState)]) -> Vec<Transfer> {
    let seed = job("folder-1", "剧集", EntryKind::Folder, &[]);
    let mut records = vec![row_from(7, 0, "folder-id", &seed)];
    for (index, (name, state)) in children.iter().enumerate() {
        let seed = job(name, name, EntryKind::File, &["剧集"]);
        let mut child = row_from(7, index, "folder-id", &seed);
        child.child = true;
        child.state = *state;
        records.push(child);
    }
    records
}

#[test]
fn retry_target_keeps_folder_ledger_identity() {
    let record = record("folder-1", EntryKind::Folder, &[], TransferState::Failed);
    let found = retry_target_in(&[record], "7:folder").expect("失败的文件夹总账行可重试");
    assert_eq!(found.id, "folder-1", "重试的是文件夹本身");
    assert_eq!(found.kind, EntryKind::Folder, "走文件夹分支重新读目录");
    assert!(found.sub_dir.is_empty(), "总账行没有层级");
    assert_eq!(
        found.dir,
        PathBuf::from("/home/me/Downloads"),
        "回到原批次的目录快照"
    );
    assert_eq!(found.size, 1234, "大小原样保留");
}

#[test]
fn retry_target_keeps_file_identity_and_hierarchy() {
    let record = record(
        "file-9",
        EntryKind::File,
        &["剧集", "第一季"],
        TransferState::Failed,
    );
    let found = retry_target_in(&[record], "7:0").expect("失败的文件行可重试");
    assert_eq!(found.id, "file-9", "只重下这个文件，不重下整个文件夹");
    assert_eq!(found.kind, EntryKind::File);
    assert_eq!(found.sub_dir, vec!["剧集", "第一季"], "保留原子目录");
    assert_eq!(found.size, 1234);
}

#[test]
fn folder_with_children_never_retries_as_a_whole() {
    let mut records = folder_batch(&[
        ("a.mkv", TransferState::Done),
        ("b.mkv", TransferState::Failed),
    ]);
    records[0].state = TransferState::Failed;
    assert!(
        retry_target_in(&records, "7:folder").is_none(),
        "部分失败 / 全部失败都在详情页逐个重试"
    );
    assert!(
        retry_target_in(&records, "7:1").is_some(),
        "失败的子文件自己仍然可以重试"
    );
}

#[test]
fn only_failed_records_can_be_retried() {
    for state in [
        TransferState::Queued,
        TransferState::Preparing,
        TransferState::Running,
        TransferState::Done,
        TransferState::Cancelled,
    ] {
        let record = record("file-1", EntryKind::File, &[], state);
        assert!(
            retry_target_in(&[record], "7:0").is_none(),
            "只有 failed 能重试，{state:?} 不行"
        );
    }
    assert!(
        retry_target_in(&[], "7:0").is_none(),
        "不存在的 key 返回 None"
    );
}

#[test]
fn remove_only_drops_the_matching_row() {
    let mut records = vec![
        record("file-1", EntryKind::File, &[], TransferState::Failed),
        record("file-2", EntryKind::File, &[], TransferState::Done),
        record("file-3", EntryKind::File, &[], TransferState::Queued),
    ];
    // 三条都是 7:0，先改掉后两条的身份再测删除更贴近真实
    records[1].index = 1;
    records[2].index = 2;

    assert!(remove_in(&mut records, "7:0"));
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].job.id, "file-2", "其它行一动不动");
    assert!(!remove_in(&mut records, "7:0"), "删过就不存在了");
    assert!(!remove_in(&mut records, "9:9"), "不存在的 key 返回 false");
}

#[test]
fn removing_one_child_keeps_the_parent() {
    let mut records = folder_batch(&[
        ("a.mkv", TransferState::Done),
        ("b.mkv", TransferState::Failed),
    ]);
    records[0].state = TransferState::Failed;

    assert!(remove_in(&mut records, "7:1"));
    assert_eq!(records.len(), 2, "父行 + 剩下那个子行");
    assert_eq!(
        records[0].state,
        TransferState::Done,
        "删掉失败子行后父行按剩下的子行重新聚合"
    );
    assert!(!records[0].child);
}

#[test]
fn removing_the_last_child_drops_the_finished_parent() {
    let mut records = folder_batch(&[("a.mkv", TransferState::Failed)]);
    records[0].state = TransferState::Failed;

    assert!(remove_in(&mut records, "7:0"));
    assert!(records.is_empty(), "最后一个子行被重试后父行也一起消失");
}

#[test]
fn path_comes_only_from_finished_rows() {
    let mut done = record("file-1", EntryKind::File, &[], TransferState::Done);
    done.path = Some(PathBuf::from("/home/me/Downloads/a.mkv"));
    let failed = record("file-2", EntryKind::File, &[], TransferState::Failed);
    let mut records = vec![failed, done];
    records[1].index = 1;

    assert_eq!(
        path_for_in(&records, "7:1"),
        Some(PathBuf::from("/home/me/Downloads/a.mkv")),
        "完成行给出落盘路径"
    );
    assert_eq!(path_for_in(&records, "7:0"), None, "失败行没有路径");
    assert_eq!(path_for_in(&records, "9:0"), None, "不存在的 key 没有路径");
}

#[test]
fn folder_path_needs_at_least_one_done_child() {
    let mut records = folder_batch(&[
        ("a.mkv", TransferState::Failed),
        ("b.mkv", TransferState::Queued),
    ]);
    records[0].state = TransferState::Failed;
    assert_eq!(
        path_for_in(&records, "7:folder"),
        None,
        "没有子文件落盘时父行不可定位"
    );

    records[1].state = TransferState::Done;
    assert_eq!(
        path_for_in(&records, "7:folder"),
        Some(PathBuf::from("/home/me/Downloads/剧集")),
        "文件夹路径 = 下载目录 + 安全化后的文件夹名"
    );
    assert_eq!(
        path_for_in(&records, "7:0"),
        None,
        "失败的那个子行仍然没有路径"
    );
}
