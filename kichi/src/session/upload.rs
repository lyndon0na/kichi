//! 上传门面：断点加载 / 保存、限速器与取消后的清理都收在这里。
//!
//! 临时 S3 凭据只通过 `UploadResume` 在 core 与 KWallet 之间传递，不进入
//! 传输历史或 QML。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use kichi_core::{DriveClient, UploadProgress, UploadRateLimiter, UploadResult, UploadResume};

use super::download::Cancel;
use super::{upload_resume, Failure, Session};

pub(crate) struct SessionUploadError {
    pub(crate) failure: Failure,
    pub(crate) keep_resume: bool,
    pub(crate) stop_queue: bool,
}

impl std::fmt::Display for SessionUploadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.failure.fmt(formatter)
    }
}

impl Session {
    /// 用一个已取得的网盘 client 上传本地文件（阻塞）。
    ///
    /// worker 必须在启动后先调用 [`Session::drive`] 并持有返回的 `Arc`；
    /// 这样用户登出清掉会话令牌时，正在收尾的上传仍能尽力清理临时记录。
    pub fn upload_file(
        &self,
        client: Arc<DriveClient>,
        path: &Path,
        parent_id: &str,
        cancel: Arc<Cancel>,
        limiter: Option<Arc<UploadRateLimiter>>,
        progress: impl FnMut(UploadProgress),
    ) -> Result<UploadResult, SessionUploadError> {
        let Some(user_id) = self.user_id() else {
            return Err(SessionUploadError {
                failure: Failure::Auth("登录状态已失效，请重新登录".to_owned()),
                keep_resume: false,
                stop_queue: true,
            });
        };
        let key = resume_key(path, parent_id);
        let mut checkpoint = upload_resume::load(&user_id, &key);
        let mut last_saved_upload_id = checkpoint
            .as_ref()
            .map(|state| state.upload_id.clone())
            .unwrap_or_default();
        let mut last_content_uploaded = checkpoint
            .as_ref()
            .is_some_and(|state| state.content_uploaded);
        let mut last_task_id = checkpoint
            .as_ref()
            .map(|state| state.task_id.clone())
            .unwrap_or_default();
        let mut last_checkpoint = checkpoint.clone();
        let mut save_error = None;

        let cancelled = {
            let cancel = Arc::clone(&cancel);
            move || cancel.is_cancelled()
        };
        let result = client.upload_file_resumable(
            path,
            parent_id,
            cancelled,
            progress,
            checkpoint.take(),
            limiter,
            |state| {
                last_checkpoint = Some(state.clone());
                let critical = state.upload_id != last_saved_upload_id
                    || state.content_uploaded != last_content_uploaded
                    || state.task_id != last_task_id;
                if let Err(error) = upload_resume::save(&user_id, &key, state, critical) {
                    if save_error.is_none() {
                        save_error = Some(error);
                    }
                }
                last_saved_upload_id = state.upload_id.clone();
                last_content_uploaded = state.content_uploaded;
                last_task_id = state.task_id.clone();
            },
        );

        match result {
            Ok(result) => {
                upload_resume::remove(&user_id, &key);
                Ok(result)
            }
            Err(error) => {
                if cancel.is_abandoned() {
                    // 登出/换账号：断点按账号留在钥匙串，旧线程不清理服务端。
                } else if cancel.is_halted() {
                    // 官方限制 / 鉴权失败：保留断点，等待用户处理后再手动重试。
                } else if cancel.is_paused() {
                    // 用户暂停：保留本地断点与服务端 multipart，继续时接着传。
                } else if cancel.is_cancelled() {
                    upload_resume::remove(&user_id, &key);
                    if let Some(state) = last_checkpoint.clone() {
                        cleanup_async(Arc::clone(&client), state);
                    }
                } else if !error.keep_resume {
                    upload_resume::remove(&user_id, &key);
                }
                if let Some(warning) = save_error {
                    eprintln!("[kichi] {warning}");
                }
                Err(SessionUploadError {
                    failure: Failure::from(error.error),
                    keep_resume: error.keep_resume,
                    stop_queue: error.stop_queue,
                })
            }
        }
    }
}

fn resume_key(path: &Path, parent_id: &str) -> String {
    format!("{}\u{1f}{}", path.to_string_lossy(), parent_id.trim())
}

/// 用户明确取消上传：先清本地断点，再用当前会话的 client 尽力清理服务端。
///
/// 删除钥匙串条目同步完成，避免取消后马上重试又捡回旧断点；网络清理放到
/// 后台线程，失败只记日志，不改变「已取消」的结果。
pub(crate) fn discard_upload_resumes(session: &Arc<Session>, targets: Vec<(PathBuf, String)>) {
    if targets.is_empty() {
        return;
    }
    let Some(user_id) = session.user_id() else {
        return;
    };
    let keys: Vec<String> = targets
        .into_iter()
        .map(|(path, parent_id)| resume_key(&path, &parent_id))
        .collect();
    let states = upload_resume::take_many(&user_id, keys);
    cleanup_many_async(session, states);
}

fn cleanup_async(client: Arc<DriveClient>, state: UploadResume) {
    let spawned = std::thread::Builder::new()
        .name("kichi-upload-cleanup".to_owned())
        .spawn(move || client.cleanup_upload(&state));
    if let Err(error) = spawned {
        eprintln!("[kichi] 无法启动上传清理线程：{error}");
    }
}

fn cleanup_many_async(session: &Arc<Session>, states: Vec<UploadResume>) {
    if states.is_empty() {
        return;
    }
    let session = Arc::clone(session);
    let spawned = std::thread::Builder::new()
        .name("kichi-upload-cleanup".to_owned())
        .spawn(move || {
            let Ok(client) = session.drive() else {
                eprintln!("[kichi] 取消上传后无法取得网盘客户端，跳过服务端清理");
                return;
            };
            for state in states {
                client.cleanup_upload(&state);
            }
        });
    if let Err(error) = spawned {
        eprintln!("[kichi] 无法启动上传清理线程：{error}");
    }
}
