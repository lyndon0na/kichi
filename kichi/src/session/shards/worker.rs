//! 分片下载的 worker：共享状态、逐片 Range 打开 / 重试 / 写盘、线程收拢。
//!
//! 从 `shards.rs` 拆出来守行数上限：那边管「要不要分片、part 准备与收尾」，
//! 这里管「每一片怎么从断点继续、失败怎么重试、进度怎么写回」。

use std::fs::File;
use std::io::Read;
use std::os::unix::fs::FileExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use kichi_core::dl::download_idle;
use kichi_core::{Error as CoreError, FileInfo};

use super::super::download::{sleep_cancellable, Cancel, DownloadCallbacks};
use super::super::download_resume::{
    classify_open_error, download_retries, resume_failure, retry_delay, OpenAction, RetryBudget,
    CHUNK,
};
use super::super::media::{file_info_for_download, KnownHint};
use super::super::partials::{self, PartialEntry, ShardSpan};
use super::super::Session;
use crate::session::Failure;

/// 分片进度写索引的最小间隔（避免每读一块就写一次 JSON）。
const INDEX_INTERVAL: Duration = Duration::from_secs(1);

/// 各 worker 的结果（每线程一个，不是每片一个）。
pub(super) enum ShardOutcome {
    Done,
    Stopped,
    Cancelled,
    /// 服务器不支持 Range：退回单连接（会截断 part）
    Fallback(String),
    /// 本地写盘错误：删 part
    Local(String),
    /// 云端大小变了：删 part，让下次重新取详情
    Changed(String),
    /// 网络 / 登录类失败：保留 part 与逐片进度
    Kept(String),
    Panicked,
}

struct ShardState {
    start: u64,
    end: u64,
    done: AtomicU64,
}

impl ShardState {
    fn len(&self) -> u64 {
        self.end.saturating_sub(self.start)
    }

    fn snapshot(&self) -> ShardSpan {
        ShardSpan {
            start: self.start,
            end: self.end,
            done: self.done.load(Ordering::SeqCst).min(self.len()),
        }
    }
}

pub(super) struct ShardShared {
    file_id: String,
    pub(super) part_path: PathBuf,
    pub(super) final_path: PathBuf,
    pub(super) total: u64,
    spans: Vec<ShardState>,
    info: Mutex<FileInfo>,
    refresh_lock: Mutex<()>,
    generation: AtomicU64,
    last_save: Mutex<Instant>,
}

impl ShardShared {
    pub(super) fn new(
        info: FileInfo,
        file_id: &str,
        part_path: PathBuf,
        final_path: PathBuf,
        total: u64,
        spans: &[ShardSpan],
    ) -> Self {
        Self {
            file_id: file_id.to_owned(),
            part_path,
            final_path,
            total,
            spans: spans
                .iter()
                .map(|span| ShardState {
                    start: span.start,
                    end: span.end,
                    done: AtomicU64::new(span.done.min(span.len())),
                })
                .collect(),
            info: Mutex::new(info),
            refresh_lock: Mutex::new(()),
            generation: AtomicU64::new(0),
            last_save: Mutex::new(Instant::now()),
        }
    }

