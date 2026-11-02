//! 上传的 QML 入口：系统多选文件、串行调度、成功后的当前目录校准。

use std::path::PathBuf;
use std::pin::Pin;

use cxx_qt_lib::{QString, QStringList};

use super::qobject;
use super::transfers::{self, TransferDirection};
use super::upload_queue;
use crate::session::State;

impl qobject::Kichi {
    /// 通过 XDG Portal 多选本地文件，并入队到调用时所在的云端目录。
    pub fn choose_upload_files(mut self: Pin<&mut Self>, parent_id: QString, parent_name: QString) {
        if self.backend.session.state() != State::SignedIn {
            self.as_mut().set_message("请先登录再上传文件");
            return;
        }
        let parent_id = parent_id.to_string();
        let parent_name = parent_name.to_string();
        let start = self.settings.download_dir_path();
        let my_epoch = upload_queue::current_epoch();
        let spawned = std::thread::Builder::new()
            .name("kichi-file-picker".into())
            .spawn(move || match super::folder_picker::choose_files(&start) {
                Ok(paths) if paths.is_empty() => {}
                Ok(paths) => {
                    super::actions::enqueue_ui(move |mut qichi| {
                        if my_epoch != upload_queue::current_epoch()
                            || qichi.as_ref().auth_state.to_string() != "signedIn"
                            || qichi.as_ref().backend.session.state() != State::SignedIn
                        {
                            return;
                        }
                        qichi.as_mut().enqueue_upload_paths(
                            paths,
                            &parent_id,
                            &parent_name,
                            0,
                            String::new(),
                        );
                    });
                }
                Err(reason) => {
                    super::actions::enqueue_ui(move |mut qichi| {
                        if my_epoch != upload_queue::current_epoch()
                            || qichi.as_ref().auth_state.to_string() != "signedIn"
                            || qichi.as_ref().backend.session.state() != State::SignedIn
                        {
                            return;
                        }
                        qichi.as_mut().set_message(&reason);
                    });
                }
            });
        if let Err(err) = spawned {
            self.as_mut()
                .set_message(&format!("无法启动系统文件选择器：{err}"));
        }
    }

    /// 文件管理器把本地文件拖进文件页时，直接并入同一条上传队列。
    ///
    /// QML 只负责提供当前目录快照和 URI；本地路径解析、普通文件检查、去重和
    /// 结果提示都由 Rust 完成，避免拖放与 Portal 两套入口的规则漂移。
    pub fn upload_dropped_files(
        mut self: Pin<&mut Self>,
        parent_id: QString,
        parent_name: QString,
        uris: QStringList,
    ) {
        if self.backend.session.state() != State::SignedIn {
            self.as_mut().set_message("请先登录再上传文件");
            return;
        }
        let parent_id = parent_id.to_string();
        let parent_name = parent_name.to_string();
        let (paths, invalid, first_error) =
            classify_dropped_uris(uris.iter().map(ToString::to_string));
        self.as_mut()
            .enqueue_upload_paths(paths, &parent_id, &parent_name, invalid, first_error);
    }

    /// 两个上传入口共用的入队与结果提示。
    fn enqueue_upload_paths(
        mut self: Pin<&mut Self>,
        paths: Vec<PathBuf>,
        parent_id: &str,
        parent_name: &str,
        mut invalid: u32,
        mut first_error: String,
    ) {
        let (mut added, mut already) = (0u32, 0u32);
        for path in paths {
            match upload_queue::reserve(&path, parent_id, parent_name) {
                Ok(_) => added += 1,
                Err(upload_queue::ReserveError::AlreadyPending) => already += 1,
                Err(err) => {
                    invalid += 1;
                    if first_error.is_empty() {
                        first_error = err.reason();
                    }
                }
            }
        }
        if added > 0 {
            self.as_mut().advance_uploads();
        } else {
            self.as_mut().sync_transfers();
        }
        let message = upload_selection_message(added, already, invalid, &first_error);
        if !message.is_empty() {
            self.as_mut().set_message(&message);
        }
    }

