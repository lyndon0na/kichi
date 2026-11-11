//! 「下载到…」的纯逻辑单测：分类、共享目录入队、汇总文案、选择器结果校验。
//!
//! 不弹真实 Portal、不起下载线程：这些属于实机验证。

use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};

use kichi_core::{EntryKind, FileEntry};

use super::*;

fn entry(id: &str) -> FileEntry {
    FileEntry {
        id: id.to_string(),
        name: format!("{id}.mkv"),
        kind: EntryKind::File,
        size: 0,
        created_at: None,
        modified_at: None,
        parent_id: String::new(),
        md5: None,
        audit_flagged: false,
        starred: false,
        media: false,
        video: false,
        thumbnail_url: String::new(),
    }
}

fn find_known(id: &str) -> Option<FileEntry> {
    matches!(id, "a" | "b" | "c").then(|| entry(id))
}

#[test]
fn pending_entries_are_skipped_and_counted() {
    let targets = classify_targets(
        ["a", "b", "c"].map(String::from),
        find_known,
        |id| id == "b",
        |_| false,
    );
    let ids: Vec<&str> = targets
        .entries
        .iter()
        .map(|entry| entry.id.as_str())
        .collect();
    assert_eq!(ids, vec!["a", "c"], "已在队列的条目不再入队");
    assert_eq!(targets.already, 1);
    assert_eq!(targets.missing, 0);
    assert_eq!(targets.syncing, 0);
}

#[test]
fn missing_and_syncing_entries_are_counted_separately() {
    let targets = classify_targets(
        ["a", "gone", "temp"].map(String::from),
        |id| (id != "gone").then(|| entry(id)),
        |_| false,
        |id| id == "temp",
    );
    let ids: Vec<&str> = targets
        .entries
        .iter()
        .map(|entry| entry.id.as_str())
        .collect();
    assert_eq!(ids, vec!["a"]);
    assert_eq!(targets.missing, 1);
    assert_eq!(targets.syncing, 1);
}

#[test]
fn classification_keeps_the_entry_snapshot() {
    let targets = classify_targets(["a"].map(String::from), find_known, |_| false, |_| false);
    assert_eq!(targets.entries.len(), 1);
    assert_eq!(
        targets.entries[0].name, "a.mkv",
        "选择器开着时翻目录也不丢条目"
    );
}

#[test]
fn every_entry_uses_the_same_picked_directory() {
    let dir = PathBuf::from("/tmp/kichi-picked");
    let mut seen = Vec::new();
    let outcome = enqueue_targets(
        PickedTargets {
            entries: vec![entry("a"), entry("b")],
            ..PickedTargets::default()
        },
        &dir,
        |_, used| {
            seen.push(used.to_path_buf());
            true
        },
    );
    assert_eq!(outcome.added, 2, "多选每个条目各自一批");
    assert_eq!(seen, vec![dir.clone(), dir], "整个选择共用同一个目录快照");
}

#[test]
fn a_race_into_the_queue_counts_as_already() {
    let outcome = enqueue_targets(
        PickedTargets {
            entries: vec![entry("a"), entry("b")],
            ..PickedTargets::default()
        },
        Path::new("/tmp/kichi-picked"),
        |entry, _| entry.id == "a",
    );
    assert_eq!(outcome.added, 1);
    assert_eq!(outcome.already, 1, "选择器期间被别的入口排上，不重复入队");
}

#[test]
fn summary_reports_added_entries() {
    assert_eq!(summary_message(3, 0, 0, 0), "已加入下载队列：3 项");
    assert_eq!(
        summary_message(2, 1, 0, 0),
        "已加入下载队列：2 项（1 项已在队列中）"
    );
}

#[test]
fn summary_explains_skipped_pending_entries() {
    let text = summary_message(0, 2, 0, 0);
    assert!(text.contains("已在下载队列中"), "{text}");
    let text = summary_message(1, 2, 1, 1);
    assert!(text.contains("2 项已在队列中"), "{text}");
    assert!(text.contains("1 项已不在当前列表中"), "{text}");
    assert!(text.contains("正在同步真实 id"), "{text}");
}

#[test]
fn summary_handles_nothing_added_and_nothing_pending() {
    let text = summary_message(0, 0, 2, 0);
    assert!(text.contains("2 项已不在当前列表中"), "{text}");
    assert!(summary_message(0, 0, 0, 0).is_empty());
}

#[test]
fn cancelled_picker_does_nothing() {
    assert_eq!(resolve_picked_dir(Ok(None)).expect("取消不是错误"), None);
}

#[test]
fn existing_directory_is_accepted() {
    let dir = std::env::temp_dir();
    assert_eq!(
        resolve_picked_dir(Ok(Some(dir.clone()))).expect("目录有效"),
        Some(dir)
    );
}

#[test]
fn missing_directory_is_rejected() {
    let missing = std::env::temp_dir().join(format!(
        "kichi-pick-dir-missing-{}-never-created",
        std::process::id()
    ));
    let err = resolve_picked_dir(Ok(Some(missing))).expect_err("不存在的目录不能用");
    assert!(err.to_string().contains("无法访问"), "{err}");
}

#[test]
fn a_file_is_not_a_download_directory() {
    let file = std::env::temp_dir().join(format!("kichi-pick-dir-{}-file", std::process::id()));
    std::fs::write(&file, b"x").expect("创建临时文件");
    let result = resolve_picked_dir(Ok(Some(file.clone())));
    let _ = std::fs::remove_file(&file);
    let err = result.expect_err("文件不能当下载目录");
    assert!(err.to_string().contains("不是文件夹"), "{err}");
}

#[test]
fn non_utf8_directory_is_rejected() {
    let path = PathBuf::from(OsString::from_vec(vec![b'/', b't', b'm', b'p', b'/', 0xff]));
    let err = resolve_picked_dir(Ok(Some(path))).expect_err("非 UTF-8 路径不能用");
    assert!(err.to_string().contains("UTF-8"), "{err}");
}

#[test]
fn picker_error_becomes_a_status_message() {
    let err = resolve_picked_dir(Err("系统未提供目录选择器".to_string())).expect_err("原样报错");
    assert_eq!(err.to_string(), "系统未提供目录选择器");
}
