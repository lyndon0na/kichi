//! 播放时自动字幕的编排：点播放时定计划、后台与取链并发下载、合并提示。
//!
//! 从 `play.rs` 拆出来守行数；`play.rs` 只负责界面状态，网络与计时都在这里。

use std::collections::HashMap;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use kichi_core::subtitles::is_video_name;
use kichi_core::{FileEntry, PlayOption};

use crate::player::subtitles::{take_slot, LateSubtitles, SubtitleSlot};
use crate::session::download::Cancel;
use crate::session::media;
use crate::session::subtitles::{
    plan_from_snapshot, prepare, SubtitlePlan, SubtitlePrep, SUBTITLE_BUDGET,
};
use crate::session::{Failure, SearchHit, Session};

pub(super) use crate::session::subtitles::combine_notes;

/// 点播放时决定这次要不要字幕、候选从哪来。
///
/// 当前目录的条目已经在内存里（0 请求）；搜索结果只记录了文件 id 与所在
/// 文件夹，需要后台再列一次那个目录。
pub(super) fn plan_for(
    id: &str,
    entries: &[FileEntry],
    search: &HashMap<String, SearchHit>,
) -> SubtitlePlan {
    if let Some(entry) = entries.iter().find(|entry| entry.id == id) {
        return plan_from_snapshot(&entry.name, entries);
    }
    let Some(hit) = search.get(id) else {
        return SubtitlePlan::None;
    };
    if !is_video_name(&hit.entry.name) {
        return SubtitlePlan::None;
    }
    SubtitlePlan::Folder {
        video_name: hit.entry.name.clone(),
        folder_id: hit.folder_id.clone(),
    }
}

/// 后台线程：取播放档位的同时准备字幕，字幕最多再等 [`SUBTITLE_BUDGET`]。
///
/// 预算从字幕线程启动算起（不是从取链返回算起），**绝不**动主播放的 `cancel`。
/// 2 秒内下完的轨直接交给播放器；没下完的让字幕线程继续（上限
/// [`crate::session::subtitles::SUBTITLE_DOWNLOAD_CAP`]），返回 [`LateSubtitles`]
/// 让播放器监控线程在播放中补挂。
pub(super) fn run_work(
    session: Arc<Session>,
    id: String,
    cancel: Arc<Cancel>,
    audit_flagged: bool,
    plan: SubtitlePlan,
) -> Result<(Vec<PlayOption>, SubtitlePrep), Failure> {
    let started = Instant::now();
    let work_cancel = Arc::new(Cancel::default());
    let wants_subtitles = !matches!(plan, SubtitlePlan::None);
    eprintln!(
        "[kichi] 字幕：计划 {}，{}",
        plan.summary(),
        if wants_subtitles {
            "开始准备"
        } else {
            "本次不准备"
        }
    );
    let (sender, receiver) = mpsc::channel::<Result<Option<String>, Failure>>();
    let slot: SubtitleSlot = Arc::new(Mutex::new(None));

    if wants_subtitles {
        let subs_session = session.clone();
        let subs_cancel = cancel.clone();
        let subs_work = work_cancel.clone();
        let subs_plan = plan.clone();
        let subs_slot = slot.clone();
        let spawned = std::thread::Builder::new()
            .name("kichi-subs".into())
            .spawn(move || {
                let result = prepare(
                    &subs_session,
                    &subs_plan,
                    &subs_cancel,
                    &subs_work,
                    &subs_slot,
                );
                let _ = sender.send(result);
            });
        if let Err(err) = spawned {
            // 线程起不来时 sender 已随闭包丢弃：下面会走 Disconnected 分支
            eprintln!("[kichi] 启动字幕线程失败：{err}");
        }
    } else {
        drop(sender);
    }

    let options = match media::play_options(&session, &id, Some(&cancel), audit_flagged) {
        Ok(options) => options,
        Err(err) => {
            eprintln!("[kichi] 字幕：取链失败，取消字幕准备");
            work_cancel.cancel();
            return Err(err);
        }
    };
    eprintln!(
        "[kichi] 字幕：取链用时 {} ms",
        started.elapsed().as_millis()
    );
    if !wants_subtitles {
        return Ok((options, SubtitlePrep::default()));
    }

    let remaining = SUBTITLE_BUDGET.saturating_sub(started.elapsed());
    match receiver.recv_timeout(remaining) {
        Ok(Ok(note)) => {
            let prep = SubtitlePrep {
                assets: take_slot(&slot),
                note,
                late: None,
            };
            eprintln!(
                "[kichi] 字幕：就绪 {} 条（等待 {} ms），说明 {:?}",
                track_count(prep.assets.as_ref()),
                started.elapsed().as_millis(),
                prep.note
            );
            Ok((options, prep))
        }
        Ok(Err(failure)) => {
            // prepare 只在登录失效 / 人机验证 / 限流时返回 Err
            eprintln!("[kichi] 字幕：准备失败（全局）：{failure}");
            work_cancel.cancel();
            Err(failure)
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            let prep = SubtitlePrep {
                assets: take_slot(&slot),
                note: None,
                // 不取消：字幕线程继续下，播放器就绪后由监控线程补挂
                late: Some(LateSubtitles::new(slot.clone(), work_cancel.clone())),
            };
            eprintln!(
                "[kichi] 字幕：2 秒内只下完 {} 条，先开播，其余下完再补挂",
                track_count(prep.assets.as_ref())
            );
            Ok((options, prep))
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            work_cancel.cancel();
            let assets = take_slot(&slot);
            let note = assets
                .is_none()
                .then(|| "字幕准备异常，已直接播放".to_owned());
            eprintln!(
                "[kichi] 字幕：准备线程异常退出，带已完成的 {} 条开播",
                track_count(assets.as_ref())
            );
            Ok((
                options,
                SubtitlePrep {
                    assets,
                    note,
                    late: None,
                },
            ))
        }
    }
}

