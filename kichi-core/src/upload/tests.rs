use super::cid::{cid_for_file, gcid_block_size, sha256_file_range, sha256_hex};
use super::io::{encode_path, hex_lower, ByteSliceReader, CancelReader, FileRangeReader};
use super::s3_error::{retry_delay, s3_attempt_error, upload_error_from_s3, UploadAttemptError};
use super::s3_request::{canonical_query, normalize_header, sign_request};
use super::s3_xml::parse_s3_error_body;
use super::util::{
    ensure_file_size, multipart_part_size, non_empty_name, resume_matches, validate_name,
    LocalFileInfo,
};
use super::*;
use crate::drive::DriveClient;
use crate::upload_state::{valid_bucket, UploadCredentials, UploadResume};
use crate::Config;
use std::fs::File;
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::sync::Arc;

use chrono::Utc;
use reqwest::StatusCode;
use sha2::{Digest as Sha256Digest, Sha256};

#[test]
fn upload_client_is_reused_across_files() {
    let device_id = "0".repeat(32);
    let client = DriveClient::new(&Config::default(), &device_id, "token", "user")
        .expect("构造客户端（无 I/O）");
    let first = client.upload_client().expect("建上传 client 失败");
    let second = client.upload_client().expect("取缓存失败");
    assert!(Arc::ptr_eq(&first, &second), "同一会话必须复用上传连接池");
}

#[test]
fn zero_byte_cid_is_the_sha1_of_empty_input() {
    let path = temp_path("zero");
    std::fs::write(&path, []).expect("create empty file");
    let mut progress = |_| {};
    let cid = cid_for_file(&path, 0, &|| false, &mut progress).expect("cid");
    let _ = std::fs::remove_file(path);
    assert_eq!(cid, "DA39A3EE5E6B4B0D3255BFEF95601890AFD80709");
}

#[test]
fn small_file_cid_hashes_the_whole_file() {
    let path = temp_path("small");
    std::fs::write(&path, b"hello").expect("create file");
    let mut progress = |_| {};
    let cid = cid_for_file(&path, 5, &|| false, &mut progress).expect("cid");
    let _ = std::fs::remove_file(path);
    assert_eq!(cid, "AAF4C61DDCC5E8A2DABEDE0F3B482CD9AEA9434D");
}

#[test]
fn gcid_block_size_matches_rclone_boundaries() {
    assert_eq!(gcid_block_size(0x40000 * 0x200), 0x40000);
    assert_eq!(gcid_block_size(0x40000 * 0x200 + 1), 0x80000);
    assert_eq!(gcid_block_size(u64::MAX), 0x200000);
}

#[test]
fn multipart_part_size_respects_the_ten_thousand_part_cap() {
    let part = multipart_part_size(5 * 1024 * 1024 * 10_001);
    assert!((5_u64 * 1024 * 1024 * 10_001).div_ceil(part) <= MAX_MULTIPART_PARTS);
    assert!(part >= SMALL_FILE_LIMIT);
}

#[test]
fn path_encoding_keeps_separators_but_encodes_spaces_and_unicode() {
    assert_eq!(encode_path("a b/中文.mkv"), "/a%20b/%E4%B8%AD%E6%96%87.mkv");
}

#[test]
fn canonical_query_is_sorted_and_encoded() {
    let query = vec![
        ("uploadId", "a b+c".to_owned()),
        ("partNumber", "2".to_owned()),
    ];
    assert_eq!(canonical_query(&query), "partNumber=2&uploadId=a%20b%2Bc");
}

#[test]
fn header_values_collapse_whitespace() {
    assert_eq!(normalize_header(" a \t b\n c "), "a b c");
}

#[test]
fn sigv4_matches_an_independent_aws_v4_calculation() {
    let amz_date = format!("{}{}", "202601", "01T000000Z");
    let query = vec![
        ("uploadId", "abc+/=".to_owned()),
        ("partNumber", "1".to_owned()),
    ];
    let authorization = sign_request(
        "PUT",
        "/uploads/a%20b",
        &query,
        "bucket.mypikpak.com",
        Some(5),
        Some("application/octet-stream"),
        "token",
        "abc123",
        &amz_date,
        "20260101",
        "pikpak",
        "s3",
        "access",
        "secret",
    )
    .expect("signature");
    let credential = [
        "Cred",
        "ential=",
        "access",
        "/20260101/pikpak/s3/aws4_request",
    ]
    .concat();
    let expected = [
        "AWS4-HMAC-SHA256 ",
        credential.as_str(),
        ", SignedHeaders=content-length;content-type;host;x-amz-content-sha256;",
        "x-amz-date;x-amz-security-token, ",
        "Signature=85736f30a4e1086c370356f75c60edda5f87cb60aabc37a868ce3544cd677608",
    ]
    .concat();
    assert_eq!(authorization, expected);
}

