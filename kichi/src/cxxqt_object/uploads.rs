//! 上传的 QML 入口：系统多选文件 / 文件夹、拖放、后台扫描与串行调度。
//!
//! 本地路径解析、普通文件 / 文件夹检查、去重和结果提示都在 Rust 侧完成，
//! 让 Portal、拖放和扫描三条入口遵守同一套规则。

use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use cxx_qt_lib::{QString, QStringList};
use kichi_core::EntryKind;

use super::qobject;
use super::transfers::{self, TransferDirection, TransferJob, UploadJob};
use super::upload_queue::{self, ReserveError};
use super::upload_scan::{self, ScanOutcome, ScanResult, ScannedFile};
use super::upload_worker::UploadFile;
use crate::session::download::Cancel;
use crate::session::State;

#[derive(Default)]
struct UploadAddSummary {
    files: u32,
    folders: u32,
    already: u32,
    invalid: u32,
    first_error: String,
}

impl UploadAddSummary {
    fn merge(&mut self, other: Self) {
        self.files = self.files.saturating_add(other.files);
        self.folders = self.folders.saturating_add(other.folders);
        self.already = self.already.saturating_add(other.already);
        self.invalid = self.invalid.saturating_add(other.invalid);
        if self.first_error.is_empty() {
            self.first_error = other.first_error;
        }
    }

    fn added(&self) -> u32 {
        self.files.saturating_add(self.folders)
    }
}

