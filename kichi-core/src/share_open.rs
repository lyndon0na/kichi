//! 打开 / 浏览 / 转存别人的分享链接。
//!
//! 端点形状对照 pikpak-kotlin（MIT，2026-09-24 真实账号记录）与
//! pikpakcli（MIT）核对：
//! - `GET /drive/v1/share`：校验分享并返回 `share_status` / `pass_code_token`
//!   / 标题 / 首页顶层条目；密码缺失与密码错误都是 HTTP 200 +
//!   `share_status`（`PASS_CODE_EMPTY` / `PASS_CODE_ERROR`）。
//! - `GET /drive/v1/share/detail`：按 `parent_id` + `page_token` 翻页浏览。
//! - `POST /drive/v1/share/restore`：转存到 `parent_id`（空串 = 根目录），
//!   `specify_parent_id: true`；返回 `RESTORE_START` + `restore_task_id`，
//!   任务完成状态用 `GET /drive/v1/tasks/{id}` 轮询。
//!
//! 分享链接里的第二段路径是分享内文件夹 id，不是密码；密码只从分享文本 /
//! 链接查询参数里解析。密码永远不进入日志。

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::drive::{DriveClient, FileDto};
use crate::http::Body;
use crate::share::lenient_u64;
use crate::tasks::phase_from;
use crate::types::{FileEntry, TaskPhase};
use crate::Error;

/// 单页分享目录请求条数（与官方 Web 客户端一致）。
pub const SHARE_PAGE_SIZE: u32 = 100;
/// 一次转存请求最多携带的条目数（防御异常输入，不是官方限制）。
const MAX_RESTORE_FILES: usize = 1000;

/// 从分享链接 / 分享文本解析出的信息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShareLinkInput {
    pub share_id: String,
    /// 分享文本或链接查询参数里带的访问密码；没有则为 `None`。
    pub pass_code: Option<String>,
}

/// 分享的元数据 + 首页顶层条目。
#[derive(Debug, Clone)]
pub struct ShareOpenInfo {
    pub share_id: String,
    /// 后续浏览 / 转存都要带上的令牌；公开分享也会下发。
    pub pass_code_token: String,
    pub title: String,
    /// 分享者昵称（官方会打码）；可能为空。
    pub owner: String,
    pub file_count: u64,
    pub expiration_at: Option<DateTime<Utc>>,
    /// 官方 `expiration_at = "-1"`：永久有效。
    pub never_expires: bool,
    pub entries: Vec<FileEntry>,
    /// 还有下一页顶层条目；由会话层继续翻完。
    pub next_page_token: Option<String>,
    /// 会话层达到分页上限，界面应提示只显示了前 N 项。
    pub truncated: bool,
}

/// 分享里某一层目录的结果。
#[derive(Debug, Clone)]
pub struct ShareListing {
    pub entries: Vec<FileEntry>,
    /// 达到分页上限，未读取完。
    pub truncated: bool,
}

/// 转存请求。
#[derive(Debug, Clone)]
pub struct RestoreShareRequest {
    pub share_id: String,
    pub pass_code_token: String,
    pub file_ids: Vec<String>,
    /// 自己的目标目录；空串 = 网盘根目录。
    pub parent_id: String,
    /// 分享内从顶层到所选条目父目录的文件夹 id（顶层条目为空）。
    pub ancestor_ids: Vec<String>,
}

/// 转存的同步响应。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreShareResult {
    /// `RESTORE_START` / `RESTORE_COMPLETE` / 未知。
    pub status: String,
    pub task_id: Option<String>,
}

impl RestoreShareResult {
    pub fn is_complete(&self) -> bool {
        self.status.eq_ignore_ascii_case("RESTORE_COMPLETE")
    }

    /// 服务端已接受，走异步任务。
    pub fn is_started(&self) -> bool {
        self.status.eq_ignore_ascii_case("RESTORE_START") && self.task_id.is_some()
    }
}

/// 转存任务的一次查询结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreTaskState {
    pub phase: TaskPhase,
    /// 失败原因；官方可能放在 `message` 或 `params.error_detail`。
    pub message: Option<String>,
}

