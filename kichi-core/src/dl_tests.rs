//! `dl.rs` 的单元测试：Range 响应语义、空闲超时与下载 client 复用。

use super::*;
use std::io::{Read as _, Write as _};
use std::sync::Mutex;

/// 环境变量测试全局锁（避免并行测试互相踩踏，照 `config.rs` 的写法）。
static ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn idle_timeout_env_parses_with_fallbacks() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let cases = [
        ("7", Duration::from_secs(7)),
        (" 12 ", Duration::from_secs(12)),
        ("abc", DEFAULT_DOWNLOAD_IDLE),
        ("", DEFAULT_DOWNLOAD_IDLE),
        ("0", DEFAULT_DOWNLOAD_IDLE),
        ("-3", DEFAULT_DOWNLOAD_IDLE),
    ];
    for (raw, expected) in cases {
        std::env::set_var(DOWNLOAD_IDLE_ENV, raw);
        assert_eq!(download_idle(), expected, "输入 {raw:?}");
    }
    std::env::remove_var(DOWNLOAD_IDLE_ENV);
    assert_eq!(download_idle(), DEFAULT_DOWNLOAD_IDLE);
}

#[test]
fn content_range_parses_normal_wildcard_and_bad_input() {
    assert_eq!(
        parse_content_range("bytes 6-9/10"),
        Some(ContentRange {
            start: Some(6),
            end: Some(9),
            total: Some(10)
        })
    );
    assert_eq!(
        parse_content_range("bytes */10"),
        Some(ContentRange {
            start: None,
            end: None,
            total: Some(10)
        })
    );
    assert_eq!(
        parse_content_range("bytes 6-9/*"),
        Some(ContentRange {
            start: Some(6),
            end: Some(9),
            total: None
        })
    );
    assert_eq!(parse_content_range("items 0-1/2"), None);
    assert_eq!(parse_content_range("bytes 9-6/10"), None);
    assert_eq!(parse_content_range("bytes 6-9"), None);
    assert_eq!(parse_content_range(""), None);
}

/// 本地假服务器应答模式。
#[derive(Debug, Clone, Copy)]
enum Mode {
    /// 正常处理 Range（有界请求也忽略 end，回 start 到文件末尾）
    Range,
    /// 严格按 `bytes=start-end` 回 206 分片
    BoundedRange,
    /// 无论有没有 Range 都回 200 全量
    IgnoreRange,
    /// 有 Range 时回 206，但故意不带 Content-Range
    MissingContentRange,
    /// 有 Range 时回 206，但 Content-Range 的起点对不上
    MismatchedStart,
    /// 有 Range 时回 416
    Unsatisfiable,
    /// 读完请求后什么都不发，模拟 CDN 半死
    Stall,
}

