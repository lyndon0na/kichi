use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::Error;

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
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = serde_json::to_string_pretty(session)?;
    std::fs::write(&path, text)?;
    Ok(())
}

pub fn clear_session() -> Result<(), Error> {
    let Some(path) = session_path() else {
        return Ok(());
    };
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}
