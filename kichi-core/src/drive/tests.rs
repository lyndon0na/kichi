use super::account::{AboutResponse, TransferQuotaResponse};
use super::files::{list_query, recent_query, trash_query, FileDto, FileListResponse};
use super::media::{is_image, is_media, is_video};
use super::*;
use crate::types::{EntryKind, FileEntry, TrashEntry};

#[test]
fn action_uses_path_only() {
    assert_eq!(
        action_for("GET", "https://api-drive.mypikpak.com/drive/v1/about"),
        "GET:/drive/v1/about"
    );
    assert_eq!(
        action_for("GET", "https://user.mypikpak.com/v1/user/me?x=1"),
        "GET:/v1/user/me"
    );
    assert_eq!(action_for("GET", "/drive/v1/about"), "GET:/drive/v1/about");
}

#[test]
fn media_flag_survives_empty_official_category() {
    // 实测：不带 kind 过滤时官方分类字段为空，靠扩展名兜底
    assert!(is_media("Screencast_20261023.webm", "", "", ""));
    assert!(is_media("预告片.MP4", "", "", ""));
    assert!(is_media("song.flac", "", "", ""));
    assert!(is_media(".hidden.txt.mp3", "", "", ""));
    assert!(is_video("Screencast.webm", "", "", ""));
    assert!(!is_video("song.flac", "", "", ""));
    assert!(is_image("cover.JPG", "", "", ""));
    // 官方给了分类时优先采信（即使扩展名不认识）
    assert!(is_media("noext", "VIDEO", "", ""));
    assert!(is_media("file.bin", "", "video/mp4", ""));
    // 非媒体
    assert!(!is_media("settings.json", "", "", ""));
    assert!(!is_media("archive.zip", "", "", ""));
    assert!(!is_media("no-extension", "", "", ""));
    assert!(!is_media("fake.mp4.txt", "", "", ""));
}

#[test]
fn quota_numbers_may_be_strings() {
    let json = r#"{"quota":{"limit":"10995116277760","usage":"123","is_unlimited":false}}"#;
    let Ok(parsed) = serde_json::from_str::<AboutResponse>(json) else {
        panic!("quota 字符串形式应可解析");
    };
    let Some(quota) = parsed.quota else {
        panic!("缺少 quota");
    };
    assert_eq!(quota.usage, 123);
    assert_eq!(quota.limit, 10_995_116_277_760);
}

#[test]
fn user_info_parses_nickname_avatar_and_email() {
    let raw = r#"{
        "sub":"user-1",
        "name":"夏日用户",
        "picture":"https://user.mypikpak.com/avatar/1",
        "email":"mail@example.com"
    }"#;
    let user: UserInfo = serde_json::from_str(raw).expect("用户信息应可解析");
    assert_eq!(user.id, "user-1");
    assert_eq!(user.name, "夏日用户");
    assert_eq!(user.picture, "https://user.mypikpak.com/avatar/1");
    assert_eq!(user.email, "mail@example.com");
}