impl DriveClient {
    /// 打开分享：读取元数据与首页顶层条目。
    ///
    /// `pass_code` 只在分享需要密码时传；分享不存在 / 已删除 / 需要密码等情况
    /// 都通过 [`Error::ShareUnavailable`] 交给界面分类。
    pub fn open_share(
        &self,
        share_id: &str,
        pass_code: Option<&str>,
    ) -> Result<ShareOpenInfo, Error> {
        let share_id = share_id.trim();
        if share_id.is_empty() {
            return Err(Error::InvalidInput("分享 id 为空".into()));
        }
        let mut query = vec![
            ("share_id".to_owned(), share_id.to_owned()),
            ("limit".to_owned(), SHARE_PAGE_SIZE.to_string()),
            ("thumbnail_size".to_owned(), "SIZE_SMALL".to_owned()),
        ];
        if let Some(code) = pass_code.map(str::trim).filter(|code| !code.is_empty()) {
            query.push(("pass_code".to_owned(), code.to_owned()));
        }
        let response: ShareOpenResponse =
            self.get_with_query(&self.config().share_open_url(), &query)?;
        response.into_info(share_id)
    }

    /// 读取分享里某一层目录的一页。
    ///
    /// `parent_id = None` / 空串表示顶层；`page_token = None` 表示第一页。
    pub fn list_share_files(
        &self,
        share_id: &str,
        pass_code_token: &str,
        parent_id: Option<&str>,
        page_token: Option<&str>,
    ) -> Result<ShareFolderPage, Error> {
        let share_id = share_id.trim();
        if share_id.is_empty() {
            return Err(Error::InvalidInput("分享 id 为空".into()));
        }
        let mut query = vec![
            ("share_id".to_owned(), share_id.to_owned()),
            (
                "pass_code_token".to_owned(),
                pass_code_token.trim().to_owned(),
            ),
            ("limit".to_owned(), SHARE_PAGE_SIZE.to_string()),
            ("thumbnail_size".to_owned(), "SIZE_SMALL".to_owned()),
        ];
        if let Some(parent) = parent_id.map(str::trim).filter(|id| !id.is_empty()) {
            query.push(("parent_id".to_owned(), parent.to_owned()));
        }
        if let Some(token) = page_token.map(str::trim).filter(|token| !token.is_empty()) {
            query.push(("page_token".to_owned(), token.to_owned()));
        }
        let response: ShareDetailResponse =
            self.get_with_query(&self.config().share_detail_url(), &query)?;
        response.into_page()
    }

    /// 转存分享里的条目到自己的网盘。
    pub fn restore_share(
        &self,
        request: &RestoreShareRequest,
    ) -> Result<RestoreShareResult, Error> {
        let share_id = request.share_id.trim();
        if share_id.is_empty() {
            return Err(Error::InvalidInput("分享 id 为空".into()));
        }
        let file_ids = normalized_ids(&request.file_ids, "转存条目")?;
        if file_ids.len() > MAX_RESTORE_FILES {
            return Err(Error::InvalidInput(format!(
                "一次最多转存 {MAX_RESTORE_FILES} 项（当前 {} 项）",
                file_ids.len()
            )));
        }
        let ancestor_ids = normalized_ids(&request.ancestor_ids, "分享目录")?;
        let body = serde_json::json!({
            "share_id": share_id,
            "pass_code_token": request.pass_code_token.trim(),
            "file_ids": file_ids,
            "ancestor_ids": ancestor_ids,
            "parent_id": request.parent_id.trim(),
            "specify_parent_id": true,
        });
        let response: RestoreShareResponse = self.request(
            reqwest::Method::POST,
            &self.config().share_restore_url(),
            &[],
            Body::Json(&body),
        )?;
        response.into_result()
    }

    /// 查询一次转存任务。转存任务与离线任务共用 `/drive/v1/tasks/{id}`。
    pub fn share_restore_task(&self, task_id: &str) -> Result<RestoreTaskState, Error> {
        let task_id = task_id.trim();
        if task_id.is_empty() {
            return Err(Error::InvalidInput("转存任务 id 为空".into()));
        }
        let response: TaskStateResponse =
            self.get_with_query(&self.config().task_url(task_id), &[])?;
        response.into_state()
    }
}

/// 分享里某一层目录的一页（对外暴露给会话层翻页）。
#[derive(Debug, Clone)]
pub struct ShareFolderPage {
    pub entries: Vec<FileEntry>,
    pub next_page_token: Option<String>,
}

// ———————————————————— 链接解析 ————————————————————

const SHARE_MARKERS: [&str; 2] = ["mypikpak.com/s/", "mypikpak.net/s/"];
/// 分享文本里的密码标签；先长后短，避免「访问密码」被「密码」抢先。
const PASS_LABELS: [&str; 8] = [
    "访问密码",
    "提取码",
    "分享密码",
    "密码",
    "passcode",
    "pass code",
    "password",
    "pwd",
];

