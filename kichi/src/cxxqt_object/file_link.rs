//! 复制文件直链：取一次详情，把 `download_url` 交给界面写剪贴板。
//!
//! 链接是带签名的临时地址：只通过 `fileLinkReady` 交给 QML 立即复制，
//! 不写日志、不存属性、不进状态栏文案。详情仍走播放 / 下载共用的
//! `session::media::file_info`，链接未生成时的重试、0 字节 / 违规判定保持一致。

use std::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt_lib::QString;
use kichi_core::{EntryKind, NoLinkReason};

use super::actions::run_in_background;
use super::qobject;
use crate::session::media::{file_info, RESTRICTED_NO_LINK};
use crate::session::Failure;

/// 文件夹没有可直接下载的直链。
pub(crate) const FOLDER_NO_LINK: &str = "文件夹没有直接下载链接";
/// 0 字节文件没有可下载的内容。
pub(crate) const ZERO_BYTE_LINK: &str = "这个文件是 0 字节，没有可下载的内容";
/// 详情两次都没给链接且原因不明。
pub(crate) const LINK_MISSING: &str = "官方没有提供这个文件的下载链接";

impl qobject::Kichi {
    /// 右键「复制下载链接」：后台取一次详情，成功后发 `fileLinkReady`。
    pub fn copy_link(mut self: Pin<&mut Self>, id: QString) {
        let id = id.to_string();
        if id.trim().is_empty() {
            return;
        }
        let entry = self.as_ref().rust().find_entry(&id);
        if entry
            .as_ref()
            .is_some_and(|entry| entry.kind == EntryKind::Folder)
        {
            self.as_mut().set_message(FOLDER_NO_LINK);
            return;
        }
        let audit_flagged = entry.as_ref().is_some_and(|entry| entry.audit_flagged);
        let name = entry.map(|entry| entry.name).unwrap_or_default();
        let for_message = if name.is_empty() {
            "所选文件".to_owned()
        } else {
            format!("「{name}」")
        };

        let session = self.backend.session.clone();
        let signal_id = id.clone();
        self.as_mut().set_message("正在获取下载链接……");
        run_in_background(
            move || file_info(&session, &id, None, audit_flagged),
            move |mut qichi, result| match result {
                Ok(info) => match info.download_url() {
                    Some(url) => {
                        qichi.as_mut().file_link_ready(
                            QString::from(signal_id.as_str()),
                            QString::from(url.as_str()),
                        );
                        qichi
                            .as_mut()
                            .set_message(&format!("已复制{for_message}的下载链接"));
                    }
                    None => {
                        let text = match info.no_link_reason() {
                            Some(NoLinkReason::ZeroByte) => ZERO_BYTE_LINK,
                            Some(NoLinkReason::Restricted) => RESTRICTED_NO_LINK,
                            None => LINK_MISSING,
                        };
                        qichi.as_mut().set_message(text);
                    }
                },
                Err(
                    far
                    @ (Failure::Auth(_) | Failure::Captcha { .. } | Failure::RateLimited { .. }),
                ) => qichi.as_mut().on_failure(far),
                Err(other) => qichi
                    .as_mut()
                    .set_message(&format!("复制链接失败：{other}")),
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_texts_explain_the_reason() {
        assert!(FOLDER_NO_LINK.contains("文件夹"));
        assert!(ZERO_BYTE_LINK.contains("0 字节"));
        assert!(LINK_MISSING.contains("下载链接"));
    }
}
