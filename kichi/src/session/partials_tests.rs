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
fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
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
    assert_eq!(part.abort("f1", &abandon).to_string(), "下载已取消");
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
