//! 筛选条件与扩展名分类的单元测试。

use super::*;
use chrono::{TimeZone, Utc};

fn entry(name: &str, kind: EntryKind, size: u64, modified_secs: i64) -> FileEntry {
    FileEntry {
        id: name.to_owned(),
        name: name.to_owned(),
        kind,
        size,
        created_at: None,
        modified_at: Some(
            Utc.timestamp_opt(modified_secs, 0)
                .single()
                .expect("合法时间"),
        ),
        parent_id: String::new(),
        md5: None,
        audit_flagged: false,
        starred: false,
        media: false,
        video: false,
        thumbnail_url: String::new(),
    }
}

fn file(name: &str, size: u64, modified_secs: i64) -> FileEntry {
    entry(name, EntryKind::File, size, modified_secs)
}

const NOW: i64 = 1_700_000_000;

fn of_type(kind: TypeFilter) -> SearchFilter {
    SearchFilter {
        types: vec![kind],
        ..Default::default()
    }
}

fn of_modified(window: ModifiedFilter) -> SearchFilter {
    SearchFilter {
        modified: vec![window],
        ..Default::default()
    }
}

fn of_size(range: SizeFilter) -> SearchFilter {
    SearchFilter {
        sizes: vec![range],
        ..Default::default()
    }
}

#[test]
fn extension_classification_covers_all_categories() {
    assert_eq!(classify_name("a.MKV"), FileCategory::Video);
    assert_eq!(classify_name("a.flac"), FileCategory::Audio);
    assert_eq!(classify_name("a.JPEG"), FileCategory::Image);
    assert_eq!(classify_name("a.pdf"), FileCategory::Document);
    assert_eq!(classify_name("a.tar.gz"), FileCategory::Archive);
    assert_eq!(classify_name("a.bin"), FileCategory::Other);
    assert_eq!(
        classify_name("README"),
        FileCategory::Other,
        "没有扩展名归其他"
    );
}

#[test]
fn type_filter_isolates_each_category() {
    let video = file("a.mkv", 1, NOW);
    let audio = file("b.mp3", 1, NOW);
    let image = file("c.png", 1, NOW);
    let document = file("d.pdf", 1, NOW);
    let archive = file("e.zip", 1, NOW);
    let other = file("f.bin", 1, NOW);
    let folder = entry("g", EntryKind::Folder, 0, NOW);

    let pick = |filter: SearchFilter| {
        [&video, &audio, &image, &document, &archive, &other, &folder]
            .into_iter()
            .filter(|item| filter.matches(item, NOW))
            .map(|item| item.name.as_str())
            .collect::<Vec<_>>()
    };
    assert_eq!(pick(of_type(TypeFilter::Video)), vec!["a.mkv"]);
    assert_eq!(pick(of_type(TypeFilter::Audio)), vec!["b.mp3"]);
    assert_eq!(pick(of_type(TypeFilter::Image)), vec!["c.png"]);
    assert_eq!(pick(of_type(TypeFilter::Document)), vec!["d.pdf"]);
    assert_eq!(pick(of_type(TypeFilter::Archive)), vec!["e.zip"]);
    assert_eq!(pick(of_type(TypeFilter::Other)), vec!["f.bin"]);
    assert_eq!(
        pick(of_type(TypeFilter::File)),
        vec!["a.mkv", "b.mp3", "c.png", "d.pdf", "e.zip", "f.bin"]
    );
    assert_eq!(pick(of_type(TypeFilter::Folder)), vec!["g"]);
    assert_eq!(pick(SearchFilter::default()).len(), 7);
}

#[test]
fn type_filter_multi_select_takes_the_union() {
    let video = file("a.mkv", 1, NOW);
    let audio = file("b.mp3", 1, NOW);
    let document = file("d.pdf", 1, NOW);
    let folder = entry("g", EntryKind::Folder, 0, NOW);

    let video_and_audio = SearchFilter {
        types: vec![TypeFilter::Video, TypeFilter::Audio],
        ..Default::default()
    };
    let names = [&video, &audio, &document, &folder]
        .into_iter()
        .filter(|item| video_and_audio.matches(item, NOW))
        .map(|item| item.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["a.mkv", "b.mp3"]);

    let folder_and_video = SearchFilter {
        types: vec![TypeFilter::Folder, TypeFilter::Video],
        ..Default::default()
    };
    let names = [&video, &audio, &document, &folder]
        .into_iter()
        .filter(|item| folder_and_video.matches(item, NOW))
        .map(|item| item.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        vec!["a.mkv", "g"],
        "文件夹 + 视频 = 两类都显示（并集）"
    );
}

