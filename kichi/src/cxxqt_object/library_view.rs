//! 资料库领域条目 → QML 表格模型。

use cxx_qt_lib::{QMap, QMapPair_QString_QVariant, QString, QVariant};
use kichi_core::{LibraryEntry, LibraryKind, PlaybackProgress};

use crate::cxxqt_object::files;
use crate::cxxqt_object::QVariantList;

type QVariantMap = QMap<QMapPair_QString_QVariant>;

pub(super) fn list_from(entries: &[LibraryEntry], kind: LibraryKind) -> QVariantList {
    let mut rows = QVariantList::default();
    let today = chrono::Local::now().date_naive();
    for entry in entries {
        rows.append(QVariant::from(&entry_map(entry, kind, today)));
    }
    rows
}

fn entry_map(entry: &LibraryEntry, kind: LibraryKind, today: chrono::NaiveDate) -> QVariantMap {
    let mut map = files::entry_map(&entry.entry);
    let event_time = entry
        .occurred_at
        .or(entry.entry.created_at)
        .or(entry.entry.modified_at);
    let event_text = event_time
        .map(format_time)
        .unwrap_or_else(|| "—".to_owned());
    let progress_text = entry
        .progress
        .map(format_progress)
        .unwrap_or_else(|| "—".to_owned());

    let primary_text = match kind {
        LibraryKind::RecentSaved | LibraryKind::Starred => size_text(entry),
        LibraryKind::PlaybackHistory => progress_text.clone(),
    };

    map.insert_clone(
        &QString::from("eventId"),
        &QVariant::from(&QString::from(
            entry.event_id.as_deref().unwrap_or_default(),
        )),
    );
    map.insert_clone(
        &QString::from("eventText"),
        &QVariant::from(&QString::from(
            event_time.map(format_time).unwrap_or_default(),
        )),
    );
    map.insert_clone(
        &QString::from("progressText"),
        &QVariant::from(&QString::from(
            entry
                .progress
                .map(format_progress)
                .unwrap_or_else(|| "—".to_owned()),
        )),
    );
    map.insert_clone(
        &QString::from("primaryText"),
        &QVariant::from(&QString::from(primary_text)),
    );
    map.insert_clone(
        &QString::from("secondaryText"),
        &QVariant::from(&QString::from(event_text)),
    );
    map.insert_clone(
        &QString::from("dateGroup"),
        &QVariant::from(&QString::from(if kind == LibraryKind::PlaybackHistory {
            playback_date_group(entry.occurred_at, today)
        } else {
            ""
        })),
    );
    map
}

fn playback_date_group(
    occurred_at: Option<chrono::DateTime<chrono::Utc>>,
    today: chrono::NaiveDate,
) -> &'static str {
    let Some(event_date) = occurred_at.map(|time| time.with_timezone(&chrono::Local).date_naive())
    else {
        return "earlier";
    };
    date_group_for_date(event_date, today)
}

fn date_group_for_date(event_date: chrono::NaiveDate, today: chrono::NaiveDate) -> &'static str {
    if event_date == today {
        "today"
    } else if today.pred_opt() == Some(event_date) {
        "yesterday"
    } else {
        "earlier"
    }
}

fn size_text(entry: &LibraryEntry) -> String {
    if entry.entry.kind == kichi_core::EntryKind::Folder {
        "—".to_owned()
    } else {
        crate::cxxqt_object::actions::format_size(entry.entry.size)
    }
}

fn format_progress(progress: PlaybackProgress) -> String {
    if progress.duration == 0 {
        return "—".to_owned();
    }
    let percent = ((progress.seconds.min(progress.duration) as f64 / progress.duration as f64)
        * 100.0)
        .round()
        .clamp(0.0, 100.0);
    format!("{percent:.0}%")
}

fn format_time(time: chrono::DateTime<chrono::Utc>) -> String {
    time.with_timezone(&chrono::Local)
        .format("%Y-%m-%d %H:%M")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, TimeZone, Utc};
    use kichi_core::{EntryKind, FileEntry};

    fn entry() -> LibraryEntry {
        LibraryEntry {
            entry: FileEntry {
                id: "file".to_owned(),
                name: "movie.mkv".to_owned(),
                kind: EntryKind::File,
                size: 2048,
                created_at: None,
                modified_at: None,
                parent_id: String::new(),
                md5: None,
                audit_flagged: false,
                starred: true,
                media: true,
                video: true,
                thumbnail_url: String::new(),
            },
            event_id: Some("event".to_owned()),
            occurred_at: Utc.with_ymd_and_hms(2026, 10, 31, 8, 0, 0).single(),
            progress: Some(PlaybackProgress {
                seconds: 45,
                duration: 100,
            }),
        }
    }

    fn text(row: &QVariantMap, key: &str) -> String {
        row.get(&QString::from(key))
            .and_then(|value| value.value::<QString>())
            .map(|value| value.to_string())
            .unwrap_or_default()
    }

    #[test]
    fn playback_uses_progress_and_event_time() {
        let list = list_from(&[entry()], LibraryKind::PlaybackHistory);
        let row = list.at(0).and_then(QVariant::value::<QVariantMap>).unwrap();
        assert_eq!(text(&row, "primaryText"), "45%");
        assert!(text(&row, "secondaryText").starts_with("2026-10-31"));
        assert_eq!(text(&row, "eventId"), "event");
        assert!(["today", "yesterday", "earlier"].contains(&text(&row, "dateGroup").as_str()));
    }

    #[test]
    fn starred_uses_size_and_modified_or_event_time() {
        let list = list_from(&[entry()], LibraryKind::Starred);
        let row = list.at(0).and_then(QVariant::value::<QVariantMap>).unwrap();
        assert_eq!(text(&row, "primaryText"), "2 KB");
        assert!(row.get(&QString::from("starred")).is_some());
        assert!(text(&row, "dateGroup").is_empty());
    }

    #[test]
    fn playback_date_groups_split_local_natural_days() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 31).unwrap();
        assert_eq!(
            date_group_for_date(NaiveDate::from_ymd_opt(2026, 10, 31).unwrap(), today),
            "today"
        );
        assert_eq!(
            date_group_for_date(NaiveDate::from_ymd_opt(2026, 10, 30).unwrap(), today),
            "yesterday"
        );
        assert_eq!(
            date_group_for_date(NaiveDate::from_ymd_opt(2026, 10, 29).unwrap(), today),
            "earlier"
        );
    }

    #[test]
    fn playback_history_without_event_time_is_earlier() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 31).unwrap();
        assert_eq!(playback_date_group(None, today), "earlier");
    }
}
