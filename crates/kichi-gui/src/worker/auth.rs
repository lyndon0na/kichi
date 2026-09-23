use std::sync::atomic::Ordering;
use std::sync::mpsc::Sender;
use std::sync::Arc;

use kichi_core::{session, Error, KichiClient};

use crate::credentials;
use crate::msg::Msg;

use super::{refresh_quota, tasks::refresh_tasks, WorkerState};

/// 账号密码登录的公共实现(手动登录与密钥环自动登录共用)。
pub(super) async fn do_login(
    st: &mut WorkerState,
    tx: &Sender<Msg>,
    username: String,
    password: String,
) {
    let device_id = kichi_core::captcha::generate_device_id();
    let mut client = KichiClient::new(device_id.clone());
    install_saver(&mut client);
    client.set_part_concurrency(st.part_concurrency);
    tracing::info!("开始登录: {username}");
    match client.login(&username, &password).await {
        Ok(sess) => {
            if let Err(e) = session::save_session(&sess) {
                let _ = tx.send(Msg::Error {
                    what: e.to_string(),
                });
            }
            st.client = Some(Arc::new(client));
            tracing::info!("登录成功: {username}");
            let _ = tx.send(Msg::LoginOk { username });
            refresh_quota(st, tx).await;
            refresh_tasks(st, tx).await;
        }
        Err(e) => {
            tracing::warn!("登录失败: {e}");
            let (what, verify_url) = match &e {
                Error::CaptchaReview { url, description } => {
                    (format!("需要人机验证: {description}"), url.clone())
                }
                other => (format!("登录失败: {other}"), None),
            };
            let _ = tx.send(Msg::LoginFailed { what, verify_url });
        }
    }
}

/// 用系统密钥环里的密码自动登录; 没记住密码时回 `Msg::AutoLoginUnavailable`。
pub(super) async fn auto_login(st: &mut WorkerState, tx: &Sender<Msg>, username: String) {
    // 密钥环读取是阻塞的 D-Bus 调用, 放到阻塞线程池执行。
    let lookup = {
        let username = username.clone();
        tokio::task::spawn_blocking(move || credentials::load(&username))
            .await
            .ok()
            .flatten()
    };
    match lookup {
        Some(password) => do_login(st, tx, username, password).await,
        None => {
            let _ = tx.send(Msg::AutoLoginUnavailable);
        }
    }
}

/// 把账号密码写入系统密钥环; 失败时回 `Msg::Error`(不影响本次登录)。
pub(super) async fn remember_password(tx: &Sender<Msg>, username: String, password: String) {
    let err = tokio::task::spawn_blocking(move || credentials::save(&username, &password))
        .await
        .ok()
        .and_then(|r| r.err());
    if let Some(what) = err {
        let _ = tx.send(Msg::Error { what });
    }
}

/// 从系统密钥环删除已记住的密码(退出登录时清理)。
pub(super) async fn forget_password(username: String) {
    let _ = tokio::task::spawn_blocking(move || credentials::delete(&username)).await;
}

/// 用持久化会话恢复登录态; 凭据失效回 `Msg::SessionInvalid`, 网络类错误保留会话稍后重试。
pub(super) async fn resume_session(
    st: &mut WorkerState,
    tx: &Sender<Msg>,
    device_id: String,
    access_token: String,
    refresh_token: String,
    user_id: String,
    username: String,
) {
    let sess = session::Session {
        device_id: device_id.clone(),
        access_token: access_token.clone(),
        refresh_token: refresh_token.clone(),
        user_id: user_id.clone(),
        username: username.clone(),
    };
    let mut client = KichiClient::new(device_id);
    install_saver(&mut client);
    client.set_part_concurrency(st.part_concurrency);
    client.set_session(&sess).await;
    let client = Arc::new(client);
    tracing::info!("尝试恢复登录态");
    match client.quota().await {
        Ok(_) => {
            st.client = Some(client);
            tracing::info!("恢复登录态成功");
            let _ = tx.send(Msg::LoginOk { username });
            refresh_quota(st, tx).await;
            refresh_tasks(st, tx).await;
        }
        // refresh token 已过期/被吊销: 明确要求重新登录。
        Err(Error::AuthExpired(what)) => {
            tracing::warn!("会话失效(需重新登录): {what}");
            let _ = tx.send(Msg::SessionInvalid {
                reason: format!("登录已过期: {what}"),
            });
        }
        // 只有服务端明确判定凭据失效(API 错误)才强制重新登录;
        // 其余(网络/解析等)先保留本地会话, 由后台周期刷新自动重试。
        Err(e @ Error::Api { .. }) => {
            tracing::warn!("会话失效: {e}");
            let _ = tx.send(Msg::SessionInvalid {
                reason: format!("登录已过期: {e}"),
            });
        }
        Err(e) => {
            st.client = Some(client);
            tracing::warn!("会话暂不可用, 将自动重试: {e}");
            let _ = tx.send(Msg::LoginOk { username });
            let _ = tx.send(Msg::Error {
                what: format!("会话暂不可用, 稍后自动重试: {e}"),
            });
        }
    }
}

/// 退出登录: 取消全部传输, 清空占位表, 清会话并通知服务端。
pub(super) async fn logout(st: &mut WorkerState, tx: &Sender<Msg>) {
    // 取消全部下载(含排队中的), 任务自行退出并清理。
    {
        let mut map = st.cancel.lock().await;
        for flag in map.values() {
            flag.store(true, Ordering::Relaxed);
        }
        map.clear();
    }
    {
        let mut r = st.reserved.lock().unwrap_or_else(|e| e.into_inner());
        r.clear();
    }
    let _ = session::clear_session();
    if let Some(c) = &st.client {
        c.logout().await;
    }
    st.client = None;
    let _ = tx.send(Msg::LoggedOut);
}

pub(super) fn install_saver(client: &mut KichiClient) {
    let saver: kichi_core::client::TokenSaver = Arc::new(|sess| {
        let _ = session::save_session(sess);
    });
    client.set_token_saver(saver);
}
