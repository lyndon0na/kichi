//! 系统密钥环读写账号密码。
//!
//! 通过 Secret Service（KDE Wallet / GNOME Keyring 等）加密保存密码, 避免明文入库。
//! 所有函数都是阻塞的 D-Bus 调用, 调用方应放到阻塞线程上执行。

/// 密钥环条目的 service 名。
const SERVICE: &str = "pikpak-linux";

fn entry(username: &str) -> keyring::Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, username)
}

/// 把账号密码写入系统密钥环。
pub fn save(username: &str, password: &str) -> Result<(), String> {
    entry(username)
        .and_then(|e| e.set_password(password))
        .map_err(|e| format!("保存密码到密钥环失败: {e}"))
}

/// 读取已保存的密码; 密钥环不可用或没有条目时返回 None。
pub fn load(username: &str) -> Option<String> {
    let e = entry(username).ok()?;
    e.get_password().ok()
}

/// 删除已保存的密码(尽力而为)。
pub fn delete(username: &str) {
    if let Ok(e) = entry(username) {
        let _ = e.delete_credential();
    }
}
