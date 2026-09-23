use std::sync::mpsc::Sender;
use std::sync::Arc;

use kichi_core::{session, Error, KichiClient};

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

pub(super) fn install_saver(client: &mut KichiClient) {
    let saver: kichi_core::client::TokenSaver = Arc::new(|sess| {
        let _ = session::save_session(sess);
    });
    client.set_token_saver(saver);
}
