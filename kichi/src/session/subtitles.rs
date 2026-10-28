//! 自动字幕：选出视频同目录的字幕并下载到播放器临时目录。全部阻塞。
//!
//! 字幕是网盘里的普通文件（官方 API 不下发字幕 URL）：列表里找到候选，
//! `file_detail` 取签名直链，`open_download` 读进内存，再交给
//! [`crate::player::subtitles::SubtitleAssets`] 落盘。
//!
//! 这是播放的辅助功能，**不能拖垮播放**：单条失败只记原因继续；只有
//! `Auth` / `Captcha` / `RateLimited` 向上冒泡。
//!
//! 已下完的轨放在共享槽里（[`SubtitleSlot`]）：2 秒预等到点时，播放器先带着
//! 已完成的轨开播，字幕线程继续下，播放器监控线程在播放中把后续轨补挂上去
//! （[`LateSubtitles`]）。2026-10-28 实机踩过两次：先是超时整批丢弃，接着是
//! 某集两条都没在 2 秒内下完 → mpv 里一条字幕都没有。

use std::collections::BTreeMap;
use std::io::Read;
use std::sync::{mpsc, Mutex};
use std::time::{Duration, Instant};

use kichi_core::subtitles::{is_video_name, matching_subtitles, subtitle_extension};
use kichi_core::FileEntry;

use crate::player::subtitles::{LateSubtitles, SubtitleAssets, SubtitleSlot};

use super::download::Cancel;
use super::{Failure, Session};

/// 单条字幕的大小上限：正常字幕几百 KB，超过 8 MiB 基本是选错了文件。
pub const MAX_SUBTITLE_BYTES: u64 = 8 * 1024 * 1024;
/// 一次最多挂多少条轨（多语言 + 多版本；实际目录极少触发）。
pub const MAX_TRACKS: usize = 32;
/// 预等预算：点播放后最多等这么久字幕，到点先开播（之后还能补挂）。
pub const SUBTITLE_BUDGET: Duration = Duration::from_secs(2);
/// 字幕线程的总下载上限（含预等）：到点还没下完的不再挂，避免长时间占着网络。
pub const SUBTITLE_DOWNLOAD_CAP: Duration = Duration::from_secs(15);
/// 同时下载几条字幕：每条要「详情 + 直链 + 下载」三次往返，串行时两条就吃满预算。
const DOWNLOAD_WORKERS: usize = 3;

/// 点播放时对字幕要做的事。
#[derive(Debug, Default, Clone)]
pub enum SubtitlePlan {
    /// 开关关闭 / 非 mpv / 非视频 / 没有候选。
    #[default]
    None,
    /// 当前目录播放：候选已经匹配好，不额外发请求。
    Snapshot {
        video_name: String,
        matches: Vec<FileEntry>,
    },
    /// 搜索结果播放：需要后台列一次视频真实所在目录。
    /// `folder_id` 为空串表示根目录。
    Folder {
        video_name: String,
        folder_id: String,
    },
}

impl SubtitlePlan {
    /// 日志用的一句话（不打印云端文件名，排障看数量就够）。
    pub fn summary(&self) -> String {
        match self {
            SubtitlePlan::None => "无".to_owned(),
            SubtitlePlan::Snapshot { matches, .. } => {
                format!("当前目录 {} 条候选", matches.len())
            }
            SubtitlePlan::Folder { .. } => "按视频所在目录查找".to_owned(),
        }
    }
}

/// 一次字幕准备的结果：预等期间拿到的资产 + 提示条说明 + 播放中补挂的句柄。
#[derive(Debug, Default)]
pub struct SubtitlePrep {
    pub assets: Option<SubtitleAssets>,
    pub note: Option<String>,
    /// 预等结束时字幕还没下完：播放器监控线程靠它继续收。
    pub late: Option<LateSubtitles>,
}