#[test]
fn recent_query_pins_everything_parent_and_tries_modified_order() {
    let query = recent_query(None);
    let get = |key: &str| {
        query
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    };
    assert_eq!(
        get("parent_id"),
        Some("*"),
        "探针必须看全盘，不能只看根目录"
    );
    assert_eq!(get("order"), Some("modified_time desc"));
    assert_eq!(
        get("filters"),
        Some(r#"{"phase":{"eq":"PHASE_TYPE_COMPLETE"},"trashed":{"eq":false}}"#),
        "与全盘列举保持一致：只取完成且未进回收站的条目"
    );
    assert_eq!(get("page_token"), None);

    let second = recent_query(Some(" page-2 "));
    assert!(second.contains(&("page_token".to_owned(), "page-2".to_owned())));
}

#[test]
fn list_query_matches_rclone_parameters() {
    let root = list_query(None, None);
    let get = |key: &str| root.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str());
    assert_eq!(get("limit"), Some("500"));
    assert_eq!(get("thumbnail_size"), Some("SIZE_MEDIUM"));
    assert_eq!(get("with_audit"), Some("true"));
    assert_eq!(
        get("filters"),
        Some(r#"{"phase":{"eq":"PHASE_TYPE_COMPLETE"},"trashed":{"eq":false}}"#)
    );
    assert_eq!(get("parent_id"), None, "根目录不应带 parent_id");
    assert_eq!(get("page_token"), None);
    assert!(!get("filters").unwrap_or_default().contains("kind"));

    let child = list_query(Some("folder-1"), Some("page-2"));
    assert!(child.contains(&("parent_id".to_owned(), "folder-1".to_owned())));
    assert!(child.contains(&("page_token".to_owned(), "page-2".to_owned())));
}

#[test]
fn trash_query_covers_all_directories_and_excludes_active_files() {
    let query = trash_query(Some("next"));
    let get = |key: &str| {
        query
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    };
    assert_eq!(get("parent_id"), Some("*"), "必须列全账号，不只根目录");
    assert_eq!(
        get("filters"),
        Some(r#"{"trashed":{"eq":true}}"#),
        "回收站只按 trashed 过滤，不要求 phase=complete"
    );
    assert_eq!(get("page_token"), Some("next"));
}

#[test]
fn file_list_parses_lenient_fields() {
    let json = r#"{
        "kind": "drive#fileList",
        "files": [
            {"id":"f1","name":"电影","kind":"drive#folder","size":"0",
             "created_time":"2026-10-20T12:00:00+08:00","starred":true,
             "modified_time":"2026-10-20T12:34:56.789+08:00"},
            {"id":"f2","name":"a.mkv","kind":"drive#file","size":"1099511627776",
             "modified_time":null,"md5_checksum":"ABC",
             "parent_id":"folder-7","tags":[{"name":"STAR"}],
             "audit":{"status":"STATUS_OK"},
             "thumbnail_link":"https://sg-thumbnail-drive.mypikpak.com/v0/x/240/720"},
            {"id":"f3","name":"bad.mkv","kind":"drive#file","size":"12",
             "modified_time":"","tags":null,"audit":{"status":"STATUS_BANNED"}}
        ],
        "next_page_token": "tok"
    }"#;
    let parsed: FileListResponse = serde_json::from_str(json).expect("列表应可解析");
    assert_eq!(parsed.files.len(), 3);
    let entries: Vec<FileEntry> = parsed.files.into_iter().map(FileDto::into_entry).collect();
    assert_eq!(entries[0].kind, EntryKind::Folder);
    assert_eq!(
        entries[0].modified_at.map(|t| t.timestamp()),
        Some(1_792_470_896),
        "带毫秒与时区的时间应正确换算"
    );
    assert_eq!(
        entries[0].created_at.map(|t| t.to_rfc3339()),
        Some("2026-10-20T04:00:00+00:00".to_owned()),
        "创建时间也应宽松解析"
    );
    assert!(entries[0].starred, "显式 starred 应识别");

    assert_eq!(entries[1].kind, EntryKind::File);
    assert_eq!(entries[1].size, 1_099_511_627_776, "字符串 size 应可解析");
    assert_eq!(entries[1].parent_id, "folder-7");
    assert!(entries[1].starred, "tags 里的 STAR 也应识别为已加星标");
    assert_eq!(entries[1].md5.as_deref(), Some("ABC"));
    assert!(!entries[1].audit_flagged);
    assert!(entries[1].media);
    assert!(entries[1].video);
    assert_eq!(
        entries[1].thumbnail_url,
        "https://sg-thumbnail-drive.mypikpak.com/v0/x/240/720"
    );
    assert!(entries[0].thumbnail_url.is_empty(), "文件夹不请求缩略图");

    assert_eq!(entries[2].modified_at, None, "空时间应为 None");
    assert!(entries[2].audit_flagged, "非 STATUS_OK 的审核状态应标记");
}

#[test]
fn complete_event_resource_requires_name_and_known_kind() {
    let complete: FileDto = serde_json::from_str(
        r#"{"name":"movie.mkv","kind":"drive#file","size":"42",
            "file_extension":"mkv"}"#,
    )
    .expect("资源应可解析");
    let entry = complete
        .into_complete_entry("event-file")
        .expect("名称和类型完整时应可直接展示");
    assert_eq!(entry.id, "event-file", "资源缺 id 时使用事件文件 id");
    assert!(entry.video);

    let missing_kind: FileDto =
        serde_json::from_str(r#"{"id":"f1","name":"movie.mkv"}"#).expect("资源应可解析");
    assert!(
        missing_kind.into_complete_entry("f1").is_none(),
        "类型缺失不能猜成普通文件"
    );

    let missing_name: FileDto =
        serde_json::from_str(r#"{"id":"f1","kind":"drive#file"}"#).expect("资源应可解析");
    assert!(missing_name.into_complete_entry("f1").is_none());

    let mismatched_id: FileDto =
        serde_json::from_str(r#"{"id":"other","name":"movie.mkv","kind":"drive#file"}"#)
            .expect("资源应可解析");
    assert!(
        mismatched_id.into_complete_entry("f1").is_none(),
        "参考资源不能贴到另一个事件文件上"
    );
}

#[test]
fn search_hit_keeps_the_parent_folder() {
    // 全盘列举的条目带 parent_id，拼「所在文件夹」真实路径全靠它
    let json = r#"{"files":[
        {"id":"f1","name":"第 1 集.mkv","kind":"drive#file","parent_id":"season-3"},
        {"id":"f2","name":"根目录文件.mkv","kind":"drive#file"}
    ]}"#;
    let parsed: FileListResponse = serde_json::from_str(json).expect("应可解析");
    let hits: Vec<crate::search::FileHit> =
        parsed.files.into_iter().map(FileDto::into_hit).collect();
    assert_eq!(hits[0].parent_id, "season-3");
    assert_eq!(hits[0].entry.name, "第 1 集.mkv");
    assert_eq!(hits[1].parent_id, "", "官方没给 parent_id 按根目录处理");
}

#[test]
fn trash_entry_keeps_parent_and_delete_time() {
    let json = r#"{"files":[
        {"id":"f1","name":"旧文件.mkv","kind":"drive#file","size":"42",
         "parent_id":"folder-7","delete_time":"2026-10-28T09:10:11+08:00"},
        {"id":"f2","name":"旧目录","kind":"drive#folder","delete_time":null}
    ]}"#;
    let parsed: FileListResponse = serde_json::from_str(json).expect("回收站应可解析");
    let trash: Vec<TrashEntry> = parsed
        .files
        .into_iter()
        .map(FileDto::into_trash_entry)
        .collect();
    assert_eq!(trash[0].entry.id, "f1");
    assert_eq!(trash[0].parent_id, "folder-7");
    assert_eq!(
        trash[0].deleted_at.map(|time| time.timestamp()),
        Some(1_793_149_811)
    );
    assert_eq!(trash[1].parent_id, "", "缺 parent_id 按根目录处理");
    assert_eq!(trash[1].deleted_at, None, "空删除时间应可解析");
}