/// 解析粘贴进来的分享链接或分享文本。
///
/// 支持的形态：
/// - `https://mypikpak.com/s/<share_id>`（可带 `/子目录`、查询串、片段）；
/// - 含上述链接的多行分享文本；
/// - 文本里的「访问密码 / 提取码：xxxx」或链接查询参数 `?pwd=xxxx`。
pub fn parse_share_input(raw: &str) -> Result<ShareLinkInput, Error> {
    let text = raw.trim();
    if text.is_empty() {
        return Err(Error::InvalidInput(
            "请粘贴 PikPak 分享链接或分享文本".into(),
        ));
    }
    // ASCII 小写化不改变字节长度，偏移量可以直接用于原串。
    let lower = text.to_ascii_lowercase();
    let (marker_at, marker_len) = SHARE_MARKERS
        .iter()
        .filter_map(|marker| lower.find(marker).map(|at| (at, marker.len())))
        .min_by_key(|(at, _)| *at)
        .ok_or_else(|| {
            Error::InvalidInput("没有找到 PikPak 分享链接（形如 https://mypikpak.com/s/…）".into())
        })?;
    let id_at = marker_at + marker_len;
    let share_id: String = text
        .get(id_at..)
        .unwrap_or_default()
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
        .collect();
    if share_id.is_empty() {
        return Err(Error::InvalidInput("分享链接里没有 share_id".into()));
    }
    let pass_code = PASS_LABELS
        .iter()
        .find_map(|label| pass_code_after_label(text, &lower, label))
        .or_else(|| pass_code_from_query(text, marker_at));
    Ok(ShareLinkInput {
        share_id,
        pass_code,
    })
}

/// 标签后同一行里的访问密码。
fn pass_code_after_label(text: &str, lower: &str, label: &str) -> Option<String> {
    let mut from = 0;
    while let Some(offset) = lower.get(from..)?.find(label) {
        let at = from + offset + label.len();
        if let Some(code) = token_after_label(text.get(at..)?) {
            return Some(code);
        }
        from = at;
        if from >= lower.len() {
            break;
        }
    }
    None
}

fn token_after_label(rest: &str) -> Option<String> {
    let rest = rest.trim_start_matches([' ', '\t']);
    let rest = rest
        .strip_prefix(':')
        .or_else(|| rest.strip_prefix('：'))
        .or_else(|| rest.strip_prefix('='))
        .or_else(|| rest.strip_prefix('＝'))
        .unwrap_or(rest);
    let rest = rest.trim_start_matches([' ', '\t']);
    let code: String = rest
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric())
        .take(16)
        .collect();
    (!code.is_empty()).then_some(code)
}

/// 链接查询串里的 `pwd` / `pass_code` / `passcode` / `password`。
fn pass_code_from_query(text: &str, url_at: usize) -> Option<String> {
    let tail = text.get(url_at..)?;
    let query_at = tail.find('?')?;
    let query: String = tail
        .get(query_at + 1..)?
        .chars()
        .take_while(|ch| !ch.is_whitespace())
        .collect();
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let key = key.trim().to_ascii_lowercase();
        if !matches!(key.as_str(), "pwd" | "pass_code" | "passcode" | "password") {
            continue;
        }
        let code: String = value
            .chars()
            .take_while(|ch| ch.is_ascii_alphanumeric())
            .take(16)
            .collect();
        if !code.is_empty() {
            return Some(code);
        }
    }
    None
}

// ———————————————————— 响应 DTO ————————————————————

/// 官方分享端点共同的错误字段。
#[derive(Debug, Default, Deserialize)]
struct ApiErrorFields {
    #[serde(default)]
    error: String,
    #[serde(default)]
    error_code: i64,
    #[serde(default)]
    error_description: String,
}

impl ApiErrorFields {
    fn check(&self) -> Result<(), Error> {
        if self.error_code == 0 {
            return Ok(());
        }
        let reason = if self.error.trim().is_empty() {
            "unknown"
        } else {
            self.error.as_str()
        };
        Err(Error::from_api(
            self.error_code,
            reason,
            &self.error_description,
        ))
    }
}