/// 起一个只会说 HTTP/1.1 的假服务器；返回 URL 与收到的请求头。
///
/// `requests` 条请求后线程自己退出；测试只碰本机回环地址。
fn server(body: &'static [u8], requests: usize, mode: Mode) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("监听本地端口");
    let addr = listener.local_addr().expect("取本地地址");
    let seen = Arc::new(Mutex::new(Vec::new()));
    let seen_by_server = Arc::clone(&seen);
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
            let head = String::from_utf8_lossy(&request).into_owned();
            seen_by_server
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(head.clone());
            let range = head.lines().find_map(|line| {
                let (name, value) = line.split_once(':')?;
                if !name.trim().eq_ignore_ascii_case("range") {
                    return None;
                }
                let (start, end) = value.trim().strip_prefix("bytes=")?.split_once('-')?;
                let start = start.parse::<u64>().ok()?;
                let end = end.parse::<u64>().ok();
                Some((start, end))
            });
            let start = range.map(|(start, _)| start);
            let response = match (mode, start, range.and_then(|(_, end)| end)) {
                    (Mode::Stall, _, _) => {
                        std::thread::sleep(Duration::from_secs(10));
                        continue;
                    }
                    (Mode::Unsatisfiable, Some(_), _) => format!(
                        "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        body.len()
                    ),
                    (Mode::IgnoreRange, _, _) | (_, None, _) => {
                        let mut response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                        .into_bytes();
                        response.extend_from_slice(body);
                        let _ = socket.write_all(&response);
                        continue;
                    }
                    (Mode::BoundedRange, Some(start), requested_end) => {
                        if start >= body.len() as u64 {
                            format!(
                                "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                                body.len()
                            )
                        } else {
                            let last = requested_end
                                .unwrap_or(body.len() as u64 - 1)
                                .min(body.len() as u64 - 1);
                            let segment = &body[start as usize..=last as usize];
                            let mut response = format!(
                                "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{last}/{total}\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n",
                                total = body.len(),
                                length = segment.len()
                            )
                            .into_bytes();
                            response.extend_from_slice(segment);
                            let _ = socket.write_all(&response);
                            continue;
                        }
                    }
                    (Mode::Range, Some(start), _)
                    | (Mode::MissingContentRange, Some(start), _) => {
                        if start >= body.len() as u64 {
                            format!(
                                "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                                body.len()
                            )
                        } else {
                            let remaining = &body[start as usize..];
                            let content_range = if matches!(mode, Mode::Range) {
                                format!(
                                    "Content-Range: bytes {}-{}/{}\r\n",
                                    start,
                                    body.len() - 1,
                                    body.len()
                                )
                            } else {
                                String::new()
                            };
                            let mut response = format!(
                                "HTTP/1.1 206 Partial Content\r\n{content_range}Content-Length: {}\r\nConnection: close\r\n\r\n",
                                remaining.len()
                            )
                            .into_bytes();
                            response.extend_from_slice(remaining);
                            let _ = socket.write_all(&response);
                            continue;
                        }
                    }
                    (Mode::MismatchedStart, Some(start), _) => {
                        let remaining = &body[start.saturating_add(1).min(body.len() as u64) as usize..];
                        let mut response = format!(
                            "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {}-{}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            start.saturating_add(1),
                            body.len().saturating_sub(1),
                            body.len(),
                            remaining.len()
                        )
                        .into_bytes();
                        response.extend_from_slice(remaining);
                        let _ = socket.write_all(&response);
                        continue;
                    }
                };
            let _ = socket.write_all(response.as_bytes());
        }
    });
    (format!("http://{addr}/file"), seen)
}

fn local_info(url: &str) -> FileInfo {
    FileInfo {
        id: "f1".into(),
        name: "movie.mkv".into(),
        octet_stream: url.to_owned(),
        ..FileInfo::default()
    }
}

fn test_client() -> DriveClient {
    let device_id = "0".repeat(32);
    DriveClient::new(&crate::Config::default(), &device_id, "t", "u").expect("构造客户端（无 I/O）")
}

fn read_all(stream: &mut DownloadStream) -> Vec<u8> {
    let mut body = Vec::new();
    stream.read_to_end(&mut body).expect("读下载流");
    body
}

#[test]
fn offset_zero_does_not_send_range_and_reads_whole_file() {
    let (url, seen) = server(b"0123456789", 1, Mode::Range);
    let mut stream = test_client()
        .open_download_from_with(&local_info(&url), 0, Duration::from_secs(1))
        .expect("打开下载流");
    assert_eq!(stream.start_offset(), 0);
    assert_eq!(stream.total_bytes(), Some(10));
    assert_eq!(read_all(&mut stream), b"0123456789");
    let request = seen.lock().unwrap_or_else(|e| e.into_inner())[0].to_ascii_lowercase();
    assert!(!request.contains("\r\nrange:"), "offset 0 不该发 Range");
}

#[test]
fn range_response_reports_full_total_and_starts_at_offset() {
    let (url, seen) = server(b"0123456789", 1, Mode::Range);
    let mut stream = test_client()
        .open_download_from_with(&local_info(&url), 6, Duration::from_secs(1))
        .expect("206 应能打开");
    assert_eq!(stream.start_offset(), 6);
    assert_eq!(
        stream.total_bytes(),
        Some(10),
        "206 的 Content-Length 只是剩余字节"
    );
    assert_eq!(read_all(&mut stream), b"6789", "读到的首字节必须是偏移 6");
    let request = seen.lock().unwrap_or_else(|e| e.into_inner())[0].to_ascii_lowercase();
    assert!(request.contains("\r\nrange: bytes=6-"), "{request}");
}

