use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::Error;

/// 串行化会话文件的读写, 避免并发的 token 续期同时写盘造成文件损坏。
static SESSION_FILE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Session {
    /// 登录时注册的设备 id(32 位 hex), 持久化以保持设备一致性。
    #[serde(default)]
    pub device_id: String,
    #[serde(default)]
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: String,
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub username: String,
}

impl Session {
    pub fn is_logged_in(&self) -> bool {
        !self.access_token.is_empty() && !self.refresh_token.is_empty()
    }
}

pub fn session_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("pikpak-linux").join("session.json"))
}

pub fn load_session() -> Result<Option<Session>, Error> {
    let Some(path) = session_path() else {
        return Ok(None);
    };
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)?;
    let session: Session = serde_json::from_str(&text)?;
    if session.is_logged_in() {
        Ok(Some(session))
    } else {
        Ok(None)
    }
}

pub fn save_session(session: &Session) -> Result<(), Error> {
    let Some(path) = session_path() else {
        return Ok(());
    };
    let _guard = SESSION_FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = serde_json::to_string_pretty(session)?;
    // 原子写: 先写同目录临时文件再 rename, 避免进程中断或并发写把会话文件写坏/截断。
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

pub fn clear_session() -> Result<(), Error> {
    let Some(path) = session_path() else {
        return Ok(());
    };
    let _guard = SESSION_FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    // 顺带清掉可能遗留的临时文件。
    let _ = std::fs::remove_file(path.with_extension("json.tmp"));
    Ok(())
}
