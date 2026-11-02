//! 上传门面：后台线程持有本次会话的网盘 client，再把本地文件交给 core。
//!
//! 上传临时凭据只存在于 `kichi-core::DriveClient::upload_file` 的调用栈内；
//! 本模块不落盘、不记录凭据，只负责把取消信号与进度回调转交给 core。

use std::path::Path;
use std::sync::Arc;

use kichi_core::{DriveClient, UploadProgress, UploadResult};

use super::download::Cancel;
use super::{Failure, Session};

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
        progress: impl FnMut(UploadProgress),
    ) -> Result<UploadResult, Failure> {
        let cancelled = {
            let cancel = Arc::clone(&cancel);
            move || cancel.is_cancelled()
        };
        client
            .upload_file(path, parent_id, cancelled, progress)
            .map_err(Failure::from)
    }
}