    fn current_info(&self) -> FileInfo {
        self.info
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    /// 403 / 410 后换新链接；同一代只让一个分片真正发详情请求。
    fn refresh_info(
        &self,
        session: &Session,
        hint: KnownHint,
        cancel: &Cancel,
    ) -> Result<(), Failure> {
        let seen = self.generation.load(Ordering::SeqCst);
        let _guard = self
            .refresh_lock
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if self.generation.load(Ordering::SeqCst) != seen {
            return Ok(());
        }
        let fresh = file_info_for_download(session, &self.file_id, hint, Some(cancel))?;
        *self.info.lock().unwrap_or_else(|error| error.into_inner()) = fresh;
        self.generation.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    pub(super) fn done_bytes(&self) -> u64 {
        self.spans
            .iter()
            .map(|span| span.done.load(Ordering::SeqCst))
            .sum()
    }

    fn snapshot(&self) -> Vec<ShardSpan> {
        self.spans.iter().map(ShardState::snapshot).collect()
    }

    pub(super) fn save_index(&self) {
        partials::remember(&PartialEntry::new_sharded(
            &self.file_id,
            &self.part_path,
            &self.final_path,
            self.total,
            self.snapshot(),
        ));
    }

    fn save_index_if_due(&self) {
        let mut last = self
            .last_save
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if last.elapsed() < INDEX_INTERVAL {
            return;
        }
        *last = Instant::now();
        drop(last);
        self.save_index();
    }
}

/// 各 worker 的结果（每线程一个，不是每片一个）。
///
/// 当前线程也是 worker 之一：线程起不来时它把剩余分片做完，不会丢任务。
pub(super) fn run_workers<F>(
    workers: usize,
    pending: &[usize],
    cursor: &AtomicUsize,
    stop: &AtomicBool,
    work: F,
) -> Vec<ShardOutcome>
where
    F: Fn(usize) -> ShardOutcome + Sync,
{
    let run = || loop {
        if stop.load(Ordering::SeqCst) {
            return ShardOutcome::Stopped;
        }
        let position = cursor.fetch_add(1, Ordering::SeqCst);
        let Some(index) = pending.get(position) else {
            return ShardOutcome::Done;
        };
        let outcome = work(*index);
        if !matches!(outcome, ShardOutcome::Done | ShardOutcome::Stopped) {
            return outcome;
        }
    };
    let run = &run;
    std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for number in 1..workers {
            let spawned = std::thread::Builder::new()
                .name(format!("kichi-shard-{number}"))
                .spawn_scoped(scope, move || {
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(run))
                        .unwrap_or(ShardOutcome::Panicked)
                });
            match spawned {
                Ok(handle) => handles.push(handle),
                Err(err) => {
                    eprintln!("[kichi] 无法创建分片下载线程（{number}）：{err}");
                }
            }
        }
        let mut outcomes = vec![std::panic::catch_unwind(std::panic::AssertUnwindSafe(run))
            .unwrap_or(ShardOutcome::Panicked)];
        for handle in handles {
            outcomes.push(handle.join().unwrap_or(ShardOutcome::Panicked));
        }
        outcomes
    })
}

pub(super) fn first_local(outcomes: &[ShardOutcome]) -> Option<String> {
    outcomes.iter().find_map(|outcome| match outcome {
        ShardOutcome::Local(reason) => Some(reason.clone()),
        _ => None,
    })
}

pub(super) fn first_changed(outcomes: &[ShardOutcome]) -> Option<String> {
    outcomes.iter().find_map(|outcome| match outcome {
        ShardOutcome::Changed(reason) => Some(reason.clone()),
        _ => None,
    })
}

pub(super) fn first_fallback(outcomes: &[ShardOutcome]) -> Option<String> {
    outcomes.iter().find_map(|outcome| match outcome {
        ShardOutcome::Fallback(reason) => Some(reason.clone()),
        _ => None,
    })
}

pub(super) fn first_kept(outcomes: &[ShardOutcome]) -> Option<String> {
    outcomes.iter().find_map(|outcome| match outcome {
        ShardOutcome::Kept(reason) => Some(reason.clone()),
        _ => None,
    })
}

