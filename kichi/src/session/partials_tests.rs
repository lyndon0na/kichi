//! `partials.rs` 的单元测试：plan_resume 矩阵与断点索引读写。

use super::super::download::Cancel;
use super::super::download_resume::{prepare_part, Part, Prepared};
use super::*;

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kichi-partials-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建临时目录");
    dir
}

/// `KICHI_PARTIALS_FILE` 是进程级环境变量：用到它的用例必须串行。
///
/// 与 `shards_tests` 共用 `partials::ENV_LOCK`，否则两个测试模块会互相踩环境变量。
fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner())
}

/// 造一个已经写了若干字节的 part（绕过真实下载循环）。
fn make_part(dir: &Path, id: &str, body: &[u8]) -> Part {
    use std::io::Write;
    let part_path = dir.join(format!("{id}.kichi-part"));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&part_path)
        .expect("建 part");
    file.write_all(body).expect("写 part");
    Part {
        file,
        part_path,
        final_path: dir.join(format!("{id}.mkv")),
        offset: body.len() as u64,
        total: Some(body.len() as u64),
    }
}

fn entry_in(dir: &Path, id: &str, total: Option<u64>) -> (PartialEntry, PathBuf) {
    let part = dir.join(format!("{id}.kichi-part"));
    std::fs::write(&part, b"x").expect("写 part");
    let entry = PartialEntry::new(id, &part, &dir.join(format!("{id}.mkv")), total);
    (entry, part)
}

#[test]
fn plan_without_entry_starts_from_zero() {
    let plan = plan_resume(None, Some(100), 0);
    assert_eq!(
        plan,
        ResumePlan {
            offset: 0,
            truncate: false,
            total: Some(100),
            complete: false
        }
    );
}

#[test]
fn plan_resets_when_remote_size_changed() {
    let (entry, _) = entry_in(&temp_dir("plan-changed"), "file", Some(100));
    let plan = plan_resume(Some(&entry), Some(200), 40);
    assert!(plan.truncate, "大小变了必须丢掉旧 part");
    assert_eq!(plan.offset, 0);
    assert_eq!(plan.total, Some(200));
    assert!(!plan.complete);
}

#[test]
fn plan_resets_when_part_is_longer_than_remote() {
    let (entry, _) = entry_in(&temp_dir("plan-long"), "file", None);
    let plan = plan_resume(Some(&entry), Some(100), 120);
    assert!(plan.truncate);
    assert_eq!(plan.offset, 0);
    assert_eq!(plan.total, Some(100));
}

#[test]
fn plan_treats_exactly_full_part_as_complete() {
    let (entry, _) = entry_in(&temp_dir("plan-complete"), "file", Some(100));
    let plan = plan_resume(Some(&entry), Some(100), 100);
    assert!(plan.complete, "崩溃在 rename 前：只需 sync + rename");
    assert_eq!(plan.offset, 100);
    assert!(!plan.truncate);
    assert_eq!(plan.total, Some(100));
}

#[test]
fn plan_resumes_from_part_length() {
    let (entry, _) = entry_in(&temp_dir("plan-resume"), "file", Some(100));
    let plan = plan_resume(Some(&entry), Some(100), 45);
    assert_eq!(plan.offset, 45);
    assert!(!plan.truncate && !plan.complete);
    assert_eq!(plan.total, Some(100));
}

#[test]
fn plan_handles_unknown_sizes_by_trusting_part_length() {
    let (entry, _) = entry_in(&temp_dir("plan-unknown"), "file", None);
    let plan = plan_resume(Some(&entry), None, 45);
    assert_eq!(plan.offset, 45);
    assert!(!plan.truncate && !plan.complete);
    assert_eq!(plan.total, None);
}

#[test]
fn plan_uses_indexed_total_when_detail_has_none() {
    let (entry, _) = entry_in(&temp_dir("plan-index-total"), "file", Some(80));
    let plan = plan_resume(Some(&entry), None, 80);
    assert!(plan.complete);
    assert_eq!(plan.total, Some(80));
}

// —— v2 分片计划（2026-10-28 单文件多分片）——

fn shard(start: u64, end: u64, done: u64) -> ShardSpan {
    ShardSpan { start, end, done }
}

#[test]
fn shard_plan_is_only_for_known_large_files() {
    assert!(
        plan_shards(None, None, 0, 100, 4).is_none(),
        "大小未知不分片"
    );
    assert!(
        plan_shards(None, Some(99), 0, 100, 4).is_none(),
        "小于阈值不分片"
    );
    assert!(
        plan_shards(None, Some(100), 0, 100, 1).is_none(),
        "wanted=1 是串行回滚开关"
    );
    assert!(plan_shards(None, Some(100), 0, 100, 4).is_some());
}