#[derive(Default)]
struct DroppedUploads {
    files: Vec<PathBuf>,
    folders: Vec<PathBuf>,
    invalid: u32,
    first_error: String,
}

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
                        if !upload_callback_current(qichi.as_ref(), my_epoch) {
                            return;
                        }
                        let summary = enqueue_upload_files(paths, &parent_id, &parent_name);
                        qichi.as_mut().finish_upload_add(summary);
                    });
                }
                Err(reason) => {
                    super::actions::enqueue_ui(move |mut qichi| {
                        if !upload_callback_current(qichi.as_ref(), my_epoch) {
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

    /// 通过 XDG Portal 多选本地文件夹；每个目录各自成为一个上传批次。
    pub fn choose_upload_folders(
        mut self: Pin<&mut Self>,
        parent_id: QString,
        parent_name: QString,
    ) {
        if self.backend.session.state() != State::SignedIn {
            self.as_mut().set_message("请先登录再上传文件夹");
            return;
        }
        let parent_id = parent_id.to_string();
        let parent_name = parent_name.to_string();
        let start = self.settings.download_dir_path();
        let my_epoch = upload_queue::current_epoch();
        let spawned = std::thread::Builder::new()
            .name("kichi-folder-picker".into())
            .spawn(move || match super::folder_picker::choose_folders(&start) {
                Ok(paths) if paths.is_empty() => {}
                Ok(paths) => {
                    super::actions::enqueue_ui(move |mut qichi| {
                        if !upload_callback_current(qichi.as_ref(), my_epoch) {
                            return;
                        }
                        let summary = enqueue_upload_folders(paths, &parent_id, &parent_name);
                        qichi.as_mut().finish_upload_add(summary);
                    });
                }
                Err(reason) => {
                    super::actions::enqueue_ui(move |mut qichi| {
                        if !upload_callback_current(qichi.as_ref(), my_epoch) {
                            return;
                        }
                        qichi.as_mut().set_message(&reason);
                    });
                }
            });
        if let Err(err) = spawned {
            self.as_mut()
                .set_message(&format!("无法启动系统目录选择器：{err}"));
        }
    }

    /// 文件管理器把本地文件 / 文件夹拖进文件页时，直接并入同一条上传队列。
    ///
    /// 文件夹走后台扫描，普通文件走现有单文件入口；符号链接、设备文件、
    /// socket 等特殊项在这里先挡掉，扫描线程仍会对目录内部再挡一遍。
    pub fn upload_dropped_files(
        mut self: Pin<&mut Self>,
        parent_id: QString,
        parent_name: QString,
        uris: QStringList,
    ) {
        if self.backend.session.state() != State::SignedIn {
            self.as_mut().set_message("请先登录再上传文件或文件夹");
            return;
        }
        let parent_id = parent_id.to_string();
        let parent_name = parent_name.to_string();
        let dropped = classify_dropped_uris(uris.iter().map(ToString::to_string));
        let mut summary = enqueue_upload_files(dropped.files, &parent_id, &parent_name);
        summary.merge(enqueue_upload_folders(
            dropped.folders,
            &parent_id,
            &parent_name,
        ));
        summary.invalid = summary.invalid.saturating_add(dropped.invalid);
        if summary.first_error.is_empty() {
            summary.first_error = dropped.first_error;
        }
        self.as_mut().finish_upload_add(summary);
    }

    /// 文件夹扫描完成（后台线程回投到主线程）。
    pub(crate) fn on_upload_folder_scanned(
        mut self: Pin<&mut Self>,
        batch: u64,
        outcome: ScanOutcome,
    ) {
        match outcome {
            ScanOutcome::Cancelled => {
                transfers::cancel_batch(batch, "");
                self.as_mut().sync_transfers();
                self.as_mut().advance_uploads();
            }
            ScanOutcome::Failed(reason) => {
                transfers::fail_batch(batch, &reason);
                self.as_mut().sync_transfers();
                self.as_mut().advance_uploads();
            }
            ScanOutcome::Done(result) => {
                self.as_mut().enqueue_scanned_folder(batch, result);
            }
        }
    }

    fn enqueue_scanned_folder(mut self: Pin<&mut Self>, batch: u64, result: ScanResult) {
        let Some(parent_job) = transfers::upload_folder_job(batch) else {
            self.as_mut().sync_transfers();
            return;
        };
        if !matches!(
            transfers::upload_folder_state(batch),
            Some(transfers::TransferState::Preparing | transfers::TransferState::Queued)
        ) {
            self.as_mut().sync_transfers();
            self.as_mut().advance_uploads();
            return;
        }
        let Some(parent_upload) = parent_job.upload.as_ref() else {
            transfers::fail_batch(batch, "上传文件夹缺少目标目录信息");
            self.as_mut().sync_transfers();
            self.as_mut().advance_uploads();
            return;
        };

        let jobs: Vec<TransferJob> = result
            .files
            .iter()
            .map(|file| scanned_job(parent_upload, file))
            .collect();
        let upload_files: Vec<UploadFile> = jobs
            .iter()
            .enumerate()
            .map(|(index, job)| UploadFile {
                index,
                job: job.clone(),
                cancel: Arc::new(Cancel::default()),
            })
            .collect();

        transfers::expand_folder(batch, &jobs, &result.dirs);
        transfers::set_skipped(batch, result.skipped);
        match upload_queue::enqueue_folder(
            batch,
            parent_job.clone(),
            upload_files,
            result.dirs.clone(),
            Vec::new(),
        ) {
            Ok(()) => {
                transfers::mark_folder_queued(batch);
                self.as_mut().advance_uploads();
            }
            Err(err) => {
                transfers::fail_batch(batch, &err.reason());
                self.as_mut().sync_transfers();
                self.as_mut().advance_uploads();
            }
        }
    }

    fn finish_upload_add(mut self: Pin<&mut Self>, summary: UploadAddSummary) {
        if summary.added() > 0 {
            self.as_mut().advance_uploads();
        } else {
            self.as_mut().sync_transfers();
        }
        let message = upload_selection_message(&summary);
        if !message.is_empty() {
            self.as_mut().set_message(&message);
        }
    }

    /// 推进串行上传队列（空闲时启动下一个文件 / 文件夹批次）。
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

    /// 页头「取消全部」：下载与上传都取消，扫描中的文件夹也停下。
    pub fn cancel_all_transfers(mut self: Pin<&mut Self>) {
        self.as_mut().cancel_downloads();
        upload_scan::cancel_all();
        let _ = upload_queue::cancel_all();
        transfers::cancel_active_for(TransferDirection::Upload, "");
        self.as_mut().set_message("正在取消全部传输……");
        self.as_mut().sync_transfers();
    }

    /// 登出 / 失效时作废在途上传。上传没有断点，下次只能从头重试。
    pub(crate) fn reset_uploads(mut self: Pin<&mut Self>) {
        super::upload_worker::clear_notice_group();
        upload_scan::reset();
        upload_queue::reset();
        transfers::interrupt_active_for(TransferDirection::Upload);
        transfers::interrupt_paused_for(TransferDirection::Upload);
        super::downloads::flush_history();
        self.as_mut().sync_transfers();
    }
}

fn upload_callback_current(qichi: std::pin::Pin<&super::qobject::Kichi>, my_epoch: u64) -> bool {
    my_epoch == upload_queue::current_epoch()
        && qichi.auth_state.to_string() == "signedIn"
        && qichi.backend.session.state() == State::SignedIn
}

fn enqueue_upload_files(
    paths: Vec<PathBuf>,
    parent_id: &str,
    parent_name: &str,
) -> UploadAddSummary {
    let mut summary = UploadAddSummary::default();
    for path in paths {
        match upload_queue::reserve(&path, parent_id, parent_name) {
            Ok(_) => summary.files += 1,
            Err(ReserveError::AlreadyPending) => summary.already += 1,
            Err(err) => {
                summary.invalid += 1;
                if summary.first_error.is_empty() {
                    summary.first_error = err.reason();
                }
            }
        }
    }
    summary
}

fn enqueue_upload_folders(
    paths: Vec<PathBuf>,
    parent_id: &str,
    parent_name: &str,
) -> UploadAddSummary {
    let mut summary = UploadAddSummary::default();
    for path in paths {
        let job = match upload_queue::prepare_folder_job(&path, parent_id, parent_name) {
            Ok(job) => job,
            Err(err) => {
                summary.invalid += 1;
                if summary.first_error.is_empty() {
                    summary.first_error = err.reason();
                }
                continue;
            }
        };
        if upload_queue::is_taken(&job.id) || upload_scan::is_taken(&job.id) {
            summary.already += 1;
            continue;
        }
        let batch = transfers::new_batch();
        transfers::start_batch(batch, &job.id, &job);
        transfers::mark_preparing(batch);
        transfers::set_upload_folder_detail(batch, "正在扫描本地文件夹…");
        match upload_scan::start(batch, job) {
            Ok(()) => summary.folders += 1,
            Err(reason) => {
                transfers::fail_batch(batch, &reason);
                summary.invalid += 1;
                if summary.first_error.is_empty() {
                    summary.first_error = reason;
                }
            }
        }
    }
    summary
}

fn scanned_job(parent_upload: &UploadJob, file: &ScannedFile) -> TransferJob {
    TransferJob {
        id: format!("upload:{}", file.path.to_string_lossy()),
        name: file.name.clone(),
        kind: EntryKind::File,
        size: file.size,
        audit_flagged: false,
        sub_dir: file.parent_path.clone(),
        dir: PathBuf::new(),
        direction: TransferDirection::Upload,
        upload: Some(UploadJob {
            local_path: file.path.clone(),
            parent_id: parent_upload.parent_id.clone(),
            parent_name: parent_upload.parent_name.clone(),
            batch_root_id: parent_upload.batch_root_id.clone(),
            cloud_parent_id: String::new(),
        }),
    }
}

fn classify_dropped_uris(uris: impl IntoIterator<Item = String>) -> DroppedUploads {
    let mut dropped = DroppedUploads::default();
    for uri in uris {
        if uri.trim().is_empty() {
            continue;
        }
        let path = match super::folder_picker::drop_path_from_uri(&uri) {
            Ok(path) => path,
            Err(reason) => {
                record_drop_error(&mut dropped, reason);
                continue;
            }
        };
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(err) => {
                record_drop_error(&mut dropped, format!("无法读取拖入的路径：{err}"));
                continue;
            }
        };
        let file_type = metadata.file_type();
        if file_type.is_symlink() {
            record_drop_error(&mut dropped, "不跟随符号链接".to_owned());
        } else if file_type.is_file() {
            dropped.files.push(path);
        } else if file_type.is_dir() {
            dropped.folders.push(path);
        } else {
            record_drop_error(&mut dropped, "只支持普通文件或文件夹".to_owned());
        }
    }
    dropped
}

fn record_drop_error(dropped: &mut DroppedUploads, reason: String) {
    dropped.invalid = dropped.invalid.saturating_add(1);
    if dropped.first_error.is_empty() {
        dropped.first_error = reason;
    }
}

fn upload_selection_message(summary: &UploadAddSummary) -> String {
    if summary.added() == 0 && summary.already == 0 && summary.invalid == 0 {
        return String::new();
    }
    let mut parts = Vec::new();
    let mut added = Vec::new();
    if summary.files > 0 {
        added.push(format!("{} 个文件", summary.files));
    }
    if summary.folders > 0 {
        added.push(format!("{} 个文件夹", summary.folders));
    }
    if !added.is_empty() {
        parts.push(format!("已加入上传队列：{}", added.join("、")));
    }
    if summary.already > 0 {
        parts.push(format!("{} 个已在上传队列中", summary.already));
    }
    if summary.invalid > 0 {
        let mut text = format!("{} 个无法上传", summary.invalid);
        if !summary.first_error.is_empty() {
            text.push('：');
            text.push_str(&summary.first_error);
        }
        parts.push(text);
    }
    parts.join("；")
}

#[cfg(test)]
mod tests {
    use super::{classify_dropped_uris, upload_selection_message, UploadAddSummary};

    #[test]
    fn selection_message_reports_files_and_folders() {
        let summary = UploadAddSummary {
            files: 2,
            folders: 1,
            already: 1,
            invalid: 1,
            first_error: "不跟随符号链接".to_owned(),
        };
        assert_eq!(
            upload_selection_message(&summary),
            "已加入上传队列：2 个文件、1 个文件夹；1 个已在上传队列中；1 个无法上传：不跟随符号链接"
        );
    }

    #[test]
    fn cancelled_selection_stays_silent() {
        assert_eq!(upload_selection_message(&UploadAddSummary::default()), "");
    }

    #[test]
    fn dropped_uris_keep_local_files_and_report_the_first_error() {
        let dir =
            std::env::temp_dir().join(format!("kichi-drop-{}-{}", std::process::id(), "files"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建拖放测试目录");
        let first = dir.join("a b.mkv");
        let second = dir.join("电影.mp4");
        std::fs::write(&first, b"first").expect("写测试文件");
        std::fs::write(&second, b"second").expect("写测试文件");
        let first_uri = ashpd::url::Url::from_file_path(&first)
            .expect("文件路径转 URI")
            .to_string();
        let second_uri = ashpd::url::Url::from_file_path(&second)
            .expect("文件路径转 URI")
            .to_string();
        let dropped = classify_dropped_uris(
            [
                first_uri,
                "https://example.com/c.mkv".to_owned(),
                second_uri,
            ]
            .into_iter(),
        );
        assert_eq!(dropped.files, vec![first.clone(), second.clone()]);
        assert_eq!(dropped.invalid, 1);
        assert_eq!(dropped.first_error, "只能拖入本地文件或文件夹");
        let _ = std::fs::remove_dir_all(dir);
    }
}
