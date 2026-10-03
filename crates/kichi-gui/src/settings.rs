use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write as _;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};

/// 传输参数默认值与取值范围(UI / worker 共用, 加载时统一钳制)。
pub const DEFAULT_DL_CONCURRENCY: usize = 3;
pub const DEFAULT_UL_CONCURRENCY: usize = 2;
pub const DEFAULT_PART_CONCURRENCY: usize = 4;
pub const DEFAULT_MAX_ATTEMPTS: usize = 5;
pub const DL_CONCURRENCY_RANGE: (usize, usize) = (1, 8);
pub const UL_CONCURRENCY_RANGE: (usize, usize) = (1, 4);
pub const PART_CONCURRENCY_RANGE: (usize, usize) = (1, 10);
pub const MAX_ATTEMPTS_RANGE: (usize, usize) = (1, 10);

fn default_dl_concurrency() -> usize {
    DEFAULT_DL_CONCURRENCY
}
fn default_ul_concurrency() -> usize {
    DEFAULT_UL_CONCURRENCY
}
fn default_part_concurrency() -> usize {
    DEFAULT_PART_CONCURRENCY
}
fn default_max_attempts() -> usize {
    DEFAULT_MAX_ATTEMPTS
}

/// aria2 JSON-RPC 默认地址(aria2 与 Motrix 常见监听地址)。
pub const DEFAULT_ARIA2_RPC_URL: &str = "http://127.0.0.1:6800/jsonrpc";

fn default_aria2_rpc_url() -> String {
    DEFAULT_ARIA2_RPC_URL.to_string()
}

/// aria2 外部下载器设置(设置页维护, 推送时随命令下发)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Aria2Settings {
    /// 是否启用「发送到 aria2」入口。
    #[serde(default)]
    pub enabled: bool,
    /// JSON-RPC 地址。
    #[serde(default = "default_aria2_rpc_url")]
    pub rpc_url: String,
    /// RPC 密钥(可空); 只留本机配置, 不进日志。
    #[serde(default)]
    pub secret: String,
    /// 下载目录; 留空 = 读 aria2 自己的全局 `dir`。
    #[serde(default)]
    pub dir: String,
}

impl Default for Aria2Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            rpc_url: DEFAULT_ARIA2_RPC_URL.to_string(),
            secret: String::new(),
            dir: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub username: String,
    /// 上一次选择的本地下载目录(可空)。
    #[serde(default)]
    pub download_dir: String,
    /// 上一次本地选择框停留的目录(上传文件/文件夹对话框共用, 可空)。
    #[serde(default)]
    pub last_dir: String,
    /// 是否把密码保存到系统密钥环, 用于自动登录。
    #[serde(default)]
    pub remember_password: bool,
    /// 本地下载并发上限。
    #[serde(default = "default_dl_concurrency")]
    pub dl_concurrency: usize,
    /// 本地上传并发上限。
    #[serde(default = "default_ul_concurrency")]
    pub ul_concurrency: usize,
    /// 单个上传任务的分片并发数。
    #[serde(default = "default_part_concurrency")]
    pub part_concurrency: usize,
    /// 单个传输任务的最大尝试次数。
    #[serde(default = "default_max_attempts")]
    pub max_attempts: usize,
    /// aria2 外部下载器(「发送到 aria2」)。
    #[serde(default)]
    pub aria2: Aria2Settings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            username: String::new(),
            download_dir: String::new(),
            last_dir: String::new(),
            remember_password: false,
            dl_concurrency: DEFAULT_DL_CONCURRENCY,
            ul_concurrency: DEFAULT_UL_CONCURRENCY,
            part_concurrency: DEFAULT_PART_CONCURRENCY,
            max_attempts: DEFAULT_MAX_ATTEMPTS,
            aria2: Aria2Settings::default(),
        }
    }
}

/// 下载记录状态（仅保存已完成/取消/失败的）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum DownloadRecordStatus {
    Done,
    Cancelled,
    Failed(String),
}

/// 目录下载记录内联的树节点快照(不单独占用历史条目)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadChildRecord {
    #[serde(default)]
    pub file_id: String,
    pub name: String,
    /// 文件条目: 目标目录; 目录条目: 该目录的本地路径。
    pub dir: PathBuf,
    #[serde(default)]
    pub total: u64,
    #[serde(default)]
    pub done: u64,
    /// 文件条目状态; 目录条目固定为 Done(占位)。
    pub status: DownloadRecordStatus,
    /// 完成/失败时间(unix 秒)。
    #[serde(default)]
    pub at: u64,
    /// 是否为子目录条目。
    #[serde(default)]
    pub is_dir: bool,
    /// 层级: 目录卡片(root)=0, 其直接子项=1。
    #[serde(default)]
    pub depth: u32,
}