/// 一条 worker 线程负责一个分片（可能多片：`KICHI_DOWNLOAD_CONNECTIONS=1`）。
#[allow(clippy::too_many_arguments)]
pub(super) fn run_shard(
    session: &Session,
    shared: &ShardShared,
    index: usize,
    file: File,
    hint: KnownHint,
    callbacks: DownloadCallbacks<'_>,
    cancel: &Cancel,
    stop: &AtomicBool,
) -> ShardOutcome {
    let state = &shared.spans[index];
    let len = state.len();
    let client = match session.drive() {
        Ok(client) => client,
        Err(err) => return ShardOutcome::Kept(err.to_string()),
    };
    let mut buffer = vec![0u8; CHUNK];
    let mut budget = RetryBudget::new(download_retries());

    loop {
        if cancel.is_cancelled() {
            return ShardOutcome::Cancelled;
        }
        if stop.load(Ordering::SeqCst) {
            return ShardOutcome::Stopped;
        }
        let done = state.done.load(Ordering::SeqCst);
        if done >= len {
            return ShardOutcome::Done;
        }
        let info = shared.current_info();
        if info.size.is_some_and(|size| size != shared.total) {
            return ShardOutcome::Changed(format!(
                "云端文件大小已变化（{} → {}），已放弃旧断点",
                shared.total,
                info.size.unwrap_or(0)
            ));
        }
        let absolute = state.start + done;
        let opened = client.open_download_range_with(
            &info,
            absolute,
            state.end.saturating_sub(1),
            download_idle(),
        );
        let mut stream = match opened {
            Ok(stream) => stream,
            Err(err) => {
                if cancel.is_cancelled() {
                    return ShardOutcome::Cancelled;
                }
                if matches!(err, CoreError::RangeNotSatisfiable { .. }) {
                    return ShardOutcome::Fallback(
                        "服务器不支持按区间下载（或分片位置越界）".to_owned(),
                    );
                }
                match classify_open_error(&err, absolute) {
                    OpenAction::Fatal => return ShardOutcome::Kept(err.to_string()),
                    OpenAction::RestartFromZero => {
                        return ShardOutcome::Fallback("服务器要求从头下载".to_owned())
                    }
                    OpenAction::Retry { refresh } => {
                        let Some(attempt) = budget.grant() else {
                            return ShardOutcome::Kept(
                                resume_failure(err.to_string(), shared.done_bytes()).to_string(),
                            );
                        };
                        let delay = retry_delay(attempt);
                        (callbacks.note)(&format!(
                            "第 {} 片连接中断，{} 秒后继续（{}/{}）",
                            index + 1,
                            delay.as_secs(),
                            budget.consecutive(),
                            budget.max_consecutive()
                        ));
                        if !sleep_cancellable(cancel, delay) {
                            return ShardOutcome::Cancelled;
                        }
                        if refresh {
                            if let Err(err) = shared.refresh_info(session, hint, cancel) {
                                if cancel.is_cancelled() {
                                    return ShardOutcome::Cancelled;
                                }
                                if budget.exhausted()
                                    || matches!(
                                        err,
                                        Failure::Auth(_)
                                            | Failure::RateLimited { .. }
                                            | Failure::Captcha { .. }
                                    )
                                {
                                    return ShardOutcome::Kept(err.to_string());
                                }
                            }
                        }
                        continue;
                    }
                }
            }
        };

        match read_shard(
            &mut stream,
            &file,
            state,
            shared,
            &mut buffer,
            callbacks,
            cancel,
            stop,
        ) {
            ReadShard::Done => return ShardOutcome::Done,
            ReadShard::Cancelled => return ShardOutcome::Cancelled,
            ReadShard::Stopped => return ShardOutcome::Stopped,
            ReadShard::Local(reason) => return ShardOutcome::Local(reason),
            ReadShard::Retryable(reason) => {
                if cancel.is_cancelled() {
                    return ShardOutcome::Cancelled;
                }
                // 这一轮真读进了新字节：连续失败计数清零
                if state.done.load(Ordering::SeqCst) > done {
                    budget.progressed();
                }
                let Some(attempt) = budget.grant() else {
                    return ShardOutcome::Kept(
                        resume_failure(reason, shared.done_bytes()).to_string(),
                    );
                };
                let delay = retry_delay(attempt);
                (callbacks.note)(&format!(
                    "第 {} 片连接中断，{} 秒后继续（{}/{}）",
                    index + 1,
                    delay.as_secs(),
                    budget.consecutive(),
                    budget.max_consecutive()
                ));
                if !sleep_cancellable(cancel, delay) {
                    return ShardOutcome::Cancelled;
                }
            }
        }
    }
}

enum ReadShard {
    Done,
    Retryable(String),
    Local(String),
    Cancelled,
    Stopped,
}

#[allow(clippy::too_many_arguments)]
fn read_shard(
    stream: &mut kichi_core::DownloadStream,
    file: &File,
    state: &ShardState,
    shared: &ShardShared,
    buffer: &mut [u8],
    callbacks: DownloadCallbacks<'_>,
    cancel: &Cancel,
    stop: &AtomicBool,
) -> ReadShard {
    let len = state.len();
    loop {
        if cancel.is_cancelled() {
            return ReadShard::Cancelled;
        }
        if stop.load(Ordering::SeqCst) {
            return ReadShard::Stopped;
        }
        let done = state.done.load(Ordering::SeqCst);
        let remaining = len.saturating_sub(done);
        if remaining == 0 {
            return ReadShard::Done;
        }
        let want = buffer
            .len()
            .min(usize::try_from(remaining).unwrap_or(usize::MAX));
        let read = match stream.read(&mut buffer[..want]) {
            Ok(read) => read,
            Err(err) => {
                if cancel.is_cancelled() {
                    return ReadShard::Cancelled;
                }
                if stop.load(Ordering::SeqCst) {
                    return ReadShard::Stopped;
                }
                let reason = if err.kind() == std::io::ErrorKind::TimedOut {
                    format!("下载超时：{err}")
                } else {
                    format!("下载中断：{err}")
                };
                return ReadShard::Retryable(reason);
            }
        };
        if read == 0 {
            return ReadShard::Retryable(format!(
                "连接提前结束：还差 {}",
                crate::format::size(remaining)
            ));
        }
        let offset = state.start + done;
        if let Err(err) = file.write_all_at(&buffer[..read], offset) {
            return ReadShard::Local(format!("写入本地文件失败：{err}"));
        }
        state.done.store(done + read as u64, Ordering::SeqCst);
        (callbacks.progress)(shared.done_bytes(), Some(shared.total));
        shared.save_index_if_due();
    }
}
