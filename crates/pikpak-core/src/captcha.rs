use std::time::{SystemTime, UNIX_EPOCH};

use md5::{Digest, Md5};
use uuid::Uuid;

use crate::consts::*;

pub fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

pub fn md5_hex(input: &str) -> String {
    let mut hasher = Md5::new();
    hasher.update(input.as_bytes());
    hex_of(hasher.finalize())
}

/// 生成 32 位十六进制 device id(与官方一致)。
pub fn generate_device_id() -> String {
    Uuid::new_v4().simple().to_string()
}

/// captcha_sign: `1.` + 对 (client_id+version+package+device_id+timestamp) 做多轮 md5。
pub fn captcha_sign(device_id: &str, timestamp_ms: u128) -> String {
    let mut sign = format!(
        "{}{}{}{}{}",
        CLIENT_ID, CLIENT_VERSION, PACKAGE_NAME, device_id, timestamp_ms
    );
    for salt in CAPTCHA_SALTS {
        sign = md5_hex(&format!("{sign}{salt}"));
    }
    format!("1.{sign}")
}

/// 仿官方 app 的 user agent。
pub fn build_user_agent(device_id: &str, user_id: &str) -> String {
    let device_sign = generate_device_sign(device_id);
    format!(
        "ANDROID-{APP_NAME}/{CLIENT_VERSION} \
         protocolVersion/200 \
         accesstype/ \
         clientid/{CLIENT_ID} \
         clientversion/{CLIENT_VERSION} \
         action_type/ \
         networktype/WIFI \
         sessionid/ \
         deviceid/{device_id} \
         providername/NONE \
         devicesign/{device_sign} \
         refresh_token/ \
         sdkversion/{SDK_VERSION} \
         datetime/{} \
         usrno/{user_id} \
         appname/{APP_NAME} \
         session_origin/ \
         grant_type/ \
         appid/ \
         clientip/ \
         devicename/{DEVICE_NAME} \
         osversion/{OS_VERSION} \
         platformversion/{PLATFORM_VERSION} \
         accessmode/ \
         devicemodel/{DEVICE_MODEL}",
        now_ms()
    )
}

/// device_sign = "div101." + device_id + md5(sha1(device_id+package+1appkey))
fn generate_device_sign(device_id: &str) -> String {
    let base = format!("{device_id}{PACKAGE_NAME}1appkey");
    use sha1::{Digest, Sha1};
    let mut hasher = Sha1::new();
    hasher.update(base.as_bytes());
    let sha1_hex = hex_of(hasher.finalize());
    let md5_of_sha1 = md5_hex(&sha1_hex);
    format!("div101.{device_id}{md5_of_sha1}")
}

fn hex_of(bytes: impl AsRef<[u8]>) -> String {
    use std::fmt::Write;
    let bytes = bytes.as_ref();
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captcha_sign_matches_reference() {
        // 参考值由官方 Python 库 Quan666/PikPakAPI 在固定输入下计算得出。
        let device_id = "5f4dcc3b5aa765d61d8327deb882cf99";
        let ts: u128 = 1737357618123;
        let sign = captcha_sign(device_id, ts);
        assert_eq!(sign, "1.97278c66b465b115e1f38c40ff728757");
    }

    #[test]
    fn device_sign_matches_reference() {
        let device_id = "5f4dcc3b5aa765d61d8327deb882cf99";
        assert_eq!(
            generate_device_sign(device_id),
            "div101.5f4dcc3b5aa765d61d8327deb882cf9972c43e8d4464589d0ccd410d95a862cd"
        );
    }

    #[test]
    fn device_id_is_32_hex() {
        let id = generate_device_id();
        assert_eq!(id.len(), 32);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
