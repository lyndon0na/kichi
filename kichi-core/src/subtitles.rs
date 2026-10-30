//! 外挂字幕的识别与同名匹配（纯函数，无网络、无 Qt）。
//!
//! 官方 API 不下发字幕 URL；字幕就是网盘里的普通文件。这里只回答
//! 「目录里的哪些文件是这部电影的字幕、先选哪条」，下载与挂载在 GUI crate。

use crate::types::{EntryKind, FileEntry};

/// mpv 能读的外挂字幕扩展名（与 `--sub-auto-exts` 对齐）。
pub const SUBTITLE_EXTENSIONS: &[&str] = &[
    "srt", "ass", "ssa", "vtt", "smi", "sub", "sup", "pgs", "mks",
];

/// 中文语言标记（ASCII token，按 `.` / `-` / `_` / 空格 / `&` / `+` 切分后精确匹配）。
const CHINESE_ASCII: &[&str] = &[
    "zh", "zho", "chi", "chs", "cht", "sc", "tc", "cn", "hans", "hant", "chinese",
];

/// 中文语言标记（非 ASCII，按后缀包含判断）。
const CHINESE_TEXT: &[&str] = &[
    "简体", "簡體", "繁体", "繁體", "中文", "中字", "简中", "繁中", "简", "繁", "中",
];

/// 文件名去掉最后一段扩展名。
pub fn stem(name: &str) -> &str {
    name.rsplit_once('.').map_or(name, |(stem, _)| stem)
}

/// 是否是字幕文件；返回小写规范扩展名。
pub fn subtitle_extension(name: &str) -> Option<&'static str> {
    let (_, extension) = name.rsplit_once('.')?;
    let extension = extension.trim();
    SUBTITLE_EXTENSIONS
        .iter()
        .copied()
        .find(|known| known.eq_ignore_ascii_case(extension))
}

/// 是否是字幕文件名（不看条目类别）。
pub fn is_subtitle_name(name: &str) -> bool {
    subtitle_extension(name).is_some()
}

/// 是否视频文件：只有视频才尝试挂外挂字幕，音频不挂。
pub fn is_video_name(name: &str) -> bool {
    let Some((_, extension)) = name.rsplit_once('.') else {
        return false;
    };
    let extension = extension.trim().to_ascii_lowercase();
    crate::drive::VIDEO_EXTENSIONS.contains(&extension.as_str())
}

/// 字幕是不是这部电影的：字幕 stem 等于视频 stem，或以 `视频stem + "."` 开头。
///
/// 不做模糊 / 编辑距离匹配：`Movie2.srt` 不会配给 `Movie.mkv`（避免跨集错配）。
pub fn matches(video_name: &str, subtitle_name: &str) -> bool {
    subtitle_suffix(video_name, subtitle_name).is_some()
}

/// 语言优先级：0 = 中文，1 = 无语言标记，2 = 其它语言。
pub fn language_rank(video_name: &str, subtitle_name: &str) -> u8 {
    let Some(suffix) = subtitle_suffix(video_name, subtitle_name) else {
        return 2;
    };
    if suffix.is_empty() {
        return 1;
    }
    let lower = suffix.to_ascii_lowercase();
    if lower
        .split(['.', '-', '_', ' ', '&', '+'])
        .any(|token| CHINESE_ASCII.contains(&token))
    {
        return 0;
    }
    if CHINESE_TEXT.iter().any(|mark| suffix.contains(mark)) {
        return 0;
    }
    2
}

/// 选出这部电影的字幕。
///
/// 有同名匹配就返回全部（中文优先、再按文件名稳定排序）；没有同名匹配、
/// 但目录里恰好只有一条字幕时也返回它（唯一兜底）；否则空。
pub fn matching_subtitles<'a>(video_name: &str, entries: &'a [FileEntry]) -> Vec<&'a FileEntry> {
    let subtitles: Vec<&FileEntry> = entries
        .iter()
        .filter(|entry| entry.kind == EntryKind::File && is_subtitle_name(&entry.name))
        .collect();
    let mut matched: Vec<&FileEntry> = subtitles
        .iter()
        .copied()
        .filter(|entry| matches(video_name, &entry.name))
        .collect();
    if matched.is_empty() {
        return if subtitles.len() == 1 {
            subtitles
        } else {
            Vec::new()
        };
    }
    matched.sort_by(|left, right| {
        language_rank(video_name, &left.name)
            .cmp(&language_rank(video_name, &right.name))
            .then_with(|| {
                left.name
                    .to_ascii_lowercase()
                    .cmp(&right.name.to_ascii_lowercase())
            })
    });
    matched
}

