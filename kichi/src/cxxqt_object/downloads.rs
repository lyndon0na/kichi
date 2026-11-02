//! 下载到本地：QML 入口与状态属性。
//!
//! 队列节奏与后台细节在 [`super::queue`]：
//! - **独立单文件最多并发 `KICHI_DOWNLOAD_PARALLEL` 批**；文件夹批次独占，
//!   文件夹内部再按同一上限并发下载子文件，避免两层并发数叠加；
//! - 判重键是云端条目 id：同一条目在排队 / 准备 / 下载中时点下载 = 跳转定位；
//! - 文件夹下载轮到时才递归列目录，落盘时保留层级，并先建出完整目录树；
//! - **空文件夹算成功**：建出目录即完成，不弹通知；读不到的子目录按跳过处理
//!   （边界见 `docs/adr/0012-folder-download-boundaries.md`）；
//! - 进度经主线程队列回投（`pump()` 消费），后台线程不碰 Qt 对象；
//! - 登出时在途下载停止并**保留断点**，绝不落成品；旧线程退出前新批次只排队。
//!
//! 逐条记录在 [`super::transfers`]；本文件只做：入口校验、发定位请求、
//! 调度器推进、把记录表刷到 QML 属性。

use std::pin::Pin;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::Instant;

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QString, QStringList};

use super::file_optimistic::is_unconfirmed_create;
use super::qobject;
use super::queue;
use super::queue_collect;
use super::reveal;
use super::transfers;
use super::transfers_actions;
use super::transfers_detail;
use super::transfers_history::{self, StoreTarget};
use super::transfers_rate::heartbeat_due;
use super::upload_queue;
use super::upload_scan;
use crate::session::Failure;

/// 上一次推 `transferLive` 的时刻（心跳节流；只有主线程读写）。
fn last_live_push() -> &'static Mutex<Option<Instant>> {
    static LAST: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();
    LAST.get_or_init(|| Mutex::new(None))
}

/// 记下这次推送：结构同步也会顺带刷新 `transferLive`，心跳不必马上再来一次。
fn mark_live_pushed() {
    *last_live_push()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Instant::now());
}

/// 这一轮运行的历史文件目标（启动时加载定下，之后不变）。
fn history_target() -> &'static Mutex<Option<StoreTarget>> {
    static TARGET: OnceLock<Mutex<Option<StoreTarget>>> = OnceLock::new();
    TARGET.get_or_init(|| Mutex::new(None))
}

fn with_target<T>(f: impl FnOnce(&mut Option<StoreTarget>) -> T) -> T {
    let mut guard: MutexGuard<'static, Option<StoreTarget>> = history_target()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    f(&mut guard)
}

/// 启动时加载一次传输历史（首次 `sync_transfers()` 之前由构造器调用）。
///
/// 走不进 `impl Kichi`：构造器里 `self` 还没成形，而读盘之后紧接着就要把
/// 记录表刷到属性上。
pub(crate) fn load_history(object: &mut crate::cxxqt_object::KichiRust) {
    let target = transfers_history::boot(&StoreTarget::from_env());
    transfers::set_session(target.session);
    with_target(|slot| *slot = Some(target));
    let (list, active) = transfers_history::snapshot();
    object.transfers = list;
    object.transfer_revision = object.transfer_revision.wrapping_add(1);
    object.download_active = active;
    object.transfer_has_finished = transfers_history::has_finished();
    object.transfer_has_retryable = transfers_actions::has_retryable_failed();
}

/// 强制把记录表写盘（退出 / 登出前调用）。
pub(crate) fn flush_history() {
    let target = with_target(|slot| slot.clone());
    if let Some(target) = target {
        transfers_history::flush_now(&target);
    }
}

/// 节流写盘：结构同步时调用，最多 2 秒写一次。
fn maybe_write_history() {
    let target = with_target(|slot| slot.clone());
    if let Some(target) = target {
        transfers_history::maybe_flush(&target);
    }
}

/// 进度写盘：2 秒节流 + 字节没变不写（`sync_transfer_live` 调用）。
fn maybe_write_history_progress() {
    let target = with_target(|slot| slot.clone());
    if let Some(target) = target {
        transfers_history::maybe_flush_progress(&target);
    }
}

