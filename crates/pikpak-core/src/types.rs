use serde::{Deserialize, Deserializer};

/// 兼容 number / string 两种形式的字节数。
fn de_size<'de, D>(d: D) -> Result<i64, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum S {
        Int(i64),
        Float(f64),
        Str(String),
        Null,
    }
    match S::deserialize(d)? {
        S::Int(v) => Ok(v),
        S::Float(v) => Ok(v as i64),
        S::Str(s) => Ok(s.trim().parse().unwrap_or(0)),
        S::Null => Ok(0),
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
#[serde(default)]
pub struct File {
    pub id: String,
    pub kind: String,
    pub name: String,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(deserialize_with = "de_size")]
    pub size: i64,
    pub mime_type: Option<String>,
    pub created_time: Option<String>,
    pub modified_time: Option<String>,
    pub user_modified_time: Option<String>,
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
            #[serde(default, deserialize_with = "de_str_num")]
            limit: i64,
            #[serde(default, deserialize_with = "de_str_num")]
            usage: i64,
            #[serde(default, deserialize_with = "de_str_num")]
            usage_in_trash: i64,
            #[serde(default, deserialize_with = "de_str_num")]
            play_times_limit: i64,
            #[serde(default, deserialize_with = "de_str_num")]
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

fn de_str_num<'de, D>(d: D) -> Result<i64, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum N {
        I(i64),
        F(f64),
        S(String),
        Null,
    }
    match N::deserialize(d)? {
        N::I(v) => Ok(v),
        N::F(v) => Ok(v as i64),
        N::S(s) => Ok(s.trim().parse().unwrap_or(0)),
        N::Null => Ok(0),
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
