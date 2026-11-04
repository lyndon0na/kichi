//! 名字搜索的匹配档位与稳定排序（纯函数）。
//!
//! 用户 2026-11-04 定的规则：匹配质量优先——
//! 完全同名 > 名字前缀 > 词首匹配 > 任意位置包含；同档按名字排序，
//! 文件夹不因类型插队。这里只做「一条名字怎么排」，不做 IO。
//!
//! 「词首」的边界按 Unicode 字符判断：关键词前一个字符是空白或非字母数字
//! （空格、`.`、`-`、`(`、`「` 等）就算词首；`第` 这类 CJK 字母不算分隔符。
//! 匹配与排序都用同一套归一化（NFKC、片假名折平假名、繁简归并、小写），
//! 因此全角 / 假名 / 繁体输入与索引里的简体写法能落到同一档。

use std::cmp::Ordering;

use crate::normalize::{normalize_name, normalize_query};

/// 匹配档位；声明顺序就是排序优先级（越靠前质量越高）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MatchTier {
    /// 归一化后与关键词完全相同
    Exact,
    /// 归一化后的名字以关键词开头
    Prefix,
    /// 关键词出现在空白 / 非字母数字分隔符之后
    WordStart,
    /// 任意位置包含
    Contains,
}

/// 已归一化关键词的匹配器；同一次搜索复用，避免每个名字都重新归一化关键词。
#[derive(Debug, Clone)]
pub struct SearchMatcher {
    needle: String,
}

impl SearchMatcher {
    /// 关键词归一化后为空（纯空白）时返回 `None`。
    pub fn new(query: &str) -> Option<Self> {
        let needle = normalize_query(query);
        (!needle.is_empty()).then_some(Self { needle })
    }

    /// 这条名字落在哪一档；不匹配时为 `None`。
    pub fn tier(&self, name: &str) -> Option<MatchTier> {
        tier_of_normalized(&normalize_name(name), &self.needle)
    }

    pub fn matches(&self, name: &str) -> bool {
        self.tier(name).is_some()
    }

    /// 排序键：档位优先，同档按展示名（Unicode 小写）稳定排序。
    ///
    /// 调用方不再自己拼排序规则；把键交给 `sort_by_key` 即可，输入顺序
    /// （SQL 的 `name COLLATE NOCASE, id`）会作为完全相等的最后兜底。
    pub fn sort_key(&self, name: &str) -> Option<(MatchTier, String)> {
        self.tier(name).map(|tier| (tier, name.to_lowercase()))
    }

    /// 比较两条名字在该关键词下的先后；用于库内排序与单测。
    ///
    /// 不匹配的名字排在最后，两者都不匹配时按名字排序。
    pub fn compare(&self, left: &str, right: &str) -> Ordering {
        match (self.sort_key(left), self.sort_key(right)) {
            (Some(a), Some(b)) => a.cmp(&b),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => left.to_lowercase().cmp(&right.to_lowercase()),
        }
    }
}

/// 单条匹配的档位（每次都会归一化关键词；批量场景用 [`SearchMatcher`]）。
pub fn match_tier(name: &str, query: &str) -> Option<MatchTier> {
    SearchMatcher::new(query)?.tier(name)
}

fn tier_of_normalized(name: &str, needle: &str) -> Option<MatchTier> {
    if !name.contains(needle) {
        return None;
    }
    if name == needle {
        return Some(MatchTier::Exact);
    }
    if name.starts_with(needle) {
        return Some(MatchTier::Prefix);
    }
    if name
        .match_indices(needle)
        .any(|(index, _)| starts_word(name, index))
    {
        return Some(MatchTier::WordStart);
    }
    Some(MatchTier::Contains)
}

fn starts_word(name: &str, index: usize) -> bool {
    name[..index]
        .chars()
        .next_back()
        .is_some_and(|previous| previous.is_whitespace() || !previous.is_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiers_are_exact_prefix_word_start_contains() {
        let matcher = SearchMatcher::new("测试").expect("关键词");
        assert_eq!(matcher.tier("测试"), Some(MatchTier::Exact));
        assert_eq!(matcher.tier("测试文件.mkv"), Some(MatchTier::Prefix));
        assert_eq!(matcher.tier("第 1 集 测试"), Some(MatchTier::WordStart));
        assert_eq!(matcher.tier("这是测试文件"), Some(MatchTier::Contains));
        assert_eq!(matcher.tier("无关名字"), None);
    }

    #[test]
    fn separators_mark_word_starts_but_letters_do_not() {
        let cases = [
            ("汉化日记 第 1 季", "第"),
            ("Show.S01E01", "s01"),
            ("Show-S01E01", "s01"),
            ("Show(S01)", "s01"),
            ("「测试」文件", "测试"),
        ];
        for (name, query) in cases {
            assert_eq!(
                match_tier(name, query),
                Some(MatchTier::WordStart),
                "{name} 里的 {query} 应算词首"
            );
        }
        assert_eq!(
            match_tier("ShowS01E01", "s01"),
            Some(MatchTier::Contains),
            "前一个字符是字母时只能算包含"
        );
        assert_eq!(
            match_tier("第1集", "1"),
            Some(MatchTier::Contains),
            "CJK 字母不是分隔符"
        );
    }

    #[test]
    fn normalization_applies_to_both_sides() {
        assert_eq!(
            match_tier("測試檔案.MKV", "测试档案"),
            Some(MatchTier::Prefix)
        );
        assert_eq!(
            match_tier("テスト 第１話", "てすと"),
            Some(MatchTier::Prefix)
        );
        assert_eq!(match_tier("ＡＢＣ.mkv", "abc"), Some(MatchTier::Prefix));
    }

    #[test]
    fn sort_key_puts_better_tiers_first_then_name() {
        let matcher = SearchMatcher::new("第").expect("关键词");
        let mut names = vec!["包含第", "第 1 集", "第", "第 2 集", "其它 第"];
        names.sort_by(|a, b| matcher.compare(a, b));
        assert_eq!(names, vec!["第", "第 1 集", "第 2 集", "其它 第", "包含第"]);
    }

    #[test]
    fn same_tier_sorts_by_case_insensitive_name() {
        let matcher = SearchMatcher::new("abc").expect("关键词");
        let mut names = vec!["ABC 2", "abc 1", "Abc 3"];
        names.sort_by(|a, b| matcher.compare(a, b));
        assert_eq!(names, vec!["abc 1", "ABC 2", "Abc 3"]);
    }

    #[test]
    fn empty_query_never_matches() {
        assert!(SearchMatcher::new("").is_none());
        assert!(SearchMatcher::new("   ").is_none());
        assert_eq!(match_tier("名字", "  "), None);
    }
}
