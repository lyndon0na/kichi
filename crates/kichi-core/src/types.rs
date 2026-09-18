use serde::{Deserialize, Deserializer};

/// 兼容 number / string / null 三种形式的 i64 值。
fn de_number<'de, D>(d: D) -> Result<i64, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Num {
        Int(i64),
        Float(f64),
        Str(String),
        Null,
    }
    match Num::deserialize(d)? {
        Num::Int(v) => Ok(v),
        Num::Float(v) => Ok(v as i64),
        Num::Str(s) => Ok(s.trim().parse().unwrap_or(0)),
        Num::Null => Ok(0),
    }
}

/// 兼容 number / string / bool / null 四种形式的字符串值(服务端计数等字段
/// 有时返回数字、有时返回字符串)。
fn de_string<'de, D>(d: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Val {
        Str(String),
        Int(i64),
        Float(f64),
        Bool(bool),
        Null,
    }
    Ok(match Val::deserialize(d)? {
        Val::Str(s) => s,
        Val::Int(v) => v.to_string(),
        Val::Float(v) => v.to_string(),
        Val::Bool(v) => v.to_string(),
        Val::Null => String::new(),
    })
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
#[serde(default)]
pub struct File {
    pub id: String,
    pub kind: String,
    pub name: String,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(deserialize_with = "de_number")]
    pub size: i64,
    pub mime_type: Option<String>,
    pub created_time: Option<String>,
    pub modified_time: Option<String>,
    pub user_modified_time: Option<String>,
    /// 移入回收站的时间(仅回收站条目有值, 服务端字段名 delete_time)。
    pub delete_time: Option<String>,
    pub trashed: bool,
    pub starred: bool,
    pub phase: Option<String>,
    pub file_extension: Option<String>,
    pub folder_type: Option<String>,
    pub icon_link: Option<String>,
    pub thumbnail_link: Option<String>,
    pub web_content_link: Option<String>,
    pub md5_checksum: Option<String>,
    pub original_url: Option<String>,
}

impl File {
    pub fn is_folder(&self) -> bool {
        self.kind.contains("folder")
    }
}

#[derive(Debug, Clone, Default)]
pub struct FileList {
    pub files: Vec<File>,
    pub next_page_token: Option<String>,
}

impl<'de> Deserialize<'de> for FileList {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Raw {
            #[serde(default)]
            files: Vec<File>,
            #[serde(default)]
            next_page_token: Option<String>,
        }
        let raw = Raw::deserialize(deserializer)?;
        Ok(FileList {
            files: raw.files,
            next_page_token: raw.next_page_token.filter(|s| !s.is_empty()),
        })
    }
}

/// 目录递归遍历结果(整目录下载用)。
#[derive(Debug, Clone, Default)]
pub struct FolderWalk {
    /// 相对根目录的目录组件序列; 根目录为空 vec。含空目录, 便于本地按层级建目录。
    pub dirs: Vec<Vec<String>>,
    /// 文件: (相对目录组件, 文件)。
    pub files: Vec<(Vec<String>, File)>,
}

#[derive(Debug, Clone, Default)]
pub struct Quota {
    pub limit: i64,
    pub usage: i64,
    pub usage_in_trash: i64,
    pub play_times_limit: i64,
    pub play_times_usage: i64,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct QuotaResponse {
    #[serde(default)]
    pub quota: Quota,
}

impl<'de> Deserialize<'de> for Quota {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Raw {
            #[serde(default, deserialize_with = "de_number")]
            limit: i64,
            #[serde(default, deserialize_with = "de_number")]
            usage: i64,
            #[serde(default, deserialize_with = "de_number")]
            usage_in_trash: i64,
            #[serde(default, deserialize_with = "de_number")]
            play_times_limit: i64,
            #[serde(default, deserialize_with = "de_number")]
            play_times_usage: i64,
        }
        let r = Raw::deserialize(deserializer)?;
        Ok(Quota {
            limit: r.limit,
            usage: r.usage,
            usage_in_trash: r.usage_in_trash,
            play_times_limit: r.play_times_limit,
            play_times_usage: r.play_times_usage,
        })
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub refresh_token: String,
    #[serde(default)]
    pub sub: Option<String>,
    #[serde(default)]
    pub expires_in: Option<i64>,
}

/// 离线任务 / drive#task 的原始对象。字段以防御方式从 JSON 取值。
pub type Task = serde_json::Value;

#[derive(Debug, Clone, Default)]
pub struct Tasks {
    pub tasks: Vec<Task>,
    pub next_page_token: Option<String>,
}

impl Tasks {
    pub fn parse(value: serde_json::Value) -> Tasks {
        let tasks = value
            .get("tasks")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let next_page_token = value
            .get("next_page_token")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        Tasks {
            tasks,
            next_page_token,
        }
    }
}

/// 「我的分享」条目。服务端计数类字段以字符串 / 数字混用, 统一防御式转字符串。
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default)]
pub struct Share {
    pub share_id: String,
    #[serde(deserialize_with = "de_string")]
    pub share_url: String,
    #[serde(deserialize_with = "de_string")]
    pub title: String,
    #[serde(deserialize_with = "de_string")]
    pub pass_code: String,
    #[serde(deserialize_with = "de_string")]
    pub share_to: String,
    #[serde(deserialize_with = "de_string")]
    pub create_time: String,
    #[serde(deserialize_with = "de_string")]
    pub expiration_days: String,
    #[serde(deserialize_with = "de_string")]
    pub view_count: String,
    #[serde(deserialize_with = "de_string")]
    pub restore_count: String,
    #[serde(deserialize_with = "de_string")]
    pub file_num: String,
    #[serde(deserialize_with = "de_string")]
    pub share_status: String,
}

