//! PikPak 端点与客户端身份常量。
//!
//! 默认值移植自 rclone 的 PikPak 后端（MIT 许可，backend/pikpak），
//! 全部可被 `KICHI_*` 环境变量覆盖：官方接口变动时改配置即可，无需改代码。

use std::env;

/// OAuth client id（伪装官方 Web 客户端身份的一部分）
pub const DEFAULT_CLIENT_ID: &str = "YUMx5nI8ZU8Ap8pm";
/// 客户端版本号（参与 captcha 签名）
pub const DEFAULT_CLIENT_VERSION: &str = "2.0.0";
/// 包名（参与 captcha 签名）
pub const DEFAULT_PACKAGE_NAME: &str = "mypikpak.com";
/// 默认 User-Agent
pub const DEFAULT_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:129.0) Gecko/20100101 Firefox/129.0";

/// 认证服务基地址
pub const DEFAULT_AUTH_BASE: &str = "https://user.mypikpak.com";
/// 网盘 API 基地址
pub const DEFAULT_DRIVE_BASE: &str = "https://api-drive.mypikpak.com";
/// S3 上传基地址（实际对象地址为 `<bucket>.mypikpak.com/<key>`）
pub const DEFAULT_UPLOAD_BASE: &str = "https://mypikpak.com";

// —— API 路径（拼在基地址后）——
/// 登录（用户名密码）
pub const PATH_SIGNIN: &str = "/v1/auth/signin";
/// 令牌刷新（oauth2 refresh_token 流程）
pub const PATH_TOKEN: &str = "/v1/auth/token";
/// 申请 captcha 令牌
pub const PATH_CAPTCHA_INIT: &str = "/v1/shield/captcha/init";
/// 当前用户信息
pub const PATH_USER_ME: &str = "/v1/user/me";
/// 容量与用量
pub const PATH_ABOUT: &str = "/drive/v1/about";
/// 会员状态
pub const PATH_VIP: &str = "/drive/v1/privilege/vip";
/// 月度传输配额（云下载 / 下行 / 上传）
pub const PATH_TRANSFER_QUOTA: &str = "/vip/v1/quantity/list";
/// 文件列表 / 创建文件夹 / 创建离线任务
pub const PATH_FILES: &str = "/drive/v1/files";
/// 离线任务列表 / 删除任务
pub const PATH_TASKS: &str = "/drive/v1/tasks";
/// 创建分享链接
pub const PATH_SHARE: &str = "/drive/v1/share";
/// 分享列表
pub const PATH_SHARE_LIST: &str = "/drive/v1/share/list";
/// 账号事件（播放历史 / 上报播放进度）
pub const PATH_EVENTS: &str = "/drive/v1/events";
/// CID → GCID 查询（上传秒传预检）
pub const PATH_RESOURCE_CID: &str = "/drive/v1/resource/cid";
/// 解析磁力 / 云下载链接里的资源列表（秒传前选文件）
pub const PATH_RESOURCE_LIST: &str = "/drive/v1/resource/list";

/// `KICHI_*` 环境变量名。
pub mod env_key {
    pub const AUTH_BASE: &str = "KICHI_AUTH_BASE";
    pub const DRIVE_BASE: &str = "KICHI_DRIVE_BASE";
    pub const UPLOAD_BASE: &str = "KICHI_UPLOAD_BASE";
    pub const CLIENT_ID: &str = "KICHI_CLIENT_ID";
    pub const CLIENT_VERSION: &str = "KICHI_CLIENT_VERSION";
    pub const USER_AGENT: &str = "KICHI_USER_AGENT";
}

