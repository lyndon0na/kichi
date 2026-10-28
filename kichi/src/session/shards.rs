//! 单文件多分片下载：把一个文件切成 K 段，并行写同一个 `.kichi-part`。
//!
//! 只对**已知总大小且 ≥ 阈值**的文件启用（默认 32 MiB / 4 片）。小文件走
//! `download.rs` 原来的单连接路径；已有 v1 顺序断点的文件也继续顺序追加，
//! 不把已写前缀冒险转成分片。
//!
//! 写盘用 `FileExt::write_all_at`（Unix `pwrite`），每条线程一个
//! `File::try_clone()` 句柄：偏移由调用方给出，不共享带游标的 `File`。
//! 进度按「各片已写字节之和」计算，片完成顺序不定，但总和单调。
//!
//! 断点索引升 v2（`partials::ShardSpan`）：每片记 `{start, end, done}`。
//! 取消 = 不要了，删 part（ADR 0011 的决定不变）；网络失败 / 退出保留 part
//! 与逐片进度，下次从各片断点继续。
//!
//! 服务器一旦忽略了 Range（200）或 206 缺 `Content-Range`，不能把整文件写进
//! 一片：截断 part、改回 v1 索引，交给调用方走单连接重下。

mod worker;

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use kichi_core::FileInfo;

use self::worker::{
    first_changed, first_fallback, first_kept, first_local, run_shard, run_workers, ShardOutcome,
    ShardShared,
};
use super::download::{commit_lock, Cancel, DownloadCallbacks, Downloaded};
use super::download_naming::{claim_part, finalize_part};
use super::download_resume::{local_write_failure, resume_failure};
use super::media::KnownHint;
use super::partials::{self, plan_shards, PartialEntry, ShardPlan};
use super::{Failure, Session};

/// 启用分片的最小文件大小（32 MiB）。
pub(super) const SHARD_THRESHOLD: u64 = 32 * 1024 * 1024;
/// 每文件默认分片数。
pub(super) const DEFAULT_CONNECTIONS: usize = 4;
/// 覆盖分片数的环境变量；1 = 不分片。
pub(super) const CONNECTIONS_ENV: &str = "KICHI_DOWNLOAD_CONNECTIONS";
const MAX_CONNECTIONS: usize = 8;

/// 当前生效的分片数（0 / 非法回默认；1 = 不分片）。
pub(super) fn shard_connections() -> usize {
    std::env::var(CONNECTIONS_ENV)
        .ok()
        .and_then(|raw| raw.trim().parse::<usize>().ok())
        .filter(|count| *count > 0)
        .unwrap_or(DEFAULT_CONNECTIONS)
        .min(MAX_CONNECTIONS)
}

/// 尝试分片下载。返回 `None` 表示这个文件不分片，调用方走单连接路径。
#[allow(clippy::too_many_arguments)]
pub(super) fn download(
    session: &Session,
    info: &FileInfo,
    hint: KnownHint,
    file_id: &str,
    name: &str,
    target_dir: &Path,
    callbacks: DownloadCallbacks<'_>,
    cancel: &Cancel,
) -> Option<Result<Downloaded, Failure>> {
    download_with(
        session,
        info,
        hint,
        file_id,
        name,
        target_dir,
        callbacks,
        cancel,
        SHARD_THRESHOLD,
        shard_connections(),
    )
}

/// 阈值 / 分片数可注入的版本（单测用；生产走 [`download`]）。
#[allow(clippy::too_many_arguments)]
fn download_with(
    session: &Session,
    info: &FileInfo,
    hint: KnownHint,
    file_id: &str,
    name: &str,
    target_dir: &Path,
    callbacks: DownloadCallbacks<'_>,
    cancel: &Cancel,
    threshold: u64,
    wanted: usize,
) -> Option<Result<Downloaded, Failure>> {
    // 详情 / 列表已明确是小文件：连索引都不用读，直接走单连接。
    // （若索引里还留着云端变小的旧分片，顺序路径的 plan_resume 会因为
    // total 不一致把它截断，不会拿稀疏文件当已下载。）
    if info.size.is_some_and(|total| total < threshold) {
        return None;
    }
    let entry = partials::lookup(file_id, target_dir);
    let part_len = entry
        .as_ref()
        .and_then(|entry| std::fs::metadata(entry.part_path()).ok())
        .map(|meta| meta.len())
        .unwrap_or(0);
    let plan = match plan_shards(entry.as_ref(), info.size, part_len, threshold, wanted) {
        Some(plan) => plan,
        None => {
            // 极端损坏：v2 记录在、却给不出分片计划（如 total=0）。带着分片的
            // 稀疏 part 不能交给顺序追加，删掉重下比拿不可信字节冒险安全。
            if let Some(entry) = entry.as_ref().filter(|entry| !entry.shards.is_empty()) {
                let part_path = entry.part_path();
                if let Err(err) = std::fs::remove_file(&part_path) {
                    if err.kind() != std::io::ErrorKind::NotFound {
                        eprintln!(
                            "[kichi] 删除损坏的分片临时文件失败（{}）：{err}",
                            part_path.display()
                        );
                    }
                }
                partials::forget(file_id);
            }
            return None;
        }
    };
    Some(
        match run_plan(
            session,
            info,
            hint,
            file_id,
            name,
            target_dir,
            entry.as_ref(),
            plan,
            wanted,
            callbacks,
            cancel,
        ) {
            RunResult::Finished(result) => result,
            RunResult::Fallback => return None,
        },
    )
}