/// 字幕 stem 中「视频 stem 之后」的部分（去掉一个分隔点）。
///
/// `Movie.mkv` + `Movie.zh.srt` → `Some("zh")`；`Movie.mkv` + `Movie.srt` → `Some("")`；
/// `Movie.mkv` + `Movie2.srt` → `None`。
fn subtitle_suffix<'a>(video_name: &str, subtitle_name: &'a str) -> Option<&'a str> {
    let video_stem = stem(video_name);
    if video_stem.is_empty() {
        return None;
    }
    let full = stem(subtitle_name);
    let prefix = full.get(..video_stem.len())?;
    if !prefix.eq_ignore_ascii_case(video_stem) {
        return None;
    }
    let rest = &full[video_stem.len()..];
    if rest.is_empty() {
        return Some(rest);
    }
    rest.strip_prefix('.')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str) -> FileEntry {
        FileEntry {
            id: name.to_owned(),
            name: name.to_owned(),
            kind: EntryKind::File,
            size: 0,
            modified_at: None,
            md5: None,
            audit_flagged: false,
            media: false,
            video: false,
            thumbnail_url: String::new(),
        }
    }

    fn names<'a>(entries: &[&'a FileEntry]) -> Vec<&'a str> {
        entries.iter().map(|entry| entry.name.as_str()).collect()
    }

    #[test]
    fn recognizes_subtitle_extensions_case_insensitively() {
        assert_eq!(subtitle_extension("Movie.SRT"), Some("srt"));
        assert_eq!(subtitle_extension("Movie.chs.ass"), Some("ass"));
        assert_eq!(subtitle_extension("Movie.vtt "), Some("vtt"));
        assert_eq!(subtitle_extension("Movie.mkv"), None);
        assert_eq!(subtitle_extension("README"), None);
        assert!(!is_subtitle_name("Movie.mkv"));
    }

    #[test]
    fn only_video_extensions_are_videos() {
        assert!(is_video_name("Movie.MKV"));
        assert!(is_video_name("Show.ts"));
        assert!(!is_video_name("song.flac"));
        assert!(!is_video_name("Movie.srt"));
        assert!(!is_video_name("README"));
    }

    #[test]
    fn same_stem_matches_case_insensitively() {
        assert!(matches("Movie.mkv", "Movie.srt"));
        assert!(matches("Movie.mkv", "movie.zh.srt"));
        assert!(matches("Movie.mkv", "Movie.CHS.ass"));
        assert!(matches("Movie.mkv", "Movie.zh-Hans.srt"));
        assert!(!matches("Movie.mkv", "Movie2.srt"));
        assert!(!matches("Movie.mkv", "Other.srt"));
    }

    #[test]
    fn language_rank_prefers_chinese() {
        assert_eq!(language_rank("Movie.mkv", "Movie.zh.srt"), 0);
        assert_eq!(language_rank("Movie.mkv", "Movie.zho.ass"), 0);
        assert_eq!(language_rank("Movie.mkv", "Movie.chs&eng.srt"), 0);
        assert_eq!(language_rank("Movie.mkv", "Movie.zh_CN.srt"), 0);
        assert_eq!(language_rank("Movie.mkv", "Movie.chinese.srt"), 0);
        assert_eq!(language_rank("Movie.mkv", "Movie.简体.srt"), 0);
        assert_eq!(language_rank("Movie.mkv", "Movie.中字.ass"), 0);
        assert_eq!(language_rank("Movie.mkv", "Movie.srt"), 1);
        assert_eq!(language_rank("Movie.mkv", "Movie.en.srt"), 2);
        assert_eq!(language_rank("Movie.mkv", "Movie.en.forced.srt"), 2);
    }

    #[test]
    fn chinese_rank_ignores_markers_in_the_title() {
        // 片名本身含 `zh` / `中文` 时，不能把无语言后缀的字幕误判成中文
        assert_eq!(language_rank("zh.电影.mkv", "zh.电影.srt"), 1);
        assert_eq!(language_rank("中文电影.mkv", "中文电影.srt"), 1);
        assert_eq!(language_rank("中文电影.mkv", "中文电影.zh.srt"), 0);
    }

    #[test]
    fn matching_sorts_chinese_then_plain_then_other() {
        let entries = [
            file("Show.en.srt"),
            file("Show.zh-Hant.ass"),
            file("Show.zh-Hans.srt"),
            file("Show.srt"),
            file("Show.mkv"),
        ];
        let got = matching_subtitles("Show.mkv", &entries);
        assert_eq!(
            names(&got),
            vec![
                "Show.zh-Hans.srt",
                "Show.zh-Hant.ass",
                "Show.srt",
                "Show.en.srt"
            ]
        );
    }

    #[test]
    fn unique_fallback_needs_exactly_one_subtitle() {
        let only_video = [file("Movie.mkv")];
        assert!(matching_subtitles("Movie.mkv", &only_video).is_empty());

        let one = [file("Movie.mkv"), file("Other.srt")];
        assert_eq!(
            names(&matching_subtitles("Movie.mkv", &one)),
            vec!["Other.srt"]
        );

        let two = [file("Movie.mkv"), file("A.srt"), file("B.srt")];
        assert!(matching_subtitles("Movie.mkv", &two).is_empty());
    }

    #[test]
    fn folders_and_audio_are_not_subtitles() {
        let mut folder = file("Movie.srt");
        folder.kind = EntryKind::Folder;
        let entries = [file("Movie.mkv"), folder];
        assert!(matching_subtitles("Movie.mkv", &entries).is_empty());

        let entries = [file("song.flac"), file("song.srt")];
        assert!(matching_subtitles("song.flac", &entries).len() == 1);
        assert!(!is_video_name("song.flac"));
    }

    #[test]
    fn real_release_names_with_case_differences_match() {
        // 实机遇到的场景：视频写 `Bang`、字幕写 `BanG`，必须都挂上（SC / TC）
        let video = "[LoliHouse] Bang Dream! 3rd Season - 01 [BDRip 1080p HEVC-10bit FLAC].mkv";
        let entries = [
            file(video),
            file("[LoliHouse] BanG Dream! 3rd Season - 01 [BDRip 1080p HEVC-10bit FLAC].SC.ass"),
            file("[LoliHouse] BanG Dream! 3rd Season - 01 [BDRip 1080p HEVC-10bit FLAC].TC.ass"),
        ];
        let matched = matching_subtitles(video, &entries);
        let got = names(&matched);
        assert_eq!(got.len(), 2, "{got:?}");
        assert!(got.iter().all(|name| name.ends_with(".ass")));
        assert!(got.iter().all(|name| language_rank(video, name) == 0));
    }

    #[test]
    fn same_stem_without_extension_is_not_a_subtitle() {
        let entries = [file("Movie.mkv"), file("Movie")];
        assert!(matching_subtitles("Movie.mkv", &entries).is_empty());
    }
}
