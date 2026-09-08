pub const CLIENT_ID: &str = "YNxT9w7GMdWvEOKa";
pub const CLIENT_SECRET: &str = "dbw2OtmVEeuUvIptb1Coyg";
pub const CLIENT_VERSION: &str = "1.47.1";
pub const PACKAGE_NAME: &str = "com.pikcloud.pikpak";
pub const APP_NAME: &str = PACKAGE_NAME;
pub const SDK_VERSION: &str = "2.0.4.204000 ";

pub const USER_HOST: &str = "https://user.mypikpak.com";
pub const API_HOST: &str = "https://api-drive.mypikpak.com";

/// 设备/模型相关常量，仅用于构造 UA。
pub const DEVICE_NAME: &str = "Xiaomi_M2004j7ac";
pub const DEVICE_MODEL: &str = "M2004J7AC";
pub const OS_VERSION: &str = "13";
pub const PLATFORM_VERSION: &str = "10";

/// captcha_sign 的多轮 md5 盐值表(与官方 app 内 js 一致)。
pub const CAPTCHA_SALTS: [&str; 15] = [
    "Gez0T9ijiI9WCeTsKSg3SMlx",
    "zQdbalsolyb1R/",
    "ftOjr52zt51JD68C3s",
    "yeOBMH0JkbQdEFNNwQ0RI9T3wU/v",
    "BRJrQZiTQ65WtMvwO",
    "je8fqxKPdQVJiy1DM6Bc9Nb1",
    "niV",
    "9hFCW2R1",
    "sHKHpe2i96",
    "p7c5E6AcXQ/IJUuAEC9W6",
    "",
    "aRv9hjc9P+Pbn+u3krN6",
    "BzStcgE8qVdqjEH16l4",
    "SqgeZvL5j9zoHP95xWHt",
    "zVof5yaJkPe3VFpadPof",
];

/// 文件请求默认过滤条件。
pub fn default_file_filters() -> serde_json::Value {
    serde_json::json!({
        "trashed": { "eq": false },
        "phase": { "eq": "PHASE_TYPE_COMPLETE" },
    })
}

/// 离线任务全部可能的状态。
pub const OFFLINE_PHASES: [&str; 4] = [
    "PHASE_TYPE_PENDING",
    "PHASE_TYPE_RUNNING",
    "PHASE_TYPE_COMPLETE",
    "PHASE_TYPE_ERROR",
];
