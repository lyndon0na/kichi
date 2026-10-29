//! 分享链接的会话门面：创建 / 拉取 / 批量撤销。
//!
//! 阻塞调用，线程与界面状态由 `cxxqt_object::shares` 负责。

use std::collections::HashSet;

use kichi_core::share::SHARE_LIST_LIMIT;
use kichi_core::{CreateShareOptions, CreatedShare, MyShare};

use super::{Failure, Session};

/// 最多翻 20 页（2,000 条），防止异常分页把内存打满。
const MAX_SHARE_PAGES: usize = 20;

impl Session {
    pub fn create_share(
        &self,
        file_ids: &[&str],
        expiration_days: i64,
        need_password: bool,
    ) -> Result<CreatedShare, Failure> {
        let client = self.drive()?;
        client
            .create_share(
                file_ids,
                CreateShareOptions {
                    expiration_days,
                    need_password,
                },
            )
            .map_err(Failure::from)
    }

    pub fn shares(&self) -> Result<Vec<MyShare>, Failure> {
        let client = self.drive()?;
        let mut shares = Vec::new();
        let mut page_token: Option<String> = None;
        let mut seen = HashSet::new();
        for _ in 0..MAX_SHARE_PAGES {
            let (mut page, next) = client
                .list_shares(page_token.as_deref())
                .map_err(Failure::from)?;
            shares.append(&mut page);
            match next {
                Some(token) if seen.insert(token.clone()) => page_token = Some(token),
                Some(_) => {
                    return Err(Failure::Other(
                        "分享列表分页出现循环，已停止继续读取".into(),
                    ));
                }
                None => return Ok(shares),
            }
        }
        Err(Failure::Other(format!(
            "分享超过 {} 条，暂不支持完整显示",
            MAX_SHARE_PAGES * SHARE_LIST_LIMIT as usize
        )))
    }

    pub fn revoke_shares(&self, share_ids: &[&str]) -> Result<(), Failure> {
        let client = self.drive()?;
        client.delete_shares(share_ids).map_err(Failure::from)
    }
}
