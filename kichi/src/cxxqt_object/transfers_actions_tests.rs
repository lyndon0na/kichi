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
        direction: TransferDirection::Download,
        upload: None,
    }
}

fn record(id: &str, kind: EntryKind, sub_dir: &[&str], state: TransferState) -> Transfer {
    let seed = job(id, "第 1 集.mkv", kind, sub_dir);
    let mut record = row_from(7, 0, "folder-id", &seed, 1_790_000_000, 1);
    record.state = state;
    record
}

fn upload_record(state: TransferState) -> Transfer {
    let mut seed = job("upload:/home/me/电影.mkv", "电影.mkv", EntryKind::File, &[]);
    seed.direction = TransferDirection::Upload;
    seed.upload = Some(crate::cxxqt_object::transfers::UploadJob {
        local_path: PathBuf::from("/home/me/电影.mkv"),
        parent_id: "cloud-folder".to_owned(),
        parent_name: "动画".to_owned(),
        ..Default::default()
    });
    let mut row = row_from(7, 0, &seed.id, &seed, 1_790_000_000, 1);
    row.state = state;
    row
}

/// 文件夹批次：父行 + 若干子行（index 0..）。
fn folder_batch(children: &[(&str, TransferState)]) -> Vec<Transfer> {
    let seed = job("folder-1", "剧集", EntryKind::Folder, &[]);
    let mut records = vec![row_from(7, 0, "folder-id", &seed, 1_790_000_000, 1)];
    for (index, (name, state)) in children.iter().enumerate() {
        let seed = job(name, name, EntryKind::File, &["剧集"]);
        let mut child = row_from(7, index, "folder-id", &seed, 1_790_000_000, 1);
        child.child = true;
        child.state = *state;
        records.push(child);
    }
    records
}

fn upload_folder_batch(
    batch: u64,
    children: &[(&str, TransferState)],
    failed_dirs: &[&[&str]],
) -> Vec<Transfer> {
    let parent_job = TransferJob {
        id: format!("upload-folder:/home/me/剧集-{batch}"),
        name: "剧集".to_owned(),
        kind: EntryKind::Folder,
        size: 0,
        audit_flagged: false,
        sub_dir: Vec::new(),
        dir: PathBuf::new(),
        direction: TransferDirection::Upload,
        upload: Some(crate::cxxqt_object::transfers::UploadJob {
            local_path: PathBuf::from("/home/me/剧集"),
            parent_id: "cloud-folder".to_owned(),
            parent_name: "动画".to_owned(),
            batch_root_id: "cloud-root".to_owned(),
            ..Default::default()
        }),
    };
    let mut parent = row_from(batch, 0, &parent_job.id, &parent_job, 1_790_000_000, 1);
    parent.state = TransferState::Failed;
    parent.upload_dirs = vec![
        crate::cxxqt_object::transfers::UploadDir {
            path: vec!["剧集".to_owned()],
            id: "cloud-root".to_owned(),
        },
        crate::cxxqt_object::transfers::UploadDir {
            path: vec!["剧集".to_owned(), "第一季".to_owned()],
            id: "cloud-season".to_owned(),
        },
    ];
    parent.upload_failed_dirs = failed_dirs
        .iter()
        .map(|path| path.iter().map(|part| (*part).to_owned()).collect())
        .collect();
    let mut records = vec![parent];
    for (index, (name, state)) in children.iter().enumerate() {
        let child_job = TransferJob {
            id: format!("upload:/home/me/剧集/{name}"),
            name: (*name).to_owned(),
            kind: EntryKind::File,
            size: 1234,
            audit_flagged: false,
            sub_dir: vec!["剧集".to_owned()],
            dir: PathBuf::new(),
            direction: TransferDirection::Upload,
            upload: Some(crate::cxxqt_object::transfers::UploadJob {
                local_path: PathBuf::from(format!("/home/me/剧集/{name}")),
                parent_id: "cloud-folder".to_owned(),
                parent_name: "动画".to_owned(),
                batch_root_id: "cloud-root".to_owned(),
                cloud_parent_id: "cloud-root".to_owned(),
            }),
        };
        let mut child = row_from(batch, index, &parent_job.id, &child_job, 1_790_000_000, 1);
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
        "部分失败 / 全部失败都在内联子行里逐个重试"
    );
    assert!(
        retry_target_in(&records, "7:1").is_some(),
        "失败的子文件自己仍然可以重试"
    );
}