impl qobject::Kichi {
    /// 下载一个条目（文件直接下；文件夹轮到时递归读目录）。界面右键菜单的「下载」走这里。
    ///
    /// 跳转只在「这次没建新批次」时发生：说明这个条目已经在队列里了，
    /// 用户想去看看它；第一次下载不打断用户当前所在页面。
    pub fn download_entry(mut self: Pin<&mut Self>, id: QString) {
        let id = id.to_string();
        if is_unconfirmed_create(self.as_ref().rust(), &id) {
            self.as_mut()
                .set_message("文件夹正在同步真实 id，请稍后再下载");
            return;
        }
        // 搜索结果里的条目也能直接下：先在当前目录找，再找搜索命中
        let Some(entry) = self.as_ref().rust().find_entry(&id) else {
            self.as_mut()
                .set_message("该条目已不在当前列表中，请刷新后重试");
            return;
        };
        let dir = self.settings.download_dir_path();
        let source_id = entry.id.clone();
        // 登记成功后先推进队列：有空闲槽时这一下就把线程起来了；槽满就只是排队。
        // `advance_downloads` 里已经 sync 过一次，这里不再重复。
        if queue::reserve(entry, dir).is_some() {
            self.as_mut().advance_downloads();
            return;
        }
        // 已经在排队 / 准备 / 下载中：再点一次 = 去传输任务页看它
        self.as_mut()
            .locate_transfer(QString::from(source_id.as_str()));
    }

    /// 批量下载（多选后右键「下载到本地」）。每个条目各自一批，按传入顺序排队；
    /// 独立文件填满并发槽，轮到文件夹时等已跑文件结束再独占；
    /// 每个批次各发自己的通知（`docs/adr/0010`）。
    ///
    /// 已经在排队 / 下载中的条目直接跳过：多选时不像单条那样跳去传输页——
    /// 那会打断用户正在看的目录。结果用状态栏一句话说清楚。
    pub fn download_entries(mut self: Pin<&mut Self>, ids: QStringList) {
        let dir = self.settings.download_dir_path();
        let (mut added, mut already, mut missing, mut pending) = (0u32, 0u32, 0u32, 0u32);
        for raw in ids.iter() {
            let id = raw.to_string();
            let id = id.trim();
            if id.is_empty() {
                continue;
            }
            if is_unconfirmed_create(self.as_ref().rust(), id) {
                pending += 1;
                continue;
            }
            // 搜索结果里的条目也能直接下：先在当前目录找，再找搜索命中
            let Some(entry) = self.as_ref().rust().find_entry(id) else {
                missing += 1;
                continue;
            };
            if queue::reserve(entry, dir.clone()).is_some() {
                added += 1;
            } else {
                already += 1;
            }
        }
        // 同一把锁里逐条登记完再推进：第一批发车，其余排着
        if added > 0 {
            self.as_mut().advance_downloads();
        }
        if pending > 0 {
            let waiting = format!("{pending} 个文件夹正在同步真实 id，请稍后再下载");
            if added > 0 {
                self.as_mut()
                    .set_message(&format!("已加入下载队列：{added} 项；{waiting}"));
            } else {
                self.as_mut().set_message(&waiting);
            }
        } else if added > 0 && already == 0 && missing == 0 {
            self.as_mut().clear_status_message();
        } else {
            let message = batch_message(added, already, missing);
            self.as_mut().set_message(&message);
        }
    }

    /// 这个条目是否已在排队 / 准备 / 下载中（右键菜单文案用）。
    pub fn is_download_pending(&self, id: QString) -> bool {
        queue::is_pending(&id.to_string())
    }

    /// 这个文件是否有可续传的断点（右键菜单显示「继续下载」）。
    ///
    /// 只认当前下载目录里的断点：换了目录之后接着点下载会重新落一份，
    /// 菜单里不该假装「继续」。
    pub fn has_partial_download(&self, id: QString) -> bool {
        let dir = self.settings.download_dir_path();
        crate::session::partials::lookup(&id.to_string(), &dir).is_some()
    }

    /// 取消所有未结束的批次（排队中的直接放掉，正在跑的等线程收尾；已下完的文件保留）。
    pub fn cancel_downloads(mut self: Pin<&mut Self>) {
        let paused = transfers::paused_batches();
        let queued = queue::cancel_all();
        // 立刻反映到记录表：正在下的那条与排队中的都变「已取消」，
        // 不用等下载线程读到取消标志（网络卡住时可能要等当前读返回）。
        // `downloadRunning` 由调度器的收尾推进，不在这里清。
        transfers::cancel_active_for(transfers::TransferDirection::Download, "");
        // 暂停批次已经不在调度器里；未完成子行也要取消并删掉断点。
        for batch in paused {
            transfers::cancel_paused_batch(batch);
        }
        // 排队批次没有线程替它们收尾；包括「暂停后继续又马上取消」的情况，
        // 旧 part 必须在这里同步清掉。
        for batch in queued {
            transfers::discard_batch_parts(batch);
        }
        self.as_mut().set_message("正在取消下载……");
        self.as_mut().sync_transfers();
    }