#[test]
fn modified_filter_uses_modified_at_and_rejects_missing_time() {
    let recent = file("recent.mkv", 1, NOW - 3 * DAY_SECS);
    let month = file("month.mkv", 1, NOW - 20 * DAY_SECS);
    let year = file("year.mkv", 1, NOW - 200 * DAY_SECS);
    let mut missing = file("missing.mkv", 1, NOW);
    missing.modified_at = None;

    let keep = |filter: SearchFilter| {
        [&recent, &month, &year, &missing]
            .into_iter()
            .filter(|item| filter.matches(item, NOW))
            .map(|item| item.name.as_str())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        keep(of_modified(ModifiedFilter::Last7Days)),
        vec!["recent.mkv"]
    );
    assert_eq!(
        keep(of_modified(ModifiedFilter::Last30Days)),
        vec!["recent.mkv", "month.mkv"]
    );
    assert_eq!(
        keep(of_modified(ModifiedFilter::LastYear)),
        vec!["recent.mkv", "month.mkv", "year.mkv"],
        "时间缺失的条目在筛选启用时不命中"
    );
    assert_eq!(keep(SearchFilter::default()).len(), 4);

    // 多选时间窗口 = 并集，等价于其中最长的那一档
    let week_and_month = SearchFilter {
        modified: vec![ModifiedFilter::Last7Days, ModifiedFilter::Last30Days],
        ..Default::default()
    };
    assert_eq!(
        keep(week_and_month),
        vec!["recent.mkv", "month.mkv"],
        "7 天 + 30 天的并集就是 30 天"
    );
}

#[test]
fn size_filter_boundaries_are_half_open_and_skip_folders() {
    let small = file("small.bin", 10 * MIB - 1, NOW);
    let boundary = file("boundary.bin", 10 * MIB, NOW);
    let mid = file("mid.bin", 100 * MIB - 1, NOW);
    let big = file("big.bin", GIB, NOW);
    let folder = entry("dir", EntryKind::Folder, 0, NOW);

    let keep = |filter: SearchFilter| {
        [&small, &boundary, &mid, &big, &folder]
            .into_iter()
            .filter(|item| filter.matches(item, NOW))
            .map(|item| item.name.as_str())
            .collect::<Vec<_>>()
    };
    assert_eq!(keep(of_size(SizeFilter::Lt10MiB)), vec!["small.bin"]);
    assert_eq!(
        keep(of_size(SizeFilter::MiB10To100)),
        vec!["boundary.bin", "mid.bin"]
    );
    assert!(keep(of_size(SizeFilter::MiB100ToGiB)).is_empty());
    assert_eq!(keep(of_size(SizeFilter::GiBPlus)), vec!["big.bin"]);
    assert!(
        !keep(of_size(SizeFilter::Lt10MiB)).contains(&"dir"),
        "文件夹没有大小，按大小筛时不展示"
    );

    // 多选大小 = 区间并集
    let tiny_or_huge = SearchFilter {
        sizes: vec![SizeFilter::Lt10MiB, SizeFilter::GiBPlus],
        ..Default::default()
    };
    assert_eq!(keep(tiny_or_huge), vec!["small.bin", "big.bin"]);
}

#[test]
fn filters_combine_and_active_count_matches_groups() {
    let hit = file("movie.mkv", 50 * MIB, NOW - 2 * DAY_SECS);
    let filter = SearchFilter {
        types: vec![TypeFilter::Video],
        modified: vec![ModifiedFilter::Last7Days],
        sizes: vec![SizeFilter::MiB10To100],
    };
    assert!(filter.matches(&hit, NOW));
    assert_eq!(filter.active_count(), 3);
    assert_eq!(filter.summary(), "视频 · 最近 7 天 · 10–100 MB");

    let wrong_type = file("movie.txt", 50 * MIB, NOW - 2 * DAY_SECS);
    assert!(!filter.matches(&wrong_type, NOW), "任一条件不满足即不命中");

    let wrong_time = file("movie.mkv", 50 * MIB, NOW - 40 * DAY_SECS);
    assert!(!filter.matches(&wrong_time, NOW));

    let wrong_size = file("movie.mkv", 200 * MIB, NOW - 2 * DAY_SECS);
    assert!(!filter.matches(&wrong_size, NOW));
}

#[test]
fn default_filter_is_inactive_and_accepts_everything() {
    let filter = SearchFilter::default();
    assert!(!filter.is_active());
    assert_eq!(filter.active_count(), 0);
    assert_eq!(filter.summary(), "");
    assert!(filter.matches(&file("a.bin", 0, NOW), NOW));
    assert!(filter.matches(&entry("d", EntryKind::Folder, 0, NOW), NOW));
}