/// 运行时配置：端点与客户端身份。
///
/// 通过 [`Config::from_env`] 构造时，各字段可被同名 `KICHI_*` 环境变量覆盖。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// 认证服务基地址（默认 <https://user.mypikpak.com>）
    pub auth_base: String,
    /// 网盘 API 基地址（默认 <https://api-drive.mypikpak.com>）
    pub drive_base: String,
    /// S3 上传基地址（默认 <https://mypikpak.com>）
    pub upload_base: String,
    pub client_id: String,
    pub client_version: String,
    /// 包名（参与 captcha 签名）
    pub package_name: String,
    pub user_agent: String,
    /// 设备 id：32 位十六进制；空串表示尚未生成（由应用层生成并持久化）
    pub device_id: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            auth_base: DEFAULT_AUTH_BASE.into(),
            drive_base: DEFAULT_DRIVE_BASE.into(),
            upload_base: DEFAULT_UPLOAD_BASE.into(),
            client_id: DEFAULT_CLIENT_ID.into(),
            client_version: DEFAULT_CLIENT_VERSION.into(),
            package_name: DEFAULT_PACKAGE_NAME.into(),
            user_agent: DEFAULT_USER_AGENT.into(),
            device_id: String::new(),
        }
    }
}

impl Config {
    /// 从环境变量构造，未设置的字段取默认值；基地址末尾多余的 `/` 会被去掉。
    pub fn from_env() -> Self {
        let mut cfg = Config::default();
        if let Some(v) = read_env(env_key::AUTH_BASE) {
            cfg.auth_base = v;
        }
        if let Some(v) = read_env(env_key::DRIVE_BASE) {
            cfg.drive_base = v;
        }
        if let Some(v) = read_env(env_key::UPLOAD_BASE) {
            cfg.upload_base = v;
        }
        if let Some(v) = read_env(env_key::CLIENT_ID) {
            cfg.client_id = v;
        }
        if let Some(v) = read_env(env_key::CLIENT_VERSION) {
            cfg.client_version = v;
        }
        if let Some(v) = read_env(env_key::USER_AGENT) {
            cfg.user_agent = v;
        }
        cfg.auth_base = trim_base(&cfg.auth_base);
        cfg.drive_base = trim_base(&cfg.drive_base);
        cfg.upload_base = trim_base(&cfg.upload_base);
        cfg
    }

    // —— 认证侧端点 ——

    pub fn signin_url(&self) -> String {
        join(&self.auth_base, PATH_SIGNIN)
    }
    pub fn token_url(&self) -> String {
        join(&self.auth_base, PATH_TOKEN)
    }
    pub fn captcha_init_url(&self) -> String {
        join(&self.auth_base, PATH_CAPTCHA_INIT)
    }
    pub fn user_me_url(&self) -> String {
        join(&self.auth_base, PATH_USER_ME)
    }

    // —— 网盘侧端点 ——

    pub fn about_url(&self) -> String {
        join(&self.drive_base, PATH_ABOUT)
    }
    pub fn vip_url(&self) -> String {
        join(&self.drive_base, PATH_VIP)
    }
    /// 月度传输配额；`type=transfer` 是官方 Web 客户端的固定参数。
    pub fn transfer_quota_url(&self) -> String {
        format!(
            "{}?type=transfer&limit=200",
            join(&self.drive_base, PATH_TRANSFER_QUOTA)
        )
    }
    pub fn files_url(&self) -> String {
        join(&self.drive_base, PATH_FILES)
    }
    /// 服务端一次性清空回收站（返回异步 `task_id`）。
    pub fn trash_empty_url(&self) -> String {
        format!("{}/trash:empty", self.files_url())
    }
    pub fn file_url(&self, id: &str) -> String {
        format!("{}/{}", self.files_url(), id)
    }
    /// 批量操作（batchMove / batchCopy / batchTrash / batchUntrash / batchDelete）
    pub fn file_action_url(&self, action: &str) -> String {
        format!("{}:{action}", self.files_url())
    }
    pub fn tasks_url(&self) -> String {
        join(&self.drive_base, PATH_TASKS)
    }
    pub fn task_url(&self, id: &str) -> String {
        format!("{}/{}", self.tasks_url(), id)
    }
    pub fn share_url(&self) -> String {
        join(&self.drive_base, PATH_SHARE)
    }
    pub fn share_list_url(&self) -> String {
        join(&self.drive_base, PATH_SHARE_LIST)
    }
    pub fn events_url(&self) -> String {
        join(&self.drive_base, PATH_EVENTS)
    }
    /// CID 查询接口（上传前用它换取 GCID）。
    pub fn resource_cid_url(&self) -> String {
        join(&self.drive_base, PATH_RESOURCE_CID)
    }
    /// 磁链资源列表：秒传前先拿到文件清单与 GCID。
    pub fn resource_list_url(&self) -> String {
        join(&self.drive_base, PATH_RESOURCE_LIST)
    }
    /// 分享写操作（如 `batchDelete`）拼在 `/drive/v1/share:<action>`。
    pub fn share_action_url(&self, action: &str) -> String {
        format!("{}:{action}", self.share_url())
    }
}