    /// 取消一条传输记录：顶层行取消整批，文件夹子文件只取消自己。
    pub fn cancel_transfer(mut self: Pin<&mut Self>, key: QString) {
        let key = key.to_string();
        let Some(target) = transfers_actions::cancel_target(&key) else {
            self.as_mut().set_message("这条记录已经不在进行中");
            return;
        };
        if target.direction == transfers::TransferDirection::Upload {
            let session = self.backend.session.clone();
            if !target.whole_batch {
                if upload_queue::cancel_file(target.batch, target.index) {
                    let targets =
                        transfers::upload_resume_targets(target.batch, Some(target.index));
                    crate::session::upload::discard_upload_resumes(&session, targets);
                    transfers::cancel_file(target.batch, target.index);
                    self.as_mut()
                        .set_message(&format!("已取消上传：{}", target.name));
                } else {
                    self.as_mut().set_message("这个上传文件已经结束");
                }
                self.as_mut().advance_uploads();
                return;
            }
            let scanning = upload_scan::cancel(target.batch);
            let outcome = upload_queue::cancel(target.batch);
            let targets = transfers::upload_resume_targets(target.batch, None);
            crate::session::upload::discard_upload_resumes(&session, targets);
            // 扫描线程可能已经结束、回投主线程的回调还没跑；这时调度器里还
            // 没有批次，但记录仍是「准备中」。先把记录标取消，回调会据此
            // 丢掉扫描结果，不能让用户刚点的取消被随后入队覆盖。
            transfers::cancel_batch(target.batch, "");
            let prefix = if scanning || outcome == Some(upload_queue::CancelOutcome::Running) {
                "正在取消上传"
            } else {
                "已取消上传"
            };
            self.as_mut()
                .set_message(&format!("{prefix}：{}", target.name));
            self.as_mut().advance_uploads();
            return;
        }
        if target.state == transfers::TransferState::Paused {
            if !target.whole_batch {
                self.as_mut()
                    .set_message("请在文件夹总行上取消整个暂停任务");
                return;
            }
            transfers::cancel_paused_batch(target.batch);
            self.as_mut()
                .set_message(&format!("已取消并清理断点：{}", target.name));
            self.as_mut().sync_transfers();
            return;
        }
        let outcome = match queue::cancel_transfer(target.batch, target.index, target.whole_batch) {
            Some(outcome) => outcome,
            None => {
                self.as_mut().set_message("这条记录已经结束");
                return;
            }
        };
        if target.whole_batch {
            transfers::cancel_batch(target.batch, "");
            self.as_mut()
                .set_message(&format!("正在取消：{}", target.name));
            if outcome == queue::CancelOutcome::Queued {
                transfers::discard_batch_parts(target.batch);
            }
        } else {
            transfers::cancel_file(target.batch, target.index);
            self.as_mut()
                .set_message(&format!("正在取消：{}", target.name));
            if outcome == queue::CancelOutcome::Queued {
                transfers::discard_file_part(target.batch, target.index);
            }
        }
        self.as_mut().advance_downloads();
    }

    /// 暂停一个顶层传输批次。子文件级暂停不提供。
    pub fn pause_transfer(mut self: Pin<&mut Self>, key: QString) {
        let key = key.to_string();
        let Some(target) = transfers_actions::cancel_target(&key) else {
            self.as_mut().set_message("这条记录已经不能暂停");
            return;
        };
        if target.direction == transfers::TransferDirection::Upload {
            if target.state == transfers::TransferState::Paused {
                self.as_mut().set_message("这个上传任务已经暂停");
                return;
            }
            match upload_queue::pause(target.batch) {
                upload_queue::PauseOutcome::Queued => {
                    transfers::pause_queued(target.batch);
                    self.as_mut().clear_status_message();
                }
                upload_queue::PauseOutcome::Pausing => {
                    transfers::mark_pausing(target.batch);
                    self.as_mut()
                        .set_message(&format!("正在暂停：{}", target.name));
                }
                upload_queue::PauseOutcome::NotCurrent => {
                    self.as_mut().set_message("这条上传记录已经结束");
                }
            }
            self.as_mut().advance_uploads();
            return;
        }
        if !target.whole_batch {
            self.as_mut()
                .set_message("请暂停整个文件夹，不能在子文件上单独暂停");
            return;
        }
        if target.state == transfers::TransferState::Paused {
            self.as_mut().set_message("这个任务已经暂停");
            return;
        }
        match queue::pause_transfer(target.batch) {
            queue::PauseOutcome::Queued => {
                transfers::pause_queued(target.batch);
                self.as_mut().clear_status_message();
            }
            queue::PauseOutcome::Running => {
                transfers::mark_pausing(target.batch);
                self.as_mut()
                    .set_message(&format!("正在暂停：{}", target.name));
            }
            queue::PauseOutcome::NotCurrent => {
                self.as_mut().set_message("这条记录已经结束");
            }
        }
        self.as_mut().advance_downloads();
    }

