//! 桥接对象持有的会话与其启动恢复结果。

use std::sync::Arc;

use crate::session::{RestoredState, Session, State};

/// 后端：会话 + 启动时的恢复结果。
pub(crate) struct Backend {
    pub(crate) session: Arc<Session>,
    restored: RestoredState,
}

impl Backend {
    pub(crate) fn new(store_path: std::path::PathBuf, local_index_enabled: bool) -> Self {
        let (session, restored) = Session::bootstrap(store_path, local_index_enabled);
        Self {
            session: Arc::new(session),
            restored,
        }
    }

    pub(crate) fn state(&self) -> State {
        self.restored.state
    }

    pub(crate) fn restored_notice(&self) -> &str {
        self.restored.notice.as_deref().unwrap_or_default()
    }

    /// 令牌实际存到了哪里（设置页只读展示用）；`None` = 本次还没读过存储。
    pub(crate) fn restored_backend(&self) -> Option<crate::session::Backend> {
        self.restored.backend
    }

    pub(crate) fn storage(&self) -> String {
        match self.restored.backend {
            Some(crate::session::Backend::File) => {
                "⚠ 系统钥匙串不可用，登录令牌存在本机 0600 权限文件中".to_owned()
            }
            _ => String::new(),
        }
    }
}