#[test]
fn fresh_shard_plan_covers_the_file_without_gaps() {
    let plan = plan_shards(None, Some(1000), 0, 100, 4).expect("应分片");
    assert!(plan.truncate);
    assert!(!plan.complete);
    assert_eq!(
        plan.spans,
        vec![
            shard(0, 250, 0),
            shard(250, 500, 0),
            shard(500, 750, 0),
            shard(750, 1000, 0),
        ]
    );
    assert_eq!(plan.done_bytes(), 0);
}

#[test]
fn last_shard_may_be_smaller_than_the_others() {
    let plan = plan_shards(None, Some(1001), 0, 100, 4).expect("应分片");
    assert_eq!(plan.spans.len(), 4);
    // 每片 ceil(1001/4) = 251，最后一片只有 248 字节
    assert_eq!(plan.spans[3].start, 753);
    assert_eq!(plan.spans[3].end, 1001);
    assert_eq!(plan.spans[3].len(), 248);
}

#[test]
fn v1_sequential_entry_keeps_the_old_path() {
    let dir = temp_dir("shard-v1");
    let (entry, _) = entry_in(&dir, "old", Some(1000));
    assert!(
        plan_shards(Some(&entry), Some(1000), 300, 100, 4).is_none(),
        "旧顺序断点不能转成分片"
    );
}

#[test]
fn shard_plan_resets_when_remote_size_changed() {
    let dir = temp_dir("shard-changed");
    let part = dir.join("a.kichi-part");
    std::fs::write(&part, vec![0u8; 100]).expect("写 part");
    let entry = PartialEntry::new_sharded(
        "file",
        &part,
        &dir.join("a.mkv"),
        1000,
        vec![shard(0, 500, 100), shard(500, 1000, 0)],
    );
    let plan = plan_shards(Some(&entry), Some(2000), 100, 100, 4).expect("大小变了也要给出新计划");
    assert!(plan.truncate, "云端文件变了必须整文件重下");
    assert_eq!(plan.total, 2000);
    assert!(plan.spans.iter().all(|span| span.done == 0));
    assert_eq!(plan.spans.last().map(|span| span.end), Some(2000));
}

#[test]
fn shard_plan_resumes_done_bytes_and_clamps_to_disk() {
    let dir = temp_dir("shard-resume");
    let part = dir.join("a.kichi-part");
    std::fs::write(&part, vec![0u8; 300]).expect("写 part");
    let entry = PartialEntry::new_sharded(
        "file",
        &part,
        &dir.join("a.mkv"),
        1000,
        vec![shard(0, 500, 400), shard(500, 1000, 0)],
    );
    let plan = plan_shards(Some(&entry), Some(1000), 300, 100, 4).expect("应续传");
    assert!(!plan.truncate);
    assert_eq!(plan.spans[0].done, 300, "索引比磁盘多记时以磁盘为准");
    assert_eq!(plan.spans[1].done, 0);
}

#[test]
fn shard_plan_marks_complete_only_when_every_span_is_done() {
    let dir = temp_dir("shard-complete");
    let part = dir.join("a.kichi-part");
    std::fs::write(&part, vec![0u8; 1000]).expect("写满 part");
    let entry = PartialEntry::new_sharded(
        "file",
        &part,
        &dir.join("a.mkv"),
        1000,
        vec![shard(0, 500, 500), shard(500, 1000, 500)],
    );
    let plan = plan_shards(Some(&entry), Some(1000), 1000, 100, 4).expect("应能识别完成");
    assert!(plan.complete, "所有片都写完：只需 sync + rename");
    assert_eq!(plan.done_bytes(), 1000);
}

#[test]
fn broken_shard_spans_fall_back_to_a_full_restart() {
    let dir = temp_dir("shard-broken");
    let part = dir.join("a.kichi-part");
    std::fs::write(&part, vec![0u8; 100]).expect("写 part");
    let entry = PartialEntry::new_sharded(
        "file",
        &part,
        &dir.join("a.mkv"),
        1000,
        // 中间有空洞：不能信
        vec![shard(0, 400, 100), shard(500, 1000, 0)],
    );
    let plan = plan_shards(Some(&entry), Some(1000), 100, 100, 4).expect("应重下");
    assert!(plan.truncate);
    assert_eq!(plan.spans[0].start, 0);
    assert_eq!(plan.spans.last().map(|span| span.end), Some(1000));
}