    /// 继续一个已暂停的顶层批次；复用原批次、原记录和原下载目录。
    pub fn resume_transfer(mut self: Pin<&mut Self>, key: QString) {
        let key = key.to_string();
        if let Some(target) = transfers_actions::upload_resume_target(&key) {
            if !upload_queue::resume(target.batch) {
                self.as_mut().sync_transfers();
                self.as_mut().set_message(&format!(
                    "这条上传任务已经不能继续，请重试：{}",
                    target.name
                ));
                return;
            }
            transfers::resume_paused(target.batch);
            self.as_mut().clear_status_message();
            self.as_mut().advance_uploads();
            return;
        }
        let Some(target) = transfers_actions::resume_target(&key) else {
            self.as_mut().set_message("这条记录当前不能继续");
            return;
        };
        let batch = target.batch;
        if !queue::reserve_resume(target) {
            self.as_mut()
                .set_message("这个条目已有下载任务在排队或运行");
            return;
        }
        transfers::resume_paused(batch);
        self.as_mut().clear_status_message();
        self.as_mut().advance_downloads();
    }

    /// 启动恢复登录态后消费正常退出留下的自动续跑标记（整个进程只调用一次）。
    pub(crate) fn auto_resume_restored(mut self: Pin<&mut Self>) {
        let targets = transfers_actions::take_auto_resume();
        // 无论设置是否开启、是否成功入队，都先把标记清掉并落盘。
        flush_history();
        if !self.settings.auto_resume_downloads || targets.is_empty() {
            self.as_mut().sync_transfers();
            return;
        }
        let mut resumed = false;
        for target in targets {
            let batch = target.batch;
            if queue::reserve_resume(target) {
                transfers::resume_paused(batch);
                resumed = true;
            }
        }
        if resumed {
            self.as_mut().clear_status_message();
            self.as_mut().advance_downloads();
        } else {
            self.as_mut().sync_transfers();
        }
    }

    /// 在文件管理器里打开下载目录（没建就顺手建好）。
    pub fn open_download_dir(&self) {
        let dir = self.settings.download_dir_path();
        if let Err(err) = std::fs::create_dir_all(&dir) {
            eprintln!("[kichi] 创建下载目录失败：{err}");
        }
        let spawned = std::process::Command::new("xdg-open")
            .arg(&dir)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        if let Err(err) = spawned {
            eprintln!("[kichi] 打开下载目录失败：{err}");
        }
    }

    /// 清空已结束（完成 / 失败 / 已取消）的记录，进行中 / 排队 / 已中断的保留。
    ///
    /// 清的是**全部历史**里所有已结束的整批（ADR 0014）。
    pub fn clear_finished_transfers(mut self: Pin<&mut Self>) {
        transfers::clear_all();
        self.as_mut().sync_transfers();
    }

    /// 从历史里移除一条顶层记录。只接受已结束 / 已中断的行；
    /// 文件夹批次会连同子行一起移除，但不动本地文件或断点。
    pub fn remove_transfer(mut self: Pin<&mut Self>, key: QString) -> bool {
        if !transfers_actions::remove_record(&key.to_string()) {
            self.as_mut()
                .set_message("这条记录不存在或仍在进行中，不能移除");
            return false;
        }
        self.as_mut().clear_status_message();
        self.as_mut().sync_transfers();
        flush_history();
        true
    }

