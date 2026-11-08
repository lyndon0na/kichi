//! 会话状态副作用：失败分类、人机验证、容量 / 身份刷新与限流提示。

use std::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt_lib::QString;

use super::actions::run_in_background;
use super::actions_display::{quota_display, transfer_display};
use super::qobject;
use crate::session::Failure;

impl qobject::Kichi {
    fn set_captcha(mut self: Pin<&mut Self>, url: Option<&str>) {
        {
            let mut rust = self.as_mut().rust_mut();
            rust.captcha_required = true;
            rust.captcha_url = url.unwrap_or_default().to_owned();
            rust.login_running = false;
        }
        self.as_mut().notify();
    }

    pub(crate) fn on_failure(mut self: Pin<&mut Self>, failure: Failure) {
        self.as_mut().on_failure_inner(failure, true);
    }

    /// 列表自动校准失败：仍更新底栏 / 登录态，但不发系统通知、不弹窗。
    pub(crate) fn on_refresh_failure(mut self: Pin<&mut Self>, failure: Failure) {
        self.as_mut().on_failure_inner(failure, false);
    }

    fn on_failure_inner(mut self: Pin<&mut Self>, failure: Failure, notify_limited: bool) {
        match failure {
            Failure::Captcha { url } => {
                let message = match &url {
                    Some(url) => format!(
                        "账号需要人机验证：点「打开验证页面」在浏览器完成验证，再把页面上的验证令牌粘到下方。验证地址：{url}"
                    ),
                    None => "验证令牌已过期，请重新提交登录".to_owned(),
                };
                self.as_mut().set_message(&message);
                self.as_mut().set_captcha(url.as_deref());
            }
            Failure::Auth(reason) => {
                {
                    let mut rust = self.as_mut().rust_mut();
                    rust.auth_state = QString::from("expired");
                    rust.status_message = QString::from(format!("登录状态失效：{reason}").as_str());
                }
                self.as_mut().notify();
            }
            limited @ Failure::RateLimited { .. } => {
                // 配额/限速要解释清楚，否则用户会以为客户端坏了
                let text = crate::session::rate_limit_text(&limited);
                self.as_mut().set_message(&text);
                if notify_limited {
                    self.as_mut().notify_rate_limited(&text);
                }
            }
            other => self.as_mut().set_message(&other.to_string()),
        }
    }

    /// 配额/限速弹一条系统通知（通知开关关闭时跳过）。
    fn notify_rate_limited(self: Pin<&mut Self>, text: &str) {
        if !self.notify_enabled {
            return;
        }
        super::notifications::show("PikPak 配额或限速限制", text, "dialog-warning");
    }

    fn on_quota(mut self: Pin<&mut Self>, quota: kichi_core::Quota) {
        let display = quota_display(&quota);
        {
            let mut rust = self.as_mut().rust_mut();
            rust.quota_used_text = QString::from(display.used.as_str());
            rust.quota_limit_text = QString::from(display.limit.as_str());
            rust.quota_ratio = display.ratio;
            rust.quota_vip = quota.vip;
        }
        self.as_mut().notify();
    }

    /// 月度传输配额（云下载 / 下行 / 上传）。
    fn on_transfer_quota(mut self: Pin<&mut Self>, quota: kichi_core::TransferQuota) {
        let display = transfer_display(&quota, chrono::Utc::now());
        {
            let mut rust = self.as_mut().rust_mut();
            rust.quota_transfer_cloud = QString::from(display.cloud.as_str());
            rust.quota_transfer_download = QString::from(display.download.as_str());
            rust.quota_transfer_upload = QString::from(display.upload.as_str());
            rust.quota_transfer_reset = QString::from(display.reset.as_str());
            rust.quota_transfer_cloud_ratio = display.cloud_ratio;
            rust.quota_transfer_download_ratio = display.download_ratio;
            rust.quota_transfer_upload_ratio = display.upload_ratio;
        }
        self.as_mut().notify();
    }

    /// 后台刷新身份与容量（身份失败不阻断容量）。
    pub(crate) fn refresh_identity_and_quota(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().quota_loading = true;
        self.as_mut().notify();
        let session = self.backend.session.clone();
        run_in_background(
            move || {
                let identity = session.refresh_identity();
                let quota = session.quota();
                // 容量随会话落盘：下次启动先显示旧值，再等这次刷新。
                if let Ok(ref quota) = quota {
                    session.cache_quota(quota);
                }
                let transfer = session.transfer_quota();
                Ok((identity, quota, transfer))
            },
            |mut qichi, result| {
                let (identity, quota, transfer) = match result {
                    Ok(triple) => triple,
                    Err(err) => {
                        qichi.as_mut().rust_mut().quota_loading = false;
                        qichi.as_mut().on_failure(err);
                        return;
                    }
                };
                match identity {
                    Ok(()) => {
                        qichi.as_mut().sync_identity_from_session();
                        qichi.as_mut().refresh_avatar_in_background();
                    }
                    // 网络失败时仍先显示已持久化的昵称 / 邮箱 / 头像，不能一直停在骨架。
                    Err(err) => {
                        qichi.as_mut().sync_identity_from_session();
                        qichi
                            .as_mut()
                            .set_message(&format!("账号信息获取失败：{err}"));
                    }
                }
                qichi.as_mut().rust_mut().quota_loading = false;
                match quota {
                    Ok(quota) => qichi.as_mut().on_quota(quota),
                    Err(err) => qichi.as_mut().on_failure(err),
                }
                match transfer {
                    Ok(transfer) => qichi.as_mut().on_transfer_quota(transfer),
                    // 传输配额是设置页的补充信息，失败只报状态，不弹系统通知
                    Err(err) => qichi.as_mut().on_refresh_failure(err),
                }
            },
        );
    }

    /// 把会话里已持久化的身份同步到 QML；头像只读本地缓存。
    pub(crate) fn sync_identity_from_session(mut self: Pin<&mut Self>) {
        let (account, display_name, avatar) = {
            let session = &self.as_ref().backend.session;
            (
                session.email().unwrap_or_default(),
                session.display_name().unwrap_or_default(),
                session
                    .cached_avatar_path()
                    .map(|path| super::reveal::file_uri(&path)),
            )
        };
        {
            let mut rust = self.as_mut().rust_mut();
            rust.account = QString::from(account.as_str());
            rust.display_name = QString::from(display_name.as_str());
            rust.avatar_path = QString::from(avatar.unwrap_or_default().as_str());
        }
        self.as_mut().notify();
    }

    /// 头像下载独立于身份 / 容量请求，慢网络不会拖住昵称和容量显示。
    fn refresh_avatar_in_background(self: Pin<&mut Self>) {
        let session = self.backend.session.clone();
        let Some(user_id) = session.user_id() else {
            return;
        };
        run_in_background(
            move || session.refresh_avatar(),
            move |mut qichi, result| {
                if qichi.as_ref().backend.session.user_id().as_deref() != Some(user_id.as_str()) {
                    return;
                }
                match result {
                    Ok(Some(path)) => {
                        qichi.as_mut().rust_mut().avatar_path =
                            QString::from(super::reveal::file_uri(&path).as_str());
                        qichi.as_mut().notify();
                    }
                    Ok(None) => {
                        let changed = !qichi.as_ref().avatar_path.is_empty();
                        qichi.as_mut().rust_mut().avatar_path = QString::default();
                        if changed {
                            qichi.as_mut().notify();
                        }
                    }
                    // 头像是装饰性数据：失败就保留旧缓存 / 首字母，不打断账号区。
                    Err(err) => eprintln!("[kichi] 头像更新失败：{err}"),
                }
            },
        );
    }
}