#[test]
fn cancel_reader_stops_before_reading_more_bytes() {
    let mut reader = CancelReader::new(Cursor::new(b"payload".to_vec()), || true);
    let mut buffer = [0u8; 4];
    let error = reader.read(&mut buffer).expect_err("must be cancelled");
    assert_eq!(error.kind(), std::io::ErrorKind::Interrupted);
}

#[test]
fn small_payload_hash_rejects_a_file_that_changed_size() {
    let path = temp_path("changed-size");
    std::fs::write(&path, b"hello").expect("create file");
    let mut file = File::open(&path).expect("open file");
    let mut progress = |_| {};
    assert!(sha256_hex(&mut file, &|| false, &mut progress, 4).is_err());
    file.seek(SeekFrom::Start(0)).expect("rewind file");
    assert!(sha256_hex(&mut file, &|| false, &mut progress, 6).is_err());
    let _ = std::fs::remove_file(path);
}

#[test]
fn upload_rechecks_the_original_file_size() {
    let path = temp_path("size-check");
    std::fs::write(&path, b"hello").expect("create file");
    assert!(ensure_file_size(&path, 5).is_ok());
    assert!(ensure_file_size(&path, 4).is_err());
    assert!(ensure_file_size(&path, 6).is_err());
    let _ = std::fs::remove_file(path);
}

#[test]
fn file_range_reader_stops_at_the_requested_boundary() {
    let path = temp_path("range");
    std::fs::write(&path, b"0123456789").expect("create file");
    let mut reader = FileRangeReader::open(&path, 2, 5).expect("open range");
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).expect("read range");
    let _ = std::fs::remove_file(path);
    assert_eq!(bytes, b"23456");
}

#[test]
fn sha256_file_range_matches_the_same_slice() {
    let path = temp_path("range-hash");
    let data = b"abcdefghijklmnopqrstuvwxyz";
    std::fs::write(&path, data).expect("create file");
    let digest = sha256_file_range(&path, 7, 11, &|| false).expect("hash range");
    let _ = std::fs::remove_file(path);
    assert_eq!(digest, Sha256::digest(&data[7..18]).to_vec());
}

#[test]
fn sha256_file_range_detects_a_truncated_range() {
    let path = temp_path("range-short");
    std::fs::write(&path, b"short").expect("create file");
    assert!(
        sha256_file_range(&path, 1, 8, &|| false).is_err(),
        "区间越过文件末尾时必须报错，而不是悄悄少算"
    );
    let _ = std::fs::remove_file(path);
}

#[test]
fn resume_identity_accepts_a_collision_renamed_ticket() {
    let local = LocalFileInfo {
        size: 12,
        modified_unix_secs: 34,
        modified_nanos: 56,
    };
    let mut state = UploadResume::new(
        local.size,
        local.modified_unix_secs,
        local.modified_nanos,
        "parent".into(),
        "file".into(),
        "task".into(),
        "GCID".into(),
        "电影 (1).mkv".into(),
        "电影 (1).mkv".into(),
        credentials("2099-01-01T00:00:00Z"),
    );
    assert!(
        resume_matches(&state, &local, "parent"),
        "撞名后的票据名不能把断点误判为失效"
    );

    let changed = LocalFileInfo {
        size: 12,
        modified_unix_secs: 34,
        modified_nanos: 56,
    };
    let changed = LocalFileInfo {
        size: changed.size + 1,
        ..changed
    };
    assert!(!resume_matches(&state, &changed, "parent"));
    assert!(!resume_matches(&state, &local, "other-parent"));

    state.credentials = credentials("2000-01-01T00:00:00Z");
    assert!(!resume_matches(&state, &local, "parent"));
    state.content_uploaded = true;
    assert!(
        resume_matches(&state, &local, "parent"),
        "内容已提交时只需继续任务，不再依赖过期的 S3 凭据"
    );
}

