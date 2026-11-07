//! 文件名自然排序：忽略大小写，连续数字按数值比较。
//!
//! 普通字典序会把 `第10集` 排在 `第2集` 前面，文件夹连播因此会乱跳。
//! 这里只做比较，不改变任何显示文本；文件列表和播放队列共用同一份规则。

use std::cmp::Ordering;
use std::iter::Peekable;

/// 比较两个文件名：大小写不敏感，连续 ASCII 数字按数值大小比较。
///
/// 数值相同（例如 `1` 与 `01`）时再按原始数字串比较，保证排序稳定。
pub fn natural_cmp(left: &str, right: &str) -> Ordering {
    let left = left.to_lowercase();
    let right = right.to_lowercase();
    let mut left = left.chars().peekable();
    let mut right = right.chars().peekable();
    loop {
        match (left.peek().copied(), right.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(a), Some(b)) if a.is_ascii_digit() && b.is_ascii_digit() => {
                let a_run = take_digits(&mut left);
                let b_run = take_digits(&mut right);
                let order = numeric_order(&a_run, &b_run);
                if order != Ordering::Equal {
                    return order;
                }
            }
            (Some(a), Some(b)) => {
                left.next();
                right.next();
                let order = a.cmp(&b);
                if order != Ordering::Equal {
                    return order;
                }
            }
        }
    }
}

fn take_digits<I: Iterator<Item = char>>(iter: &mut Peekable<I>) -> String {
    let mut run = String::new();
    while let Some(&ch) = iter.peek() {
        if !ch.is_ascii_digit() {
            break;
        }
        run.push(ch);
        iter.next();
    }
    run
}

fn numeric_order(left: &str, right: &str) -> Ordering {
    let left_number = left.trim_start_matches('0');
    let right_number = right.trim_start_matches('0');
    left_number
        .len()
        .cmp(&right_number.len())
        .then_with(|| left_number.cmp(right_number))
        .then_with(|| left.cmp(right))
}

#[cfg(test)]
mod tests {
    use super::natural_cmp;
    use std::cmp::Ordering;

    fn sorted(mut names: Vec<&str>) -> Vec<&str> {
        names.sort_by(|a, b| natural_cmp(a, b));
        names
    }

    #[test]
    fn episode_numbers_sort_by_value() {
        assert_eq!(
            sorted(vec!["第10集.mkv", "第2集.mkv", "第1集.mkv"]),
            vec!["第1集.mkv", "第2集.mkv", "第10集.mkv"]
        );
        assert_eq!(
            sorted(vec!["S01E10.mkv", "S01E02.mkv", "S01E01.mkv"]),
            vec!["S01E01.mkv", "S01E02.mkv", "S01E10.mkv"]
        );
    }

    #[test]
    fn comparison_is_case_insensitive() {
        assert_eq!(natural_cmp("Alpha", "alpha"), Ordering::Equal);
        assert_eq!(natural_cmp("alpha.mkv", "Beta.mkv"), Ordering::Less);
    }

    #[test]
    fn shorter_text_comes_first_when_prefix_is_equal() {
        assert_eq!(natural_cmp("a", "a1"), Ordering::Less);
        assert_eq!(natural_cmp("ep", "episode"), Ordering::Less);
    }

    #[test]
    fn leading_zeros_stay_deterministic() {
        // 数值相同，再按原始数字串比较：`01` 排在 `1` 前面。
        assert_eq!(natural_cmp("第01集", "第1集"), Ordering::Less);
        assert_eq!(natural_cmp("第02集", "第2集"), Ordering::Less);
        assert_eq!(natural_cmp("第2集", "第02集"), Ordering::Greater);
    }

    #[test]
    fn numbers_inside_names_do_not_break_suffix_order() {
        assert_eq!(
            sorted(vec!["x9y", "x10a", "x10b"]),
            vec!["x9y", "x10a", "x10b"]
        );
    }
}