    /// 重试一条失败 / 已中断的传输记录：按原 id、原下载目录、原子目录重新入队。
    ///
    /// 文件夹总账行（读目录前整体失败或整个中断）整批重来：重新读一遍目录，
    /// 断点能否接上取决于 `partials.json`（改过下载目录 = 从头下）。
    ///
    /// 顺序：取目标（记录锁）→ 查重 + 登记（调度锁）→ 成功后才删旧行。
    /// 调用方负责在批量登记完成后统一推进队列。
    fn retry_one(self: Pin<&mut Self>, key: &str) -> RetryOutcome {
        // 上传文件夹的父行可能已有子行：目录创建失败时不能走 `retry_target`
        // （它对有子行的文件夹返回 None），也不能把整批重新扫描。先在原批次
        // 里复用已创建的目录，只重排失败 / 中断的成员。
        if let Some(retry) = transfers_actions::upload_folder_batch_retry(key) {
            let batch = retry.batch;
            let source_id = retry.parent.id.clone();
            return match upload_queue::retry_folder(
                batch,
                retry.parent.clone(),
                retry.files,
                retry.dirs,
                retry.upload_dirs,
            ) {
                Ok(()) => {
                    if transfers::retry_upload_folder(batch) {
                        RetryOutcome::QueuedUpload
                    } else {
                        let _ = upload_queue::release(batch);
                        RetryOutcome::Unavailable
                    }
                }
                Err(upload_queue::ReserveError::AlreadyPending) => {
                    RetryOutcome::AlreadyPending(source_id)
                }
                Err(err) => RetryOutcome::Invalid(err.reason()),
            };
        }

        let Some(job) = transfers_actions::retry_target(key) else {
            return RetryOutcome::Unavailable;
        };
        if job.is_upload() {
            if let Some((batch, parent_job)) = transfers_actions::upload_folder_retry(key) {
                transfers::reset_upload_folder_for_scan(batch);
                return match upload_scan::start(batch, parent_job) {
                    Ok(()) => RetryOutcome::QueuedUpload,
                    Err(reason) => {
                        transfers::fail_batch(batch, &reason);
                        RetryOutcome::Invalid(reason)
                    }
                };
            }
            if let Some(retry) = transfers_actions::upload_child_retry(key) {
                let batch = retry.batch;
                let index = retry.index;
                return match upload_queue::retry_child(
                    batch,
                    index,
                    retry.parent,
                    retry.file,
                    retry.dirs,
                    retry.upload_dirs,
                ) {
                    Ok(()) => {
                        if transfers::retry_upload_child(batch, index) {
                            RetryOutcome::QueuedUpload
                        } else {
                            let _ = upload_queue::release(batch);
                            RetryOutcome::Unavailable
                        }
                    }
                    Err(upload_queue::ReserveError::AlreadyPending) => {
                        RetryOutcome::AlreadyPending(job.id)
                    }
                    Err(err) => RetryOutcome::Invalid(err.reason()),
                };
            }
            return match upload_queue::reserve_retry(&job) {
                Ok(_) => {
                    transfers_actions::remove(key);
                    RetryOutcome::QueuedUpload
                }
                Err(upload_queue::ReserveError::AlreadyPending) => {
                    RetryOutcome::AlreadyPending(job.id)
                }
                Err(err) => RetryOutcome::Invalid(err.reason()),
            };
        }
        let source_id = job.id.clone();
        // 读目录前整体失败 / 整个中断的文件夹：整批重来（会重新读一遍目录）。
        // 先把旧行抽出来（`take_whole_folder` 删掉整批），登记失败时再挂回去，历史不丢。
        let whole_folder = transfers_actions::is_whole_folder(key)
            .then(|| transfers_actions::take_whole_folder(key))
            .flatten();
        let retry_job = whole_folder
            .as_ref()
            .map(|(_, job)| job.clone())
            .unwrap_or(job);
        if queue::reserve_retry(retry_job).is_some() {
            // 整批重试时旧行已经被抽走；丢掉暂存的目录树（新批次会重新读目录）
            if let Some((batch, _)) = &whole_folder {
                transfers_actions::forget_stash(*batch);
            } else {
                transfers_actions::remove(key);
            }
            RetryOutcome::Queued
        } else {
            if let Some((batch, restored)) = whole_folder {
                // 登记失败：把抽走的整批放回去，免得「已中断」行凭空消失
                transfers_actions::restore_folder(batch, &restored);
            }
            RetryOutcome::AlreadyPending(source_id)
        }
    }

    /// 重试一条失败 / 已中断的传输记录：按原 id、原下载目录、原子目录重新入队。
    ///
    /// 文件夹总账行（读目录前整体失败或整个中断）整批重来：重新读一遍目录，
    /// 断点能否接上取决于 `partials.json`（改过下载目录 = 从头下）。
    pub fn retry_transfer(mut self: Pin<&mut Self>, key: QString) {
        match self.as_mut().retry_one(&key.to_string()) {
            RetryOutcome::Queued => {
                self.as_mut().clear_status_message();
                // advance 会 sync；线程起不来时它会用失败原因覆盖上面的提示
                self.as_mut().advance_downloads();
            }
            RetryOutcome::QueuedUpload => {
                self.as_mut().clear_status_message();
                if let Some(warning) = crate::session::upload_resume::take_persistence_warning() {
                    self.as_mut().set_message(warning);
                }
                self.as_mut().advance_uploads();
            }
            RetryOutcome::AlreadyPending(source_id) => {
                self.as_mut().sync_transfers();
                // 同一个文件 id 已在排队 / 下载中：旧行不删，带用户去看已有那一行
                self.as_mut().set_message("这个任务已在传输队列中");
                self.as_mut()
                    .locate_transfer(QString::from(source_id.as_str()));
            }
            RetryOutcome::Invalid(reason) => {
                self.as_mut().sync_transfers();
                self.as_mut().set_message(&reason);
            }
            RetryOutcome::Unavailable => {
                self.as_mut().set_message("这条记录已经不能重试了");
            }
        }
    }