/// 用 2026-10-26 真实账号抓到的 `about` 响应结构做样本。
#[test]
fn about_ignores_the_transfer_quotas_it_does_not_use() {
    let raw = r#"{
        "kind": "drive#about",
        "quota": {"kind": "drive#quota", "limit": "10995116277760", "usage": "5230119883333",
                  "usage_in_trash": "0", "is_unlimited": false},
        "quotas": {"cloud_download": {"kind": "", "limit": "-1", "usage": "0",
                   "usage_in_trash": "0", "is_unlimited": true}},
        "user_type": 1
    }"#;
    let response: AboutResponse = serde_json::from_str(raw).expect("样本应能解析");
    assert_eq!(response.quota.map(|q| q.usage), Some(5_230_119_883_333));
}

/// 2026-09-24 真实响应（来自 pikpak-kotlin 的 mock 测试，MIT），去掉标识字段。
#[test]
fn transfer_quota_parses_the_three_monthly_allowances() {
    let raw = r#"{
        "transfer":{"offline":{"info":"","total_assets":0,"assets":0}},"data":null,"has_more":false,
        "base":{"info":"","vip_status":"ok","expire_time":"2027-04-17T02:40:40+08:00","assets":"10T","size":0,
          "offline":{"total_assets":43980465111040,"assets":736368637030,"size":736368637030},
          "download":{"total_assets":4398046511104,"assets":197410452763,"size":197410452763},
          "upload":{"total_assets":1099511627776,"assets":162173574191,"size":162173574191},
          "download_daily":{"total_assets":0,"assets":0,"size":0}},
        "apps_summary":{"offline":{"total_assets":10995116277760,"assets":0,"size":0},
          "download":{"total_assets":1099511627776,"assets":0,"size":0}}
    }"#;
    let response: TransferQuotaResponse = serde_json::from_str(raw).expect("样本应能解析");
    let base = response.base.expect("base 应存在");
    assert_eq!(base.offline.allowance().limit, 43_980_465_111_040);
    assert_eq!(base.offline.allowance().used, 736_368_637_030);
    assert_eq!(base.download.allowance().limit, 4_398_046_511_104);
    assert_eq!(base.upload.allowance().limit, 1_099_511_627_776);
    assert_eq!(base.download_daily.allowance().limit, 0);
    assert_eq!(base.vip_status, "ok");
}

#[test]
fn transfer_quota_accepts_string_numbers_too() {
    let raw = r#"{"base":{"offline":{"total_assets":"4096","assets":"1024"}}}"#;
    let response: TransferQuotaResponse = serde_json::from_str(raw).expect("字符串数字也应能解析");
    let offline = response.base.expect("base 应存在").offline.allowance();
    assert_eq!(offline.limit, 4096);
    assert_eq!(offline.used, 1024);
}