#[derive(Deserialize)]
struct ShareOpenResponse {
    #[serde(flatten)]
    api_error: ApiErrorFields,
    #[serde(default)]
    share_status: String,
    #[serde(default)]
    share_status_text: String,
    #[serde(default)]
    pass_code_token: String,
    #[serde(default)]
    title: String,
    #[serde(default, deserialize_with = "lenient_u64")]
    file_num: u64,
    #[serde(default)]
    expiration_at: Option<String>,
    #[serde(default)]
    user_info: Option<ShareOwnerDto>,
    #[serde(default)]
    files: Vec<FileDto>,
    #[serde(default)]
    next_page_token: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct ShareOwnerDto {
    #[serde(default)]
    nickname: String,
}

impl ShareOpenResponse {
    fn into_info(self, share_id: &str) -> Result<ShareOpenInfo, Error> {
        self.api_error.check()?;
        check_share_status(&self.share_status, &self.share_status_text)?;
        let expiration = self.expiration_at.as_deref().map(str::trim).unwrap_or("");
        let never_expires = expiration == "-1";
        let expiration_at = if never_expires {
            None
        } else {
            parse_time(expiration)
        };
        Ok(ShareOpenInfo {
            share_id: share_id.to_owned(),
            pass_code_token: self.pass_code_token.trim().to_owned(),
            title: self.title.trim().to_owned(),
            owner: self
                .user_info
                .as_ref()
                .map(|owner| owner.nickname.trim().to_owned())
                .unwrap_or_default(),
            file_count: self.file_num,
            expiration_at,
            never_expires,
            entries: self.files.into_iter().map(FileDto::into_entry).collect(),
            next_page_token: next_token(self.next_page_token),
            truncated: false,
        })
    }
}

#[derive(Deserialize)]
struct ShareDetailResponse {
    #[serde(flatten)]
    api_error: ApiErrorFields,
    #[serde(default)]
    share_status: String,
    #[serde(default)]
    share_status_text: String,
    #[serde(default)]
    files: Vec<FileDto>,
    #[serde(default)]
    next_page_token: Option<String>,
}

impl ShareDetailResponse {
    fn into_page(self) -> Result<ShareFolderPage, Error> {
        self.api_error.check()?;
        check_share_status(&self.share_status, &self.share_status_text)?;
        Ok(ShareFolderPage {
            entries: self.files.into_iter().map(FileDto::into_entry).collect(),
            next_page_token: next_token(self.next_page_token),
        })
    }
}

#[derive(Debug, Deserialize)]
struct RestoreShareResponse {
    #[serde(flatten)]
    api_error: ApiErrorFields,
    #[serde(default)]
    share_status: String,
    #[serde(default)]
    share_status_text: String,
    #[serde(default)]
    restore_status: String,
    #[serde(default)]
    restore_task_id: String,
}

impl RestoreShareResponse {
    fn into_result(self) -> Result<RestoreShareResult, Error> {
        self.api_error.check()?;
        check_share_status(&self.share_status, &self.share_status_text)?;
        Ok(RestoreShareResult {
            status: self.restore_status.trim().to_owned(),
            task_id: (!self.restore_task_id.trim().is_empty())
                .then(|| self.restore_task_id.trim().to_owned()),
        })
    }
}

#[derive(Debug, Deserialize)]
struct TaskStateResponse {
    #[serde(default)]
    phase: String,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    params: Option<TaskParamsDto>,
}

#[derive(Debug, Default, Deserialize)]
struct TaskParamsDto {
    #[serde(default)]
    error_detail: String,
}

impl TaskStateResponse {
    fn into_state(self) -> Result<RestoreTaskState, Error> {
        let detail = self
            .params
            .as_ref()
            .map(|params| params.error_detail.trim())
            .filter(|detail| !detail.is_empty());
        let message = detail
            .map(str::to_owned)
            .or_else(|| self.message.map(|msg| msg.trim().to_owned()))
            .filter(|msg| !msg.is_empty());
        Ok(RestoreTaskState {
            phase: phase_from(&self.phase),
            message,
        })
    }
}

fn check_share_status(status: &str, text: &str) -> Result<(), Error> {
    let status = status.trim();
    if status.is_empty() || status.eq_ignore_ascii_case("OK") {
        return Ok(());
    }
    Err(share_unavailable(status, text))
}

/// `share_status` → 用户能直接看懂的错误。
///
/// 官方在读分享失败时仍回 HTTP 200，且不带 `error_code`；只有这个字段能
/// 说明原因。已知状态给出固定中文，未知状态保留官方说明。
fn share_unavailable(status: &str, text: &str) -> Error {
    let message = match status {
        "PASS_CODE_EMPTY" => "这个分享需要访问密码".to_owned(),
        "PASS_CODE_ERROR" => "访问密码不正确".to_owned(),
        "DELETED" => "分享已被撤销或删除".to_owned(),
        "EXPIRED" => "分享链接已过期".to_owned(),
        "NOT_FOUND" => "分享不存在或链接无效".to_owned(),
        "AUDITING" => "分享内容正在审核，暂时无法打开".to_owned(),
        "SENSITIVE_RESOURCE" => "分享内容被标记为敏感资源，暂时无法打开".to_owned(),
        "SENSITIVE_WORD" => "分享内容命中敏感词，暂时无法打开".to_owned(),
        "PROHIBITED" => "分享被禁止访问".to_owned(),
        _ if !text.trim().is_empty() => text.trim().to_owned(),
        _ => format!("分享无法打开（{status}）"),
    };
    Error::ShareUnavailable {
        status: status.to_owned(),
        message,
    }
}

fn parse_time(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value.trim())
        .ok()
        .map(|time| time.with_timezone(&Utc))
}

