//! 网盘侧写操作：重命名 / 新建文件夹 / 移动 / 复制 / 星标 / 回收站恢复与永久删除。
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
const ACTION_STAR: &str = "star";
const ACTION_UNSTAR: &str = "unstar";

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
        let body = create_folder_body(parent_id, name);
        self.request_no_response_body(
            reqwest::Method::POST,
            &Config::files_url(self.config()),
            &[],
            &body,
        )
    }

    /// 新建文件夹并返回云端 id。
    ///
    /// 官方新建接口有时返回条目，有时只回空体。返回空体时先记下创建前父目录
    /// 的文件夹 id，再轮询列表找新增的那一个；这样官方自动改名（例如追加
    /// `(1)`）时仍能拿到新目录，而不会误认已有的同名旧目录。
    ///
    /// 创建前列表失败就直接返回错误，不发出创建请求：否则服务端可能已经建好
    /// 目录，但客户端没有任何安全办法确认它的 id，重试还会再建一份。
    pub fn create_folder_entry(
        &self,
        parent_id: Option<&str>,
        name: &str,
    ) -> Result<String, Error> {
        let name = validate_name(name)?;
        let before = self.folder_ids_in(parent_id)?;
        let body = create_folder_body(parent_id, name);
        let response: Option<serde_json::Value> = self.request_optional_json(
            reqwest::Method::POST,
            &Config::files_url(self.config()),
            &[],
            Body::Json(&body),
        )?;
        if let Some(id) = response.as_ref().and_then(folder_id_from_response) {
            return Ok(id);
        }
        for attempt in 0..5 {
            let entries = self.list_all(parent_id)?;
            if let Some(id) = pick_new_folder_id(&entries, &before, name) {
                return Ok(id);
            }
            if attempt < 4 {
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
        }
        Err(Error::ApiChanged(
            "文件夹已创建，但服务端没有返回可用的目录 id".to_owned(),
        ))
    }

    /// 父目录里现有的文件夹 id 集合。
    fn folder_ids_in(&self, parent_id: Option<&str>) -> Result<BTreeSet<String>, Error> {
        Ok(self
            .list_all(parent_id)?
            .into_iter()
            .filter(|entry| entry.kind == crate::EntryKind::Folder)
            .map(|entry| entry.id)
            .collect())
    }

    /// 读完一个目录的所有分页（新建目录后的回查必须看到完整 id 集合）。
    fn list_all(&self, parent_id: Option<&str>) -> Result<Vec<crate::FileEntry>, Error> {
        let mut entries = Vec::new();
        let mut page_token: Option<String> = None;
        for _ in 0..100 {
            let (mut page, next) = self.list(parent_id, page_token.as_deref())?;
            entries.append(&mut page);
            match next {
                Some(token) => page_token = Some(token),
                None => return Ok(entries),
            }
        }
        Err(Error::ApiChanged(
            "目录条目过多，无法确认新建文件夹的 id".to_owned(),
        ))
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

    /// 给文件或文件夹加星标 / 取消星标。
    ///
    /// 星标在服务端是系统标签 `STAR`，写接口仍沿用批量 `ids` 形状。
    /// 成功响应可能为空，因此不能复用会解析 `task_id` 的回收站入口。
    pub fn set_starred(&self, ids: &[&str], starred: bool) -> Result<(), Error> {
        let (action, body) = star_request(ids, starred)?;
        self.request_no_response_body(
            reqwest::Method::POST,
            &Config::file_action_url(self.config(), action),
            &[],
            &body,
        )
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

fn create_folder_body(parent_id: Option<&str>, name: &str) -> serde_json::Value {
    let mut body = serde_json::json!({
        "kind": "drive#folder",
        "name": name,
    });
    if let Some(parent) = parent_id.map(str::trim).filter(|id| !id.is_empty()) {
        body["parent_id"] = serde_json::Value::String(parent.to_owned());
    }
    body
}

/// 从官方可能采用的不同响应包裹层里取新建目录 id。
fn folder_id_from_response(value: &serde_json::Value) -> Option<String> {
    for candidate in [
        &value["file"]["id"],
        &value["data"]["file"]["id"],
        &value["data"]["id"],
        &value["id"],
    ] {
        if let Some(id) = candidate
            .as_str()
            .map(str::trim)
            .filter(|id| !id.is_empty())
        {
            return Some(id.to_owned());
        }
    }
    None
}

/// 从创建后的目录列表里挑出这次新建的文件夹。
///
/// `before` 是创建前已经存在的 id 集合，因此旧同名目录不会被误认成新目录。
/// 官方自动改名时优先匹配原名或原名前缀；只有恰好出现一个新目录时才回退到
/// 它，出现多个无法区分的新目录则返回 `None`，让调用方报错而不是猜。
fn pick_new_folder_id(
    entries: &[crate::FileEntry],
    before: &BTreeSet<String>,
    name: &str,
) -> Option<String> {
    let candidates: Vec<&crate::FileEntry> = entries
        .iter()
        .filter(|entry| entry.kind == crate::EntryKind::Folder && !before.contains(&entry.id))
        .collect();
    if let Some(entry) = candidates.iter().find(|entry| entry.name == name) {
        return Some(entry.id.clone());
    }
    if let Some(entry) = candidates.iter().find(|entry| entry.name.starts_with(name)) {
        return Some(entry.id.clone());
    }
    (candidates.len() == 1).then(|| candidates[0].id.clone())
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

/// 星标 / 取消星标的动作名与请求体。
///
/// 形状由 pikpak-kotlin（MIT，真实账号探针）确认：
/// `POST /drive/v1/files:<action>`，body 只有去重后的 `ids`。
fn star_request(ids: &[&str], starred: bool) -> Result<(&'static str, serde_json::Value), Error> {
    let ids = normalize_ids(ids)?;
    let action = if starred { ACTION_STAR } else { ACTION_UNSTAR };
    Ok((action, serde_json::json!({ "ids": ids })))
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
    fn star_request_uses_the_observed_endpoint_and_batch_body() {
        let (action, body) = star_request(&[" b ", "a", "b"], true).expect("应合法");
        assert_eq!(action, "star");
        assert_eq!(body, serde_json::json!({"ids": ["a", "b"]}));

        let (action, body) = star_request(&["a"], false).expect("应合法");
        assert_eq!(action, "unstar");
        assert_eq!(body, serde_json::json!({"ids": ["a"]}));
    }

    #[test]
    fn star_request_rejects_empty_and_oversized_batches() {
        assert!(star_request(&[], true).is_err());
        let ids: Vec<String> = (0..101).map(|i| format!("id{i}")).collect();
        let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        assert!(star_request(&refs, false).is_err(), "超过 100 个要挡");
    }

    #[test]
    fn batch_task_ack_uses_rclone_shape() {
        let response: BatchTaskAck =
            serde_json::from_str(r#"{"task_id":"task-123"}"#).expect("任务确认应可解析");
        assert_eq!(response.task_id, "task-123");
    }

    fn folder_entry(id: &str, name: &str) -> crate::FileEntry {
        crate::FileEntry {
            id: id.to_owned(),
            name: name.to_owned(),
            kind: crate::EntryKind::Folder,
            size: 0,
            created_at: None,
            modified_at: None,
            parent_id: String::new(),
            md5: None,
            audit_flagged: false,
            starred: false,
            media: false,
            video: false,
            thumbnail_url: String::new(),
        }
    }

    #[test]
    fn new_folder_id_ignores_old_same_name_and_accepts_an_auto_rename() {
        let entries = vec![folder_entry("old", "剧集"), folder_entry("new", "剧集 (1)")];
        let mut before = BTreeSet::new();
        before.insert("old".to_owned());
        assert_eq!(
            pick_new_folder_id(&entries, &before, "剧集"),
            Some("new".to_owned()),
            "旧同名目录不能被当成刚创建的新目录"
        );
    }

    #[test]
    fn new_folder_id_prefers_exact_name_over_other_new_candidates() {
        let entries = vec![
            folder_entry("other", "剧集 (1)"),
            folder_entry("exact", "剧集"),
        ];
        assert_eq!(
            pick_new_folder_id(&entries, &BTreeSet::new(), "剧集"),
            Some("exact".to_owned())
        );
    }

    #[test]
    fn new_folder_id_refuses_to_guess_between_multiple_candidates() {
        let entries = vec![folder_entry("one", "甲"), folder_entry("two", "乙")];
        assert_eq!(
            pick_new_folder_id(&entries, &BTreeSet::new(), "剧集"),
            None,
            "无法确认哪个是新建目录时不能猜"
        );
    }
}
