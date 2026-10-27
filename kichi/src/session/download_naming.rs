//! 下载落盘的文件名策略：同名不覆盖、`.kichi-part` 原子占位。
//!
//! 从 `download.rs` 拆出来守行数上限：那边管「取链接 → 流式写盘」，
//! 这里管「这次写到哪个文件」。两条纪律（见
//! `docs/adr/0002-download-never-overwrite.md`）：
//! - 同名冲突自动加序号 `name (1).ext`，**绝不覆盖**已有文件；
//! - 先写 `<名>.kichi-part`，完成才 rename，断电不留半个文件冒充成品。
//!
//! 名字长度也在这里收敛：单个名字按字节截到 [`MAX_NAME_BYTES`]（保留扩展名，
//! 加 8 位短哈希区分），见 `docs/adr/0007-long-filename-truncation.md`。

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

use super::Failure;

/// 临时文件后缀。
const PART_SUFFIX: &str = ".kichi-part";

/// 单个名字的字节预算：255（NAME_MAX）− 11（`.kichi-part`）− 8（`-<pid>` 兜底）= 236。
const MAX_NAME_BYTES: usize = 236;
/// 超过这个长度的「扩展名」不当扩展名看，整名一起截（防止超长假扩展名把名字吃光）。
const MAX_EXT_BYTES: usize = 16;
/// 截断标记：`-` + 8 位十六进制短哈希。
const HASH_LEN: usize = 9;

/// 临时文件名：`<正式名>.kichi-part`。
fn part_path_for(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(PART_SUFFIX);
    path.with_file_name(name)
}

/// 不覆盖已有文件：`a.mkv` 已存在 → `a (1).mkv`，依此类推。
///
/// 只算名字、不占位；真正落盘走 [`claim_part`]（它用 `create_new` 原子占位，
/// 两个线程同时选同一个名字时只有一个能成功）。单测用它断言选名规则。
pub fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !exists(&candidate) {
        return candidate;
    }
    let (stem, ext) = split_ext(name);
    for index in 1..10_000 {
        let candidate = dir.join(format!("{stem} ({index}){ext}"));
        if !exists(&candidate) {
            return candidate;
        }
    }
    // 一万个同名文件还撞上：加时间戳兜底，总比覆盖强
    dir.join(format!("{stem}-{}{ext}", std::process::id()))
}

/// 选名 + 原子占位：返回 `(已抢到的 part 文件句柄, part 路径, 最终路径)`。
///
/// `unique_path` 的 `exists()` 检查不是原子的：两个线程同时跑，可能都看到
/// 「不存在」并选到同一个名字，然后一起写同一个 `.kichi-part`。这里改成
/// `create_new` 抢 `<候选>.kichi-part`——抢不到就试下一个序号，抢到即锁定
/// 这个名字（期间别的线程再选会看到 part 文件已存在）。
///
/// 已存在的**正式文件**也要跳过：rename 会覆盖目标，只有确认没有同名成品
/// 才能选这个名字（`docs/adr/0002-download-never-overwrite.md`）。
pub(super) fn claim_part(dir: &Path, name: &str) -> Result<(File, PathBuf, PathBuf), Failure> {
    let (stem, ext) = split_ext(name);
    for index in 0..10_000 {
        let candidate = if index == 0 {
            dir.join(name)
        } else {
            dir.join(format!("{stem} ({index}){ext}"))
        };
        if candidate.exists() {
            continue;
        }
        match claim_candidate(&candidate) {
            Ok(claimed) => return Ok(claimed),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => {
                return Err(Failure::Other(format!(
                    "无法创建下载文件（{}）：{err}",
                    part_path_for(&candidate).display()
                )));
            }
        }
    }
    // 一万个同名文件还撞上：加进程号兜底，总比覆盖强
    let fallback = dir.join(format!("{stem}-{}{ext}", std::process::id()));
    if !fallback.exists() {
        if let Ok(claimed) = claim_candidate(&fallback) {
            return Ok(claimed);
        }
    }
    Err(Failure::Other(
        "无法创建下载文件：同名文件过多，换一个下载目录再试".to_owned(),
    ))
}

/// 用 `create_new` 抢占一个候选名的 `.kichi-part`。
fn claim_candidate(candidate: &Path) -> io::Result<(File, PathBuf, PathBuf)> {
    let part_path = part_path_for(candidate);
    let file = File::options()
        .write(true)
        .create_new(true)
        .open(&part_path)?;
    Ok((file, part_path, candidate.to_path_buf()))
}

fn exists(path: &Path) -> bool {
    path.exists() || part_path_for(path).exists()
}

/// 拆成（不含扩展名的主体，含点的扩展名）。
fn split_ext(name: &str) -> (String, String) {
    match name.rsplit_once('.') {
        // 以点开头的隐藏文件（`.bashrc`）没有扩展名
        Some((stem, ext)) if !stem.is_empty() => (stem.to_owned(), format!(".{ext}")),
        _ => (name.to_owned(), String::new()),
    }
}

/// 向前退到 UTF-8 字符边界（不截断半个汉字）。
fn floor_boundary(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// FNV-1a 32 位：稳定、无依赖、跨版本结果不变。
///
/// 只用 `DefaultHasher` 是不够的——标准库不承诺它的哈希值跨版本稳定，
/// 换个 Rust 版本同一个云端文件就会换成另一个本地名。
fn short_hash(text: &str) -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in text.as_bytes() {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

/// 把超长名字截到 `budget` 字节以内：保留扩展名，插 `-<8 位哈希>` 区分。
///
/// 没超长时原样返回（一个字节都不改）。超长时扩展名（含点）最多 16 字节；
/// 更长的「扩展名」当假扩展名，整名一起截。哈希来源是调用方给的原始名字，
/// 保证同一个云端名永远映射到同一个本地名。
fn cap_name(name: &str, budget: usize, hash_of: &str) -> String {
    if name.len() <= budget {
        return name.to_owned();
    }
    let marker = format!("-{:08x}", short_hash(hash_of));
    let (stem, ext) = split_ext(name);
    // 离谱的假扩展名不当扩展名：整名一起截
    let (stem, ext) = if ext.len() <= MAX_EXT_BYTES {
        (stem, ext)
    } else {
        (name.to_owned(), String::new())
    };
    let keep = budget.saturating_sub(marker.len() + ext.len());
    format!("{}{marker}{ext}", floor_boundary(&stem, keep))
}

/// 把官方文件名收敛成安全的本地文件名。
///
/// 官方允许 `/`、`\` 等字符出现在名字里（在它那边是合法文件名），
/// 落到本地可能被当成路径分隔符写到别的目录去——一律替换掉。
/// 超过 [`MAX_NAME_BYTES`] 的名字会被截断（保留扩展名 + 短哈希），
/// 否则 255 字节的 `NAME_MAX` 会让下载直接失败。
pub fn safe_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | '\0' => '_',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    // 去掉末尾的点（Windows/部分挂载点会吃掉它，留着容易变成隐藏或空名）
    let trimmed = cleaned.trim().trim_end_matches('.').to_owned();
    if trimmed.is_empty() || trimmed.chars().all(|c| c == '.') {
        return "未命名文件".to_owned();
    }
    // 以点开头会被当成隐藏文件；官方名字很少这样，加个下划线前缀保住可见性
    if trimmed.starts_with('.') {
        // 前缀也占字节预算
        format!("_{}", cap_name(&trimmed, MAX_NAME_BYTES - 1, name))
    } else {
        cap_name(&trimmed, MAX_NAME_BYTES, name)
    }
}

#[cfg(test)]
mod tests {
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
}