fn next_token(raw: Option<String>) -> Option<String> {
    raw.map(|token| token.trim().to_owned())
        .filter(|token| !token.is_empty())
}

fn normalized_ids(ids: &[String], subject: &str) -> Result<Vec<String>, Error> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for id in ids {
        let id = id.trim();
        if id.is_empty() || !seen.insert(id.to_owned()) {
            continue;
        }
        result.push(id.to_owned());
    }
    if result.is_empty() {
        return Err(Error::InvalidInput(format!("没有要操作的{subject}")));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_share_link_with_folder_and_query() {
        let parsed =
            parse_share_input(" https://mypikpak.com/s/VOab_c/VOfolder?act=play&pwd=zq47 ")
                .expect("链接应可解析");
        assert_eq!(parsed.share_id, "VOab_c");
        assert_eq!(parsed.pass_code.as_deref(), Some("zq47"));
    }

    #[test]
    fn parses_multiline_share_text_with_chinese_labels() {
        let text = "我分享了一个文件给你\n\
                    https://mypikpak.com/s/VP2J59n2xU-aH3jyvst_a1UPo2\n\
                    访问密码：a1B2";
        let parsed = parse_share_input(text).expect("分享文本应可解析");
        assert_eq!(parsed.share_id, "VP2J59n2xU-aH3jyvst_a1UPo2");
        assert_eq!(parsed.pass_code.as_deref(), Some("a1B2"));

        let full_width = "链接 https://mypikpak.com/s/VS1\n提取码＝9xYz";
        let parsed = parse_share_input(full_width).expect("全角分隔符应可解析");
        assert_eq!(parsed.pass_code.as_deref(), Some("9xYz"));
    }

    #[test]
    fn share_id_is_case_sensitive_and_stops_at_path_boundary() {
        let parsed = parse_share_input("https://mypikpak.com/s/AbC-_09/sub").expect("应可解析");
        assert_eq!(parsed.share_id, "AbC-_09");
        assert_eq!(parsed.pass_code, None);
    }

    #[test]
    fn rejects_text_without_share_link() {
        assert!(parse_share_input("").is_err());
        assert!(parse_share_input("https://example.com/s/abc").is_err());
        assert!(parse_share_input("https://mypikpak.com/s/").is_err());
        assert!(parse_share_input("密码 abc123").is_err());
    }

    #[test]
    fn password_label_without_code_is_ignored() {
        let parsed =
            parse_share_input("https://mypikpak.com/s/VS1\n密码保护，需要密码").expect("应可解析");
        assert_eq!(parsed.share_id, "VS1");
        assert_eq!(parsed.pass_code, None);
    }

    #[test]
    fn open_response_parses_recorded_shape() {
        // 形状来自 pikpak-kotlin（MIT）2026-09-24 的记录，内容已替换。
        let json = r#"{
            "share_status": "OK",
            "share_status_text": "",
            "pass_code_token": "TOKEN",
            "title": "probe",
            "file_num": "2",
            "expiration_at": "2026-09-25T23:46:22.772+08:00",
            "user_info": {"user_id": "U", "nickname": "Ry**eI"},
            "files": [
                {"kind": "drive#folder", "id": "D1", "name": "probe"},
                {"kind": "drive#file", "id": "F2", "parent_id": "D1",
                 "name": "a.bin", "size": "4096"}
            ],
            "next_page_token": "NEXT"
        }"#;
        let response: ShareOpenResponse = serde_json::from_str(json).expect("响应应可解析");
        let info = response.into_info("S1").expect("应可用");
        assert_eq!(info.share_id, "S1");
        assert_eq!(info.pass_code_token, "TOKEN");
        assert_eq!(info.title, "probe");
        assert_eq!(info.owner, "Ry**eI");
        assert_eq!(info.file_count, 2);
        assert!(!info.never_expires);
        assert!(info.expiration_at.is_some());
        assert_eq!(info.entries.len(), 2);
        assert_eq!(info.entries[1].size, 4096);
        assert_eq!(info.next_page_token.as_deref(), Some("NEXT"));
    }

    #[test]
    fn missing_pass_code_is_its_own_status() {
        let json = r#"{
            "share_status": "PASS_CODE_EMPTY",
            "share_status_text": "Incorrect password, please try again",
            "file_num": "0", "files": [], "pass_code_token": "", "title": ""
        }"#;
        let response: ShareOpenResponse = serde_json::from_str(json).expect("响应应可解析");
        let err = response.into_info("S1").expect_err("需要密码应报错");
        assert!(err.needs_pass_code(), "{err:?}");
        assert!(!err.wrong_pass_code());
        assert!(err.to_string().contains("访问密码"));
    }

    #[test]
    fn wrong_pass_code_maps_to_a_distinct_status() {
        let json = r#"{"share_status":"PASS_CODE_ERROR","share_status_text":"bad"}"#;
        let response: ShareOpenResponse = serde_json::from_str(json).expect("响应应可解析");
        let err = response.into_info("S1").expect_err("密码错误应报错");
        assert!(err.wrong_pass_code(), "{err:?}");
        assert!(!err.needs_pass_code());
    }

    #[test]
    fn deleted_and_unknown_statuses_keep_a_readable_reason() {
        let json = r#"{"share_status":"DELETED","share_status_text":""}"#;
        let response: ShareOpenResponse = serde_json::from_str(json).expect("响应应可解析");
        let err = response.into_info("S1").expect_err("已删除应报错");
        assert!(err.to_string().contains("已被撤销或删除"));

        let json = r#"{"share_status":"SOME_FUTURE_STATUS","share_status_text":"官方说明"}"#;
        let response: ShareOpenResponse = serde_json::from_str(json).expect("响应应可解析");
        let err = response.into_info("S1").expect_err("未知状态应报错");
        assert!(err.to_string().contains("官方说明"));
    }

    #[test]
    fn restore_response_reads_task_start() {
        let json = r#"{
            "share_status":"OK","share_status_text":"",
            "file_id":"","restore_status":"RESTORE_START","restore_task_id":"TK","params":{}
        }"#;
        let response: RestoreShareResponse = serde_json::from_str(json).expect("响应应可解析");
        let result = response.into_result().expect("应可用");
        assert!(result.is_started());
        assert!(!result.is_complete());
        assert_eq!(result.task_id.as_deref(), Some("TK"));
    }

    #[test]
    fn restore_task_prefers_error_detail() {
        let json = r#"{
            "phase":"PHASE_TYPE_ERROR","message":"failed",
            "params":{"error_detail":"空间不足"}
        }"#;
        let response: TaskStateResponse = serde_json::from_str(json).expect("响应应可解析");
        let state = response.into_state().expect("应可用");
        assert_eq!(state.phase, TaskPhase::Error);
        assert_eq!(state.message.as_deref(), Some("空间不足"));
    }

    #[test]
    fn restore_request_validates_ids() {
        let config = crate::Config::default();
        let client = DriveClient::new(&config, "0123456789abcdef0123456789abcdef", "", "")
            .expect("客户端应可构造");
        let request = RestoreShareRequest {
            share_id: "S1".into(),
            pass_code_token: String::new(),
            file_ids: vec!["  ".into()],
            parent_id: String::new(),
            ancestor_ids: Vec::new(),
        };
        assert!(matches!(
            client.restore_share(&request),
            Err(Error::InvalidInput(_))
        ));
        assert!(matches!(
            client.restore_share(&RestoreShareRequest {
                share_id: "  ".into(),
                ..request
            }),
            Err(Error::InvalidInput(_))
        ));
    }

    #[test]
    fn ids_are_trimmed_and_deduplicated_in_order() {
        let ids = normalized_ids(&[" b ".into(), "a".into(), "b".into()], "条目").expect("应可用");
        assert_eq!(ids, vec!["b".to_owned(), "a".to_owned()]);
    }
}
