//! 名字归一化：搜索使用的纯函数。
//!
//! 归一化只影响索引里的 `norm_name`，不改变展示用的原始名字。当前规则：
//! NFKC（全角/半角、组合字符）→ 片假名折平假名 → 繁简归并 → 小写。
//!
//! 繁简表是**人工挑选的常用字**，不追求 OpenCC 级别的覆盖；第一版也
//! 不做跳跃子序列（`abc` 不匹配 `a1b2c3`）。扩展表时只追加，见
//! `normalize_table.rs` 顶部说明。

use crate::normalize_table::simplify_traditional;

/// 把名字归一化成索引键。结果可重复归一化（幂等）。
pub fn normalize_name(name: &str) -> String {
    let normalized = icu_normalizer::ComposingNormalizerBorrowed::new_nfkc().normalize(name);
    let mut out = String::with_capacity(normalized.len());
    for ch in normalized.chars() {
        // 小写可能展开成多个字符（例如带附标的大写字母），逐个过繁简表
        for lower in fold_katakana(ch).to_lowercase() {
            out.push(simplify_traditional(lower));
        }
    }
    out
}

/// 搜索关键词的归一化：与 [`normalize_name`] 同一套规则，额外去掉首尾空白。
pub fn normalize_query(query: &str) -> String {
    normalize_name(query.trim())
}

/// 片假名 → 平假名；其余字符原样返回。
///
/// NFKC 已把半角片假名转成全角；这里只需处理全角区间。ヷヸヹヺ 没有
/// 对应平假名，保持原样。
fn fold_katakana(ch: char) -> char {
    match ch {
        '\u{30A1}'..='\u{30F6}' => char::from_u32(ch as u32 - 0x60).unwrap_or(ch),
        '\u{30FD}' => '\u{309D}',
        '\u{30FE}' => '\u{309E}',
        _ => ch,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nfkc_folds_full_width_forms() {
        assert_eq!(normalize_name("ｅｆｆｅｃｔ．ｍｋｖ"), "effect.mkv");
        assert_eq!(normalize_name("第１話"), "第1话");
    }

    #[test]
    fn katakana_is_folded_to_hiragana() {
        assert_eq!(normalize_name("テスト"), "てすと");
        assert_eq!(normalize_name("ヴ"), "ゔ");
        assert_eq!(normalize_name("ｱﾆﾒ"), "あにめ");
    }

    #[test]
    fn traditional_and_simplified_share_one_key() {
        assert_eq!(normalize_name("測試檔案"), normalize_name("测试档案"));
        assert_eq!(normalize_name("電影片"), "电影片");
        assert_eq!(normalize_name("資料庫"), "资料库");
    }

    #[test]
    fn case_is_folded() {
        assert_eq!(normalize_name("ABC.MKV"), "abc.mkv");
        assert_eq!(normalize_name("Straße"), "straße");
    }

    #[test]
    fn query_trims_but_name_does_not() {
        assert_eq!(normalize_query("  測試  "), "测试");
        assert_eq!(normalize_name(" 測試 "), " 测试 ");
    }

    #[test]
    fn normalization_is_idempotent() {
        for text in ["測試檔案.MKV", "テスト 第１話", "ＡＢＣ"] {
            let once = normalize_name(text);
            assert_eq!(normalize_name(&once), once, "{text} 二次归一化应保持不变");
        }
    }
}