    /// 重试全部失败记录。先一次性登记完，再推进调度器，避免每一条都重建列表。
    pub fn retry_failed_transfers(mut self: Pin<&mut Self>) {
        let keys = transfers_actions::retryable_failed_keys();
        if keys.is_empty() {
            self.as_mut().set_message("没有可重试的失败项");
            return;
        }
        let (mut queued, mut pending, mut invalid) = (0u32, 0u32, 0u32);
        let mut first_invalid = String::new();
        for key in keys {
            match self.as_mut().retry_one(&key) {
                RetryOutcome::Queued | RetryOutcome::QueuedUpload => queued += 1,
                RetryOutcome::AlreadyPending(_) => pending += 1,
                RetryOutcome::Invalid(reason) => {
                    invalid += 1;
                    if first_invalid.is_empty() {
                        first_invalid = reason;
                    }
                }
                RetryOutcome::Unavailable => {}
            }
        }
        if queued > 0 {
            if pending > 0 || invalid > 0 {
                let mut text = format!("已重试 {queued} 项失败任务；{pending} 项已在传输队列中");
                if invalid > 0 {
                    text.push_str(&format!("；{invalid} 项无法重试：{first_invalid}"));
                }
                self.as_mut().set_message(&text);
            } else {
                self.as_mut().clear_status_message();
            }
            self.as_mut().advance_downloads();
            self.as_mut().advance_uploads();
        } else {
            self.as_mut().sync_transfers();
            if pending > 0 {
                self.as_mut().set_message("失败项已在传输队列中");
            } else if invalid > 0 {
                self.as_mut()
                    .set_message(&format!("{invalid} 项无法重试：{first_invalid}"));
            } else {
                self.as_mut().set_message("没有可重试的失败项");
            }
        }
    }

    /// 展开某个文件夹批次：只有「已有子文件」的父行有效。
    ///
    /// 无效 key（排队中 / 准备中 / 空文件夹）不会激活子行快照；
    /// QML 侧看到 `transferDetailActive` 仍为 false 就不插入子行。
    pub fn open_transfer_folder(mut self: Pin<&mut Self>, key: QString) {
        transfers_detail::select_folder(&key.to_string());
        self.as_mut().sync_transfers();
    }

    /// 收起当前文件夹的内联子行。
    pub fn close_transfer_folder(mut self: Pin<&mut Self>) {
        if transfers_detail::clear_selection() {
            self.as_mut().sync_transfers();
        }
    }

    /// 在文件管理器里显示一条已完成记录的落盘文件。
    pub fn reveal_transfer(mut self: Pin<&mut Self>, key: QString) {
        // 路径从 PathBuf 直接交给后台线程，非 UTF-8 文件名不会在往返中失真
        let Some(path) = transfers_actions::path_for(&key.to_string()) else {
            self.as_mut().set_message("这条记录没有可定位的落盘路径");
            return;
        };
        if let Err(err) = reveal::spawn(path) {
            self.as_mut()
                .set_message(&format!("无法打开文件管理器：{err}"));
        }
    }

    /// 推进调度器：空闲且队列非空时起下一批的线程。主线程调用。
    pub(crate) fn advance_downloads(mut self: Pin<&mut Self>) {
        let session = self.backend.session.clone();
        let failures = queue::advance(&session);
        let had_failures = !failures.is_empty();
        for failure in &failures {
            self.as_mut()
                .notify_batch_failed(&failure.name, failure.folder, &failure.reason);
        }
        if had_failures {
            // 失败不进状态栏（用户选择），只清掉可能残留的「开始下载…」
            self.as_mut().set_message("");
        }
        self.as_mut().sync_transfers();
    }

