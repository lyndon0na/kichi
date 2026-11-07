use super::*;

use std::collections::HashMap;

use chrono::{TimeZone, Utc};
use cxx_qt_lib::{QMap, QMapPair_QString_QVariant, QString};
use kichi_core::{EntryKind, OfflineTask, TaskPhase};

type QVariantMap = QMap<QMapPair_QString_QVariant>;

fn task(id: &str, phase: TaskPhase) -> OfflineTask {
    OfflineTask {
        id: id.to_owned(),
        name: id.to_owned(),
        phase,
        progress: 0,
        message: None,
        created_at: None,
        file_id: None,
        output_parent_id: None,
        file_size: 0,
        kind: None,
        thumbnail_url: String::new(),
        media: false,
        video: false,
    }
}

#[test]
fn a_stale_generation_stops_polling() {
    poll_running().store(true, Ordering::SeqCst);
    let first = poll_generation().fetch_add(1, Ordering::SeqCst) + 1;
    assert!(still_polling(first), "刚启动的代际有效");

    // 登出再登录：代际前进，旧的线程应当立刻醒来退出，不会睡满一个间隔
    let second = poll_generation().fetch_add(1, Ordering::SeqCst) + 1;
    assert!(!still_polling(first), "被新一代顶掉的线程要退出");
    assert!(still_polling(second), "新登录的线程继续轮询");
    let revision = poll_wait_revision().load(Ordering::SeqCst);
    assert!(
        matches!(
            wait_for_poll(Duration::from_secs(5), first, revision),
            PollWait::Stopped
        ),
        "旧代际不睡满"
    );

    // 登出：开关关掉，谁都不再算数
    poll_running().store(false, Ordering::SeqCst);
    assert!(!still_polling(second));
}

#[test]
fn interval_depends_on_active_tasks() {
    assert_eq!(next_interval(&[]), POLL_IDLE);
    let mut deleted = task("deleted", TaskPhase::Error);
    deleted.message = Some("File deleted".into());
    assert_eq!(
        next_interval(&[
            task("a", TaskPhase::Completed),
            task("b", TaskPhase::Error),
            deleted,
        ]),
        POLL_IDLE
    );
    assert_eq!(next_interval(&[task("c", TaskPhase::Running)]), POLL_ACTIVE);
    assert_eq!(next_interval(&[task("d", TaskPhase::Pending)]), POLL_ACTIVE);
    assert_eq!(next_interval(&[task("e", TaskPhase::Paused)]), POLL_ACTIVE);
}

#[test]
fn completed_batch_dedupes_folders_and_ignores_failures() {
    let first = task("first", TaskPhase::Completed);
    let second = task("second", TaskPhase::Completed);
    let failed = task("failed", TaskPhase::Error);
    let destinations = HashMap::from([
        ("first".to_owned(), "folder-current".to_owned()),
        ("second".to_owned(), "folder-current".to_owned()),
    ]);

    let folders = completed_task_folders(&[first, second, failed], &destinations);
    assert_eq!(folders.len(), 1, "同一批任务完成只校准一次目录");
    assert!(folders.contains("folder-current"));
}

#[test]
fn completed_task_falls_back_to_resource_parent_and_keeps_root() {
    let mut submitted = task("submitted", TaskPhase::Completed);
    submitted.output_parent_id = Some("folder-moved".into());
    let mut restarted = task("restarted", TaskPhase::Completed);
    restarted.output_parent_id = Some("folder-resource".into());
    let mut root = task("root", TaskPhase::Completed);
    root.output_parent_id = Some(String::new());
    let destinations = HashMap::from([("submitted".to_owned(), "folder-submitted".to_owned())]);

    let folders = completed_task_folders(&[submitted, restarted, root], &destinations);
    assert!(folders.contains("folder-submitted"), "会话内目标优先");
    assert!(folders.contains("folder-resource"), "重启后回退资源父目录");
    assert!(folders.contains(""), "根目录用空串表示");
}

#[test]
fn submitted_tasks_merge_by_id_without_duplicates() {
    let mut updated = task("same", TaskPhase::Running);
    updated.progress = 30;
    let appended = task("new", TaskPhase::Pending);
    let merged = merge_submitted_tasks(
        vec![
            task("old", TaskPhase::Completed),
            task("same", TaskPhase::Pending),
        ],
        vec![updated, appended],
    );

    assert_eq!(merged.len(), 3);
    assert_eq!(
        merged
            .iter()
            .find(|task| task.id == "same")
            .map(|task| task.progress),
        Some(30)
    );
    assert!(merged.iter().any(|task| task.id == "new"));
}