/// 分片执行结果：要么给最终结果，要么退回单连接。
enum RunResult {
    Finished(Result<Downloaded, Failure>),
    Fallback,
}

struct Prepared {
    file: File,
    part_path: PathBuf,
    final_path: PathBuf,
}

/// 打开（或创建）part 文件；续传时沿用索引里的路径。
fn prepare(
    entry: Option<&PartialEntry>,
    name: &str,
    target_dir: &Path,
    truncate: bool,
) -> Result<Prepared, Failure> {
    let (file, part_path, final_path) = match entry {
        Some(entry) => {
            let part_path = entry.part_path();
            let file = OpenOptions::new()
                .write(true)
                .open(&part_path)
                .map_err(local_write_failure)?;
            (file, part_path, entry.final_path())
        }
        None => claim_part(target_dir, name)?,
    };
    if truncate {
        file.set_len(0).map_err(local_write_failure)?;
    }
    Ok(Prepared {
        file,
        part_path,
        final_path,
    })
}

#[allow(clippy::too_many_arguments)]
fn run_plan(
    session: &Session,
    info: &FileInfo,
    hint: KnownHint,
    file_id: &str,
    name: &str,
    target_dir: &Path,
    entry: Option<&PartialEntry>,
    plan: ShardPlan,
    wanted: usize,
    callbacks: DownloadCallbacks<'_>,
    cancel: &Cancel,
) -> RunResult {
    let prepared = match prepare(entry, name, target_dir, plan.truncate) {
        Ok(prepared) => prepared,
        Err(err) => return RunResult::Finished(Err(err)),
    };
    let Prepared {
        file,
        part_path,
        final_path,
    } = prepared;

    let shared = ShardShared::new(
        info.clone(),
        file_id,
        part_path.clone(),
        final_path.clone(),
        plan.total,
        &plan.spans,
    );
    shared.save_index();

    if plan.complete {
        return RunResult::Finished(finish_complete(
            &shared,
            file,
            file_id,
            &part_path,
            &final_path,
            cancel,
        ));
    }

    (callbacks.progress)(plan.done_bytes(), Some(plan.total));

    let pending: Vec<usize> = plan
        .spans
        .iter()
        .enumerate()
        .filter(|(_, span)| span.done < span.len())
        .map(|(index, _)| index)
        .collect();
    if pending.is_empty() {
        // 索引说各片都写完了，但 part 长度不够：记录不可信，整文件重下
        drop(file);
        return fallback(
            file_id,
            &part_path,
            &final_path,
            plan.total,
            "分片记录与磁盘不一致",
        );
    }
    let workers = wanted.max(1).min(pending.len()).max(1);
    let cursor = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let outcomes = run_workers(workers, &pending, &cursor, &stop, |index| {
        let worker_file = match file.try_clone() {
            Ok(file) => file,
            Err(err) => {
                stop.store(true, Ordering::SeqCst);
                return ShardOutcome::Local(local_write_failure(err).to_string());
            }
        };
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_shard(
                session,
                &shared,
                index,
                worker_file,
                hint,
                callbacks,
                cancel,
                &stop,
            )
        }))
        .unwrap_or(ShardOutcome::Panicked);
        if !matches!(outcome, ShardOutcome::Done | ShardOutcome::Stopped) {
            stop.store(true, Ordering::SeqCst);
        }
        outcome
    });

    if outcomes
        .iter()
        .any(|outcome| matches!(outcome, ShardOutcome::Panicked))
    {
        // 让外层 `run_jobs` 的 catch_unwind 走中断收尾；part 与索引留着
        std::panic::resume_unwind(Box::new("下载线程异常中断"));
    }
    if cancel.is_cancelled()
        || outcomes
            .iter()
            .any(|o| matches!(o, ShardOutcome::Cancelled))
    {
        // 登出与用户取消分开：登出保留 part + 逐片索引，下次登录续传
        if cancel.is_abandoned() {
            shared.save_index();
        } else {
            cleanup(file_id, &prepared_paths(&shared));
        }
        return RunResult::Finished(Err(Failure::Other("下载已取消".to_owned())));
    }
    if let Some(reason) = first_local(&outcomes) {
        cleanup(file_id, &prepared_paths(&shared));
        return RunResult::Finished(Err(Failure::Other(reason)));
    }
    if let Some(reason) = first_changed(&outcomes) {
        cleanup(file_id, &prepared_paths(&shared));
        return RunResult::Finished(Err(Failure::Other(reason)));
    }
    if outcomes
        .iter()
        .all(|outcome| matches!(outcome, ShardOutcome::Done))
        && shared.done_bytes() == plan.total
        && file.metadata().map(|meta| meta.len()).unwrap_or(0) >= plan.total
    {
        return RunResult::Finished(finish_complete(
            &shared,
            file,
            file_id,
            &part_path,
            &final_path,
            cancel,
        ));
    }
    if let Some(reason) = first_fallback(&outcomes) {
        drop(file);
        return fallback(file_id, &part_path, &final_path, plan.total, &reason);
    }
    // 有片没写完但也没报错（被别的片 stop 掉）：按断点保留，别假装成功
    shared.save_index();
    let reason = first_kept(&outcomes).unwrap_or_else(|| "分片下载未完成".to_owned());
    RunResult::Finished(Err(resume_failure(reason, shared.done_bytes())))
}

