//! 云端搜索的底层零件：名字字面包含匹配 + 全盘列举的单页请求。
//!
//! **为什么不是「搜索接口」**（2026-10-26 真实账号只读探针，见项目笔记）：
//! 官方没有可用的按名字搜索接口——`filters={"name":{"contains":…}}` 回
//! 404 `file_invalid_file_filters`，而 `q` / `name` / `keyword` 参数被静默
//! 忽略。唯一可行的路径是 `parent_id=*` 全盘列举，再由客户端做包含匹配。
//!
//! 本模块只管「一次请求」与「一个纯函数」；逐页扫描、可中断、路径还原属于
//! 会话状态，放在 `kichi` 的 `session::search`。

use crate::drive::{DriveClient, FileDto};
use crate::types::FileEntry;
use crate::Error;

/// 官方全盘列举的父目录占位符：不是任何真实文件夹的 id。
pub const EVERYTHING_PARENT: &str = "*";

impl DriveClient {
    /// 全盘列举的一页（`parent_id=*`：根目录 + 所有子目录里的条目）。
    ///
    /// 官方**固定每页 400 条**（2026-10-26 实测），`next_page_token` 正常翻页；
    /// 与目录列表一样，条目自带 `parent_id`（见 [`FileHit`]），可据此还原真实路径。
    pub fn search_page(
        &self,
        query: &str,
        page_token: Option<&str>,
    ) -> Result<(Vec<FileHit>, Option<String>), Error> {
        if query.trim().is_empty() {
            return Err(Error::InvalidInput("搜索关键词不能为空".into()));
        }
        let query_params = search_query(page_token);
        let response: PageResponse =
            self.get_with_query(&self.config().files_url(), &query_params)?;
        if !response.kind.is_empty() && response.kind != "drive#fileList" {
            return Err(Error::ApiChanged(format!(
                "全盘列举返回了意外的类型：{}",
                response.kind
            )));
        }
        let files = response.files.into_iter().map(FileDto::into_hit).collect();
        let next = (!response.next_page_token.is_empty()).then_some(response.next_page_token);
        Ok((files, next))
    }
}

/// 全盘列举的一页结果：条目本身 + 官方给的父目录 id。
#[derive(Debug, Clone)]
pub struct FileHit {
    pub entry: FileEntry,
    /// `parent_id`：拼「所在文件夹」的真实路径要用；官方偶尔不给，按根目录处理
    pub parent_id: String,
}

/// 名字字面包含匹配（大小写不敏感）；文件与文件夹一视同仁。
///
/// **不做通配符**（用户决策）：`*`、`?` 就是普通字符。关键词首尾空白会被去掉，
/// 全空白视为「不匹配任何东西」。
pub fn matches(name: &str, query: &str) -> bool {
    let needle = query.trim();
    if needle.is_empty() {
        return false;
    }
    contains_ignore_case(name, needle)
}

fn contains_ignore_case(haystack: &str, needle: &str) -> bool {
    let hay = haystack.to_lowercase();
    let needle = needle.to_lowercase();
    hay.contains(&needle)
}

/// 全盘列举的查询参数（单独成函数便于单测）。
///
/// 与目录列表同一组过滤（只取已完成、未进回收站），区别只在于
/// `parent_id=*`。
pub(crate) fn search_query(page_token: Option<&str>) -> Vec<(String, String)> {
    let mut query = vec![
        ("limit".to_owned(), super::drive::LIST_LIMIT.to_string()),
        ("thumbnail_size".to_owned(), "SIZE_MEDIUM".to_owned()),
        ("with_audit".to_owned(), "true".to_owned()),
        ("filters".to_owned(), super::drive::LIST_FILTERS.to_owned()),
        ("parent_id".to_owned(), EVERYTHING_PARENT.to_owned()),
    ];
    if let Some(token) = page_token.map(str::trim).filter(|token| !token.is_empty()) {
        query.push(("page_token".to_owned(), token.to_owned()));
    }
    query
}

#[derive(serde::Deserialize)]
struct PageResponse {
    #[serde(default)]
    kind: String,
    #[serde(default)]
    files: Vec<FileDto>,
    #[serde(default)]
    next_page_token: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_is_literal_containment() {
        assert!(matches("汉化日记 第 1 季.mkv", "汉化日记"));
        assert!(matches("汉化日记 第 1 季.mkv", "日记 第"));
        assert!(!matches("其它名字.mkv", "汉化日记"));
        // 关键词首尾空白忽略
        assert!(matches("汉化日记.mkv", "  汉化日记 "));
    }

    #[test]
    fn matches_is_case_insensitive() {
        assert!(matches("My MOVIE.MKV", "my movie"));
        assert!(matches("my movie.mkv", "MOVIE"));
    }

    #[test]
    fn wildcards_are_ordinary_characters() {
        // 用户决策：不做通配符
        assert!(matches("a*b.mkv", "*"));
        assert!(!matches("ab.mkv", "a*b"), "`*` 不该当成任意字符");
        assert!(matches("第?集.mkv", "?"));
        assert!(!matches("第1集.mkv", "第?集"), "`?` 不该当成单字符");
    }

    #[test]
    fn matches_folders_and_files_alike() {
        // 匹配函数只看名字，类别由调用方决定（两者都要参与）
        assert!(matches("汉化日记（文件夹）", "汉化日记"));
    }

    #[test]
    fn empty_query_matches_nothing() {
        assert!(!matches("汉化日记.mkv", ""));
        assert!(!matches("汉化日记.mkv", "   "));
    }

    #[test]
    fn search_query_pins_the_everything_parent() {
        let query = search_query(None);
        let get = |key: &str| {
            query
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(get("parent_id"), Some("*"), "全盘列举必须用 parent_id=*");
        assert_eq!(get("limit"), Some("500"));
        assert_eq!(
            get("filters"),
            Some(r#"{"phase":{"eq":"PHASE_TYPE_COMPLETE"},"trashed":{"eq":false}}"#)
        );
        assert_eq!(get("page_token"), None, "第一页不带 token");

        let second = search_query(Some(" page-2 "));
        assert!(second.contains(&("page_token".to_owned(), "page-2".to_owned())));
    }
}
