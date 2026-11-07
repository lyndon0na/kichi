//! 从文件名里提取集数标签，仅用于播放列表显示。
//!
//! 不做自然语言理解：只认几种常见写法（`S01E02`、`第2集`、`[02]`、`EP07`、
//! ` - 05 `）。解析不出来就返回 `None`，由界面退回队列位置 `#N`，不猜。
//!
//! `EpisodeLabel` 同时给出显示文本和数值：数值用来判断「集数是否等于队列位置」，
//! 两者不一致时界面才额外显示一个很淡的 `#N`。

/// 文件名里常见的分辨率 / 年份数字，避免把 `[1080]`、`2024` 当成集数。
fn is_resolution_or_year(text: &str) -> bool {
    matches!(
        text,
        "480" | "540" | "576" | "720" | "1080" | "1440" | "2160"
    ) || text
        .parse::<u32>()
        .is_ok_and(|value| (1900..=2099).contains(&value))
}

/// 去掉文件扩展名；只去掉最后一段短后缀，避免把 `Show.S01E02` 的 `E02` 当成后缀。
fn stem(name: &str) -> &str {
    match name.rsplit_once('.') {
        Some((stem, extension))
            if !stem.is_empty()
                && extension.len() <= 5
                && extension.chars().all(|c| c.is_ascii_alphanumeric()) =>
        {
            stem
        }
        _ => name,
    }
}

/// 从文件名提取集数；显示文本与数值一起返回。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpisodeLabel {
    /// 给用户看的短标签：`02` / `S01E02` / `第2集` / `EP07`
    pub text: String,
    /// 集数数值，用于和队列位置比较
    pub number: u32,
}

impl EpisodeLabel {
    fn new(text: impl Into<String>, number: u32) -> Self {
        Self {
            text: text.into(),
            number,
        }
    }
}

/// 提取集数标签；优先 `S01E02`，其次 `第2集`、`[02]`、`EP07`、独立数字。
pub fn episode_label(name: &str) -> Option<EpisodeLabel> {
    let stem = stem(name);
    season_episode(stem)
        .or_else(|| chinese_episode(stem))
        .or_else(|| bracketed_number(stem))
        .or_else(|| prefixed_number(stem))
        .or_else(|| separated_number(stem))
}

/// `S01E02` / `s1e2`：返回原大小写统一成大写。
fn season_episode(text: &str) -> Option<EpisodeLabel> {
    let bytes = text.as_bytes();
    for (index, byte) in bytes.iter().enumerate() {
        if !matches!(byte, b's' | b'S') {
            continue;
        }
        if index > 0 && bytes[index - 1].is_ascii_alphanumeric() {
            continue;
        }
        let mut cursor = index + 1;
        let season_start = cursor;
        while cursor < bytes.len() && bytes[cursor].is_ascii_digit() && cursor - season_start < 2 {
            cursor += 1;
        }
        if cursor == season_start {
            continue;
        }
        if cursor >= bytes.len() || !matches!(bytes[cursor], b'e' | b'E') {
            continue;
        }
        cursor += 1;
        let episode_start = cursor;
        while cursor < bytes.len() && bytes[cursor].is_ascii_digit() && cursor - episode_start < 4 {
            cursor += 1;
        }
        if cursor == episode_start {
            continue;
        }
        if cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
            continue;
        }
        let number = text[episode_start..cursor].parse().ok()?;
        return Some(EpisodeLabel::new(
            text[index..cursor].to_ascii_uppercase(),
            number,
        ));
    }
    None
}

/// `第2集` / `第02话` / `第3回`。
fn chinese_episode(text: &str) -> Option<EpisodeLabel> {
    let mut start = 0;
    while let Some(offset) = text[start..].find('第') {
        let marker = start + offset;
        let mut cursor = marker + '第'.len_utf8();
        let digits_start = cursor;
        while cursor < text.len() && text.as_bytes()[cursor].is_ascii_digit() {
            cursor += 1;
        }
        if cursor == digits_start || cursor - digits_start > 4 {
            start = marker + '第'.len_utf8();
            continue;
        }
        for suffix in ["集", "话", "話", "回"] {
            if text[cursor..].starts_with(suffix) {
                let number = text[digits_start..cursor].parse().ok()?;
                return Some(EpisodeLabel::new(
                    &text[marker..cursor + suffix.len()],
                    number,
                ));
            }
        }
        start = marker + '第'.len_utf8();
    }
    None
}

