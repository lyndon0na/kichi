//! 图片预览桥接：按文件 id 取详情 → 直链 → 本地临时文件。
//!
//! QML 只拿到 `file://` URI；签名直链、大小上限与超时判定全在 Rust。
//! 同一时间只保留一张预览：换图 / 关闭 / 登出都会作废在途结果并删掉旧临时文件。

use std::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt_lib::QString;
use kichi_core::EntryKind;

use super::actions::run_in_background;
use super::qobject;
use super::reveal::file_uri;
use crate::session::image_preview;
use crate::session::media::KnownHint;

/// 这个条目不是文件夹、而且是按扩展名认得的图片。
pub(crate) fn previewable(entry: &kichi_core::FileEntry) -> bool {
    entry.kind == EntryKind::File && kichi_core::is_image_name(&entry.name)
}

impl qobject::Kichi {
    /// 打开图片预览：立刻进入加载态，后台下载完成后回填本地 URI。
    pub fn preview_image(mut self: Pin<&mut Self>, id: QString) {
        let id = id.to_string();
        if id.trim().is_empty() {
            return;
        }
        let Some(entry) = self.as_ref().rust().find_entry(&id) else {
            self.as_mut()
                .fail_image_preview("找不到这个文件，可能已被移动或删除");
            return;
        };
        if entry.kind == EntryKind::Folder {
            self.as_mut().fail_image_preview("文件夹不能预览");
            return;
        }
        if !previewable(&entry) {
            self.as_mut()
                .fail_image_preview("这个文件不是支持的图片格式");
            return;
        }

        let known = KnownHint::from_list(entry.size, entry.audit_flagged);
        let name = entry.name;
        let (epoch, previous) = {
            let mut rust = self.as_mut().rust_mut();
            rust.image_preview_epoch = rust.image_preview_epoch.wrapping_add(1);
            rust.image_preview_loading = true;
            rust.image_preview_error = QString::default();
            rust.image_preview_path = QString::default();
            (rust.image_preview_epoch, rust.image_preview_file.take())
        };
        if let Some(path) = previous {
            image_preview::remove_file(&path);
        }
        self.as_mut().notify();

        let session = self.backend.session.clone();
        run_in_background(
            move || image_preview::download(&session, &id, &name, known),
            move |mut qichi, result| {
                if qichi.as_ref().rust().image_preview_epoch != epoch {
                    // 关闭 / 换图 / 登出后回来的结果：删掉临时文件，不动界面。
                    if let Ok(path) = result {
                        image_preview::remove_file(&path);
                    }
                    return;
                }
                match result {
                    Ok(path) => {
                        {
                            let mut rust = qichi.as_mut().rust_mut();
                            rust.image_preview_loading = false;
                            rust.image_preview_error = QString::default();
                            rust.image_preview_path = QString::from(file_uri(&path).as_str());
                            rust.image_preview_file = Some(path);
                        }
                        qichi.as_mut().notify();
                    }
                    Err(failure) => {
                        let text = failure.to_string();
                        let session_failure = failure.is_session_failure();
                        {
                            let mut rust = qichi.as_mut().rust_mut();
                            rust.image_preview_loading = false;
                            rust.image_preview_path = QString::default();
                            rust.image_preview_error = QString::from(text.as_str());
                        }
                        qichi.as_mut().notify();
                        if session_failure {
                            qichi.as_mut().on_failure(failure);
                        }
                    }
                }
            },
        );
    }

    /// 关闭预览：作废在途请求并删除本地临时文件。
    pub fn close_image_preview(mut self: Pin<&mut Self>) {
        let previous = {
            let mut rust = self.as_mut().rust_mut();
            rust.image_preview_epoch = rust.image_preview_epoch.wrapping_add(1);
            rust.image_preview_loading = false;
            rust.image_preview_error = QString::default();
            rust.image_preview_path = QString::default();
            rust.image_preview_file.take()
        };
        if let Some(path) = previous {
            image_preview::remove_file(&path);
        }
        self.as_mut().notify();
    }

    /// 退出登录：丢掉预览状态与临时文件，不额外通知（登出流程自己会 notify）。
    pub(crate) fn reset_image_preview(mut self: Pin<&mut Self>) {
        let previous = {
            let mut rust = self.as_mut().rust_mut();
            rust.image_preview_epoch = rust.image_preview_epoch.wrapping_add(1);
            rust.image_preview_loading = false;
            rust.image_preview_error = QString::default();
            rust.image_preview_path = QString::default();
            rust.image_preview_file.take()
        };
        if let Some(path) = previous {
            image_preview::remove_file(&path);
        }
    }

    /// 尚未开始下载就失败（找不到 / 文件夹 / 非图片）：直接进失败态。
    fn fail_image_preview(mut self: Pin<&mut Self>, reason: &str) {
        {
            let mut rust = self.as_mut().rust_mut();
            rust.image_preview_epoch = rust.image_preview_epoch.wrapping_add(1);
            rust.image_preview_loading = false;
            rust.image_preview_path = QString::default();
            rust.image_preview_error = QString::from(reason);
        }
        self.as_mut().notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kichi_core::{EntryKind, FileEntry};

    fn entry(name: &str, kind: EntryKind) -> FileEntry {
        FileEntry {
            id: "id".to_owned(),
            name: name.to_owned(),
            kind,
            size: 1,
            created_at: None,
            modified_at: None,
            parent_id: String::new(),
            md5: None,
            audit_flagged: false,
            starred: false,
            media: false,
            video: false,
            thumbnail_url: String::new(),
        }
    }

    #[test]
    fn only_image_files_are_previewable() {
        assert!(previewable(&entry("a.JPG", EntryKind::File)));
        assert!(previewable(&entry("b.webp", EntryKind::File)));
        assert!(!previewable(&entry("c.mp4", EntryKind::File)));
        assert!(!previewable(&entry("album.png", EntryKind::Folder)));
    }
}