#[test]
fn plan_shards_keeps_resuming_a_v2_part_when_sharding_is_turned_off() {
    let dir = temp_dir("shard-off");
    let part = dir.join("a.kichi-part");
    std::fs::write(&part, vec![0u8; 200]).expect("写 part");
    let entry = PartialEntry::new_sharded(
        "file",
        &part,
        &dir.join("a.mkv"),
        1000,
        vec![shard(0, 500, 200), shard(500, 1000, 0)],
    );
    let plan = plan_shards(Some(&entry), Some(1000), 200, 100, 1)
        .expect("已有 v2 断点不能退回顺序追加（稀疏文件会把空洞当数据）");
    assert!(!plan.truncate);
    assert_eq!(plan.spans[0].done, 200);
}

#[test]
fn shard_index_roundtrips_and_v1_json_still_loads() {
    let dir = temp_dir("shard-json");
    let path = dir.join("partials.json");
    let part = dir.join("a.kichi-part");
    std::fs::write(&part, b"x").expect("写 part");

    let sharded = PartialEntry::new_sharded(
        "file-v2",
        &part,
        &dir.join("a.mkv"),
        1000,
        vec![shard(0, 500, 100), shard(500, 1000, 0)],
    );
    save(&path, std::slice::from_ref(&sharded)).expect("保存 v2");
    assert_eq!(lookup_in(&path, "file-v2", &dir), Some(sharded));

    // 旧 v1 文件（没有 shards 字段、version=1）必须照旧读出来
    let v1 = format!(
        r#"{{"version":1,"entries":[{{"id":"file-v1","part":{},"final":{},"total":10,"updated_unix":0}}]}}"#,
        serde_json::to_string(&part.display().to_string()).expect("序列化路径"),
        serde_json::to_string(&dir.join("a.mkv").display().to_string()).expect("序列化路径"),
    );
    std::fs::write(&path, v1).expect("写 v1 索引");
    let loaded = lookup_in(&path, "file-v1", &dir).expect("v1 条目应能读");
    assert!(loaded.shards.is_empty(), "v1 条目按顺序续传处理");
}

#[test]
fn index_remembers_looks_up_and_forgets() {
    let dir = temp_dir("index-roundtrip");
    let path = dir.join("partials.json");
    let part = dir.join("a.kichi-part");
    std::fs::write(&part, b"x").expect("写 part");
    let entry = PartialEntry::new("file-a", &part, &dir.join("a.mkv"), Some(10));
    save(&path, std::slice::from_ref(&entry)).expect("保存索引");

    assert_eq!(lookup_in(&path, "file-a", &dir), Some(entry.clone()));
    assert_eq!(
        lookup_in(&path, "file-a", &dir.join("other")),
        None,
        "断点不能续到别的目录"
    );
    assert_eq!(lookup_in(&path, "file-missing", &dir), None);

    // 文件夹批次的 part 在子目录：从下载根目录仍要能续上
    let nested = dir.join("剧集");
    std::fs::create_dir_all(&nested).expect("建子目录");
    let nested_part = nested.join("b.kichi-part");
    std::fs::write(&nested_part, b"x").expect("写嵌套 part");
    let nested_entry = PartialEntry::new("file-b", &nested_part, &nested.join("b.mkv"), Some(10));
    save(&path, &[entry.clone(), nested_entry.clone()]).expect("保存索引");
    assert_eq!(lookup_in(&path, "file-b", &dir), Some(nested_entry));

    let mut entries = load(&path);
    entries.retain(|item| item.id != "file-a" && item.id != "file-b");
    save(&path, &entries).expect("保存空索引");
    assert_eq!(lookup_in(&path, "file-a", &dir), None);
}

#[test]
fn missing_part_is_dropped_on_load() {
    let dir = temp_dir("index-missing-part");
    let path = dir.join("partials.json");
    let (entry, part) = entry_in(&dir, "file-b", Some(10));
    save(&path, &[entry]).expect("保存索引");
    std::fs::remove_file(part).expect("删 part");
    assert_eq!(lookup_in(&path, "file-b", &dir), None);
}

#[test]
fn corrupt_json_falls_back_to_empty_index() {
    let dir = temp_dir("index-corrupt");
    let path = dir.join("partials.json");
    std::fs::write(&path, b"{ not json").expect("写坏文件");
    assert!(load(&path).is_empty(), "损坏的索引不能 panic");
}

#[test]
fn saved_index_is_private() {
    let dir = temp_dir("index-mode");
    let path = dir.join("partials.json");
    save(&path, &[]).expect("保存空索引");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path)
            .expect("读权限")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "断点索引应仅本人可读写");
    }
}