#[test]
fn tasks_sort_by_creation_time_across_statuses() {
    let mut new_completed = task("new-completed", TaskPhase::Completed);
    new_completed.created_at = Utc.with_ymd_and_hms(2026, 11, 1, 22, 23, 0).single();

    let mut old_deleted = task("old-deleted", TaskPhase::Error);
    old_deleted.message = Some("File deleted".into());
    old_deleted.created_at = Utc.with_ymd_and_hms(2026, 10, 31, 10, 6, 0).single();

    let mut older_running = task("older-running", TaskPhase::Running);
    older_running.created_at = Utc.with_ymd_and_hms(2026, 10, 30, 18, 0, 0).single();

    let mut tasks = vec![
        old_deleted,
        task("missing-time", TaskPhase::Completed),
        older_running,
        new_completed,
    ];
    sort_tasks_newest_first(&mut tasks);

    let ids: Vec<_> = tasks.iter().map(|task| task.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "new-completed",
            "old-deleted",
            "older-running",
            "missing-time"
        ]
    );
}

#[test]
fn finished_task_ids_include_all_terminal_states_only() {
    let mut failed = task("failed", TaskPhase::Error);
    failed.message = Some("network error".into());
    let mut deleted = task("deleted", TaskPhase::Error);
    deleted.message = Some("File deleted".into());
    let mut blank = task(" ", TaskPhase::Completed);
    blank.name = "blank id".into();

    let ids = finished_task_ids(&[
        task("done", TaskPhase::Completed),
        task("done", TaskPhase::Completed),
        failed,
        deleted,
        task("running", TaskPhase::Running),
        task("pending", TaskPhase::Pending),
        task("paused", TaskPhase::Paused),
        task("unknown", TaskPhase::Unknown),
        blank,
    ]);

    assert_eq!(ids, ["done", "failed", "deleted"]);
}

#[test]
fn manual_refresh_has_a_short_cooldown() {
    let now = Instant::now();
    assert!(refresh_due(None, now), "首次刷新不受冷却限制");
    assert!(
        !refresh_due(Some(now), now + Duration::from_secs(2)),
        "冷却期内不重复请求"
    );
    assert_eq!(REFRESH_COOLDOWN, Duration::from_secs(3));
    assert!(refresh_due(Some(now), now + REFRESH_COOLDOWN));
}

#[test]
fn task_map_exposes_ui_fields() {
    let mut t = task("id1", TaskPhase::Running);
    t.name = "电影.mkv".to_owned();
    t.progress = 42;
    t.file_size = 123_456;
    t.kind = Some(EntryKind::File);
    t.file_id = Some("file-1".to_owned());
    t.thumbnail_url = "https://thumb.example/secret".to_owned();
    t.media = true;
    t.video = true;
    let map = task_map(&t);
    let get = |key: &str| {
        map.get(&QString::from(key))
            .and_then(|v| v.value::<QString>())
            .map(|s| s.to_string())
            .unwrap_or_default()
    };
    assert_eq!(get("phase"), "running");
    assert_eq!(get("progressText"), "42%");
    assert_eq!(get("name"), "电影.mkv");
    assert_eq!(get("kind"), "file");
    assert_eq!(get("fileId"), "file-1");
    assert_eq!(get("sizeText"), "121 KB");
    assert_eq!(
        map.get(&QString::from("hasThumbnail"))
            .and_then(|value| value.value::<bool>()),
        Some(true)
    );
    assert_eq!(
        map.get(&QString::from("video"))
            .and_then(|value| value.value::<bool>()),
        Some(true)
    );
    assert_eq!(
        map.get(&QString::from("playable"))
            .and_then(|value| value.value::<bool>()),
        Some(true)
    );
    assert!(map.get(&QString::from("thumbnailUrl")).is_none());
}

#[test]
fn progress_is_integer_percent() {
    let mut t = task("id1", TaskPhase::Running);
    t.progress = 50;
    let map = task_map(&t);
    let value = map
        .get(&QString::from("progress"))
        .and_then(|v| v.value::<i32>());
    assert_eq!(value, Some(50));
}

#[test]
fn settled_flag_matches_phase() {
    let running = task_map(&task("a", TaskPhase::Running));
    let done = task_map(&task("b", TaskPhase::Completed));
    let mut deleted = task("c", TaskPhase::Error);
    deleted.message = Some("File deleted".into());
    let deleted = task_map(&deleted);
    let read = |map: &QVariantMap| {
        map.get(&QString::from("settled"))
            .and_then(|v| v.value::<bool>())
    };
    assert_eq!(read(&running), Some(false));
    assert_eq!(read(&done), Some(true));
    assert_eq!(read(&deleted), Some(true));
}

#[test]
fn deleted_output_is_not_presented_as_a_failure() {
    let mut deleted = task("deleted", TaskPhase::Error);
    deleted.message = Some("File deleted".into());
    let map = task_map(&deleted);
    let string = |key: &str| {
        map.get(&QString::from(key))
            .and_then(|value| value.value::<QString>())
            .map(|value| value.to_string())
    };
    assert_eq!(string("phase").as_deref(), Some("deleted"));
    assert_eq!(string("phaseText").as_deref(), Some("产物已删除"));
    assert_eq!(string("detail").as_deref(), Some("File deleted"));

    let mut failed = task("failed", TaskPhase::Error);
    failed.message = Some("Save failed, retry please".into());
    let map = task_map(&failed);
    assert_eq!(
        map.get(&QString::from("phase"))
            .and_then(|value| value.value::<QString>())
            .map(|value| value.to_string())
            .as_deref(),
        Some("error")
    );
}
