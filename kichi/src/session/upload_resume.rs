//! 上传断点的 KWallet 存储。
//!
//! 断点包含 S3 临时凭据，只能进系统钥匙串；钥匙串不可用时退回本次进程
//! 内存，**绝不**写明文文件。分片 ETag 不落盘，重启后由 `ListParts` 重建。

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use kichi_core::UploadResume;
use serde::{Deserialize, Serialize};

const SERVICE: &str = "kichi";
const ACCOUNT: &str = "upload-resume";
const STORE_VERSION: u32 = 1;
const MAX_AGE: Duration = Duration::from_secs(14 * 24 * 60 * 60);
const MAX_ENTRIES_PER_USER: usize = 100;
const MIN_WRITE_INTERVAL: Duration = Duration::from_secs(1);

static WARNED_NOT_PERSISTENT: AtomicBool = AtomicBool::new(false);

#[derive(Default, Serialize, Deserialize)]
struct DiskStore {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    accounts: BTreeMap<String, BTreeMap<String, UploadResume>>,
}

struct State {
    loaded: bool,
    persistent: bool,
    disk: DiskStore,
    last_write: Option<Instant>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            loaded: false,
            persistent: false,
            disk: DiskStore {
                version: STORE_VERSION,
                accounts: BTreeMap::new(),
            },
            last_write: None,
        }
    }
}

fn state() -> &'static Mutex<State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(State::default()))
}

fn with_state<T>(f: impl FnOnce(&mut State) -> T) -> T {
    let mut guard = state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    f(&mut guard)
}

fn ensure_loaded(state: &mut State) {
    if state.loaded {
        return;
    }
    state.loaded = true;
    if memory_only_for_test() {
        state.persistent = false;
        return;
    }
    let entry = match keyring::Entry::new(SERVICE, ACCOUNT) {
        Ok(entry) => entry,
        Err(_) => {
            state.persistent = false;
            return;
        }
    };
    match entry.get_password() {
        Ok(text) => match decode(&text) {
            Ok(disk) => {
                state.disk = disk;
                state.persistent = true;
            }
            Err(_) => {
                // 损坏的条目不能静默覆盖；本次运行只使用内存，界面会提示。
                state.persistent = false;
            }
        },
        Err(keyring::Error::NoEntry) => {
            state.persistent = true;
        }
        Err(_) => {
            state.persistent = false;
        }
    }
}

/// 读取一个断点；过期或损坏的条目直接丢弃。
pub(crate) fn load(user_id: &str, key: &str) -> Option<UploadResume> {
    with_state(|state| {
        ensure_loaded(state);
        prune(state);
        state
            .disk
            .accounts
            .get(user_id)
            .and_then(|entries| entries.get(key))
            .cloned()
    })
}

/// 写入一个断点。`force=false` 时最多每秒落盘一次；关键节点应传 `true`。
pub(crate) fn save(
    user_id: &str,
    key: &str,
    resume: &UploadResume,
    force: bool,
) -> Result<(), String> {
    with_state(|state| {
        ensure_loaded(state);
        let mut resume = resume.clone();
        if resume.updated_unix == 0 {
            resume.touch();
        }
        state
            .disk
            .accounts
            .entry(user_id.to_owned())
            .or_default()
            .insert(key.to_owned(), resume);
        prune(state);
        if !state.persistent {
            return Err("系统钥匙串不可用，上传断点只保留在本次运行内存".to_owned());
        }
        let due = state
            .last_write
            .is_none_or(|last| last.elapsed() >= MIN_WRITE_INTERVAL);
        if !force && !due {
            return Ok(());
        }
        persist(state)
    })
}

/// 删除一个断点；钥匙串不可用时至少清掉内存副本。
pub(crate) fn remove(user_id: &str, key: &str) {
    with_state(|state| {
        ensure_loaded(state);
        let changed = state
            .disk
            .accounts
            .get_mut(user_id)
            .is_some_and(|entries| entries.remove(key).is_some());
        if changed && state.persistent {
            let _ = persist(state);
        }
    })
}

/// 原子取走一批断点；取消上传时用它拿到清理服务端残留所需的状态。
///
/// 一次调用只重写一次钥匙串条目，整批取消大文件夹时不会逐文件阻塞主线程。
pub(crate) fn take_many(
    user_id: &str,
    keys: impl IntoIterator<Item = String>,
) -> Vec<UploadResume> {
    with_state(|state| {
        ensure_loaded(state);
        prune(state);
        let mut resumes = Vec::new();
        if let Some(entries) = state.disk.accounts.get_mut(user_id) {
            for key in keys {
                if let Some(resume) = entries.remove(&key) {
                    resumes.push(resume);
                }
            }
        }
        if !resumes.is_empty() && state.persistent {
            let _ = persist(state);
        }
        resumes
    })
}

/// 当前断点是否能跨重启保存。
pub(crate) fn persistent() -> bool {
    with_state(|state| {
        ensure_loaded(state);
        state.persistent
    })
}

