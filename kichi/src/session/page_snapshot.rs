//! 页面快照：重启后先把上次成功结果放回界面，随后静默校准。
//!
//! 快照只是展示缓存，和目录树索引共用一个账号库；解析失败、索引未就绪或
//! 已登出都当作「没有快照」，页面照常走网络，不能因为缓存问题挡住真实数据。

use serde::{de::DeserializeOwned, Serialize};

use super::Session;

pub(crate) const LIBRARY_RECENT_SAVED: &str = "library-recent-saved";
pub(crate) const LIBRARY_PLAYBACK_HISTORY: &str = "library-playback-history";
pub(crate) const SHARES: &str = "shares";
pub(crate) const OFFLINE_TASKS: &str = "offline-tasks";

/// 页面快照键；不需要持久化的集合（已加星标走索引）返回 `None`。
pub(crate) fn library_key(kind: kichi_core::LibraryKind) -> Option<&'static str> {
    match kind {
        kichi_core::LibraryKind::RecentSaved => Some(LIBRARY_RECENT_SAVED),
        kichi_core::LibraryKind::PlaybackHistory => Some(LIBRARY_PLAYBACK_HISTORY),
        kichi_core::LibraryKind::Starred => None,
    }
}

/// 读取快照；任何失败都返回 `None`，让调用方显示加载态并联网。
pub(crate) fn load<T: DeserializeOwned>(session: &Session, key: &str) -> Option<T> {
    let index = session.tree_index()?;
    if index.is_closed() {
        return None;
    }
    let payload = index.page_snapshot(key).ok().flatten()?;
    serde_json::from_str(&payload).ok()
}

/// 写入快照；写失败只记索引错误，不影响本次已经成功的网络结果。
pub(crate) fn save<T: Serialize>(session: &Session, key: &str, value: &T) {
    let Ok(payload) = serde_json::to_string(value) else {
        return;
    };
    session.write_index(|index| index.put_page_snapshot(key, &payload));
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use kichi_core::{EntryKind, FileEntry, LibraryEntry};

    fn sample() -> LibraryEntry {
        LibraryEntry {
            entry: FileEntry {
                id: "f1".to_owned(),
                name: "x.mkv".to_owned(),
                kind: EntryKind::File,
                size: 7,
                created_at: None,
                modified_at: Utc.with_ymd_and_hms(2026, 11, 1, 8, 0, 0).single(),
                parent_id: String::new(),
                md5: None,
                audit_flagged: false,
                starred: true,
                media: true,
                video: true,
                thumbnail_url: "https://signed.example/x?token=secret".to_owned(),
            },
            event_id: Some("e1".to_owned()),
            occurred_at: Utc.with_ymd_and_hms(2026, 11, 1, 9, 0, 0).single(),
            progress: None,
            folder_path: None,
        }
    }

    #[test]
    fn snapshot_round_trip_keeps_entries_but_drops_signed_thumbnail_url() {
        let payload = serde_json::to_string(&vec![sample()]).expect("序列化");
        assert!(
            !payload.contains("signed.example"),
            "签名缩略图地址不能落盘"
        );
        let restored: Vec<LibraryEntry> = serde_json::from_str(&payload).expect("反序列化");
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].entry.id, "f1");
        assert!(restored[0].entry.starred);
        assert!(restored[0].entry.thumbnail_url.is_empty());
        assert_eq!(restored[0].event_id.as_deref(), Some("e1"));
    }
}