    /// 文件夹批次读完目录后的收尾（主线程，由下载线程回投）。
    pub(crate) fn on_folder_collected(
        mut self: Pin<&mut Self>,
        batch: u64,
        result: Result<queue_collect::Collected, Failure>,
        cancelled: bool,
        paused: bool,
        my_epoch: u64,
    ) {
        // 期间退出过登录：这一批属于上个会话（在途行已落成「已中断」），只还调度槽、不碰记录。
        // 旧线程真正退出后，重新登录排队的批次才能开始。
        if my_epoch != queue::current_epoch() {
            let _ = queue::release(batch);
            self.as_mut().advance_downloads();
            return;
        }
        if let Some(collected) = self
            .as_mut()
            .settle_collected(batch, result, cancelled, paused)
        {
            let total = collected.files.len();
            let session = self.backend.session.clone();
            match queue::start_download(&session, batch, collected) {
                // 空文件夹没有「开始下载 0 个文件」这种提示：
                // 建完目录后由收尾写「已创建空文件夹」
                queue::StartOutcome::Started if total > 0 => self
                    .as_mut()
                    .set_message(&format!("开始下载 {total} 个文件……")),
                queue::StartOutcome::Started => {}
                // 线程起不来：`start_download` 已把这一批标失败并释放
                queue::StartOutcome::Failed(failure) => {
                    self.as_mut().notify_batch_failed(
                        &failure.name,
                        failure.folder,
                        &failure.reason,
                    );
                    self.as_mut().set_message("");
                }
                // 这一批已不是当前批次 / 期间被取消：记录已收尾，下面照常推进
                queue::StartOutcome::NotCurrent
                | queue::StartOutcome::Cancelled
                | queue::StartOutcome::Paused => {}
            }
        }
        // 不管这一批是开跑、失败还是空文件夹，都要让后面排队的顶上
        self.as_mut().advance_downloads();
    }

    /// 收集结果 → 记录表状态。返回收集到的目录树；`None` = 这一批到此为止。
    fn settle_collected(
        mut self: Pin<&mut Self>,
        batch: u64,
        result: Result<queue_collect::Collected, Failure>,
        cancelled: bool,
        paused: bool,
    ) -> Option<queue_collect::Collected> {
        let failure = match result {
            _ if paused => {
                // 先于取消分支：暂停同样会置停止标志，但不能删断点或写成已取消。
                transfers::mark_paused(batch);
                let _ = queue::release(batch);
                self.as_mut().sync_transfers();
                return None;
            }
            _ if cancelled => {
                // 用户主动取消：不弹系统通知
                transfers::cancel_batch(batch, "");
                let _ = queue::release(batch);
                self.as_mut().sync_transfers();
                return None;
            }
            // 空文件夹也是成功：目录由下载线程建好后标完成
            Ok(collected) => return Some(collected),
            Err(err) => err,
        };
        transfers::fail_batch(batch, &failure.to_string());
        let report = queue::release(batch);
        self.as_mut().on_download_failure(failure, report);
        self.as_mut().sync_transfers();
        None
    }

    /// 把记录表刷到 QML 属性（主线程调用）。
    ///
    /// 结构变化（排队 / 准备 / 开始 / 完成 / 失败 / 取消 / 清空）走这里，整个模型重建；
    /// 顺手按 2 秒节流写一次传输历史（`docs/adr/0014-download-history-on-disk.md`）。
    pub(crate) fn sync_transfers(mut self: Pin<&mut Self>) {
        let (list, active) = transfers_history::snapshot();
        let has_finished = transfers_history::has_finished();
        let has_retryable = transfers_actions::has_retryable_failed();
        let live = transfers::live_snapshot();
        let (detail, detail_active) = transfers_detail::detail_snapshot();
        let (running, queued) = queue::counts();
        let (upload_running, upload_queued) = upload_queue::counts();
        let upload_active = transfers::upload_active_count();
        {
            let mut rust = self.as_mut().rust_mut();
            rust.transfers = list;
            rust.transfer_revision = rust.transfer_revision.wrapping_add(1);
            rust.transfer_live = live;
            rust.transfer_detail = detail;
            rust.transfer_detail_active = detail_active;
            rust.download_active = active;
            rust.transfer_has_finished = has_finished;
            rust.transfer_has_retryable = has_retryable;
            rust.download_running = running;
            rust.download_queued = queued;
            rust.upload_queued = upload_queued;
            rust.upload_active = upload_active.max(if upload_running { 1 } else { 0 });
        }
        mark_live_pushed();
        maybe_write_history();
        self.as_mut().notify();
    }

    /// 只刷新进度（进度节拍用）。
    ///
    /// `transfers` 模型不动：给 ListView 的 model 赋新值会整列表重灌、
    /// 滚动位置回到顶部（2026-10-27 实测），下载期间就没法往下翻。
    pub(crate) fn sync_transfer_live(mut self: Pin<&mut Self>) {
        let live = transfers::live_snapshot();
        mark_live_pushed();
        {
            let mut rust = self.as_mut().rust_mut();
            rust.transfer_live = live;
        }
        self.as_mut().notify();
        // 进度也要落盘（2 秒节流 + 字节没变不写）：只在结构事件写盘的话，
        // 长下载被 SIGKILL / 直接注销后，重启的「已中断」行会停在上次结构变化的
        // 进度（常见是 0），而不是实际下载到的位置（ADR 0014「代价与后果」）。
        maybe_write_history_progress();
    }

