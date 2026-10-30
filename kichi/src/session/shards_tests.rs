//! `shards.rs` 的单元测试：分片游标、环境变量、本地假服务器全流程。

use super::super::partials::ShardSpan;
use super::worker::{run_workers, ShardOutcome};
use super::*;
use std::io::{Read as _, Write as _};
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex};

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kichi-shards-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建临时目录");
    dir
}

/// 起一个支持（或故意忽略）Range 的假服务器；返回 URL。
fn serve(body: Vec<u8>, requests: usize, honor_range: bool) -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("监听本地端口");
    let addr = listener.local_addr().expect("取本地地址");
    std::thread::spawn(move || {
        for _ in 0..requests {
            let Ok((mut socket, _)) = listener.accept() else {
                return;
            };
            let mut request = Vec::new();
            let mut chunk = [0u8; 1024];
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                match socket.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => request.extend_from_slice(&chunk[..read]),
                }
            }
            let head = String::from_utf8_lossy(&request);
            let range = head.lines().find_map(|line| {
                let (name, value) = line.split_once(':')?;
                if !name.trim().eq_ignore_ascii_case("range") {
                    return None;
                }
                let (start, end) = value.trim().strip_prefix("bytes=")?.split_once('-')?;
                Some((start.parse::<u64>().ok()?, end.parse::<u64>().ok()?))
            });
            let response = match (honor_range, range) {
                (false, _) | (true, None) => {
                    let mut response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .into_bytes();
                    response.extend_from_slice(&body);
                    response
                }
                (true, Some((start, end))) => {
                    let last = end.min(body.len() as u64 - 1);
                    let segment = &body[start as usize..=last as usize];
                    let mut response = format!(
                        "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{last}/{total}\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n",
                        total = body.len(),
                        length = segment.len()
                    )
                    .into_bytes();
                    response.extend_from_slice(segment);
                    response
                }
            };
            let _ = socket.write_all(&response);
        }
    });
    format!("http://{addr}/file")
}

fn test_session(tag: &str) -> Session {
    let store = temp_dir(tag).join("session.token");
    let saved = super::super::PersistedSession {
        access_token: "test-access".into(),
        refresh_token: "test-refresh".into(),
        expires_unix: chrono::Utc::now().timestamp() + 3600,
        device_id: "0".repeat(32),
        user_id: "user-1".into(),
        email: String::new(),
        display_name: String::new(),
        avatar_url: String::new(),
    };
    let device_id = saved.device_id.clone();
    Session::new(kichi_core::Config::default(), store, Some(saved), device_id)
}

#[test]
fn connections_env_defaults_and_serial_switch() {
    static ENV_LOCK: Mutex<()> = Mutex::new(());
    let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let cases = [
        ("1", 1),
        (" 4 ", 4),
        ("0", DEFAULT_CONNECTIONS),
        ("", DEFAULT_CONNECTIONS),
        ("abc", DEFAULT_CONNECTIONS),
        ("999", MAX_CONNECTIONS),
    ];
    for (raw, expected) in cases {
        std::env::set_var(CONNECTIONS_ENV, raw);
        assert_eq!(shard_connections(), expected, "输入 {raw:?}");
    }
    std::env::remove_var(CONNECTIONS_ENV);
    assert_eq!(shard_connections(), DEFAULT_CONNECTIONS);
}

#[test]
fn worker_cursor_visits_each_shard_exactly_once() {
    let pending: Vec<usize> = (0..12).collect();
    let hits: Vec<AtomicUsize> = (0..12).map(|_| AtomicUsize::new(0)).collect();
    let cursor = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let outcomes = run_workers(3, &pending, &cursor, &stop, |index| {
        hits[index].fetch_add(1, Ordering::SeqCst);
        ShardOutcome::Done
    });
    assert!(outcomes
        .iter()
        .all(|outcome| matches!(outcome, ShardOutcome::Done)));
    assert!(
        hits.iter().all(|hit| hit.load(Ordering::SeqCst) == 1),
        "每个分片必须恰好处理一次"
    );
}