#[test]
fn bounded_range_requests_exact_segment_and_reports_full_total() {
    let (url, seen) = server(b"0123456789", 1, Mode::BoundedRange);
    let mut stream = test_client()
        .open_download_range_with(&local_info(&url), 2, 5, Duration::from_secs(1))
        .expect("有界 206 应能打开");
    assert_eq!(stream.start_offset(), 2);
    assert_eq!(
        stream.total_bytes(),
        Some(10),
        "总大小取 Content-Range 的 total，不是这一片的长度"
    );
    assert_eq!(read_all(&mut stream), b"2345", "只能读请求的闭区间");
    let request = seen.lock().unwrap_or_else(|e| e.into_inner())[0].to_ascii_lowercase();
    assert!(request.contains("\r\nrange: bytes=2-5"), "{request}");
}

#[test]
fn bounded_range_ignored_by_server_is_rejected() {
    let (url, _) = server(b"0123456789", 1, Mode::IgnoreRange);
    let err = test_client()
        .open_download_range_with(&local_info(&url), 2, 5, Duration::from_secs(1))
        .err()
        .expect("200 全量不能当分片写");
    assert!(
        matches!(err, Error::RangeNotSatisfiable { .. }),
        "应让会话层退回单连接：{err:?}"
    );
}

#[test]
fn bounded_range_without_content_range_is_rejected() {
    let (url, _) = server(b"0123456789", 1, Mode::MissingContentRange);
    let err = test_client()
        .open_download_range_with(&local_info(&url), 2, 5, Duration::from_secs(1))
        .err()
        .expect("缺 Content-Range 无法确认分片起点");
    assert!(matches!(err, Error::RangeNotSatisfiable { .. }));
}

#[test]
fn bounded_range_start_mismatch_is_rejected() {
    let (url, _) = server(b"0123456789", 1, Mode::MismatchedStart);
    let err = test_client()
        .open_download_range_with(&local_info(&url), 2, 5, Duration::from_secs(1))
        .err()
        .expect("分片起点对不上必须报错");
    assert!(matches!(err, Error::RangeNotSatisfiable { .. }));
}

#[test]
fn bounded_range_beyond_the_file_reports_total() {
    let (url, _) = server(b"0123456789", 1, Mode::Unsatisfiable);
    match test_client()
        .open_download_range_with(&local_info(&url), 99, 100, Duration::from_secs(1))
        .err()
        .expect("416 必须报错")
    {
        Error::RangeNotSatisfiable { total } => assert_eq!(total, Some(10)),
        other => panic!("应是 RangeNotSatisfiable，实际 {other:?}"),
    }
}

#[test]
fn bounded_range_rejects_an_inverted_interval() {
    let (url, _) = server(b"0123456789", 1, Mode::BoundedRange);
    let err = test_client()
        .open_download_range_with(&local_info(&url), 5, 2, Duration::from_secs(1))
        .err()
        .expect("start > end 是调用方错误");
    assert!(matches!(err, Error::InvalidInput(_)), "{err:?}");
}

#[test]
fn range_start_mismatch_is_rejected() {
    let (url, _) = server(b"0123456789", 1, Mode::MismatchedStart);
    let err = test_client()
        .open_download_from_with(&local_info(&url), 6, Duration::from_secs(1))
        .err()
        .expect("起点对不上必须报错");
    assert!(
        matches!(err, Error::RangeNotSatisfiable { .. }),
        "应交给调用方从 0 重来：{err:?}"
    );
}

#[test]
fn server_ignoring_range_falls_back_to_zero() {
    let (url, _) = server(b"0123456789", 1, Mode::IgnoreRange);
    let mut stream = test_client()
        .open_download_from_with(&local_info(&url), 6, Duration::from_secs(1))
        .expect("200 也要能打开");
    assert_eq!(stream.start_offset(), 0, "服务器忽略了 Range");
    assert_eq!(stream.total_bytes(), Some(10));
    assert_eq!(read_all(&mut stream), b"0123456789");
}