#[test]
fn non_retrying_s3_probes_keep_hard_limits_hard() {
    let denied = "<Error><Code>AccessDenied</Code><Message>no</Message></Error>";
    let forbidden = upload_error_from_s3(StatusCode::FORBIDDEN, denied);
    assert!(forbidden.keep_resume);
    assert!(forbidden.stop_queue);

    let rate_limited = upload_error_from_s3(StatusCode::TOO_MANY_REQUESTS, "");
    assert!(rate_limited.keep_resume);
    assert!(rate_limited.stop_queue);

    let quota = "<Error><Code>QuotaExceeded</Code><Message>full</Message></Error>";
    let quota = upload_error_from_s3(StatusCode::BAD_REQUEST, quota);
    assert!(quota.keep_resume);
    assert!(quota.stop_queue);

    let server_error = upload_error_from_s3(StatusCode::INTERNAL_SERVER_ERROR, "");
    assert!(!server_error.keep_resume);
    assert!(!server_error.stop_queue);
}

fn credentials(expiration: &str) -> UploadCredentials {
    UploadCredentials {
        access_key_id: "access".into(),
        access_key_secret: "secret".into(),
        security_token: "token".into(),
        bucket: "bucket".into(),
        key: "key".into(),
        endpoint: "https://bucket.mypikpak.com".into(),
        expiration: expiration.into(),
    }
}

#[test]
fn empty_sha256_constant_is_stable() {
    assert_eq!(
        hex_lower(&Sha256::digest([])),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[test]
fn s3_error_xml_is_sanitized_to_code_and_message() {
    let xml = "<Error><Code>AccessDenied</Code><Message>bad &amp; denied</Message></Error>";
    assert_eq!(
        parse_s3_error_body(xml),
        Some(("AccessDenied".to_owned(), "bad & denied".to_owned()))
    );
}

#[test]
fn transient_s3_failures_are_retryable_but_client_errors_are_not() {
    assert!(matches!(
        s3_attempt_error(StatusCode::SERVICE_UNAVAILABLE, ""),
        UploadAttemptError::Retryable(_)
    ));
    let timeout = "<Error><Code>RequestTimeout</Code><Message>try again</Message></Error>";
    assert!(matches!(
        s3_attempt_error(StatusCode::BAD_REQUEST, timeout),
        UploadAttemptError::Retryable(_)
    ));
    let denied = "<Error><Code>AccessDenied</Code><Message>no</Message></Error>";
    assert!(matches!(
        s3_attempt_error(StatusCode::FORBIDDEN, denied),
        UploadAttemptError::Hard(_)
    ));
    assert!(matches!(
        s3_attempt_error(StatusCode::TOO_MANY_REQUESTS, ""),
        UploadAttemptError::Hard(_)
    ));
    let quota = "<Error><Code>QuotaExceeded</Code><Message>full</Message></Error>";
    assert!(matches!(
        s3_attempt_error(StatusCode::BAD_REQUEST, quota),
        UploadAttemptError::Hard(_)
    ));
}

#[test]
fn upload_retry_backoff_is_bounded() {
    assert_eq!(retry_delay(0), UPLOAD_RETRY_BASE);
    assert_eq!(retry_delay(1), UPLOAD_RETRY_BASE.saturating_mul(2));
    assert_eq!(retry_delay(20), UPLOAD_RETRY_MAX);
}

#[test]
fn shared_part_bytes_can_be_replayed() {
    let bytes = Arc::new(b"reusable".to_vec());
    let mut first = CancelReader::new(ByteSliceReader::new(Arc::clone(&bytes)), || false);
    let mut second = CancelReader::new(ByteSliceReader::new(bytes), || false);
    let mut first_out = Vec::new();
    let mut second_out = Vec::new();
    first.read_to_end(&mut first_out).expect("read first body");
    second
        .read_to_end(&mut second_out)
        .expect("read replayed body");
    assert_eq!(first_out, b"reusable");
    assert_eq!(second_out, b"reusable");
}

#[test]
fn upload_names_are_validated_before_network_work() {
    assert!(validate_name("电影.mkv").is_ok());
    assert!(validate_name(" 两侧空格保留.mkv ").is_ok());
    assert!(validate_name("").is_err());
    assert!(validate_name("a/b").is_err());
    assert!(validate_name("a\nb").is_err());
    assert_eq!(
        non_empty_name(" 两侧空格保留.mkv ".to_owned()).as_deref(),
        Some(" 两侧空格保留.mkv ")
    );
}

#[test]
fn bucket_validation_rejects_host_injection() {
    assert!(valid_bucket("abc-123"));
    assert!(!valid_bucket("abc.example"));
    assert!(!valid_bucket("../abc"));
    assert!(!valid_bucket("-abc"));
}

fn temp_path(tag: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "kichi-upload-{tag}-{}-{}",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    path
}