/// 读取非空环境变量。
fn read_env(key: &str) -> Option<String> {
    env::var(key).ok().filter(|v| !v.is_empty())
}

/// 去掉基地址末尾的 `/`，避免拼出 `//`。
fn trim_base(base: &str) -> String {
    base.trim_end_matches('/').to_owned()
}

fn join(base: &str, path: &str) -> String {
    format!("{base}{path}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 环境变量测试全局锁（避免并行测试互相踩踏）
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn defaults_match_rclone_constants() {
        let cfg = Config::default();
        assert_eq!(cfg.client_id, "YUMx5nI8ZU8Ap8pm");
        assert_eq!(cfg.client_version, "2.0.0");
        assert_eq!(cfg.device_id, "");

        let files = cfg.files_url();
        assert!(files.starts_with("https://api-drive.mypikpak.com/drive/v1/files"));
        assert_eq!(cfg.file_url("abc"), format!("{files}/abc"));
        assert_eq!(
            cfg.file_action_url("batchTrash"),
            format!("{files}:batchTrash")
        );
        assert_eq!(cfg.trash_empty_url(), format!("{files}/trash:empty"));
        assert_eq!(
            cfg.share_list_url(),
            "https://api-drive.mypikpak.com/drive/v1/share/list"
        );
        assert_eq!(
            cfg.share_action_url("batchDelete"),
            "https://api-drive.mypikpak.com/drive/v1/share:batchDelete"
        );
        assert_eq!(
            cfg.events_url(),
            "https://api-drive.mypikpak.com/drive/v1/events"
        );
        assert_eq!(
            cfg.resource_cid_url(),
            "https://api-drive.mypikpak.com/drive/v1/resource/cid"
        );
        assert_eq!(
            cfg.resource_list_url(),
            "https://api-drive.mypikpak.com/drive/v1/resource/list"
        );
        assert_eq!(cfg.upload_base, "https://mypikpak.com");
    }

    #[test]
    fn env_overrides_bases_and_trims_trailing_slash() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        env::set_var(env_key::AUTH_BASE, "http://127.0.0.1:1/");
        env::set_var(env_key::DRIVE_BASE, "http://127.0.0.1:2");
        env::set_var(env_key::UPLOAD_BASE, "https://mypikpak.com/");
        let cfg = Config::from_env();
        env::remove_var(env_key::AUTH_BASE);
        env::remove_var(env_key::DRIVE_BASE);
        env::remove_var(env_key::UPLOAD_BASE);

        assert_eq!(cfg.signin_url(), "http://127.0.0.1:1/v1/auth/signin");
        assert_eq!(cfg.tasks_url(), "http://127.0.0.1:2/drive/v1/tasks");
        assert_eq!(cfg.upload_base, "https://mypikpak.com");
    }

    #[test]
    fn transfer_quota_url_carries_the_official_query() {
        let cfg = Config::default();
        assert_eq!(
            cfg.transfer_quota_url(),
            "https://api-drive.mypikpak.com/vip/v1/quantity/list?type=transfer&limit=200"
        );
    }

    #[test]
    fn token_url_is_rclone_verified_path() {
        let cfg = Config::default();
        assert_eq!(cfg.token_url(), "https://user.mypikpak.com/v1/auth/token");
        assert_eq!(
            cfg.captcha_init_url(),
            "https://user.mypikpak.com/v1/shield/captcha/init"
        );
    }
}