/// 往槽里写一条字幕；第一条才真正建临时目录。
fn push_track(
    slot: &SubtitleSlot,
    extension: &str,
    bytes: &[u8],
    title: &str,
) -> Result<(), String> {
    let mut guard = slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if guard.is_none() {
        *guard = Some(SubtitleAssets::create()?);
    }
    match guard.as_mut() {
        Some(assets) => assets.push(extension, bytes, title),
        None => Err("创建字幕临时目录失败".to_owned()),
    }
}

/// 当前目录播放：从已知条目里匹配。
pub fn plan_from_snapshot(video_name: &str, entries: &[FileEntry]) -> SubtitlePlan {
    if !is_video_name(video_name) {
        return SubtitlePlan::None;
    }
    let matches: Vec<FileEntry> = matching_subtitles(video_name, entries)
        .into_iter()
        .cloned()
        .collect();
    if matches.is_empty() {
        SubtitlePlan::None
    } else {
        SubtitlePlan::Snapshot {
            video_name: video_name.to_owned(),
            matches,
        }
    }
}

/// 下载计划里的字幕。阻塞；由 `run_work` 放在独立的 `kichi-subs` 线程里跑。
///
/// 已下完的轨直接写进 `slot`（调用方随时可取）；返回值只是给提示条的一句
/// 说明，`None` = 不用打扰用户。
pub fn prepare(
    session: &Session,
    plan: &SubtitlePlan,
    user_cancel: &Cancel,
    work_cancel: &Cancel,
    slot: &SubtitleSlot,
) -> Result<Option<String>, Failure> {
    let started = Instant::now();
    let deadline = started + SUBTITLE_DOWNLOAD_CAP;

    let mut candidates = match plan {
        SubtitlePlan::None => return Ok(None),
        SubtitlePlan::Snapshot { matches, .. } => matches.clone(),
        SubtitlePlan::Folder {
            video_name,
            folder_id,
        } => {
            let parent = (!folder_id.is_empty()).then_some(folder_id.as_str());
            eprintln!("[kichi] 字幕：列视频所在目录（folder_id 为空表示根目录）");
            let entries = match session.list_dir_with_cancel(parent, Some(user_cancel)) {
                Ok(entries) => entries,
                Err(_) if user_cancel.is_cancelled() || work_cancel.is_cancelled() => {
                    return Ok(None);
                }
                Err(failure) if is_global(&failure) => {
                    eprintln!("[kichi] 字幕：列目录被拒（全局）：{failure}");
                    return Err(failure);
                }
                Err(failure) => {
                    eprintln!("[kichi] 字幕：列视频所在目录失败：{failure}");
                    return Ok(Some("未能读取视频所在文件夹，已跳过字幕".to_owned()));
                }
            };
            matching_subtitles(video_name, &entries)
                .into_iter()
                .cloned()
                .collect()
        }
    };

    if candidates.is_empty() {
        eprintln!("[kichi] 字幕：没有候选");
        return Ok(None);
    }
    eprintln!("[kichi] 字幕：{} 条候选，开始下载", candidates.len());
    let truncated = if candidates.len() > MAX_TRACKS {
        candidates.truncate(MAX_TRACKS);
        Some(format!("字幕过多，只加载了前 {MAX_TRACKS} 条"))
    } else {
        None
    };

    let mut loaded = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;
    let mut first_reason: Option<String> = None;

    // 最多 3 条字幕并发下载；结果按**候选顺序**写槽（首选中文必须仍是第一条，
    // `attach` 只 `select` 第一条）。某条先下完时，它前面的候选没到齐就先压着，
    // 保证轨道顺序稳定；超时的话没写进槽的那些直接放弃。
    let (sender, receiver) = mpsc::channel::<(usize, FetchOutcome)>();
    let cursor = Mutex::new(0usize);
    std::thread::scope(|scope| {
        for _ in 0..DOWNLOAD_WORKERS.min(candidates.len()) {
            let sender = sender.clone();
            let cursor = &cursor;
            let candidates = &candidates;
            scope.spawn(move || loop {
                let index = {
                    let mut cursor = cursor
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    let index = *cursor;
                    *cursor += 1;
                    index
                };
                let Some(entry) = candidates.get(index) else {
                    break;
                };
                if user_cancel.is_cancelled() || work_cancel.is_cancelled() {
                    break;
                }
                let outcome = fetch_one(session, entry, user_cancel, work_cancel);
                if sender.send((index, outcome)).is_err() {
                    break;
                }
            });
        }
        drop(sender);

        let mut pending: BTreeMap<usize, Vec<u8>> = BTreeMap::new();
        let mut next_push = 0usize;
        let mut received = 0usize;
        let mut cancelled = false;
        while received < candidates.len() {
            if user_cancel.is_cancelled() || work_cancel.is_cancelled() {
                cancelled = true;
                break;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                eprintln!("[kichi] 字幕：预算到点，已下完 {loaded} 条，其余放弃");
                break;
            }
            match receiver.recv_timeout(remaining) {
                Ok((index, outcome)) => {
                    received += 1;
                    match outcome {
                        FetchOutcome::Loaded(bytes) => {
                            pending.insert(index, bytes);
                        }
                        FetchOutcome::Skipped(reason) => {
                            skipped += 1;
                            eprintln!("[kichi] 字幕：「{}」跳过：{reason}", candidates[index].name);
                            first_reason.get_or_insert(reason);
                        }
                        FetchOutcome::Failed(reason) => {
                            failed += 1;
                            eprintln!(
                                "[kichi] 字幕：「{}」下载失败：{reason}",
                                candidates[index].name
                            );
                            first_reason.get_or_insert(reason);
                        }
                        FetchOutcome::Global(failure) => {
                            // 登录失效 / 人机验证 / 限流：播放也不能继续，原样冒泡
                            eprintln!("[kichi] 字幕：全局错误：{failure}");
                            work_cancel.cancel();
                            return Err(failure);
                        }
                        FetchOutcome::Cancelled => cancelled = true,
                    }
                    // 把「从 next_push 开始连续到齐」的那一段按顺序写进槽
                    while let Some(bytes) = pending.remove(&next_push) {
                        let entry = &candidates[next_push];
                        let extension = subtitle_extension(&entry.name).unwrap_or("srt");
                        next_push += 1;
                        if let Err(reason) = push_track(slot, extension, &bytes, &entry.name) {
                            failed += 1;
                            eprintln!("[kichi] 字幕：写「{}」失败：{reason}", entry.name);
                            first_reason.get_or_insert(reason);
                            continue;
                        }
                        loaded += 1;
                        eprintln!(
                            "[kichi] 字幕：已写入第 {loaded} 条（{} 字节，{extension}）",
                            bytes.len()
                        );
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    eprintln!("[kichi] 字幕：预算到点，已下完 {loaded} 条，其余放弃");
                    break;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        if cancelled {
            // 已写进槽的轨保留：超时 / 取消由调用方决定用不用
            eprintln!("[kichi] 字幕：已取消，保留已下载的 {loaded} 条");
            return Ok(None);
        }

        let untried = candidates.len().saturating_sub(loaded + skipped + failed);
        let note = finish_note(truncated, loaded, skipped, failed, untried, first_reason);
        eprintln!(
            "[kichi] 字幕：收尾 成功 {loaded} / 跳过 {skipped} / 失败 {failed} / 未尝试 {untried}，说明 {note:?}"
        );
        Ok(note)
    })
}

/// 单条字幕的下载结果（不含「写盘」这一步：写盘必须按候选顺序串行做）。
enum FetchOutcome {
    Loaded(Vec<u8>),
    /// 太大 / 空文件：不算失败，但也不会挂上。
    Skipped(String),
    Failed(String),
    /// 登录失效 / 人机验证 / 限流：整次播放都要按它收尾。
    Global(Failure),
    Cancelled,
}

/// 一条字幕的完整下载（详情 → 直链 → 读进内存）。在 `prepare` 的并行批里跑。
fn fetch_one(
    session: &Session,
    entry: &FileEntry,
    user_cancel: &Cancel,
    work_cancel: &Cancel,
) -> FetchOutcome {
    // 列表里的 0 不能当「空文件」（0 也可能是官方没给大小），交给详情判断。
    if entry.size > MAX_SUBTITLE_BYTES {
        return FetchOutcome::Skipped(format!("「{}」超过 8 MiB", entry.name));
    }
    // 下过的字幕直接读本地缓存：再播同一集不联网，2 秒预等基本都够用
    if !user_cancel.is_cancelled() && !work_cancel.is_cancelled() {
        if let Some(bytes) = super::subtitle_cache::load(&entry.id, &entry.name) {
            eprintln!(
                "[kichi] 字幕：「{}」命中缓存（{} 字节）",
                entry.name,
                bytes.len()
            );
            return FetchOutcome::Loaded(bytes);
        }
    }
    let client = match session.drive() {
        Ok(client) => client,
        Err(failure) if is_global(&failure) => return FetchOutcome::Global(failure),
        Err(failure) => return FetchOutcome::Failed(failure.to_string()),
    };
    let info = match client.file_detail(&entry.id) {
        Ok(info) => info,
        Err(err) => {
            let failure = Failure::from(err);
            return if is_global(&failure) {
                FetchOutcome::Global(failure)
            } else {
                FetchOutcome::Failed(failure.to_string())
            };
        }
    };
    // 详情说空就是真空；列表的 0 只代表「不知道」。
    if info.size == Some(0) {
        return FetchOutcome::Skipped("字幕文件是空的".to_owned());
    }
    if info.size.is_some_and(|size| size > MAX_SUBTITLE_BYTES) {
        return FetchOutcome::Skipped(format!("「{}」超过 8 MiB", entry.name));
    }
    let mut stream = match client.open_download(&info) {
        Ok(stream) => stream,
        Err(err) => {
            let failure = Failure::from(err);
            return if is_global(&failure) {
                FetchOutcome::Global(failure)
            } else {
                FetchOutcome::Failed(failure.to_string())
            };
        }
    };
    match read_capped(&mut stream, MAX_SUBTITLE_BYTES, user_cancel, work_cancel) {
        Ok(bytes) => {
            // 缓存失败只记一句，不能影响这一次播放
            if let Err(reason) = super::subtitle_cache::store(&entry.id, &entry.name, &bytes) {
                eprintln!("[kichi] 字幕：缓存「{}」失败：{reason}", entry.name);
            }
            FetchOutcome::Loaded(bytes)
        }
        Err(_) if user_cancel.is_cancelled() || work_cancel.is_cancelled() => {
            FetchOutcome::Cancelled
        }
        Err(reason) => FetchOutcome::Failed(reason),
    }
}

/// 收尾说明：不静默吞掉失败，但成功加载首选轨时尽量不打扰。
fn finish_note(
    truncated: Option<String>,
    loaded: usize,
    skipped: usize,
    failed: usize,
    untried: usize,
    first_reason: Option<String>,
) -> Option<String> {
    let unloaded = skipped + failed + untried;
    if loaded == 0 {
        let tail = if skipped + failed > 0 {
            Some(format!(
                "字幕加载失败：{}",
                first_reason.unwrap_or_else(|| "未知原因".to_owned())
            ))
        } else if untried > 0 {
            Some("字幕准备超时，已直接播放".to_owned())
        } else {
            None
        };
        return combine_notes(truncated, tail);
    }
    if unloaded > 0 {
        return combine_notes(
            truncated,
            Some(format!("已加载 {loaded} 条字幕，{unloaded} 条未加载")),
        );
    }
    truncated
}

/// 把两句说明拼成一行；空的那边直接省略。
pub(crate) fn combine_notes(first: Option<String>, second: Option<String>) -> Option<String> {
    match (first, second) {
        (Some(first), Some(second)) => Some(format!("{first}；{second}")),
        (Some(first), None) => Some(first),
        (None, second) => second,
    }
}

/// 往下读最多 `cap` 字节；每 64 KiB 查一次取消。
///
/// 刚好 `cap` 字节算成功；再多一个字节就报错（避免异常文件把内存打满）。
pub(crate) fn read_capped<R: Read>(
    reader: &mut R,
    cap: u64,
    user_cancel: &Cancel,
    work_cancel: &Cancel,
) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        if user_cancel.is_cancelled() || work_cancel.is_cancelled() {
            return Err("已取消".to_owned());
        }
        let read = reader
            .read(&mut buffer)
            .map_err(|err| format!("读字幕失败：{err}"))?;
        if read == 0 {
            return Ok(out);
        }
        if out.len() as u64 + read as u64 > cap {
            return Err(format!("字幕文件超过 {} MiB", cap / (1024 * 1024)));
        }
        out.extend_from_slice(&buffer[..read]);
    }
}

fn is_global(failure: &Failure) -> bool {
    matches!(
        failure,
        Failure::Auth(_) | Failure::Captcha { .. } | Failure::RateLimited { .. }
    )
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    fn file(name: &str, size: u64) -> FileEntry {
        FileEntry {
            id: name.to_owned(),
            name: name.to_owned(),
            kind: kichi_core::EntryKind::File,
            size,
            modified_at: None,
            md5: None,
            audit_flagged: false,
            media: false,
        }
    }

    #[test]
    fn plan_needs_a_video_with_candidates() {
        let entries = [file("Movie.mkv", 1), file("Movie.zh.srt", 10)];
        assert!(matches!(
            plan_from_snapshot("Movie.mkv", &entries),
            SubtitlePlan::Snapshot { .. }
        ));
        assert!(matches!(
            plan_from_snapshot("song.flac", &entries),
            SubtitlePlan::None
        ));
        assert!(matches!(
            plan_from_snapshot("Movie.mkv", &[file("Movie.mkv", 1)]),
            SubtitlePlan::None
        ));
    }

    #[test]
    fn read_capped_accepts_exactly_the_cap() {
        let mut reader = Cursor::new(vec![7u8; 100]);
        let ok = read_capped(&mut reader, 100, &Cancel::default(), &Cancel::default());
        assert_eq!(ok.map(|bytes| bytes.len()), Ok(100));
    }

    #[test]
    fn read_capped_rejects_one_byte_over() {
        let mut reader = Cursor::new(vec![7u8; 101]);
        let err = read_capped(&mut reader, 100, &Cancel::default(), &Cancel::default());
        assert!(err.is_err(), "{err:?}");
    }

    #[test]
    fn read_capped_stops_when_either_side_cancels() {
        let user = Cancel::default();
        user.cancel();
        let mut reader = Cursor::new(vec![7u8; 10]);
        assert!(read_capped(&mut reader, 100, &user, &Cancel::default()).is_err());

        let work = Cancel::default();
        work.cancel();
        let mut reader = Cursor::new(vec![7u8; 10]);
        assert!(read_capped(&mut reader, 100, &Cancel::default(), &work).is_err());
    }

    #[test]
    fn note_reports_failures_but_stays_quiet_on_success() {
        assert_eq!(finish_note(None, 2, 0, 0, 0, None), None);
        assert_eq!(
            finish_note(None, 1, 1, 0, 0, Some("坏了".into())),
            Some("已加载 1 条字幕，1 条未加载".to_owned())
        );
        assert_eq!(
            finish_note(None, 0, 1, 0, 0, Some("坏了".into())),
            Some("字幕加载失败：坏了".to_owned())
        );
        assert_eq!(
            finish_note(None, 0, 0, 0, 2, None),
            Some("字幕准备超时，已直接播放".to_owned())
        );
        assert_eq!(
            finish_note(Some("太多了".into()), 0, 0, 1, 0, Some("坏了".into())),
            Some("太多了；字幕加载失败：坏了".to_owned())
        );
    }

    #[test]
    fn combine_notes_keeps_both_sides() {
        assert_eq!(combine_notes(None, None), None);
        assert_eq!(
            combine_notes(Some("档位".into()), Some("字幕".into())),
            Some("档位；字幕".to_owned())
        );
    }
}