/// 本次运行第一次确认钥匙串不可用时返回一句提示；成功后不再重复打扰。
pub(crate) fn take_persistence_warning() -> Option<&'static str> {
    if persistent() {
        return None;
    }
    if WARNED_NOT_PERSISTENT.swap(true, Ordering::Relaxed) {
        None
    } else {
        Some("系统钥匙串不可用：上传断点只保留到本次退出，重启后无法续传")
    }
}

fn prune(state: &mut State) {
    let now = chrono::Utc::now().timestamp();
    let max_age = i64::try_from(MAX_AGE.as_secs()).unwrap_or(i64::MAX);
    for entries in state.disk.accounts.values_mut() {
        entries.retain(|_, resume| {
            resume.updated_unix == 0 || now.saturating_sub(resume.updated_unix) <= max_age
        });
        if entries.len() > MAX_ENTRIES_PER_USER {
            let mut ordered: Vec<(i64, String)> = entries
                .iter()
                .map(|(key, resume)| (resume.updated_unix, key.clone()))
                .collect();
            ordered.sort_by_key(|(updated, _)| *updated);
            for (_, key) in ordered
                .into_iter()
                .take(entries.len().saturating_sub(MAX_ENTRIES_PER_USER))
            {
                entries.remove(&key);
            }
        }
    }
}

fn persist(state: &mut State) -> Result<(), String> {
    let entry = keyring::Entry::new(SERVICE, ACCOUNT)
        .map_err(|_| "系统钥匙串不可用，上传断点只保留在本次运行内存".to_owned())?;
    let encoded = encode(&state.disk)?;
    entry
        .set_password(&encoded)
        .map_err(|_| "无法写入系统钥匙串，上传断点只保留在本次运行内存".to_owned())?;
    state.last_write = Some(Instant::now());
    Ok(())
}

fn encode(disk: &DiskStore) -> Result<String, String> {
    let json = serde_json::to_vec(disk).map_err(|_| "上传断点序列化失败".to_owned())?;
    Ok(BASE64.encode(json))
}

fn decode(text: &str) -> Result<DiskStore, String> {
    let raw = BASE64
        .decode(text.trim())
        .map_err(|_| "上传断点不是有效的 Base64".to_owned())?;
    serde_json::from_slice(&raw).map_err(|_| "上传断点 JSON 已损坏".to_owned())
}

#[cfg(test)]
static MEMORY_ONLY: AtomicBool = AtomicBool::new(false);

#[cfg(test)]
fn memory_only_for_test() -> bool {
    MEMORY_ONLY.load(Ordering::Relaxed)
}

#[cfg(not(test))]
fn memory_only_for_test() -> bool {
    false
}

#[cfg(test)]
pub(crate) fn set_memory_only_for_test(value: bool) {
    MEMORY_ONLY.store(value, Ordering::Relaxed);
    WARNED_NOT_PERSISTENT.store(false, Ordering::Relaxed);
    *state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = State::default();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> UploadResume {
        serde_json::from_value(serde_json::json!({
            "version": 1,
            "parent_id": "parent",
            "file_id": "file",
            "task_id": "task",
            "gcid": "GCID",
            "requested_name": "a.mkv",
            "final_name": "a.mkv",
            "size": 12,
            "local_modified_unix_secs": 34,
            "local_modified_nanos": 56,
            "credentials": {
                "access_key_id": "access",
                "access_key_secret": "secret",
                "security_token": "token",
                "bucket": "bucket",
                "key": "key",
                "endpoint": "https://bucket.mypikpak.com",
                "expiration": "2099-01-01T00:00:00Z"
            },
            "upload_id": "upload",
            "part_size": 5,
            "bytes_sent": 5,
            "content_uploaded": false,
            "updated_unix": 1
        }))
        .expect("sample")
    }

    #[test]
    fn memory_fallback_stores_and_removes_without_keyring() {
        set_memory_only_for_test(true);
        let key = "path\u{1f}parent";
        let mut resume = sample();
        resume.updated_unix = chrono::Utc::now().timestamp();
        assert!(save("user", key, &resume, true).is_err());
        assert_eq!(
            load("user", key).map(|item| item.file_id),
            Some("file".into())
        );
        assert!(!persistent());
        assert_eq!(
            take_persistence_warning(),
            Some("系统钥匙串不可用：上传断点只保留到本次退出，重启后无法续传")
        );
        assert_eq!(take_persistence_warning(), None, "本次运行只提示一次");
        assert_eq!(
            take_many("user", [key.to_owned()])
                .pop()
                .map(|item| item.file_id),
            Some("file".into())
        );
        assert!(load("user", key).is_none());
        set_memory_only_for_test(false);
    }

    #[test]
    fn disk_store_roundtrip_is_base64_json() {
        let mut disk = DiskStore {
            version: STORE_VERSION,
            accounts: BTreeMap::new(),
        };
        disk.accounts
            .entry("user".into())
            .or_default()
            .insert("key".into(), sample());
        let encoded = encode(&disk).expect("encode");
        let back = decode(&encoded).expect("decode");
        assert_eq!(back.accounts["user"]["key"].file_id, "file");
    }
}
