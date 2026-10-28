//! 自动字幕的临时资产与 mpv 挂载。
//!
//! 字幕是网盘文件，mpv 的 `sub-add` 只读本地路径：先把下载好的字节写到
//! `/tmp/kichi-play/subs-<pid>-<毫秒>-<序号>/subN.<ext>`（目录 0700、文件 0600），
//! 播放器就绪后经 IPC 逐条 `sub-add`。临时目录由 [`SubtitleAssets`] 的 `Drop`
//! 清理；进程崩溃留下的目录由 [`sweep_stale`] 在下次启动时按 24 小时清理。
//!
//! 2 秒预算内没下完的字幕不会丢：资产生在共享槽 [`SubtitleSlot`] 里，播放器
//! 监控线程在播放期间轮询，拿到一条就 `sub-add` 一条（[`LateSubtitles`]）。

use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use kichi_core::subtitles::SUBTITLE_EXTENSIONS;

use crate::session::download::Cancel;

use super::{ensure_play_dir, ipc, play_dir};

/// 等 `sub-add` 回执的总预算：命令已经发出去，等不到也照常播放。
const REPLY_TIMEOUT: Duration = Duration::from_millis(500);

/// 一条已落盘的字幕轨。
#[derive(Debug)]
pub struct SubtitleTrack {
    pub path: PathBuf,
    /// 云端原文件名，只作为 mpv 的 track title（不进路径）。
    pub title: String,
}

/// 一次播放的字幕临时目录 + 已落盘的轨。
#[derive(Debug)]
pub struct SubtitleAssets {
    dir: PathBuf,
    tracks: Vec<SubtitleTrack>,
}

impl SubtitleAssets {
    /// 在播放临时目录下建一个只属于本次播放的子目录。
    pub fn create() -> Result<Self, String> {
        let root = ensure_play_dir()?;
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let millis = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis())
            .unwrap_or(0);
        let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = root.join(format!("subs-{}-{millis}-{sequence}", std::process::id()));
        #[cfg(unix)]
        let created = {
            use std::os::unix::fs::DirBuilderExt;
            std::fs::DirBuilder::new().mode(0o700).create(&dir)
        };
        #[cfg(not(unix))]
        let created = std::fs::create_dir(&dir);
        created.map_err(|err| format!("创建字幕临时目录失败：{err}"))?;
        Ok(Self {
            dir,
            tracks: Vec::new(),
        })
    }

    /// 写一条字幕；`extension` 必须是 mpv 认识的字幕后缀。
    pub fn push(&mut self, extension: &str, bytes: &[u8], title: &str) -> Result<(), String> {
        let extension = extension.to_ascii_lowercase();
        if !SUBTITLE_EXTENSIONS.contains(&extension.as_str()) {
            return Err(format!("不支持的字幕格式：{extension}"));
        }
        let index = self.tracks.len();
        let path = self.dir.join(format!("sub{index}.{extension}"));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&path)
            .map_err(|err| format!("写字幕临时文件失败：{err}"))?;
        if let Err(err) = file.write_all(bytes) {
            let _ = std::fs::remove_file(&path);
            return Err(format!("写字幕临时文件失败：{err}"));
        }
        drop(file);
        self.tracks.push(SubtitleTrack {
            path,
            title: title.to_owned(),
        });
        Ok(())
    }

    pub fn tracks(&self) -> &[SubtitleTrack] {
        &self.tracks
    }

    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }
}

impl Drop for SubtitleAssets {
    fn drop(&mut self) {
        match std::fs::remove_dir_all(&self.dir) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => eprintln!("[kichi] 清理字幕临时目录失败：{err}"),
        }
    }
}

/// 已写盘的字幕资产槽：字幕下载线程往里放，播放器监控线程取。
///
/// 预等预算到了也继续下载，播放中每拿到一条就挂一条；同一时刻槽里最多一份
/// 资产（一条轨一个临时目录），取走后线程再下一条会新建一份。
pub type SubtitleSlot = Arc<Mutex<Option<SubtitleAssets>>>;

/// 取走槽里已完成的轨（没有就 `None`）。
pub fn take_slot(slot: &SubtitleSlot) -> Option<SubtitleAssets> {
    slot.lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take()
}

/// 「还可以在播放中补挂」的句柄：槽 + 取消旗标。
///
/// 交给播放器监控线程；播放结束 / 启动失败 / 取消时 `Drop` 会置取消，
/// 让还在下载的字幕线程立刻收手（最多白下一个读切片的字节）。
pub struct LateSubtitles {
    pub slot: SubtitleSlot,
    cancel: Arc<Cancel>,
}

impl LateSubtitles {
    pub fn new(slot: SubtitleSlot, cancel: Arc<Cancel>) -> Self {
        Self { slot, cancel }
    }
}