/// 单条下载历史记录。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadRecord {
    /// 云端文件 id(旧记录可能缺失, 用于重试)。目录记录为目录 id。
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
    /// 是否为整目录下载记录。
    #[serde(default)]
    pub is_folder: bool,
    /// 目录记录的个子文件快照(仅 is_folder 时有值)。
    #[serde(default)]
    pub children: Vec<DownloadChildRecord>,
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

/// 「转存自分享」暂存目录的持久化 ID。
/// 单独存一个文件, 避免与 GUI 线程整体覆写 settings.json 产生竞争。
fn pack_folder_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("kichi").join("pack_folder_id"))
}

/// 上传续传的有效期安全余量(秒): 距到期不足该值时不再尝试续传, 直接全量重传。
pub const RESUME_EXPIRY_MARGIN_SECS: i64 = 600;

/// 上传跨重启续传记录: 落盘的 OSS 临时凭证 + upload_id + 已传分片断点。
/// 含短期凭证, 文件权限 0600; 仅在有效期内且本地文件指纹一致时可用。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadResumeOss {
    pub endpoint: String,
    pub access_key_id: String,
    pub access_key_secret: String,
    pub security_token: String,
    pub bucket: String,
    pub key: String,
}

/// 单条上传续传记录(local_path + parent 唯一)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadResumeRecord {
    pub local_path: PathBuf,
    pub name: String,
    /// 目标网盘目录 (None = 根目录)。
    #[serde(default)]
    pub parent: Option<String>,
    /// 目标目录层级快照 (id, label), 用于还原卡片展示 / 导航。
    #[serde(default)]
    pub dest_stack: Vec<(Option<String>, String)>,
    /// 本地文件大小与 mtime(unix 毫秒), 续传前指纹校验。
    pub size: u64,
    pub mtime_ms: i64,
    /// 服务端占位文件 id(凭证失效 / 取消后清理用)。
    #[serde(default)]
    pub file_id: Option<String>,
    /// OSS 分片上传的 upload_id。
    pub upload_id: String,
    /// OSS 临时凭证与目标对象。
    pub oss: UploadResumeOss,
    /// 凭证到期时间(unix 秒); None = 未知(不可续传)。
    #[serde(default)]
    pub expiration_unix: Option<i64>,
    /// 已成功分片的 ETag (part_number -> etag)。
    #[serde(default)]
    pub etags: BTreeMap<u64, String>,
    /// 记录创建时间(unix 秒)。
    #[serde(default)]
    pub at: u64,
}

impl UploadResumeRecord {
    /// 记录当前是否可续传: 凭证距到期仍有余量, 且本地文件指纹一致。
    pub fn usable(&self, now: i64, size: u64, mtime_ms: i64) -> bool {
        let Some(expires) = self.expiration_unix else {
            return false;
        };
        now + RESUME_EXPIRY_MARGIN_SECS < expires && self.size == size && self.mtime_ms == mtime_ms
    }
}

/// 文件 mtime 的 unix 毫秒(取不到时为 0; 同一文件前后两次读取一致即可比对)。
pub fn mtime_ms(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn upload_resume_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("kichi").join("upload_resume.json"))
}