    /// 主线程 1Hz 心跳：没有进度回调时也让「已卡住 N 秒」继续走。
    ///
    /// QML 的 `pump()` 每 100ms 调一次，这里按 `LIVE_HEARTBEAT` 节流。
    /// 卡住时下载线程一个字节都收不到、自然没有回调；只靠 `progress` 推送的话
    /// 秒数会停在卡住的那一刻（见 `docs/pitfalls.md`）。
    ///
    /// 只碰 `transferLive`：`transfers` / `transferRevision` 一概不动，
    /// 否则整个列表会被重建、滚动位置被冲回顶部。
    pub(crate) fn maybe_heartbeat_live(mut self: Pin<&mut Self>) {
        // 进度只在下载中推送，空闲时 transferLive 是空的；用记录表兜底，
        // 免得残留值让心跳每秒空转一次。
        if self.transfer_live.is_empty() || !transfers::has_running_file() {
            return;
        }
        let now = Instant::now();
        let due = {
            let mut last = last_live_push()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let due = heartbeat_due(*last, now);
            if due {
                *last = Some(now);
            }
            due
        };
        if due {
            self.as_mut().sync_transfer_live();
        }
    }

    /// 下载相关错误的统一提示（令牌失效仍走登录失效分支）。
    fn on_download_failure(
        mut self: Pin<&mut Self>,
        failure: Failure,
        report: Option<queue::BatchReport>,
    ) {
        if matches!(&failure, Failure::Auth(_) | Failure::RateLimited { .. }) {
            // 这两类已有专门提示：限流弹配额通知，登录失效直接回登录页
            self.as_mut().on_failure(failure);
            return;
        }
        if let Some(report) = report {
            self.as_mut()
                .notify_batch_failed(&report.name, report.folder, &failure.to_string());
        }
        // 失败不进状态栏（用户选择）
        self.as_mut().set_message("");
    }

    /// 批次级失败通知（准备阶段失败 / 线程起不来）；失败原因不写状态栏。
    fn notify_batch_failed(self: Pin<&mut Self>, name: &str, folder: bool, reason: &str) {
        if !self.notify_enabled {
            return;
        }
        super::notifications::show_notice(&super::notifications::batch_failed(
            name, folder, reason,
        ));
    }

    /// 登出时作废在途下载；**传输历史保留**（ADR 0014）。
    ///
    /// 在途线程只置 `abandon`（停止 + 保留 part + 绝不 rename），调度槽留到
    /// 线程自己收尾；重新登录后同一文件可以入队，但旧线程退出前只显示「排队中」。
    ///
    /// 记录表**不清**：在途行先落成「已中断」（进度与重试入口都留着），再强制写盘。
    /// 旧线程退出后 `post_finished` 的 epoch 检查保证它们不再改记录。
    pub(crate) fn reset_downloads(mut self: Pin<&mut Self>) {
        queue::reset();
        // 在途的批次已经作废：记录落成「已中断」（保留重试入口），再强制写盘。
        // 旧线程退出后 `post_finished` 的 epoch 检查保证它们不再改记录。
        transfers::interrupt_active_for(transfers::TransferDirection::Download);
        flush_history();
        // 文件夹展开状态不跨会话；记录与「打开位置」都保留
        transfers_detail::clear_selection();
        self.as_mut().sync_transfers();
    }
}

enum RetryOutcome {
    Queued,
    QueuedUpload,
    AlreadyPending(String),
    Invalid(String),
    Unavailable,
}

/// 批量下载之后状态栏那句话。
///
/// 四种情况分开说，免得「已加入 3 项」掩盖「另外 2 项本来就在队列里」。
fn batch_message(added: u32, already: u32, missing: u32) -> String {
    match (added, already) {
        (0, 0) => format!("选中的 {missing} 项已不在当前列表中，请刷新后重试"),
        (0, _) => "这些条目已在下载队列中".to_owned(),
        (_, 0) => format!("已加入下载队列：{added} 项"),
        (_, _) => format!("已加入下载队列：{added} 项（{already} 项已在队列中）"),
    }
}

#[cfg(test)]
mod batch_message_tests {
    use super::batch_message;

    #[test]
    fn all_added_counts_only_new_batches() {
        assert_eq!(batch_message(3, 0, 0), "已加入下载队列：3 项");
    }

    #[test]
    fn partially_queued_says_both_numbers() {
        assert_eq!(
            batch_message(2, 1, 0),
            "已加入下载队列：2 项（1 项已在队列中）"
        );
    }

    #[test]
    fn nothing_new_points_at_the_existing_queue() {
        assert_eq!(batch_message(0, 2, 0), "这些条目已在下载队列中");
    }

    #[test]
    fn everything_missing_asks_for_a_refresh() {
        assert_eq!(
            batch_message(0, 0, 2),
            "选中的 2 项已不在当前列表中，请刷新后重试"
        );
    }
}