#[test]
fn only_failed_or_interrupted_records_can_be_retried() {
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
            "只有 failed / interrupted 能重试，{state:?} 不行"
        );
    }
    for state in [TransferState::Failed, TransferState::Interrupted] {
        let record = record("file-1", EntryKind::File, &[], state);
        assert!(
            retry_target_in(&[record], "7:0").is_some(),
            "{state:?} 可以重试"
        );
    }
    assert!(
        retry_target_in(&[], "7:0").is_none(),
        "不存在的 key 返回 None"
    );
}

#[test]
fn failed_keys_retry_children_but_not_a_partially_failed_folder_as_a_whole() {
    let mut records = folder_batch(&[
        ("done.mkv", TransferState::Done),
        ("failed.mkv", TransferState::Failed),
        ("cancelled.mkv", TransferState::Cancelled),
    ]);
    records[0].state = TransferState::Failed;

    let mut standalone = record(
        "standalone-file",
        EntryKind::File,
        &[],
        TransferState::Failed,
    );
    standalone.batch = 8;
    standalone.index = 0;
    records.push(standalone);

    let mut interrupted = record(
        "interrupted-file",
        EntryKind::File,
        &[],
        TransferState::Interrupted,
    );
    interrupted.batch = 9;
    interrupted.index = 0;
    records.push(interrupted);

    assert_eq!(
        retryable_failed_keys_in(&records),
        vec!["7:1".to_owned(), "8:0".to_owned()],
        "失败父行不整批重来；只重试失败子文件和独立失败文件"
    );
}

#[test]
fn failed_keys_include_a_folder_that_failed_before_reading_children() {
    let mut records = folder_batch(&[]);
    records[0].state = TransferState::Failed;

    assert_eq!(
        retryable_failed_keys_in(&records),
        vec!["7:folder".to_owned()],
        "没有子行时文件夹自身重新读目录"
    );
}

#[test]
fn failed_keys_use_a_upload_folder_parent_when_a_directory_failed() {
    let records = upload_folder_batch(
        7,
        &[
            ("done.mkv", TransferState::Done),
            ("failed.mkv", TransferState::Failed),
        ],
        &[&["剧集", "空目录"]],
    );

    assert_eq!(
        retryable_failed_keys_in(&records),
        vec!["7:folder".to_owned()],
        "目录失败由父行一次重试，避免重复登记同一批子文件"
    );
}

#[test]
fn upload_folder_batch_retry_keeps_dirs_and_selects_unfinished_children() {
    let batch = 9_700_001u64;
    let records = upload_folder_batch(
        batch,
        &[
            ("done.mkv", TransferState::Done),
            ("failed.mkv", TransferState::Failed),
            ("cancelled.mkv", TransferState::Cancelled),
        ],
        &[&["剧集", "空目录"]],
    );
    with_records(|global| global.extend(records));
    with_trees(|trees| {
        trees.insert(
            batch,
            vec![
                vec!["剧集".to_owned()],
                vec!["剧集".to_owned(), "第一季".to_owned()],
                vec!["剧集".to_owned(), "空目录".to_owned()],
            ],
        );
    });

    let retry = upload_folder_batch_retry(&format!("{batch}:folder")).expect("父批次可重试");
    assert_eq!(retry.batch, batch);
    assert_eq!(retry.files.len(), 1, "只选中失败子文件");
    assert_eq!(retry.files[0].0, 1);
    assert_eq!(retry.upload_dirs.len(), 2, "已创建的云端目录保留");
    assert!(!retry.dirs.is_empty(), "目录树保留，空目录也可以补建");

    with_records(|global| global.retain(|record| record.batch != batch));
    with_trees(|trees| {
        trees.remove(&batch);
    });
}

