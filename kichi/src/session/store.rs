//! KWallet（Secret Service）存取令牌；不可用时降级到 0600 权限文件。
//!
//! 降级文件只用于自用场景的容错，界面必须把降级状态显式告诉用户。

use std::fs;
use std::io::Write;
use std::path::Path;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;

use crate::session::PersistedSession;

const SERVICE: &str = "kichi";
const ACCOUNT: &str = "session";

/// 令牌落盘位置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// 系统钥匙串（KWallet / KSecretService）
    Keychain,
    /// 降级：0600 文件
    File,
}

/// 会话存储错误（含降级原因，给界面提示用）。
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("钥匙串不可用：{0}")]
    Keychain(String),
    #[error("文件读写失败：{0}")]
    Io(String),
    #[error("保存的数据已损坏：{0}")]
    Corrupt(String),
}

impl From<keyring::Error> for StoreError {
    fn from(err: keyring::Error) -> Self {
        StoreError::Keychain(keyring_error_reason(&err))
    }
}

fn keyring_error_reason(err: &keyring::Error) -> String {
    match err {
        keyring::Error::NoEntry => "没有已保存的会话".to_owned(),
        keyring::Error::NoStorageAccess(e) => format!("无法访问系统存储：{e}"),
        keyring::Error::PlatformFailure(e) => format!("系统存储故障：{e}"),
        other => other.to_string(),
    }
}

/// 读取会话：先钥匙串，再降级文件。
///
/// 返回 `(会话, 来源)`；两处都没有则 `Ok(None)`。
pub fn load(fallback_path: &Path) -> Result<Option<(PersistedSession, Backend)>, StoreError> {
    // 钥匙串不可用不是错误：继续看降级文件
    if let Ok(Some(session)) = load_keychain() {
        return Ok(Some((session, Backend::Keychain)));
    }
    match load_file(fallback_path) {
        Ok(Some(session)) => Ok(Some((session, Backend::File))),
        Ok(None) => Ok(None),
        Err(err) => Err(err),
    }
}

/// 保存会话：优先钥匙串；失败时降级到文件并返回实际使用的后端。
pub fn save(fallback_path: &Path, session: &PersistedSession) -> Result<Backend, StoreError> {
    let data = encode(session)?;
    match save_keychain(&data) {
        Ok(()) => Ok(Backend::Keychain),
        Err(err) => {
            eprintln!("[kichi] 钥匙串不可用，降级为 0600 文件：{err}");
            save_file(fallback_path, data.as_bytes())?;
            Ok(Backend::File)
        }
    }
}

/// 清除会话（两处都清，忽略“不存在”）。
pub fn clear(fallback_path: &Path) -> Result<(), StoreError> {
    if let Ok(entry) = keyring::Entry::new(SERVICE, ACCOUNT) {
        // 不存在或后端不可用都视为“已清除”；后端不可用时本来也没存进去
        if let Err(err) = entry.delete_credential() {
            if !matches!(err, keyring::Error::NoEntry) {
                eprintln!("[kichi] 清除钥匙串条目失败：{err}");
            }
        }
    }
    match fs::remove_file(fallback_path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(StoreError::Io(err.to_string())),
    }
}

fn load_keychain() -> Result<Option<PersistedSession>, StoreError> {
    let entry = keyring::Entry::new(SERVICE, ACCOUNT)?;
    match entry.get_password() {
        Ok(text) => decode(&text).map(Some),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(err) => Err(err.into()),
    }
}

fn save_keychain(data: &str) -> Result<(), StoreError> {
    let entry = keyring::Entry::new(SERVICE, ACCOUNT)?;
    entry.set_password(data)?;
    Ok(())
}

fn load_file(path: &Path) -> Result<Option<PersistedSession>, StoreError> {
    match fs::read_to_string(path) {
        Ok(text) => decode(&text).map(Some),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(StoreError::Io(err.to_string())),
    }
}

fn save_file(path: &Path, bytes: &[u8]) -> Result<(), StoreError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| StoreError::Io(e.to_string()))?;
    }
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|e| StoreError::Io(e.to_string()))?;
    // `mode` 只在新建时生效：旧版本按 umask 留下的 0644 文件不会自动收紧，
    // 这里补一次，保证每次写入后都是 0600。
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|e| StoreError::Io(e.to_string()))?;
    }
    file.write_all(bytes)
        .map_err(|e| StoreError::Io(e.to_string()))?;
    Ok(())
}

/// 编码为 Base64(JSON)，避免钥匙串对特殊字符的兼容性问题。
fn encode(session: &PersistedSession) -> Result<String, StoreError> {
    let json = serde_json::to_vec(session).map_err(|e| StoreError::Corrupt(e.to_string()))?;
    Ok(BASE64.encode(json))
}

fn decode(text: &str) -> Result<PersistedSession, StoreError> {
    let raw = BASE64
        .decode(text.trim())
        .map_err(|e| StoreError::Corrupt(e.to_string()))?;
    serde_json::from_slice(&raw).map_err(|e| StoreError::Corrupt(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn sample() -> PersistedSession {
        PersistedSession {
            access_token: "a".into(),
            refresh_token: "r".into(),
            expires_unix: Utc::now().timestamp(),
            device_id: "d".into(),
            user_id: "u".into(),
            email: "e@example.com".into(),
            display_name: "昵称".into(),
            avatar_url: "https://user.mypikpak.com/avatar/1".into(),
        }
    }

    #[test]
    fn encode_decode_roundtrip() {
        let text = encode(&sample()).unwrap_or_else(|e| panic!("{e}"));
        let back = decode(&text).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(back.refresh_token, "r");
    }

    #[test]
    fn corrupt_payload_is_reported() {
        assert!(matches!(decode("不是 base64"), Err(StoreError::Corrupt(_))));
    }

    #[test]
    fn file_fallback_is_private() {
        let dir = std::env::temp_dir().join(format!("kichi-store-{}", std::process::id()));
        let path = dir.join("session.token");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // 模拟旧版本按 umask 留下的宽松文件：写入时必须收紧到 0600
            fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
            fs::write(&path, b"old").unwrap_or_else(|e| panic!("{e}"));
            fs::set_permissions(&path, fs::Permissions::from_mode(0o644))
                .unwrap_or_else(|e| panic!("{e}"));
        }
        save_file(&path, b"secret").unwrap_or_else(|e| panic!("{e}"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path)
                .map(|m| m.permissions().mode() & 0o777)
                .unwrap_or(0);
            assert_eq!(mode, 0o600);
        }
        let _ = fs::remove_dir_all(&dir);
    }
}