fn track_count(assets: Option<&crate::player::subtitles::SubtitleAssets>) -> usize {
    assets.map_or(0, |assets| assets.tracks().len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use kichi_core::EntryKind;

    fn file(id: &str, name: &str) -> FileEntry {
        FileEntry {
            id: id.to_owned(),
            name: name.to_owned(),
            kind: EntryKind::File,
            size: 1,
            modified_at: None,
            md5: None,
            audit_flagged: false,
            media: false,
            video: false,
            thumbnail_url: String::new(),
        }
    }

    #[test]
    fn current_directory_plays_get_a_snapshot_plan() {
        let entries = [
            file("v", "Movie.mkv"),
            file("s1", "Movie.zh.srt"),
            file("s2", "Movie.en.srt"),
        ];
        match plan_for("v", &entries, &HashMap::new()) {
            SubtitlePlan::Snapshot { matches, .. } => assert_eq!(matches.len(), 2),
            other => panic!("应当有快照计划：{other:?}"),
        }
    }

    #[test]
    fn search_hits_point_at_their_real_folder() {
        let entry = file("v", "Movie.mkv");
        let hit = SearchHit {
            entry,
            folder_id: "folder-1".to_owned(),
            folder_path: None,
            order: 0,
        };
        let mut search = HashMap::new();
        search.insert("v".to_owned(), hit);
        match plan_for("v", &[], &search) {
            SubtitlePlan::Folder {
                video_name,
                folder_id,
            } => {
                assert_eq!(video_name, "Movie.mkv");
                assert_eq!(folder_id, "folder-1");
            }
            other => panic!("应当按目录计划：{other:?}"),
        }
    }

    #[test]
    fn audio_and_unknown_ids_get_no_plan() {
        let entries = [file("a", "song.flac")];
        assert!(matches!(
            plan_for("a", &entries, &HashMap::new()),
            SubtitlePlan::None
        ));
        assert!(matches!(
            plan_for("missing", &entries, &HashMap::new()),
            SubtitlePlan::None
        ));
    }

    #[test]
    fn notes_join_with_a_separator() {
        assert_eq!(
            combine_notes(Some("档位".into()), Some("字幕".into())),
            Some("档位；字幕".to_owned())
        );
    }
}