#[test]
fn cancel_target_distinguishes_parent_and_child_rows() {
    let mut records = folder_batch(&[
        ("a.mkv", TransferState::Running),
        ("b.mkv", TransferState::Queued),
    ]);
    records[0].state = TransferState::Running;

    let parent = cancel_target_in(&records, "7:folder").expect("活动文件夹父行可取消");
    assert!(parent.whole_batch, "父行取消整批");
    assert_eq!(parent.batch, 7);

    let child = cancel_target_in(&records, "7:0").expect("活动子行可取消");
    assert!(!child.whole_batch, "子行只取消自己");
    assert_eq!((child.batch, child.index), (7, 0));

    records[1].state = TransferState::Done;
    assert!(
        cancel_target_in(&records, "7:0").is_none(),
        "已结束子行不能再取消"
    );
}

#[test]
fn cancel_target_accepts_active_single_files_only() {
    let active = record("file-1", EntryKind::File, &[], TransferState::Running);
    let target = cancel_target_in(&[active], "7:0").expect("下载中的文件可取消");
    assert!(target.whole_batch, "顶层文件取消整批");

    let done = record("file-1", EntryKind::File, &[], TransferState::Done);
    assert!(cancel_target_in(&[done], "7:0").is_none());
}

#[test]
fn upload_actions_keep_direction_cancel_and_retry_but_have_no_local_path() {
    let running = upload_record(TransferState::Running);
    let target = cancel_target_in(std::slice::from_ref(&running), "7:0").expect("上传可取消");
    assert_eq!(target.direction, TransferDirection::Upload);
    assert!(target.whole_batch);

    let mut failed = upload_record(TransferState::Failed);
    failed.path = Some(PathBuf::from("/tmp/should-not-be-used"));
    let retry = retry_target_in(std::slice::from_ref(&failed), "7:0").expect("失败上传可重试");
    assert!(retry.is_upload());
    assert_eq!(
        retry
            .upload
            .as_ref()
            .map(|upload| upload.parent_id.as_str()),
        Some("cloud-folder")
    );
    assert!(
        path_for_in(std::slice::from_ref(&failed), "7:0").is_none(),
        "上传记录没有可打开的本地落点"
    );
}

#[test]
fn paused_child_is_cancelled_through_its_parent_only() {
    let mut records = folder_batch(&[
        ("a.mkv", TransferState::Paused),
        ("b.mkv", TransferState::Done),
    ]);
    records[0].state = TransferState::Paused;
    let parent = cancel_target_in(&records, "7:folder").expect("暂停批次父行可取消");
    assert!(parent.whole_batch);
    assert_eq!(parent.state, TransferState::Paused);
    assert!(
        cancel_target_in(&records, "7:0").is_none(),
        "暂停中的子文件不给单独取消入口"
    );
}

#[test]
fn resume_target_reuses_only_unfinished_children_and_their_indexes() {
    let mut records = folder_batch(&[
        ("a.mkv", TransferState::Paused),
        ("b.mkv", TransferState::Done),
        ("c.mkv", TransferState::Paused),
    ]);
    records[0].state = TransferState::Paused;
    let target = resume_batch_in(
        &records,
        |record| record_key(record) == "7:folder",
        |record| record.state == TransferState::Paused,
    )
    .expect("暂停文件夹可继续");
    assert_eq!(target.batch, 7);
    assert_eq!(target.source_id, "folder-id");
    assert_eq!(target.entry.kind, EntryKind::Folder);
    let files = target.files.expect("已有子行时直接复用");
    assert_eq!(files.len(), 2, "完成的行不重下");
    assert_eq!(files[0].index, 0);
    assert_eq!(files[1].index, 2);
}

#[test]
fn resume_target_for_a_paused_file_keeps_its_download_dir() {
    let mut row = record("file-1", EntryKind::File, &["剧集"], TransferState::Paused);
    row.job.dir = PathBuf::from("/tmp/kichi-resume");
    let target = resume_batch_in(
        &[row],
        |record| record_key(record) == "7:0",
        |record| record.state == TransferState::Paused,
    )
    .expect("暂停文件可继续");
    assert_eq!(target.dir, PathBuf::from("/tmp/kichi-resume"));
    assert_eq!(target.files.expect("单文件直接复用").len(), 1);
}