#[test]
fn unsatisfiable_range_reports_total() {
    let (url, _) = server(b"0123456789", 1, Mode::Unsatisfiable);
    match test_client()
        .open_download_from_with(&local_info(&url), 99, Duration::from_secs(1))
        .err()
        .expect("416 必须报错")
    {
        Error::RangeNotSatisfiable { total } => assert_eq!(total, Some(10)),
        other => panic!("应是 RangeNotSatisfiable，实际 {other:?}"),
    }
}

#[test]
fn partial_response_without_content_range_restarts_from_zero() {
    let (url, seen) = server(b"0123456789", 2, Mode::MissingContentRange);
    let mut stream = test_client()
        .open_download_from_with(&local_info(&url), 6, Duration::from_secs(1))
        .expect("缺 Content-Range 时改用不带 Range 的请求");
    assert_eq!(stream.start_offset(), 0);
    assert_eq!(stream.total_bytes(), Some(10));
    assert_eq!(read_all(&mut stream), b"0123456789");
    let requests = seen.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(requests.len(), 2, "必须改发第二次请求");
    assert!(requests[0].to_ascii_lowercase().contains("\r\nrange:"));
    assert!(
        !requests[1].to_ascii_lowercase().contains("\r\nrange:"),
        "第二次不能再带 Range"
    );
}

#[test]
fn no_response_headers_times_out_instead_of_hanging() {
    let (url, _) = server(b"", 1, Mode::Stall);
    let started = std::time::Instant::now();
    let result = test_client().open_download_with(&local_info(&url), Duration::from_secs(1));
    let elapsed = started.elapsed();
    let err = result.err().expect("服务器不发响应头，必须报错");
    assert!(
        elapsed < Duration::from_secs(4),
        "应在超时附近返回，实际等了 {elapsed:?}"
    );
    assert!(err.to_string().contains("下载超时"), "{err}");
}

#[test]
fn stalled_body_read_reports_timed_out_and_keeps_received_bytes() {
    // 这个假服务器故意先回响应头，再只发 10 字节并挂住
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("监听本地端口");
    let addr = listener.local_addr().expect("取本地地址");
    std::thread::spawn(move || {
        let Ok((mut socket, _)) = listener.accept() else {
            return;
        };
        let mut request = Vec::new();
        let mut chunk = [0u8; 1024];
        while !request.windows(4).any(|window| window == b"\r\n\r\n") {
            match socket.read(&mut chunk) {
                Ok(0) | Err(_) => return,
                Ok(read) => request.extend_from_slice(&chunk[..read]),
            }
        }
        let _ = socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1048576\r\n\r\n0123456789");
        std::thread::sleep(Duration::from_secs(10));
        drop(socket);
    });
    let info = local_info(&format!("http://{addr}/file"));
    let mut stream = test_client()
        .open_download_with(&info, Duration::from_secs(1))
        .unwrap_or_else(|err| panic!("响应头正常，应能打开流：{err:?}"));
    assert_eq!(stream.total_bytes(), Some(1048576));

    let mut buffer = [0u8; 64];
    let mut received = 0;
    while received < 10 {
        let read = stream.read(&mut buffer).expect("停发前的读应成功");
        assert!(read > 0, "还没到 10 字节就 EOF 了");
        received += read;
    }
    assert_eq!(received, 10, "只发出了 10 字节");

    let started = std::time::Instant::now();
    let err = stream
        .read(&mut buffer)
        .expect_err("服务器停发后必须超时，不能永久阻塞");
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "应在超时附近返回"
    );
    assert_eq!(err.kind(), io::ErrorKind::TimedOut);
    assert!(err.to_string().contains("没有收到数据"), "{err}");
}

