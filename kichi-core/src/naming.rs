//! 上传撞名时的候选名生成。
//!
//! PikPak 的上传票据和新建目录接口遇到同目录同名会返回
//! `file_duplicated_name`，不会自动改名。这里只生成 `(1)`、`(2)` 这类候选名，
//! 是否重试由调用方决定；始终不覆盖已有条目。

/// 官方按 UTF-8 字节计算的文件名上限。
pub(crate) const MAX_NAME_BYTES: usize = 255;

/// 最多尝试多少个候选名（含原名）。正常撞名一两次就会结束；上限只防止异常情况下无限请求。
pub(crate) const MAX_DUPLICATE_ATTEMPTS: u32 = 100;

/// 文件撞名候选：保留扩展名，把 ` (n)` 插在扩展名之前。
pub(crate) fn duplicate_file_name(base: &str, attempt: u32) -> String {
    if attempt == 0 {
        return base.to_owned();
    }
    let suffix = format!(" ({attempt})");
    let (stem, extension) = split_file_name(base);
    let extension = if extension.len() + suffix.len() < MAX_NAME_BYTES {
        extension
    } else {
        ""
    };
    let budget = MAX_NAME_BYTES.saturating_sub(suffix.len() + extension.len());
    format!("{}{}{}", truncate_utf8(stem, budget), suffix, extension)
}

/// 文件夹撞名候选：文件夹没有扩展名，` (n)` 直接追加在名字末尾。
pub(crate) fn duplicate_folder_name(base: &str, attempt: u32) -> String {
    if attempt == 0 {
        return base.to_owned();
    }
    let suffix = format!(" ({attempt})");
    let budget = MAX_NAME_BYTES.saturating_sub(suffix.len());
    format!("{}{}", truncate_utf8(base, budget), suffix)
}

fn split_file_name(name: &str) -> (&str, &str) {
    let Some((stem, extension)) = name.rsplit_once('.') else {
        return (name, "");
    };
    if stem.is_empty() || extension.is_empty() {
        return (name, "");
    }
    (stem, &name[stem.len()..])
}

fn truncate_utf8(text: &str, max_bytes: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_keep_their_extension() {
        assert_eq!(duplicate_file_name("电影.mkv", 1), "电影 (1).mkv");
        assert_eq!(
            duplicate_file_name("archive.tar.gz", 2),
            "archive.tar (2).gz"
        );
        assert_eq!(duplicate_file_name("README", 1), "README (1)");
        assert_eq!(duplicate_file_name(".env", 1), ".env (1)");
    }

    #[test]
    fn folder_names_append_the_suffix_at_the_end() {
        assert_eq!(duplicate_folder_name("剧集.S01", 1), "剧集.S01 (1)");
        assert_eq!(duplicate_folder_name("目录", 10), "目录 (10)");
    }

    #[test]
    fn long_names_stay_within_the_byte_limit() {
        let long = format!("{}.mkv", "字".repeat(85));
        let candidate = duplicate_file_name(&long, 1);
        assert!(candidate.len() <= MAX_NAME_BYTES);
        assert!(candidate.ends_with(" (1).mkv"));
        assert!(std::str::from_utf8(candidate.as_bytes()).is_ok());

        let folder = "字".repeat(85);
        let candidate = duplicate_folder_name(&folder, 1);
        assert!(candidate.len() <= MAX_NAME_BYTES);
        assert!(candidate.ends_with(" (1)"));
    }
}