fn prepared_paths(shared: &ShardShared) -> (PathBuf, PathBuf) {
    (shared.part_path.clone(), shared.final_path.clone())
}

fn cleanup(file_id: &str, paths: &(PathBuf, PathBuf)) {
    if let Err(err) = std::fs::remove_file(&paths.0) {
        if err.kind() != std::io::ErrorKind::NotFound {
            eprintln!(
                "[kichi] 删除临时下载文件失败（{}）：{err}",
                paths.0.display()
            );
        }
    }
    partials::forget(file_id);
}

/// 服务器不支持分片：截断 part、改回 v1 索引，让调用方走单连接。
fn fallback(
    file_id: &str,
    part_path: &Path,
    final_path: &Path,
    total: u64,
    reason: &str,
) -> RunResult {
    match OpenOptions::new().write(true).open(part_path) {
        Ok(file) => {
            if let Err(err) = file.set_len(0) {
                drop(file);
                cleanup(
                    file_id,
                    &(part_path.to_path_buf(), final_path.to_path_buf()),
                );
                return RunResult::Finished(Err(local_write_failure(err)));
            }
        }
        Err(err) => {
            cleanup(
                file_id,
                &(part_path.to_path_buf(), final_path.to_path_buf()),
            );
            return RunResult::Finished(Err(local_write_failure(err)));
        }
    }
    // 顺序路径靠条目找到同一个 part；total 一致、part 长度 0 → 从 0 开始
    partials::remember(&PartialEntry::new(
        file_id,
        part_path,
        final_path,
        Some(total),
    ));
    eprintln!("[kichi] 服务器不支持分片下载，改回单连接重试：{reason}");
    RunResult::Fallback
}

fn finish_complete(
    shared: &ShardShared,
    file: File,
    file_id: &str,
    part_path: &Path,
    final_path: &Path,
    cancel: &Cancel,
) -> Result<Downloaded, Failure> {
    // 数据已齐但登出了：保留 part + 逐片索引，绝不 rename
    if cancel.is_abandoned() {
        drop(file);
        shared.save_index();
        return Err(Failure::Other("下载已取消".to_owned()));
    }
    if let Err(err) = file.sync_all() {
        drop(file);
        cleanup(
            file_id,
            &(part_path.to_path_buf(), final_path.to_path_buf()),
        );
        return Err(local_write_failure(err));
    }
    drop(file);
    // sync 可能耗时几秒；与登出作废互斥后再决定是否 rename
    let commit = commit_lock();
    if cancel.is_abandoned() {
        drop(commit);
        shared.save_index();
        return Err(Failure::Other("下载已取消".to_owned()));
    }
    let renamed = finalize_part(part_path, final_path);
    drop(commit);
    match renamed {
        Ok(path) => {
            partials::forget(file_id);
            Ok(Downloaded {
                path,
                bytes: shared.total,
            })
        }
        Err(err) => {
            shared.save_index();
            Err(err)
        }
    }
}

#[cfg(test)]
#[path = "shards_tests.rs"]
mod tests;