#[test]
fn shard_download_writes_every_byte_to_the_right_offset() {
    let _guard = partials::ENV_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let dir = temp_dir("download");
    let partials_file = dir.join("partials.json");
    std::env::set_var("KICHI_PARTIALS_FILE", &partials_file);

    // 用可识别的字节序列，任何一片写错偏移都会在比对时暴露
    let body: Vec<u8> = (0..4096u32).map(|value| (value % 251) as u8).collect();
    let url = serve(body.clone(), 8, true);
    let session = test_session("download");
    let info = FileInfo {
        id: "shard-file".into(),
        name: "movie.bin".into(),
        size: Some(body.len() as u64),
        octet_stream: url,
        ..FileInfo::default()
    };
    let progress_log = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&progress_log);
    let progress = move |bytes: u64, total: Option<u64>| {
        log.lock()
            .unwrap_or_else(|error| error.into_inner())
            .push((bytes, total));
    };
    let note = |_text: &str| {};
    let callbacks = DownloadCallbacks {
        progress: &progress,
        note: &note,
    };

    let result = download_with(
        &session,
        &info,
        KnownHint::from_list(body.len() as u64, false),
        "shard-file",
        "movie.bin",
        &dir,
        callbacks,
        &Cancel::default(),
        1024,
        4,
    )
    .expect("4096 字节、阈值 1024 应走分片")
    .expect("分片下载应成功");

    assert_eq!(std::fs::read(dir.join("movie.bin")).expect("读成品"), body);
    assert_eq!(result.bytes, body.len() as u64);
    assert_eq!(
        partials::lookup("shard-file", &dir),
        None,
        "成功后的索引必须清掉"
    );
    let last = progress_log
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .last()
        .copied()
        .expect("至少有一次进度回调");
    assert_eq!(last, (body.len() as u64, Some(body.len() as u64)));

    std::env::remove_var("KICHI_PARTIALS_FILE");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn v2_index_resumes_only_the_missing_bytes_of_each_shard() {
    let _guard = partials::ENV_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let dir = temp_dir("resume");
    std::env::set_var("KICHI_PARTIALS_FILE", dir.join("partials.json"));
    let body: Vec<u8> = (0..4096u32).map(|value| (value % 251) as u8).collect();
    let part = dir.join("movie.bin.kichi-part");
    let mut prefix = vec![0u8; 4096];
    prefix[..1536].copy_from_slice(&body[..1536]);
    std::fs::write(&part, prefix).expect("写部分 part");
    partials::remember(&PartialEntry::new_sharded(
        "shard-resume",
        &part,
        &dir.join("movie.bin"),
        4096,
        vec![
            ShardSpan {
                start: 0,
                end: 1024,
                done: 1024,
            },
            ShardSpan {
                start: 1024,
                end: 2048,
                done: 512,
            },
            ShardSpan {
                start: 2048,
                end: 3072,
                done: 0,
            },
            ShardSpan {
                start: 3072,
                end: 4096,
                done: 0,
            },
        ],
    ));

    let url = serve(body.clone(), 8, true);
    let session = test_session("resume");
    let info = FileInfo {
        id: "shard-resume".into(),
        name: "movie.bin".into(),
        size: Some(4096),
        octet_stream: url,
        ..FileInfo::default()
    };
    let progress = |_bytes: u64, _total: Option<u64>| {};
    let note = |_text: &str| {};
    let callbacks = DownloadCallbacks {
        progress: &progress,
        note: &note,
    };
    let result = download_with(
        &session,
        &info,
        KnownHint::from_list(4096, false),
        "shard-resume",
        "movie.bin",
        &dir,
        callbacks,
        &Cancel::default(),
        1024,
        4,
    )
    .expect("已有 v2 断点应继续分片")
    .expect("续传应成功");
    assert_eq!(std::fs::read(dir.join("movie.bin")).expect("读成品"), body);
    assert_eq!(result.bytes, 4096);

    std::env::remove_var("KICHI_PARTIALS_FILE");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cancel_deletes_the_part_and_forgets_the_index() {
    let _guard = partials::ENV_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let dir = temp_dir("cancel");
    std::env::set_var("KICHI_PARTIALS_FILE", dir.join("partials.json"));
    let session = test_session("cancel");
    let info = FileInfo {
        id: "shard-cancel".into(),
        name: "movie.bin".into(),
        size: Some(4096),
        octet_stream: "http://127.0.0.1:1/file".into(),
        ..FileInfo::default()
    };
    let cancel = Cancel::default();
    cancel.cancel();
    let progress = |_bytes: u64, _total: Option<u64>| {};
    let note = |_text: &str| {};
    let callbacks = DownloadCallbacks {
        progress: &progress,
        note: &note,
    };

    let err = download_with(
        &session,
        &info,
        KnownHint::from_list(4096, false),
        "shard-cancel",
        "movie.bin",
        &dir,
        callbacks,
        &cancel,
        1024,
        4,
    )
    .expect("应走分片路径")
    .expect_err("取消应返回错误");
    assert_eq!(err.to_string(), "下载已取消");
    assert!(!dir.join("movie.bin.kichi-part").exists(), "取消要删 part");
    assert_eq!(partials::lookup("shard-cancel", &dir), None);

    std::env::remove_var("KICHI_PARTIALS_FILE");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn server_ignoring_range_falls_back_to_the_single_connection() {
    let _guard = partials::ENV_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let dir = temp_dir("fallback");
    std::env::set_var("KICHI_PARTIALS_FILE", dir.join("partials.json"));
    let body = vec![7u8; 4096];
    let url = serve(body, 8, false);
    let session = test_session("fallback");
    let info = FileInfo {
        id: "shard-fallback".into(),
        name: "movie.bin".into(),
        size: Some(4096),
        octet_stream: url,
        ..FileInfo::default()
    };
    let progress = |_bytes: u64, _total: Option<u64>| {};
    let note = |_text: &str| {};
    let callbacks = DownloadCallbacks {
        progress: &progress,
        note: &note,
    };

    let result = download_with(
        &session,
        &info,
        KnownHint::from_list(4096, false),
        "shard-fallback",
        "movie.bin",
        &dir,
        callbacks,
        &Cancel::default(),
        1024,
        4,
    );
    assert!(result.is_none(), "服务器忽略 Range 时应退回单连接");
    let part = dir.join("movie.bin.kichi-part");
    assert_eq!(
        std::fs::metadata(&part)
            .expect("part 应保留给单连接续下")
            .len(),
        0,
        "回退前必须截断，不能把全量响应当分片写"
    );
    let entry = partials::lookup("shard-fallback", &dir).expect("应改回 v1 索引");
    assert!(entry.shards.is_empty());

    partials::forget("shard-fallback");
    std::env::remove_var("KICHI_PARTIALS_FILE");
    let _ = std::fs::remove_dir_all(&dir);
}