/// `[02]` / `【03】` / `(04)`；跳过 `[1080]`、`[2024]` 这类明显不是集数的数字。
fn bracketed_number(text: &str) -> Option<EpisodeLabel> {
    for (open, close) in [('[', ']'), ('【', '】'), ('(', ')')] {
        let mut start = 0;
        while let Some(offset) = text[start..].find(open) {
            let open_at = start + offset;
            let content_start = open_at + open.len_utf8();
            let Some(end_offset) = text[content_start..].find(close) else {
                break;
            };
            let content_end = content_start + end_offset;
            let content = &text[content_start..content_end];
            if !content.is_empty()
                && content.len() <= 4
                && content.chars().all(|ch| ch.is_ascii_digit())
                && !is_resolution_or_year(content)
            {
                let number = content.parse().ok()?;
                return Some(EpisodeLabel::new(content, number));
            }
            start = content_end + close.len_utf8();
        }
    }
    None
}

/// `EP07` / `E07`（前面必须是分隔符或开头）。
fn prefixed_number(text: &str) -> Option<EpisodeLabel> {
    let bytes = text.as_bytes();
    for index in 0..bytes.len() {
        if !matches!(bytes[index], b'e' | b'E') {
            continue;
        }
        if index > 0 && bytes[index - 1].is_ascii_alphanumeric() {
            continue;
        }
        let has_p = index + 1 < bytes.len() && matches!(bytes[index + 1], b'p' | b'P');
        let digits_start = index + if has_p { 2 } else { 1 };
        let mut cursor = digits_start;
        while cursor < bytes.len() && bytes[cursor].is_ascii_digit() && cursor - digits_start < 4 {
            cursor += 1;
        }
        if cursor == digits_start {
            continue;
        }
        if cursor < bytes.len() && bytes[cursor].is_ascii_alphanumeric() {
            continue;
        }
        let number = text[digits_start..cursor].parse().ok()?;
        return Some(EpisodeLabel::new(
            text[index..cursor].to_ascii_uppercase(),
            number,
        ));
    }
    None
}

/// ` - 05 ` / `.05.` / `_05_`：1–3 位、前后都是分隔符的数字。
fn separated_number(text: &str) -> Option<EpisodeLabel> {
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if !bytes[index].is_ascii_digit() {
            index += 1;
            continue;
        }
        let start = index;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
        let digits = &text[start..index];
        let left_ok = start == 0 || !bytes[start - 1].is_ascii_alphanumeric();
        let right_ok = index == bytes.len() || !bytes[index].is_ascii_alphanumeric();
        if left_ok && right_ok && digits.len() <= 3 && !is_resolution_or_year(digits) {
            let number = digits.parse().ok()?;
            return Some(EpisodeLabel::new(digits, number));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{episode_label, EpisodeLabel};

    fn label(text: &str, number: u32) -> Option<EpisodeLabel> {
        Some(EpisodeLabel {
            text: text.to_owned(),
            number,
        })
    }

    #[test]
    fn bracket_numbers_are_found_before_resolution_tags() {
        assert_eq!(
            episode_label(
                "[Sakurato] Ichigo Aika: Zatsu de Namaiki na Imouto to Warikirenai Ani [02][AVC-8bit 1080P AAC][CHS].mp4"
            ),
            label("02", 2)
        );
        assert_eq!(episode_label("Show [12] [1080P].mkv"), label("12", 12));
        assert_eq!(episode_label("Show [1080] [2024].mkv"), None);
    }

    #[test]
    fn season_episode_is_preferred() {
        assert_eq!(episode_label("Show.S01E10.1080p.mkv"), label("S01E10", 10));
        assert_eq!(episode_label("show.s1e2.mp4"), label("S1E2", 2));
    }

    #[test]
    fn chinese_episode_markers_work() {
        assert_eq!(episode_label("第10集.mp4"), label("第10集", 10));
        assert_eq!(episode_label("动画 第2话.mp4"), label("第2话", 2));
        assert_eq!(episode_label("第03回.mkv"), label("第03回", 3));
    }

    #[test]
    fn prefixed_and_separated_numbers_work() {
        assert_eq!(episode_label("Show.EP07.mkv"), label("EP07", 7));
        assert_eq!(episode_label("Show - 05 [1080p].mkv"), label("05", 5));
        assert_eq!(episode_label("Show_03_.mkv"), label("03", 3));
    }

    #[test]
    fn resolution_and_year_alone_do_not_become_episodes() {
        assert_eq!(episode_label("Movie.2024.1080p.mkv"), None);
        assert_eq!(episode_label("Show [2160].mkv"), None);
        assert_eq!(episode_label("no episode.mkv"), None);
    }
}
