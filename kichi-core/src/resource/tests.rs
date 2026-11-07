use super::magnet::{parse_magnet_listing, percent_decode};
use super::*;
use serde_json::{json, Value};

const MAGNET: &str = "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=%E7%A4%BA%E4%BE%8B%E7%94%B5%E5%BD%B1";

fn listing(payload: Value) -> MagnetListing {
    parse_magnet_listing(&payload, MAGNET).expect("响应应可解析")
}

#[test]
fn parses_single_root_folder_and_unwraps_it() {
    let payload = json!({
        "list": {
            "resources": [{
                "name": "剧集",
                "is_dir": true,
                "dir": { "resources": [
                    { "name": "第 1 集.mkv", "file_size": "1024", "meta": { "hash": "abc" } },
                    { "name": "字幕", "is_dir": true, "dir": { "resources": [
                        { "name": "第 1 集.ass", "size": 12 }
                    ]}}
                ]}
            }]
        }
    });
    let parsed = listing(payload);
    assert_eq!(parsed.root_name, "剧集");
    assert_eq!(parsed.files.len(), 2);
    assert_eq!(parsed.files[0].folders, Vec::<String>::new());
    assert_eq!(parsed.files[0].size, 1024);
    assert_eq!(parsed.files[0].gcid.as_deref(), Some("abc"));
    assert_eq!(parsed.files[1].folders, vec!["字幕".to_owned()]);
    assert!(parsed.files[1].gcid.is_none());
    assert_eq!(parsed.total_size, 1036);
    assert_eq!(parsed.available_count(), 1);
}

#[test]
fn parses_multiple_roots_and_nested_folders() {
    let payload = json!({
        "resources": [
            { "name": "电影.mkv", "size": 10, "gcid": "HASH" },
            { "name": "花絮", "kind": "drive#folder", "resources": [
                { "name": "片段.mp4", "size": 5 }
            ]}
        ]
    });
    let parsed = listing(payload);
    assert_eq!(parsed.root_name, "示例电影", "取磁链 dn=");
    assert_eq!(parsed.files.len(), 2);
    assert_eq!(parsed.files[0].folders, Vec::<String>::new());
    assert_eq!(parsed.files[1].folders, vec!["花絮".to_owned()]);
}

#[test]
fn treats_children_array_and_string_flags_as_folders() {
    let payload = json!({
        "list": {
            "resources": [{
                "name": "根",
                "is_dir": "true",
                "children": [
                    { "name": "子目录", "children": [
                        { "name": "文件.bin", "file_size": 3 }
                    ]}
                ]
            }]
        }
    });
    let parsed = listing(payload);
    assert_eq!(parsed.files[0].folders, vec!["子目录".to_owned()]);
    assert_eq!(parsed.files[0].size, 3);
}

#[test]
fn empty_listing_is_a_clear_error() {
    let error = parse_magnet_listing(&json!({ "list": { "resources": [] } }), MAGNET)
        .expect_err("空清单要报错");
    assert!(error.to_string().contains("没有可保存的文件"));
}

#[test]
fn missing_resources_is_an_api_change() {
    assert!(matches!(
        parse_magnet_listing(&json!({ "list": {} }), MAGNET),
        Err(Error::ApiChanged(_))
    ));
}

#[test]
fn nesting_limit_is_enforced() {
    let mut node = json!({ "name": "叶子", "size": 1 });
    for index in 0..(MAX_MAGNET_DEPTH + 2) {
        node = json!({
            "name": format!("目录{index}"),
            "is_dir": true,
            "resources": [node],
        });
    }
    let payload = json!({ "resources": [node] });
    assert!(parse_magnet_listing(&payload, MAGNET).is_err());
}

#[test]
fn percent_decoding_handles_spaces_and_invalid_sequences() {
    assert_eq!(percent_decode("a+b%20c"), "a b c");
    assert_eq!(percent_decode("%E7%A4%BA"), "示");
    assert_eq!(percent_decode("%zz"), "%zz");
}

#[test]
fn name_sanitizing_keeps_cloud_names_legal() {
    assert_eq!(sanitize_drive_name(" a/b:c*? ", "fallback"), "abc");
    assert_eq!(sanitize_drive_name("   ", "备用"), "备用");
    assert_eq!(sanitize_drive_name("", ""), "未命名");
    assert_eq!(sanitize_drive_name("名字...", "x"), "名字");
    let long = "字".repeat(100);
    let trimmed = sanitize_drive_name(&long, "x");
    assert!(trimmed.len() <= MAX_NAME_BYTES);
    assert!(trimmed.chars().all(|ch| ch == '字'));
}

#[test]
fn instant_save_body_matches_the_official_shape() {
    let body = instant_save_body(" parent ", "电影.mkv", 1024, "abc");
    assert_eq!(body["hash"], "ABC");
    assert_eq!(body["name"], "电影.mkv");
    assert_eq!(body["size"], "1024");
    assert_eq!(body["kind"], "drive#file");
    assert_eq!(body["parent_id"], "parent");
    assert_eq!(body["upload_type"], "UPLOAD_TYPE_RESUMABLE");
    assert_eq!(body["folder_type"], "NORMAL");
    assert_eq!(body["objProvider"]["provider"], "UPLOAD_TYPE_UNKNOWN");
}

#[test]
fn instant_success_detection_matches_observed_responses() {
    assert!(instant_saved(&json!({
        "upload_type": "UPLOAD_TYPE_UNKNOWN",
        "file": { "id": "f1", "phase": "PHASE_TYPE_COMPLETE" }
    })));
    assert!(instant_saved(&json!({
        "file": { "id": "f1", "phase": "PHASE_TYPE_COMPLETE" }
    })));
    assert!(!instant_saved(&json!({
        "upload_type": "UPLOAD_TYPE_RESUMABLE",
        "resumable": { "provider": "PROVIDER_ALIYUN" },
        "file": { "id": "ghost", "phase": "PHASE_TYPE_PENDING" }
    })));
    assert!(!instant_saved(&json!({
        "file": { "id": "ghost", "phase": "PHASE_TYPE_ERROR" }
    })));
}