pub fn load_upload_resume() -> Vec<UploadResumeRecord> {
    let Some(p) = upload_resume_path() else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(p) else {
        return Vec::new();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

/// 写续传记录: 0600 + 原子写(先写临时文件再 rename), 避免进程中断写坏文件。
pub fn save_upload_resume(records: &[UploadResumeRecord]) {
    let Some(p) = upload_resume_path() else {
        return;
    };
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let Ok(text) = serde_json::to_string_pretty(records) else {
        return;
    };
    let tmp = p.with_extension("json.tmp");
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true).mode(0o600);
    let Ok(mut f) = opts.open(&tmp) else {
        return;
    };
    if f.write_all(text.as_bytes()).is_ok() && f.flush().is_ok() {
        let _ = std::fs::rename(&tmp, &p);
    }
}

/// 查一条续传记录(local_path + parent 唯一)。
pub fn find_upload_resume(local_path: &Path, parent: Option<&str>) -> Option<UploadResumeRecord> {
    load_upload_resume()
        .into_iter()
        .find(|r| r.local_path == local_path && r.parent.as_deref() == parent)
}

/// 新增 / 覆盖一条续传记录(worker 单写)。
pub fn upsert_upload_resume(record: UploadResumeRecord) {
    let mut list = load_upload_resume();
    list.retain(|r| !(r.local_path == record.local_path && r.parent == record.parent));
    list.push(record);
    save_upload_resume(&list);
}

/// 更新一条记录的已传分片(节流由调用方控制)。
pub fn update_upload_resume_etags(
    local_path: &Path,
    parent: Option<&str>,
    etags: &BTreeMap<u64, String>,
) {
    let mut list = load_upload_resume();
    let Some(r) = list
        .iter_mut()
        .find(|r| r.local_path == local_path && r.parent.as_deref() == parent)
    else {
        return;
    };
    r.etags = etags.clone();
    save_upload_resume(&list);
}

/// 移除一条续传记录并返回它(供调用方随后清理云端占位条目)。
pub fn take_upload_resume(local_path: &Path, parent: Option<&str>) -> Option<UploadResumeRecord> {
    let mut list = load_upload_resume();
    let idx = list
        .iter()
        .position(|r| r.local_path == local_path && r.parent.as_deref() == parent)?;
    let rec = list.remove(idx);
    save_upload_resume(&list);
    Some(rec)
}

pub fn load_pack_folder_id() -> Option<String> {
    let p = pack_folder_path()?;
    let text = std::fs::read_to_string(p).ok()?;
    let id = text.trim().to_string();
    (!id.is_empty()).then_some(id)
}

pub fn save_pack_folder_id(id: &str) {
    let Some(p) = pack_folder_path() else {
        return;
    };
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(p, id);
}

fn path() -> Option<std::path::PathBuf> {
    dirs::config_dir().map(|d| d.join("kichi").join("settings.json"))
}

pub fn load() -> Settings {
    let Some(p) = path() else {
        return Settings::default();
    };
    if let Ok(text) = std::fs::read_to_string(p) {
        if let Ok(mut s) = serde_json::from_str::<Settings>(&text) {
            // 手工改坏 settings.json 时兜底钳制, 避免 0 并发把任务饿死。
            s.dl_concurrency = clamp(s.dl_concurrency, DL_CONCURRENCY_RANGE);
            s.ul_concurrency = clamp(s.ul_concurrency, UL_CONCURRENCY_RANGE);
            s.part_concurrency = clamp(s.part_concurrency, PART_CONCURRENCY_RANGE);
            s.max_attempts = clamp(s.max_attempts, MAX_ATTEMPTS_RANGE);
            return s;
        }
    }
    Settings::default()
}

fn clamp(v: usize, (lo, hi): (usize, usize)) -> usize {
    v.clamp(lo, hi)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> UploadResumeRecord {
        let mut etags = BTreeMap::new();
        etags.insert(1u64, "e1".to_string());
        etags.insert(2u64, "e2".to_string());
        UploadResumeRecord {
            local_path: PathBuf::from("/tmp/video.mkv"),
            name: "video.mkv".into(),
            parent: Some("P1".into()),
            dest_stack: vec![(Some("P0".into()), "我的云盘".into())],
            size: 12 * 1024 * 1024,
            mtime_ms: 1_700_000_000_123,
            file_id: Some("F1".into()),
            upload_id: "UP1".into(),
            oss: UploadResumeOss {
                endpoint: "oss.example.com".into(),
                access_key_id: "ak".into(),
                access_key_secret: "sk".into(),
                security_token: "tok".into(),
                bucket: "bkt".into(),
                key: "upload_tmp/ABC_1".into(),
            },
            expiration_unix: Some(1_791_056_314),
            etags,
            at: 1_791_000_000,
        }
    }

    #[test]
    fn resume_usable_checks_expiry_margin_and_fingerprint() {
        let r = record();
        let now = 1_791_000_000;
        // 距到期 56_314s > 600s 余量, 指纹一致。
        assert!(r.usable(now, r.size, r.mtime_ms));
        // 余量边界: 恰好 600s 时不再续传。
        assert!(!r.usable(
            1_791_056_314 - RESUME_EXPIRY_MARGIN_SECS,
            r.size,
            r.mtime_ms
        ));
        assert!(r.usable(
            1_791_056_314 - RESUME_EXPIRY_MARGIN_SECS - 1,
            r.size,
            r.mtime_ms
        ));
        // 已过期。
        assert!(!r.usable(1_791_056_315, r.size, r.mtime_ms));
        // 指纹不符: 大小 / mtime 任一变化都不可续传。
        assert!(!r.usable(now, r.size + 1, r.mtime_ms));
        assert!(!r.usable(now, r.size, r.mtime_ms + 1));
        // 到期时间未知 = 不可续传。
        let mut unknown = record();
        unknown.expiration_unix = None;
        assert!(!unknown.usable(now, unknown.size, unknown.mtime_ms));
    }

    #[test]
    fn resume_record_roundtrip() {
        let list = vec![record()];
        let text = serde_json::to_string(&list).unwrap();
        let back: Vec<UploadResumeRecord> = serde_json::from_str(&text).unwrap();
        assert_eq!(back.len(), 1);
        let r = &back[0];
        assert_eq!(r.etags.len(), 2);
        assert_eq!(r.etags.get(&2).map(String::as_str), Some("e2"));
        assert_eq!(r.parent.as_deref(), Some("P1"));
        assert_eq!(r.dest_stack.len(), 1);
        assert_eq!(r.oss.key, "upload_tmp/ABC_1");
    }
}