impl Drop for LateSubtitles {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

impl std::fmt::Debug for LateSubtitles {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LateSubtitles")
    }
}

/// 播放器就绪后把字幕挂上去。返回一句失败原因（成功 / 没有字幕时 `None`）。
///
/// `selected` 记录这次播放里有没有已经 `select` 过的轨：预等阶段挂过就一直是
/// `true`，播放中补挂的新轨用 `auto`；一条都没挂过时，补挂的第一条也 `select`。
/// `next_request_id` 由调用方持有并递增，避免补挂时复用上一次的 `request_id`。
pub(crate) fn attach(
    conn: &mut ipc::Connection,
    assets: Option<&SubtitleAssets>,
    next_request_id: &mut u64,
    selected: &mut bool,
) -> Option<String> {
    let assets = assets?;
    if assets.is_empty() {
        return None;
    }
    let mut ids = Vec::with_capacity(assets.tracks().len());
    eprintln!("[kichi] 字幕：向 mpv 挂载 {} 条", assets.tracks().len());
    for track in assets.tracks() {
        let request_id = *next_request_id;
        *next_request_id += 1;
        let select = !*selected;
        if let Err(err) = conn.sub_add(request_id, &track.path, select, &track.title) {
            eprintln!("[kichi] 字幕：sub-add 写入失败：{err}");
            return Some(format!("字幕未能加载：{err}"));
        }
        if select {
            *selected = true;
        }
        ids.push(request_id);
    }
    let reply = conn.collect_replies(&ids, REPLY_TIMEOUT);
    match &reply {
        Ok(()) => eprintln!("[kichi] 字幕：mpv 已确认 {} 条", ids.len()),
        Err(err) => eprintln!("[kichi] 字幕：mpv 回执失败：{err}"),
    }
    reply.err().map(|err| format!("字幕未能加载：{err}"))
}

/// 清理上次崩溃留下的字幕临时目录（按 mtime，不按 pid）。
///
/// 不按 pid 判断的原因：同机可能同时跑多个 kichi 实例，pid 存活检查容易误删
/// 另一个实例正在用的目录；按时间删最多多留一天，`/tmp` 重启后本来也会清空。
pub fn sweep_stale(older_than: Duration) {
    let Ok(entries) = std::fs::read_dir(play_dir()) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.starts_with("subs-") {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_dir() {
            continue;
        }
        let Ok(modified) = metadata.modified() else {
            continue;
        };
        if !is_stale(now, modified, older_than) {
            continue;
        }
        if let Err(err) = std::fs::remove_dir_all(entry.path()) {
            eprintln!("[kichi] 清理旧字幕目录失败：{err}");
        }
    }
}

fn is_stale(now: SystemTime, modified: SystemTime, older_than: Duration) -> bool {
    now.duration_since(modified)
        .map(|age| age > older_than)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn assets_write_private_ascii_files_and_clean_up() {
        let mut assets = SubtitleAssets::create().expect("建字幕临时目录");
        assets
            .push("SRT", "hello".as_bytes(), "电影.zh.srt")
            .expect("写第一条");
        assets
            .push("ass", "world".as_bytes(), "电影.en.ass")
            .expect("写第二条");
        assert_eq!(assets.tracks().len(), 2);
        let first = &assets.tracks()[0].path;
        assert_eq!(first.file_name().and_then(|n| n.to_str()), Some("sub0.srt"));
        assert_eq!(assets.tracks()[0].title, "电影.zh.srt");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(first)
                .map(|meta| meta.permissions().mode() & 0o777)
                .unwrap_or(0);
            assert_eq!(mode, 0o600, "字幕临时文件不能让同机其他用户读");
        }
        let dir = assets.tracks()[0]
            .path
            .parent()
            .map(Path::to_path_buf)
            .expect("有父目录");
        assert!(dir.exists());
        drop(assets);
        assert!(!dir.exists(), "Drop 应删掉整条字幕临时目录");
    }

    #[test]
    fn assets_reject_unknown_extensions() {
        let mut assets = SubtitleAssets::create().expect("建字幕临时目录");
        assert!(assets.push("exe", b"x", "bad.exe").is_err());
        assert!(assets.is_empty());
    }

    #[test]
    fn stale_check_uses_the_24_hour_line() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(200_000);
        let day = Duration::from_secs(24 * 60 * 60);
        let old = SystemTime::UNIX_EPOCH + Duration::from_secs(200_000 - 86_401);
        let fresh = SystemTime::UNIX_EPOCH + Duration::from_secs(200_000 - 86_399);
        let future = SystemTime::UNIX_EPOCH + Duration::from_secs(200_001);
        assert!(is_stale(now, old, day));
        assert!(!is_stale(now, fresh, day));
        assert!(!is_stale(now, future, day), "时钟回拨不该误删");
    }
}
