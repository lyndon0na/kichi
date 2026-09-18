use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Settings {
    #[serde(default)]
    pub username: String,
    /// 上一次选择的本地下载目录(可空)。
    #[serde(default)]
    pub download_dir: String,
    /// 是否把密码保存到系统密钥环, 用于自动登录。
    #[serde(default)]
    pub remember_password: bool,
}

/// 下载记录状态（仅保存已完成/取消/失败的）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum DownloadRecordStatus {
    Done,
    Cancelled,
    Failed(String),
}

/// 单条下载历史记录。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadRecord {
    /// 云端文件 id(旧记录可能缺失, 用于重试)。
    #[serde(default)]
    pub file_id: String,
    pub name: String,
    pub dir: PathBuf,
    pub total: u64,
    pub done: u64,
    pub status: DownloadRecordStatus,
    /// 完成/失败时间(unix 秒)。
    #[serde(default)]
    pub at: u64,
    /// ISO 8601 时间戳。
    pub timestamp: String,
}

fn download_history_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("kichi").join("downloads.json"))
}

pub fn load_download_history() -> Vec<DownloadRecord> {
    let Some(p) = download_history_path() else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(p) else {
        return Vec::new();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

pub fn save_download_history(records: &[DownloadRecord]) {
    let Some(p) = download_history_path() else {
        return;
    };
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(text) = serde_json::to_string_pretty(records) {
        let _ = std::fs::write(p, text);
    }
}

pub fn append_download_record(record: DownloadRecord) {
    let mut history = load_download_history();
    // 插入到最前面, 最新的在最上面
    history.insert(0, record);
    // 保留最近 200 条记录
    if history.len() > 200 {
        history.truncate(200);
    }
    save_download_history(&history);
}

/// 从下载历史中移除与给定任务匹配的最新一条记录。
/// 仅删除列表记录, 不触碰已下载到本地的文件。
/// 优先用记录唯一标识 `rec_id` 精确匹配, 缺失时回退到 file_id / 名称+目录。
pub fn remove_download_record(rec_id: &str, file_id: &str, name: &str, dir: &std::path::Path) {
    let mut history = load_download_history();
    let idx = if !rec_id.is_empty() {
        history.iter().position(|r| r.timestamp == rec_id)
    } else {
        history.iter().position(|r| {
            if !file_id.is_empty() && !r.file_id.is_empty() {
                r.file_id == file_id
            } else {
                r.name == name && r.dir == dir
            }
        })
    };
    if let Some(i) = idx {
        history.remove(i);
        save_download_history(&history);
    }
}

/// 上传记录状态(仅保存已完成/失败的)。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum UploadRecordStatus {
    Done,
    Failed(String),
}

/// 单条上传历史记录。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadRecord {
    pub local_path: PathBuf,
    pub name: String,
    /// 目标网盘目录 (None = 根目录)。
    #[serde(default)]
    pub parent: Option<String>,
    /// 目标目录层级快照 (id, label), 用于展示与导航。
    #[serde(default)]
    pub dest_stack: Vec<(Option<String>, String)>,
    pub total: u64,
    pub done: u64,
    pub status: UploadRecordStatus,
    /// 是否为目录递归上传。
    #[serde(default)]
    pub is_dir: bool,
    /// 完成/失败时间(unix 秒)。
    #[serde(default)]
    pub at: u64,
    /// 唯一标识(纳秒时间戳字符串)。
    pub timestamp: String,
}

fn upload_history_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("kichi").join("uploads.json"))
}

pub fn load_upload_history() -> Vec<UploadRecord> {
    let Some(p) = upload_history_path() else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(p) else {
        return Vec::new();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

pub fn save_upload_history(records: &[UploadRecord]) {
    let Some(p) = upload_history_path() else {
        return;
    };
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(text) = serde_json::to_string_pretty(records) {
        let _ = std::fs::write(p, text);
    }
}

pub fn append_upload_record(record: UploadRecord) {
    let mut history = load_upload_history();
    // 最新的在最上面。
    history.insert(0, record);
    if history.len() > 200 {
        history.truncate(200);
    }
    save_upload_history(&history);
}

/// 从上传历史中移除一条记录(优先按唯一标识精确匹配)。
pub fn remove_upload_record(rec_id: &str, local_path: &std::path::Path, name: &str) {
    let mut history = load_upload_history();
    let idx = if !rec_id.is_empty() {
        history.iter().position(|r| r.timestamp == rec_id)
    } else {
        history
            .iter()
            .position(|r| r.name == name && r.local_path == local_path)
    };
    if let Some(i) = idx {
        history.remove(i);
        save_upload_history(&history);
    }
}

fn path() -> Option<std::path::PathBuf> {
    dirs::config_dir().map(|d| d.join("kichi").join("settings.json"))
}

pub fn load() -> Settings {
    let Some(p) = path() else {
        return Settings::default();
    };
    if let Ok(text) = std::fs::read_to_string(p) {
        if let Ok(s) = serde_json::from_str(&text) {
            return s;
        }
    }
    Settings::default()
}

pub fn save(s: &Settings) {
    let Some(p) = path() else { return };
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(text) = serde_json::to_string_pretty(s) {
        let _ = std::fs::write(p, text);
    }
}