#[test]
fn whole_folder_retry_is_only_for_ledgers_without_children() {
    let mut whole = folder_batch(&[]);
    whole[0].state = TransferState::Interrupted;
    assert!(
        super::super::transfers_view::is_whole_folder_ledger(&whole, "7:folder"),
        "已中断、没有子行的文件夹整批重来"
    );

    let mut partial = folder_batch(&[("a.mkv", TransferState::Interrupted)]);
    partial[0].state = TransferState::Interrupted;
    assert!(
        !super::super::transfers_view::is_whole_folder_ledger(&partial, "7:folder"),
        "有子行的文件夹不整批重来"
    );

    let mut done = folder_batch(&[]);
    done[0].state = TransferState::Done;
    assert!(
        !super::super::transfers_view::is_whole_folder_ledger(&done, "7:folder"),
        "完成的文件夹不给重试"
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
fn interrupted_parent_disappears_after_its_last_child_is_retried() {
    let mut records = folder_batch(&[
        ("a.mkv", TransferState::Interrupted),
        ("b.mkv", TransferState::Interrupted),
    ]);
    records[0].state = TransferState::Interrupted;

    assert!(remove_in(&mut records, "7:0"));
    assert_eq!(records.len(), 2, "还剩父行 + 一个子行");
    assert!(remove_in(&mut records, "7:1"));
    assert!(
        records.is_empty(),
        "最后一个子行被重试后已中断的父行也一起消失"
    );
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
fn removing_a_folder_record_drops_the_whole_batch_only() {
    let mut records = folder_batch(&[
        ("a.mkv", TransferState::Done),
        ("b.mkv", TransferState::Failed),
    ]);
    records[0].state = TransferState::Failed;

    let mut other = record("other-file", EntryKind::File, &[], TransferState::Done);
    other.batch = 8;
    other.index = 0;
    records.push(other);

    assert!(remove_record_in(&mut records, "7:folder"));
    assert_eq!(records.len(), 1, "文件夹父行与全部子行一起移除");
    assert_eq!(records[0].batch, 8, "其它批次不受影响");
}

#[test]
fn remove_record_rejects_active_and_child_rows() {
    let mut records = folder_batch(&[
        ("a.mkv", TransferState::Done),
        ("b.mkv", TransferState::Failed),
    ]);
    records[0].state = TransferState::Failed;

    assert!(
        !remove_record_in(&mut records, "7:1"),
        "子行不单独从历史菜单移除"
    );

    let mut active = record("active-file", EntryKind::File, &[], TransferState::Running);
    active.batch = 8;
    active.index = 0;
    records.push(active);
    assert!(
        !remove_record_in(&mut records, "8:0"),
        "进行中的任务必须先取消，不能直接移除记录"
    );
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

// —— 空文件夹的「打开位置」（2026-10-27）——

fn folder_ledger(name: &str) -> TransferJob {
    job("folder-1", name, EntryKind::Folder, &[])
}

#[test]
fn empty_folder_done_row_has_its_directory() {
    // 真实流程里 `finish_folder_empty` 已把建出来的目录写进 path（transfers_tests 覆盖），
    // 这里只验「路径取得到、QML 的打开位置有用」
    let mut record = row_from(7, 0, "folder-1", &folder_ledger("剧集"), 1_790_000_000, 1);
    record.state = TransferState::Done;
    record.empty_folder = true;
    record.path = Some(PathBuf::from("/home/me/Downloads/剧集"));

    assert_eq!(
        path_for_in(&[record], "7:folder"),
        Some(PathBuf::from("/home/me/Downloads/剧集")),
        "空文件夹的目录已经建出来，可以打开位置"
    );
}

#[test]
fn failed_empty_batch_has_no_path() {
    let mut record = row_from(7, 0, "folder-1", &folder_ledger("剧集"), 1_790_000_000, 1);
    record.state = TransferState::Failed;
    record.detail = "所有子目录都无法读取（跳过 2 个）".into();

    assert_eq!(
        path_for_in(&[record], "7:folder"),
        None,
        "失败的批次可能压根没建目录，不给路径"
    );
}

#[test]
fn folder_path_still_comes_from_done_children_first() {
    let mut records = folder_batch(&[("a.mkv", TransferState::Failed)]);
    records[0].state = TransferState::Failed;
    assert_eq!(path_for_in(&records, "7:folder"), None);

    records[1].state = TransferState::Done;
    assert_eq!(
        path_for_in(&records, "7:folder"),
        Some(PathBuf::from("/home/me/Downloads/剧集")),
        "有子文件完成时行为不变"
    );
}

#[test]
fn empty_folder_row_is_not_retryable_as_a_whole() {
    let mut record = row_from(7, 0, "folder-1", &folder_ledger("剧集"), 1_790_000_000, 1);
    record.state = TransferState::Done;
    record.empty_folder = true;
    assert!(
        retry_target_in(&[record], "7:folder").is_none(),
        "空文件夹算成功，没有重试入口"
    );
}

#[test]
fn auto_resume_separates_downloads_and_upload_targets() {
    let mut records = folder_batch(&[
        ("a.mkv", TransferState::Interrupted),
        ("b.mkv", TransferState::Done),
    ]);
    records[0].state = TransferState::Interrupted;
    records[0].auto_resume = true;
    records[1].auto_resume = true;

    let mut single = upload_record(TransferState::Interrupted);
    single.batch = 8;
    single.auto_resume = true;
    records.push(single);

    let mut folder = upload_folder_batch(
        9,
        &[
            ("active.mkv", TransferState::Interrupted),
            ("done.mkv", TransferState::Done),
            ("old.mkv", TransferState::Interrupted),
        ],
        &[],
    );
    folder[0].state = TransferState::Interrupted;
    folder[0].auto_resume = true;
    folder[1].auto_resume = true;
    records.extend(folder);

    let targets = auto_resume_targets_in(&mut records, |batch| {
        if batch == 9 {
            vec![vec!["剧集".to_owned(), "第一季".to_owned()]]
        } else {
            Vec::new()
        }
    });

    assert_eq!(targets.downloads.len(), 1, "下载批次单独恢复");
    assert_eq!(
        targets.downloads[0].files.as_ref().map(Vec::len),
        Some(1),
        "只恢复当时在途的下载子文件"
    );
    assert_eq!(targets.uploads.len(), 2, "单文件和文件夹各一条上传目标");

    let mut saw_single = false;
    let mut saw_folder = false;
    for target in targets.uploads {
        match target {
            UploadResumeBatch::Single { batch, job, .. } => {
                saw_single = true;
                assert_eq!(batch, 8);
                assert_eq!(job.name, "电影.mkv");
            }
            UploadResumeBatch::Folder {
                batch,
                files,
                dirs,
                upload_dirs,
                ..
            } => {
                saw_folder = true;
                assert_eq!(batch, 9);
                assert_eq!(
                    files.iter().map(|(index, _)| *index).collect::<Vec<_>>(),
                    vec![0],
                    "没标过的旧中断子文件不能顺手恢复"
                );
                assert_eq!(dirs, vec![vec!["剧集".to_owned(), "第一季".to_owned()]]);
                assert!(!upload_dirs.is_empty(), "复用已创建的云端目录");
            }
        }
    }
    assert!(saw_single && saw_folder);
    assert!(
        records.iter().all(|record| !record.auto_resume),
        "消费后必须清掉全部标记"
    );
}

#[test]
fn auto_resume_skips_a_folder_that_never_got_child_rows() {
    let mut records = upload_folder_batch(11, &[], &[]);
    records[0].state = TransferState::Interrupted;
    records[0].auto_resume = true;

    let targets = auto_resume_targets_in(&mut records, |_| vec![vec!["剧集".to_owned()]]);

    assert!(targets.uploads.is_empty(), "还在扫描的文件夹没有可恢复子行");
    assert!(!records[0].auto_resume);
}