#[test]
fn part_abort_keeps_on_abandon_but_deletes_on_cancel() {
    let _guard = env_lock();
    let dir = temp_dir("abort-split");
    let index = dir.join("partials.json");
    std::env::set_var("KICHI_PARTIALS_FILE", &index);

    let abandon = Cancel::default();
    abandon.abandon();
    let part = make_part(&dir, "f1", b"abc");
    assert_eq!(part.abort("f1", &abandon).to_string(), "下载已中断");
    assert!(
        dir.join("f1.kichi-part").is_file(),
        "登出要保留 part 等下次续传"
    );
    assert!(lookup("f1", &dir).is_some(), "登出要保留断点索引");

    let cancel = Cancel::default();
    cancel.cancel();
    let part = make_part(&dir, "f1", b"abc");
    assert_eq!(part.abort("f1", &cancel).to_string(), "下载已取消");
    assert!(!dir.join("f1.kichi-part").exists(), "用户取消仍删 part");
    assert!(lookup("f1", &dir).is_none(), "用户取消仍忘索引");

    std::env::remove_var("KICHI_PARTIALS_FILE");
}

#[test]
fn prepare_part_complete_abandoned_does_not_rename() {
    let _guard = env_lock();
    let dir = temp_dir("prepare-abandon");
    let index = dir.join("partials.json");
    std::env::set_var("KICHI_PARTIALS_FILE", &index);

    let part = dir.join("f2.kichi-part");
    std::fs::write(&part, vec![0u8; 10]).expect("写 part");
    let entry = PartialEntry::new("f2", &part, &dir.join("f2.mkv"), Some(10));
    save(&index, std::slice::from_ref(&entry)).expect("保存索引");

    let cancel = Cancel::default();
    cancel.abandon();
    let result = prepare_part("f2", "f2.mkv", &dir, Some(10), &cancel);
    assert!(result.is_err(), "登出命中时即使数据已齐也不能 rename");
    assert!(part.is_file(), "part 原样保留");
    assert!(!dir.join("f2.mkv").exists(), "不能出现正式文件");

    // 对照：没有登出时，完整 part 直接改名完成
    let result =
        prepare_part("f2", "f2.mkv", &dir, Some(10), &Cancel::default()).expect("直接完成");
    assert!(matches!(result, Prepared::Complete(_)));
    assert!(dir.join("f2.mkv").is_file(), "正常路径仍能落成品");
    assert!(!part.exists(), "完成后 part 已改名");

    std::env::remove_var("KICHI_PARTIALS_FILE");
}

#[test]
fn path_override_is_used() {
    let _guard = env_lock();
    let dir = temp_dir("index-env");
    let path = dir.join("custom-partials.json");
    std::env::set_var("KICHI_PARTIALS_FILE", &path);
    let part = dir.join("a.kichi-part");
    std::fs::write(&part, b"x").expect("写 part");
    let entry = PartialEntry::new("file-env", &part, &dir.join("a.mkv"), Some(1));
    remember(&entry);
    assert!(path.is_file(), "KICHI_PARTIALS_FILE 必须生效");
    assert_eq!(lookup("file-env", &dir), Some(entry));
    forget("file-env");
    assert_eq!(lookup("file-env", &dir), None);
    std::env::remove_var("KICHI_PARTIALS_FILE");
}

#[test]
fn discard_removes_only_parts_below_the_requested_download_root() {
    let _guard = env_lock();
    let root = temp_dir("discard-root");
    let other = temp_dir("discard-other");
    let index = root.join("partials.json");
    std::env::set_var("KICHI_PARTIALS_FILE", &index);

    let root_part = root.join("same-id.kichi-part");
    let other_part = other.join("same-id.kichi-part");
    std::fs::write(&root_part, b"root").expect("写根目录 part");
    std::fs::write(&other_part, b"other").expect("写其它目录 part");
    let root_entry = PartialEntry::new("same-id", &root_part, &root.join("same-id.mkv"), Some(4));
    let other_entry =
        PartialEntry::new("same-id", &other_part, &other.join("same-id.mkv"), Some(5));
    save(&index, &[root_entry, other_entry]).expect("保存断点索引");

    discard("same-id", &root);

    assert!(!root_part.exists(), "目标下载目录里的 part 要删除");
    assert!(other_part.exists(), "其它下载目录里的 part 不能误删");
    assert_eq!(lookup("same-id", &root), None);
    assert!(
        lookup("same-id", &other).is_some(),
        "其它目录的索引仍能续传"
    );

    std::env::remove_var("KICHI_PARTIALS_FILE");
}
