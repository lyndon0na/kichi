//! 下载落盘的文件名策略：同名不覆盖、`.kichi-part` 原子占位。
//!
//! 从 `download.rs` 拆出来守行数上限：那边管「取链接 → 流式写盘」，
//! 这里管「这次写到哪个文件」。两条纪律：
//! - 同名冲突自动加序号 `name (1).ext`，**绝不覆盖**已有文件；
//! - 先写 `<名>.kichi-part`，完成才 rename，断电不留半个文件冒充成品。
//!
//! 名字长度也在这里收敛：单个名字按字节截到 [`MAX_NAME_BYTES`]（保留扩展名，
//! 加 8 位短哈希区分）。

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
/// 才能选这个名字。
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

/// 把下完的 part 改成正式名；**绝不覆盖**已有文件（冲突就加序号）。
///
/// `preferred` 是选 part 时记下的目标名（续传时来自索引，可能已经过时）。
/// 只检查正式文件是否占名；新序号的 `.kichi-part` 也要避开，免得和别的
/// 正在下载的任务抢同一个名字。
pub(super) fn finalize_part(part: &Path, preferred: &Path) -> Result<PathBuf, Failure> {
    let dir = preferred
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let final_path = available_final(dir, preferred);
    std::fs::rename(part, &final_path)
        .map_err(|err| Failure::Other(format!("保存下载文件失败：{err}")))?;
    Ok(final_path)
}

fn available_final(dir: &Path, preferred: &Path) -> PathBuf {
    if !preferred.exists() {
        return preferred.to_path_buf();
    }
    let Some(name) = preferred.file_name().and_then(|name| name.to_str()) else {
        return dir.join(format!("download-{}", std::process::id()));
    };
    let (stem, ext) = split_ext(name);
    for index in 1..10_000 {
        let candidate = dir.join(format!("{stem} ({index}){ext}"));
        if !candidate.exists() && !part_path_for(&candidate).exists() {
            return candidate;
        }
    }
    // 一万个同名文件还撞上：加进程号 + 毫秒时间戳兜底，总比覆盖强
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    dir.join(format!("{stem}-{}-{stamp}{ext}", std::process::id()))
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
#[path = "download_naming_tests.rs"]
mod tests;
