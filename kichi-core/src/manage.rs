//! 网盘侧写操作：重命名 / 新建文件夹 / 移入回收站。
//!
//! 读操作在 `drive.rs`，这里只放会改云端数据的部分。
//! 单独成文件：一是职责清晰，二是将来加「彻底删除」「移动」「复制」时好扩展。
//!
//! **删除语义**：PikPak 的删除是「移入回收站」（`batchTrash`），官方回收站
//! 有保留期，空间不会立刻归还，但可以恢复。不可恢复的 `batchDelete`
//! 这里不提供——误删代价太大，界面上也不该有这条路径。

use std::collections::BTreeSet;

use crate::drive::DriveClient;
use crate::{Config, Error};

/// 文件名的最大字节数（官方按 UTF-8 字节算，超出直接拒绝）。
const MAX_NAME_BYTES: usize = 255;
/// 一次性最多操作多少个条目（官方批量接口的实际上限附近）。
const MAX_BATCH: usize = 100;

impl DriveClient {
    /// 给文件或文件夹改名。
    pub fn rename(&self, id: &str, name: &str) -> Result<(), Error> {
        let id = require_id(id)?;
        let name = validate_name(name)?;
        let body = serde_json::json!({ "name": name });
        self.request_no_response_body(
            reqwest::Method::PATCH,
            &Config::file_url(self.config(), id),
            &[],
            &body,
        )
    }

    /// 在 `parent_id`（`None` 或空串 = 根目录）下新建一个文件夹。
    pub fn create_folder(&self, parent_id: Option<&str>, name: &str) -> Result<(), Error> {
        let name = validate_name(name)?;
        let mut body = serde_json::json!({
            "kind": "drive#folder",
            "name": name,
        });
        if let Some(parent) = parent_id.map(str::trim).filter(|id| !id.is_empty()) {
            body["parent_id"] = serde_json::Value::String(parent.to_owned());
        }
        self.request_no_response_body(
            reqwest::Method::POST,
            &Config::files_url(self.config()),
            &[],
            &body,
        )
    }

    /// 把条目移入回收站（可恢复）。重复 id 会被去重；空列表视为非法调用。
    pub fn trash(&self, ids: &[&str]) -> Result<(), Error> {
        let ids = normalize_ids(ids)?;
        self.post_ids("batchTrash", &ids)
    }

    /// 从回收站恢复条目（暂未在界面上开放，留给后续）。
    pub fn untrash(&self, ids: &[&str]) -> Result<(), Error> {
        let ids = normalize_ids(ids)?;
        self.post_ids("batchUntrash", &ids)
    }

    fn post_ids(&self, action: &str, ids: &[String]) -> Result<(), Error> {
        let body = serde_json::json!({ "ids": ids });
        self.request_no_response_body(
            reqwest::Method::POST,
            &Config::file_action_url(self.config(), action),
            &[],
            &body,
        )
    }
}

/// 校验并规范化重命名 / 新建时给的名字。
///
/// 规则在客户端先挡一遍，避免把明显无效的请求发出去（省一次往返，
/// 也免得官方用难懂的错误码回复）。首尾空格会被去掉。
fn validate_name(raw: &str) -> Result<&str, Error> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(Error::InvalidInput("名字不能为空".into()));
    }
    if name.contains('/') {
        return Err(Error::InvalidInput("名字里不能包含 /".into()));
    }
    if name.chars().any(|c| c.is_control()) {
        return Err(Error::InvalidInput("名字里不能包含控制字符".into()));
    }
    if name.len() > MAX_NAME_BYTES {
        return Err(Error::InvalidInput(format!(
            "名字太长（最多 {MAX_NAME_BYTES} 字节，中文一个字算 3 字节）"
        )));
    }
    Ok(name)
}

fn require_id(id: &str) -> Result<&str, Error> {
    let id = id.trim();
    if id.is_empty() {
        return Err(Error::InvalidInput("条目 id 为空".into()));
    }
    Ok(id)
}

/// 去空、去重复、限数量。用 `BTreeSet` 顺带得到稳定顺序，便于测试与排查。
fn normalize_ids(ids: &[&str]) -> Result<Vec<String>, Error> {
    let set: BTreeSet<String> = ids
        .iter()
        .map(|id| id.trim())
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        .collect();
    if set.is_empty() {
        return Err(Error::InvalidInput("没有要操作的条目".into()));
    }
    if set.len() > MAX_BATCH {
        return Err(Error::InvalidInput(format!(
            "一次最多操作 {MAX_BATCH} 个条目（当前 {} 个）",
            set.len()
        )));
    }
    Ok(set.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_validated_before_sending() {
        assert!(validate_name("  电影  ").is_ok());
        assert!(validate_name("").is_err(), "空名字要挡掉");
        assert!(validate_name("   ").is_err(), "全空格等于空");
        assert!(validate_name("a/b").is_err(), "不能带路径分隔符");
        assert!(validate_name("a\nb").is_err(), "不能带控制字符");
        assert!(validate_name(&"字".repeat(86)).is_err(), "超 255 字节要挡");
        assert!(validate_name(&"字".repeat(85)).is_ok(), "85 字 = 255 字节");
    }

    #[test]
    fn name_is_trimmed() {
        assert_eq!(validate_name("  x  ").ok(), Some("x"));
    }

    #[test]
    fn ids_are_deduped_and_trimmed() {
        let ids = normalize_ids(&[" b ", "a", "b", ""]).expect("应合法");
        assert_eq!(ids, vec!["a".to_owned(), "b".to_owned()], "去重且有序");
    }

    #[test]
    fn empty_id_list_is_rejected() {
        assert!(normalize_ids(&[]).is_err());
        assert!(normalize_ids(&["", "  "]).is_err());
    }

    #[test]
    fn batch_is_capped() {
        let ids: Vec<String> = (0..101).map(|i| format!("id{i}")).collect();
        let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        assert!(normalize_ids(&refs).is_err(), "超过 100 个要挡");
    }

    #[test]
    fn empty_id_is_rejected() {
        assert!(require_id(" ").is_err());
        assert_eq!(require_id(" abc ").ok(), Some("abc"));
    }
}
