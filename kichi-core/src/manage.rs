//! 网盘侧写操作：重命名 / 新建文件夹 / 移动 / 复制 / 回收站恢复与永久删除。
//!
//! 读操作在 `drive.rs`，这里只放会改云端数据的部分。
//! 单独成文件：一是职责清晰，二是将来越界操作（彻底删除等）好扩展。
//!
//! **删除语义**：普通文件页的删除只调 `batchTrash`（移入回收站）。
//! 回收站页可显式调用 `batchUntrash` 或 `batchDelete`；后者不可恢复，
//! 只能由回收站专用界面在二次确认后调用。

use std::collections::BTreeSet;

use serde::Deserialize;

use crate::drive::DriveClient;
use crate::http::Body;
use crate::{Config, Error};

/// 文件名的最大字节数（官方按 UTF-8 字节算，超出直接拒绝）。
const MAX_NAME_BYTES: usize = 255;
/// 一次性最多操作多少个条目（官方批量接口的实际上限附近）。
const MAX_BATCH: usize = 100;

/// 批量移动 / 复制的动作名，拼在 files URL 后面（`…/files:batchMove`）。
const ACTION_MOVE: &str = "batchMove";
const ACTION_COPY: &str = "batchCopy";

#[derive(Deserialize)]
struct BatchTaskAck {
    #[serde(default)]
    task_id: String,
}

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

    /// 把条目移到 `parent_id` 下。空串表示根目录。
    ///
    /// 目标不能是条目当前所在目录，也不能是条目自己的子目录——官方会返回
    /// `file_move_or_copy_to_cur`（code 9），这里只负责发请求，判断交给调用方
    /// （客户端先挡一次，服务端再兜底）。
    pub fn move_to(&self, ids: &[&str], parent_id: &str) -> Result<(), Error> {
        self.batch_to(ACTION_MOVE, ids, parent_id)
    }

    /// 把条目复制到 `parent_id` 下。空串表示根目录。
    pub fn copy_to(&self, ids: &[&str], parent_id: &str) -> Result<(), Error> {
        self.batch_to(ACTION_COPY, ids, parent_id)
    }

    fn batch_to(&self, action: &str, ids: &[&str], parent_id: &str) -> Result<(), Error> {
        let body = batch_body(ids, parent_id)?;
        self.request_no_response_body(
            reqwest::Method::POST,
            &Config::file_action_url(self.config(), action),
            &[],
            &body,
        )
    }

    /// 把条目移入回收站（可恢复）。重复 id 会被去重；空列表视为非法调用。
    pub fn trash(&self, ids: &[&str]) -> Result<(), Error> {
        let ids = normalize_ids(ids)?;
        self.post_ids("batchTrash", &ids)
    }

    /// 从回收站恢复条目。
    ///
    /// rclone 明确指出 `batchUntrash` **不保证**回到删除前的原目录，撞名时
    /// PikPak 还可能自动追加编号。界面因此只承诺“还原”，不承诺落点。
    pub fn untrash(&self, ids: &[&str]) -> Result<(), Error> {
        let ids = normalize_ids(ids)?;
        self.post_ids("batchUntrash", &ids)
    }

    /// 从回收站永久删除条目。不可恢复，仅供回收站页在确认后调用。
    pub fn delete_permanently(&self, ids: &[&str]) -> Result<(), Error> {
        let ids = normalize_ids(ids)?;
        self.post_ids("batchDelete", &ids)
    }

    /// 一次性清空整个账号的回收站。服务端返回后台任务，完成后才返回成功。
    pub fn empty_trash(&self) -> Result<(), Error> {
        let response: BatchTaskAck = self.request(
            reqwest::Method::PATCH,
            &Config::trash_empty_url(self.config()),
            &[],
            Body::Empty,
        )?;
        self.wait_task(&response.task_id)
    }

    fn post_ids(&self, action: &str, ids: &[String]) -> Result<(), Error> {
        let body = serde_json::json!({ "ids": ids });
        let response: BatchTaskAck = self.request(
            reqwest::Method::POST,
            &Config::file_action_url(self.config(), action),
            &[],
            Body::Json(&body),
        )?;
        self.wait_task(&response.task_id)
    }
}

/// 批量移动 / 复制的请求体。
///
/// 形状由 rclone（MIT）的实践确认：`{"ids":[…],"to":{"parent_id":"目标 id"}}`，
/// 根目录传空串。单独抽成纯函数，便于不经过网络断言参数。
fn batch_body(ids: &[&str], parent_id: &str) -> Result<serde_json::Value, Error> {
    let ids = normalize_ids(ids)?;
    Ok(serde_json::json!({
        "ids": ids,
        "to": { "parent_id": parent_id.trim() },
    }))
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

    #[test]
    fn batch_body_nests_target_parent() {
        let body = batch_body(&[" b ", "a", "b"], " dest ").expect("应合法");
        assert_eq!(
            body,
            serde_json::json!({"ids": ["a", "b"], "to": {"parent_id": "dest"}})
        );
    }

    #[test]
    fn batch_root_target_is_empty_string() {
        let body = batch_body(&["a"], "").expect("根目录合法");
        assert_eq!(body["to"]["parent_id"], "");
        assert_eq!(body["ids"], serde_json::json!(["a"]));
    }

    #[test]
    fn batch_body_requires_ids_and_obeys_cap() {
        assert!(batch_body(&[], "x").is_err());
        let ids: Vec<String> = (0..101).map(|i| format!("id{i}")).collect();
        let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        assert!(batch_body(&refs, "x").is_err(), "超过 100 个要挡");
    }

    #[test]
    fn batch_task_ack_uses_rclone_shape() {
        let response: BatchTaskAck =
            serde_json::from_str(r#"{"task_id":"task-123"}"#).expect("任务确认应可解析");
        assert_eq!(response.task_id, "task-123");
    }
}