#[test]
fn wire_names_round_trip_and_unknown_falls_back() {
    for (value, parsed) in [
        ("all", TypeFilter::All),
        ("folder", TypeFilter::Folder),
        ("file", TypeFilter::File),
        ("video", TypeFilter::Video),
        ("audio", TypeFilter::Audio),
        ("image", TypeFilter::Image),
        ("document", TypeFilter::Document),
        ("archive", TypeFilter::Archive),
        ("other", TypeFilter::Other),
    ] {
        assert_eq!(TypeFilter::parse(value), parsed);
        assert_eq!(parsed.as_str(), value);
    }
    assert_eq!(TypeFilter::parse("bogus"), TypeFilter::All);

    for (value, parsed) in [
        ("any", ModifiedFilter::Any),
        ("7d", ModifiedFilter::Last7Days),
        ("30d", ModifiedFilter::Last30Days),
        ("1y", ModifiedFilter::LastYear),
    ] {
        assert_eq!(ModifiedFilter::parse(value), parsed);
        assert_eq!(parsed.as_str(), value);
    }
    assert_eq!(ModifiedFilter::parse("bogus"), ModifiedFilter::Any);

    for (value, parsed) in [
        ("any", SizeFilter::Any),
        ("lt10m", SizeFilter::Lt10MiB),
        ("10m-100m", SizeFilter::MiB10To100),
        ("100m-1g", SizeFilter::MiB100ToGiB),
        ("gt1g", SizeFilter::GiBPlus),
    ] {
        assert_eq!(SizeFilter::parse(value), parsed);
        assert_eq!(parsed.as_str(), value);
    }
    assert_eq!(SizeFilter::parse("bogus"), SizeFilter::Any);
}

#[test]
fn from_wire_keeps_order_dedupes_and_ignores_placeholders() {
    let strings = |values: &[&str]| values.iter().map(|v| v.to_string()).collect::<Vec<_>>();
    let filter = SearchFilter::from_wire(
        &strings(&["video", "all", "audio", "video"]),
        &strings(&["7d", "30d", "any", "7d"]),
        &strings(&["gt1g", "any", "bogus"]),
    );
    assert_eq!(
        filter.types,
        vec![TypeFilter::Video, TypeFilter::Audio],
        "all 是「不限制」的写法，不存进条件；重复项只留一个"
    );
    assert_eq!(
        filter.modified,
        vec![ModifiedFilter::Last7Days, ModifiedFilter::Last30Days]
    );
    assert_eq!(filter.sizes, vec![SizeFilter::GiBPlus]);
    assert_eq!(filter.active_count(), 3);
    assert_eq!(
        filter.summary(),
        "视频、音频 · 最近 7 天、最近 30 天 · >1 GB"
    );
}

#[test]
fn pushdown_values_match_the_rust_side_semantics() {
    let filter = SearchFilter {
        types: vec![TypeFilter::Video],
        modified: vec![ModifiedFilter::Last30Days],
        sizes: vec![SizeFilter::MiB10To100],
    };
    assert_eq!(filter.pushdown_kind(), Some(0));
    assert_eq!(
        filter.pushdown_modified_after(NOW),
        Some(NOW - 30 * DAY_SECS)
    );
    assert_eq!(
        filter.pushdown_size_ranges(),
        Some(vec![(Some(10 * MIB), Some(100 * MIB))])
    );

    // 多选大小：SQL 里是两段区间 OR
    let mixed = SearchFilter {
        sizes: vec![SizeFilter::Lt10MiB, SizeFilter::GiBPlus],
        ..Default::default()
    };
    assert_eq!(
        mixed.pushdown_size_ranges(),
        Some(vec![(None, Some(10 * MIB)), (Some(GIB), None)])
    );

    // 只按大小筛：文件夹没有大小，SQL 直接限定成文件
    let size_only = of_size(SizeFilter::Lt10MiB);
    assert_eq!(size_only.pushdown_kind(), Some(0));

    // 只勾文件夹：SQL 直接限定 kind = 1
    let folder_only = of_type(TypeFilter::Folder);
    assert_eq!(folder_only.pushdown_kind(), Some(1));

    // 文件夹 + 文件类：并集包含两种，不能下推 kind
    let folder_and_video = SearchFilter {
        types: vec![TypeFilter::Folder, TypeFilter::Video],
        ..Default::default()
    };
    assert_eq!(folder_and_video.pushdown_kind(), None);

    // 两个文件类：仍是文件
    let video_and_audio = SearchFilter {
        types: vec![TypeFilter::Video, TypeFilter::Audio],
        ..Default::default()
    };
    assert_eq!(video_and_audio.pushdown_kind(), Some(0));

    let all = SearchFilter::default();
    assert_eq!(all.pushdown_kind(), None);
    assert_eq!(all.pushdown_modified_after(NOW), None);
    assert_eq!(all.pushdown_size_ranges(), None);
}