impl Share {
    /// 是否为需要提取码的私密分享。
    pub fn needs_pass_code(&self) -> bool {
        !self.pass_code.is_empty() || self.share_to.contains("encrypted")
    }

    /// 有效期描述: 永久 / N 天。
    pub fn expiry_label(&self) -> String {
        match self.expiration_days.trim() {
            "" | "-1" | "0" => "永久".to_string(),
            d => format!("{d} 天"),
        }
    }

    /// 是否已失效或不可访问(status 非 OK)。
    pub fn is_unavailable(&self) -> bool {
        !self.share_status.is_empty() && self.share_status != "OK"
    }

    pub fn file_count(&self) -> i64 {
        self.file_num.trim().parse().unwrap_or(0)
    }
}

#[derive(Debug, Clone, Default)]
pub struct ShareList {
    pub shares: Vec<Share>,
    pub next_page_token: Option<String>,
}

impl<'de> Deserialize<'de> for ShareList {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Raw {
            #[serde(default, alias = "shares")]
            data: Vec<Share>,
            #[serde(default)]
            next_page_token: Option<String>,
        }
        let raw = Raw::deserialize(deserializer)?;
        Ok(ShareList {
            shares: raw.data,
            next_page_token: raw.next_page_token.filter(|s| !s.is_empty()),
        })
    }
}

/// 创建分享的响应。
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default)]
pub struct ShareCreated {
    pub share_id: String,
    #[serde(deserialize_with = "de_string")]
    pub share_url: String,
    #[serde(deserialize_with = "de_string")]
    pub pass_code: String,
    #[serde(deserialize_with = "de_string")]
    pub share_text: String,
}

/// 解析他人分享链接的响应: 包含文件列表与 pass_code_token(转存时需要)。
#[derive(Debug, Clone, Default)]
pub struct ShareDetail {
    pub share_status: String,
    pub title: String,
    /// 服务端签发的令牌, 后续 detail 分页与 restore 均需携带。
    pub pass_code_token: String,
    pub files: Vec<File>,
    pub next_page_token: Option<String>,
}

impl<'de> Deserialize<'de> for ShareDetail {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Raw {
            #[serde(default)]
            share_status: String,
            #[serde(default)]
            title: String,
            #[serde(default)]
            pass_code_token: String,
            #[serde(default)]
            files: Vec<File>,
            #[serde(default)]
            next_page_token: Option<String>,
        }
        let raw = Raw::deserialize(deserializer)?;
        Ok(ShareDetail {
            share_status: raw.share_status,
            title: raw.title,
            pass_code_token: raw.pass_code_token,
            files: raw.files,
            next_page_token: raw.next_page_token.filter(|s| !s.is_empty()),
        })
    }
}

/// 转存(restore) 操作的响应。
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default)]
pub struct ShareRestoreResult {
    pub restore_status: String,
    pub restore_task_id: String,
}

pub fn task_str(task: &Task, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(s) = task.get(*key).and_then(|v| v.as_str()) {
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    None
}

pub fn task_i64(task: &Task, keys: &[&str]) -> Option<i64> {
    for key in keys {
        if let Some(v) = task.get(*key) {
            if let Some(n) = v.as_i64() {
                return Some(n);
            }
            if let Some(n) = v.as_str().and_then(|s| s.parse().ok()) {
                return Some(n);
            }
        }
    }
    None
}

pub fn task_id(task: &Task) -> Option<String> {
    task_str(task, &["id"])
}

pub fn task_name(task: &Task) -> Option<String> {
    task_str(task, &["name", "file_name"])
}

pub fn task_file_id(task: &Task) -> Option<String> {
    task_str(task, &["file_id"])
}

pub fn task_size(task: &Task) -> Option<i64> {
    task_i64(task, &["file_size", "size"])
}

pub fn task_created(task: &Task) -> Option<String> {
    task_str(task, &["created_time", "created_at"])
}

pub fn task_original_url(task: &Task) -> Option<String> {
    task_str(task, &["original_url"])
}