#[test]
fn body_cut_off_error_text_never_contains_the_url() {
    // 发不满 Content-Length 就断开：读失败原因会进传输记录并落盘，
    // 里面绝不能出现带签名的直链（docs/pitfalls.md 第 29 条）
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("监听本地端口");
    let addr = listener.local_addr().expect("取本地地址");
    std::thread::spawn(move || {
        let Ok((mut socket, _)) = listener.accept() else {
            return;
        };
        let mut request = Vec::new();
        let mut chunk = [0u8; 1024];
        while !request.windows(4).any(|window| window == b"\r\n\r\n") {
            match socket.read(&mut chunk) {
                Ok(0) | Err(_) => return,
                Ok(read) => request.extend_from_slice(&chunk[..read]),
            }
        }
        let _ = socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1024\r\n\r\n0123456789");
        // 直接 drop：连接提前关闭
    });
    let info = local_info(&format!("http://{addr}/signed-secret"));
    let mut stream = test_client()
        .open_download_with(&info, Duration::from_secs(2))
        .unwrap_or_else(|err| panic!("响应头正常，应能打开流：{err:?}"));
    let mut buffer = [0u8; 64];
    let err = loop {
        match stream.read(&mut buffer) {
            Ok(0) => panic!("连接提前断开应报错，而不是当成正常 EOF"),
            Ok(_) => continue,
            Err(err) => break err,
        }
    };
    let text = err.to_string();
    assert!(!text.contains("signed-secret"), "失败原因泄露直链：{text}");
    assert!(!text.contains("http"), "失败原因不该带 URL：{text}");
}

#[test]
fn truncated_chunked_body_is_an_error_not_eof() {
    // 未知总大小时，只有「chunked 正常结束」与「连接正常关闭」算完整；
    // 没读到结束块就断开必须报错，否则截断的文件会被当成下完
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("监听本地端口");
    let addr = listener.local_addr().expect("取本地地址");
    std::thread::spawn(move || {
        let Ok((mut socket, _)) = listener.accept() else {
            return;
        };
        let mut request = Vec::new();
        let mut chunk = [0u8; 1024];
        while !request.windows(4).any(|window| window == b"\r\n\r\n") {
            match socket.read(&mut chunk) {
                Ok(0) | Err(_) => return,
                Ok(read) => request.extend_from_slice(&chunk[..read]),
            }
        }
        // 只发一个有数据的块，结束块（0\r\n\r\n）之前就断开
        let _ = socket
            .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\n01234\r\n");
    });
    let info = local_info(&format!("http://{addr}/signed-secret"));
    let mut stream = test_client()
        .open_download_with(&info, Duration::from_secs(2))
        .unwrap_or_else(|err| panic!("响应头正常，应能打开流：{err:?}"));
    assert_eq!(stream.total_bytes(), None, "chunked 响应没有总大小");
    let mut buffer = [0u8; 64];
    let err = loop {
        match stream.read(&mut buffer) {
            Ok(0) => panic!("结束块还没收到就 EOF，不能当成完整结束"),
            Ok(_) => continue,
            Err(err) => break err,
        }
    };
    assert!(
        !err.to_string().contains("http"),
        "失败原因不该带 URL：{err}"
    );
}

#[test]
fn download_client_is_reused_for_the_same_idle() {
    let client = test_client();
    let idle = Duration::from_secs(7);
    let first = client.download_client(idle).expect("建下载 client 失败");
    let second = client.download_client(idle).expect("取缓存失败");
    assert!(
        Arc::ptr_eq(&first, &second),
        "同一个空闲超时必须复用同一个连接池"
    );
}

#[test]
fn download_client_rebuilds_when_idle_changes() {
    let client = test_client();
    let first = client
        .download_client(Duration::from_secs(7))
        .expect("建下载 client 失败");
    let second = client
        .download_client(Duration::from_secs(9))
        .expect("换空闲超时后应能重建");
    assert!(
        !Arc::ptr_eq(&first, &second),
        "空闲超时变了必须换 client（超时值存在 client 里）"
    );
    let third = client
        .download_client(Duration::from_secs(9))
        .expect("取缓存失败");
    assert!(Arc::ptr_eq(&second, &third));
}

#[test]
fn cached_download_client_keeps_its_idle_timeout() {
    let (url, _) = server(b"", 1, Mode::Stall);
    let client = test_client();
    let started = std::time::Instant::now();
    // 先建一次（模拟上一个文件已经下过，client 已在缓存里）
    let _ = client.download_client(Duration::from_secs(1));
    let result = client.open_download_with(&local_info(&url), Duration::from_secs(1));
    let elapsed = started.elapsed();
    let err = result.err().expect("缓存命中也必须按 1 秒超时");
    assert!(
        elapsed < Duration::from_secs(4),
        "应在超时附近返回，实际等了 {elapsed:?}"
    );
    assert!(err.to_string().contains("下载超时"), "{err}");
}
