//! `download_naming.rs` 的单元测试：选名、原子占位与完成改名不覆盖。

use super::*;

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kichi-dl-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建临时目录");
    dir
}

#[test]
fn same_name_never_overwrites() {
    let dir = temp_dir("unique");
    let first = unique_path(&dir, "ep1.mkv");
    assert_eq!(first, dir.join("ep1.mkv"));
    std::fs::write(&first, b"x").expect("写文件");
    assert_eq!(unique_path(&dir, "ep1.mkv"), dir.join("ep1 (1).mkv"));
    std::fs::write(dir.join("ep1 (1).mkv"), b"x").expect("写文件");
    assert_eq!(unique_path(&dir, "ep1.mkv"), dir.join("ep1 (2).mkv"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pending_part_file_counts_as_taken() {
    // 另一个任务正在下同一个名字：不能选同一个路径
    let dir = temp_dir("part");
    std::fs::write(part_path_for(&dir.join("a.bin")), b"x").expect("写临时文件");
    assert_eq!(unique_path(&dir, "a.bin"), dir.join("a (1).bin"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn claim_part_skips_existing_files_and_parts() {
    let dir = temp_dir("claim-skip");
    let (file, part, final_path) = claim_part(&dir, "a.mkv").expect("占用第一个名字");
    assert_eq!(final_path, dir.join("a.mkv"));
    assert_eq!(part, dir.join("a.mkv.kichi-part"));
    assert!(part.exists(), "占位文件要真的建出来");
    drop(file);

    // 同名再抢：part 已存在 → 跳号
    let (file, _, final_path) = claim_part(&dir, "a.mkv").expect("占用第二个名字");
    assert_eq!(final_path, dir.join("a (1).mkv"));
    drop(file);

    // 已存在的正式文件也要跳过（rename 会覆盖）
    std::fs::write(dir.join("a (2).mkv"), b"x").expect("写正式文件");
    let (file, _, final_path) = claim_part(&dir, "a.mkv").expect("占用第三个名字");
    assert_eq!(final_path, dir.join("a (3).mkv"));
    drop(file);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn concurrent_claims_never_share_a_path() {
    // 这是 unique_path 的 exists() 检查挡不住的场景：两个线程同时选名。
    let dir = temp_dir("claim-race");
    let mut handles = Vec::new();
    for _ in 0..8 {
        let dir = dir.clone();
        handles.push(std::thread::spawn(move || {
            let (file, part, final_path) = claim_part(&dir, "same.mkv").expect("抢占");
            drop(file);
            (part, final_path)
        }));
    }
    let mut paths: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().expect("线程正常结束").1)
        .collect();
    paths.sort();
    paths.dedup();
    assert_eq!(paths.len(), 8, "8 个线程必须拿到 8 个不同路径");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn finalize_uses_preferred_name_when_free() {
    let dir = temp_dir("final-free");
    let part = dir.join("a.mkv.kichi-part");
    std::fs::write(&part, b"x").expect("写 part");
    let final_path = finalize_part(&part, &dir.join("a.mkv")).expect("改名");
    assert_eq!(final_path, dir.join("a.mkv"));
    assert!(final_path.exists());
    assert!(!part.exists(), "改名后不能残留 part");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn finalize_never_overwrites_existing_final() {
    let dir = temp_dir("final-existing");
    std::fs::write(dir.join("a.mkv"), b"old").expect("写旧文件");
    let part = dir.join("a.mkv.kichi-part");
    std::fs::write(&part, b"new").expect("写 part");
    let final_path = finalize_part(&part, &dir.join("a.mkv")).expect("改名");
    assert_eq!(final_path, dir.join("a (1).mkv"));
    assert_eq!(
        std::fs::read(dir.join("a.mkv")).expect("读旧文件"),
        b"old",
        "旧文件一个字节都不能动"
    );
    assert_eq!(std::fs::read(&final_path).expect("读新文件"), b"new");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn finalize_skips_another_downloads_part_name() {
    let dir = temp_dir("final-part-taken");
    std::fs::write(dir.join("a.mkv"), b"old").expect("写旧文件");
    std::fs::write(dir.join("a (1).mkv.kichi-part"), b"busy").expect("占住序号");
    let part = dir.join("a.mkv.kichi-part");
    std::fs::write(&part, b"new").expect("写 part");
    let final_path = finalize_part(&part, &dir.join("a.mkv")).expect("改名");
    assert_eq!(
        final_path,
        dir.join("a (2).mkv"),
        "不能抢别的任务已占用的序号"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn extension_split_handles_hidden_and_dotted() {
    assert_eq!(split_ext("a.mkv"), ("a".into(), ".mkv".into()));
    assert_eq!(
        split_ext("archive.tar.gz"),
        ("archive.tar".into(), ".gz".into())
    );
    assert_eq!(split_ext(".bashrc"), (".bashrc".into(), "".into()));
    assert_eq!(split_ext("noext"), ("noext".into(), "".into()));
}

#[test]
fn file_names_cannot_escape_the_target_dir() {
    assert_eq!(
        safe_file_name("../../etc/passwd"),
        "_.._.._etc_passwd",
        "路径分隔符必须替掉，开头点加下划线保可见"
    );
    assert_eq!(safe_file_name("a\\b"), "a_b");
    assert_eq!(safe_file_name("   "), "未命名文件");
    assert_eq!(safe_file_name("正常 名字.mkv"), "正常 名字.mkv");
    assert_eq!(safe_file_name(".."), "未命名文件", "纯点会变空名");
    assert_eq!(safe_file_name("truncated."), "truncated");
}

#[test]
fn long_ascii_name_keeps_extension_and_hash() {
    let long = format!("{}.mkv", "a".repeat(300));
    let name = safe_file_name(&long);
    assert!(name.len() <= MAX_NAME_BYTES, "超长名字必须截断");
    assert!(name.ends_with(".mkv"), "扩展名要保留：{name}");
    let marker = &name[name.len() - ".mkv".len() - HASH_LEN..name.len() - ".mkv".len()];
    assert!(
        marker.starts_with('-') && marker[1..].chars().all(|c| c.is_ascii_hexdigit()),
        "扩展名前要有 -xxxxxxxx 短哈希：{marker}"
    );
    assert_eq!(marker.len(), HASH_LEN);
}

#[test]
fn long_chinese_name_cuts_on_char_boundary() {
    let long = format!("{}.mkv", "长".repeat(100));
    assert!(long.len() > MAX_NAME_BYTES);
    let name = safe_file_name(&long);
    assert!(name.len() <= MAX_NAME_BYTES);
    assert!(name.ends_with(".mkv"));
    // 没被切半个汉字：掐掉后缀与哈希后仍是完整 UTF-8 前缀
    let stem = name.trim_end_matches(".mkv");
    let head = &stem[..stem.len() - HASH_LEN];
    assert!(head.chars().all(|c| c == '长'), "只能是完整汉字：{head}");
    assert!(!head.is_empty());
}

#[test]
fn long_name_without_extension_ends_cleanly() {
    let name = safe_file_name(&"x".repeat(300));
    assert!(name.len() <= MAX_NAME_BYTES);
    assert!(!name.ends_with('.') && !name.ends_with(' '));
}

#[test]
fn absurd_extension_is_not_preserved() {
    let long = format!("abc.{}", "x".repeat(300));
    let name = safe_file_name(&long);
    assert!(name.len() <= MAX_NAME_BYTES);
    assert!(!name.ends_with(&"x".repeat(300)), "假扩展名不能整段留下");
    // 整名一起截：结尾是短哈希本身（没有保留任何扩展名）
    let tail = &name[name.len() - HASH_LEN..];
    assert!(
        tail.starts_with('-') && tail[1..].chars().all(|c| c.is_ascii_hexdigit()),
        "假扩展名被丢弃后应以短哈希结尾：{name}"
    );
}

#[test]
fn truncated_names_with_different_tails_stay_distinct() {
    let first = format!("{}{}", "a".repeat(300), "1.mkv");
    let second = format!("{}{}", "a".repeat(300), "2.mkv");
    let first = safe_file_name(&first);
    let second = safe_file_name(&second);
    assert_ne!(first, second, "尾部不同的超长名截断后不能撞名");
}

#[test]
fn name_at_budget_returns_unchanged() {
    let base = "a".repeat(MAX_NAME_BYTES - ".mkv".len());
    let exact = format!("{base}.mkv");
    assert_eq!(exact.len(), MAX_NAME_BYTES);
    assert_eq!(safe_file_name(&exact), exact, "预算内一个字节都不改");
}

#[test]
fn long_hidden_name_keeps_underscore_prefix() {
    let long = format!(".{}.mkv", "h".repeat(300));
    let name = safe_file_name(&long);
    assert!(name.len() <= MAX_NAME_BYTES);
    assert!(name.starts_with('_'), "点开头仍要加下划线：{name}");
}

/// 直接复现原 bug：截断后带上临时后缀 / 序号 / 进程号兜底仍能落盘。
#[test]
fn capped_name_actually_creates_on_disk_with_part_and_index() {
    let dir = temp_dir("long-name");
    let long = format!("{}.mkv", "长".repeat(200));
    let name = safe_file_name(&long);
    assert!(name.len() + PART_SUFFIX.len() <= 255);
    // 走到序号兜底也不会超：最坏是 " (9999)" 与 "-<pid>"
    let (stem, ext) = split_ext(&name);
    assert!(format!("{stem} (9999){ext}{PART_SUFFIX}").len() <= 255);
    assert!(format!("{stem}-{}{ext}{PART_SUFFIX}", std::process::id()).len() <= 255);
    let (file, part, _) = claim_part(&dir, &name).expect("超长名截断后必须能创建");
    assert!(part.exists());
    drop(file);
    let _ = std::fs::remove_dir_all(&dir);
}