    /// 推进串行上传队列（空闲时启动下一个文件）。
    pub(crate) fn advance_uploads(mut self: Pin<&mut Self>) {
        let session = self.backend.session.clone();
        let failures = upload_queue::advance(&session);
        if let Some(failure) = failures.first() {
            eprintln!(
                "[kichi] {} 个上传任务未能启动；{}：{}",
                failures.len(),
                failure.name,
                failure.reason
            );
            self.as_mut().set_message(&format!(
                "{} 个上传任务未能启动：{}",
                failures.len(),
                failure.reason
            ));
        }
        self.as_mut().sync_transfers();
    }

    /// 上传成功后失效目标目录快照；用户仍在该目录时静默刷新列表。
    pub(crate) fn on_upload_succeeded(mut self: Pin<&mut Self>, parent_id: &str) {
        self.as_mut().mark_file_dir_stale(parent_id);
        if self.current_dir_id.to_string() == parent_id {
            self.as_mut().refresh_files_background();
        }
    }

    /// 页头「取消全部」：下载与上传都取消，但不能把暂停下载的断点误说成上传断点。
    pub fn cancel_all_transfers(mut self: Pin<&mut Self>) {
        self.as_mut().cancel_downloads();
        let _ = upload_queue::cancel_all();
        transfers::cancel_active_for(TransferDirection::Upload, "");
        self.as_mut().set_message("正在取消全部传输……");
        self.as_mut().sync_transfers();
    }

    /// 登出 / 失效时作废在途上传。上传没有断点，下次只能从头重试。
    pub(crate) fn reset_uploads(mut self: Pin<&mut Self>) {
        super::upload_worker::clear_notice_group();
        upload_queue::reset();
        transfers::interrupt_active_for(TransferDirection::Upload);
        transfers::interrupt_paused_for(TransferDirection::Upload);
        super::downloads::flush_history();
        self.as_mut().sync_transfers();
    }
}

fn classify_dropped_uris(uris: impl IntoIterator<Item = String>) -> (Vec<PathBuf>, u32, String) {
    let mut paths = Vec::new();
    let mut invalid = 0;
    let mut first_error = String::new();
    for uri in uris {
        if uri.trim().is_empty() {
            continue;
        }
        match super::folder_picker::drop_path_from_uri(&uri) {
            Ok(path) => paths.push(path),
            Err(reason) => {
                invalid += 1;
                if first_error.is_empty() {
                    first_error = reason;
                }
            }
        }
    }
    (paths, invalid, first_error)
}

fn upload_selection_message(added: u32, already: u32, invalid: u32, first_error: &str) -> String {
    if added == 0 && already == 0 && invalid == 0 {
        return String::new();
    }
    let mut parts = Vec::new();
    if added > 0 {
        parts.push(format!("已加入上传队列：{added} 个文件"));
    }
    if already > 0 {
        parts.push(format!("{already} 个已在上传队列中"));
    }
    if invalid > 0 {
        let mut text = format!("{invalid} 个文件无法上传");
        if !first_error.is_empty() {
            text.push('：');
            text.push_str(first_error);
        }
        parts.push(text);
    }
    parts.join("；")
}

#[cfg(test)]
mod tests {
    use super::{classify_dropped_uris, upload_selection_message};
    use std::path::PathBuf;

    #[test]
    fn selection_message_reports_added_and_rejected_files() {
        assert_eq!(
            upload_selection_message(2, 1, 1, "文件名不是有效的 UTF-8"),
            "已加入上传队列：2 个文件；1 个已在上传队列中；1 个文件无法上传：文件名不是有效的 UTF-8"
        );
    }

    #[test]
    fn cancelled_selection_stays_silent() {
        assert_eq!(upload_selection_message(0, 0, 0, ""), "");
    }

    #[test]
    fn dropped_uris_keep_local_files_and_report_the_first_error() {
        let (paths, invalid, first_error) = classify_dropped_uris(
            [
                "file:///tmp/a%20b.mkv".to_owned(),
                "https://example.com/c.mkv".to_owned(),
                "file:///tmp/%E7%94%B5%E5%BD%B1.mp4".to_owned(),
            ]
            .into_iter(),
        );
        assert_eq!(
            paths,
            vec![
                PathBuf::from("/tmp/a b.mkv"),
                PathBuf::from("/tmp/电影.mp4")
            ]
        );
        assert_eq!(invalid, 1);
        assert_eq!(first_error, "只能拖入本地文件");
    }
}
