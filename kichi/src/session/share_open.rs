//! 打开 / 浏览 / 转存别人的分享：会话层门面。
//!
//! 阻塞调用；线程与界面状态由 `cxxqt_object::shares::share_open` 负责。
//! 分页统一防循环并设上限，避免异常 `next_page_token` 把内存打满。

use std::collections::HashSet;
use std::time::Duration;

use kichi_core::share_open::SHARE_PAGE_SIZE;
use kichi_core::{FileEntry, RestoreShareRequest, ShareListing, ShareOpenInfo, TaskPhase};

use super::{Failure, Session};

/// 最多翻 20 页（约 2,000 条），与分享管理列表的上限一致。
const MAX_SHARE_PAGES: usize = 20;
/// 转存任务轮询间隔与总等待（30 秒）。超过后仍算「已开始」，由用户在文件页确认。
const RESTORE_POLL_INTERVAL: Duration = Duration::from_millis(500);
const RESTORE_POLL_ATTEMPTS: usize = 60;

/// 转存结果：`completed = false` 表示服务端已接受但任务还在跑。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RestoreOutcome {
    pub completed: bool,
}

impl Session {
    /// 打开分享：元数据 + 顶层条目（自动翻完后续页）。
    pub fn open_share(
        &self,
        share_id: &str,
        pass_code: Option<&str>,
    ) -> Result<ShareOpenInfo, Failure> {
        let client = self.drive()?;
        let mut info = client
            .open_share(share_id, pass_code)
            .map_err(Failure::from)?;
        let mut seen = HashSet::new();
        let mut pages = 1;
        while let Some(token) = info.next_page_token.take() {
            if pages >= MAX_SHARE_PAGES
                || info.entries.len() >= MAX_SHARE_PAGES * SHARE_PAGE_SIZE as usize
            {
                info.truncated = true;
                break;
            }
            if !seen.insert(token.clone()) {
                return Err(Failure::Other(
                    "分享文件分页出现循环，已停止继续读取".into(),
                ));
            }
            let page = client
                .list_share_files(&info.share_id, &info.pass_code_token, None, Some(&token))
                .map_err(Failure::from)?;
            info.entries.extend(page.entries);
            info.next_page_token = page.next_page_token;
            pages += 1;
        }
        Ok(info)
    }

    /// 读取分享里某一层目录（自动翻页；空 `parent_id` = 顶层）。
    pub fn share_folder(
        &self,
        share_id: &str,
        pass_code_token: &str,
        parent_id: Option<&str>,
    ) -> Result<ShareListing, Failure> {
        let client = self.drive()?;
        let mut entries: Vec<FileEntry> = Vec::new();
        let mut seen = HashSet::new();
        let mut page_token: Option<String> = None;
        for page_index in 0..MAX_SHARE_PAGES {
            let page = client
                .list_share_files(share_id, pass_code_token, parent_id, page_token.as_deref())
                .map_err(Failure::from)?;
            entries.extend(page.entries);
            let Some(next) = page.next_page_token else {
                return Ok(ShareListing {
                    entries,
                    truncated: false,
                });
            };
            if !seen.insert(next.clone()) {
                return Err(Failure::Other(
                    "分享文件分页出现循环，已停止继续读取".into(),
                ));
            }
            if page_index + 1 == MAX_SHARE_PAGES {
                return Ok(ShareListing {
                    entries,
                    truncated: true,
                });
            }
            page_token = Some(next);
        }
        Ok(ShareListing {
            entries,
            truncated: true,
        })
    }

    /// 转存分享内容到自己的网盘。
    ///
    /// `RESTORE_START` 之后轮询任务，成功 / 失败都按现有错误分类冒泡；
    /// 轮询本身失败或超时不会把已经提交的转存判为失败，而是返回
    /// `completed = false`（界面提示稍后到文件页确认）。
    pub fn restore_share(&self, request: &RestoreShareRequest) -> Result<RestoreOutcome, Failure> {
        let client = self.drive()?;
        let result = client.restore_share(request).map_err(Failure::from)?;
        if result.is_complete() {
            return Ok(RestoreOutcome { completed: true });
        }
        if !result.is_started() {
            let reason = if result.status.trim().is_empty() {
                "转存请求没有返回任务状态".to_owned()
            } else {
                format!("转存失败：{}", result.status.trim())
            };
            return Err(Failure::Other(reason));
        }
        // `is_started()` 已确认有 id，这里不 panic。
        let Some(task_id) = result.task_id else {
            return Err(Failure::Other("转存请求没有返回任务 id".into()));
        };
        for _ in 0..RESTORE_POLL_ATTEMPTS {
            std::thread::sleep(RESTORE_POLL_INTERVAL);
            let state = match client.share_restore_task(&task_id) {
                Ok(state) => state,
                // 转存已经提交，查询失败不能反过来宣称失败。
                Err(_) => return Ok(RestoreOutcome { completed: false }),
            };
            match state.phase {
                TaskPhase::Completed => return Ok(RestoreOutcome { completed: true }),
                TaskPhase::Error => {
                    let detail = state
                        .message
                        .filter(|message| !message.trim().is_empty())
                        .map(|message| format!("：{message}"))
                        .unwrap_or_default();
                    return Err(Failure::Other(format!("转存失败{detail}")));
                }
                _ => {}
            }
        }
        Ok(RestoreOutcome { completed: false })
    }
}
