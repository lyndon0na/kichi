//! 上传的 QML 入口：系统多选文件、串行调度、成功后的当前目录校准。

use std::pin::Pin;

use cxx_qt_lib::QString;

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
                        let (mut added, mut already, mut invalid) = (0u32, 0u32, 0u32);
                        let mut first_error = String::new();
                        for path in paths {
                            match upload_queue::reserve(&path, &parent_id, &parent_name) {
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
                            qichi.as_mut().advance_uploads();
                        } else {
                            qichi.as_mut().sync_transfers();
                        }
                        let message =
                            upload_selection_message(added, already, invalid, &first_error);
                        if !message.is_empty() {
                            qichi.as_mut().set_message(&message);
                        }
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
        upload_queue::reset();
        transfers::interrupt_active_for(TransferDirection::Upload);
        super::downloads::flush_history();
        self.as_mut().sync_transfers();
    }
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
    use super::upload_selection_message;

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
}
