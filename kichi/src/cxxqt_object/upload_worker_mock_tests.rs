//! 上传 worker 的本地 HTTP 回归测试。
//!
//! 用一个只实现目录列表 / 新建目录 / 0 字节上传票据的 mock 服务，真实走一遍
//! `run_batch`：一个子目录创建失败时，兄弟子树照常完成；重试只补建失败目录、
//! 只重传失败文件，不碰已完成的目录和文件。

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use kichi_core::{Config, EntryKind};

use super::{clear_notice_group, run_batch, UploadFile, UploadStart};
use crate::cxxqt_object::transfers::{
    self, TransferDirection, TransferJob, TransferState, UploadJob,
};
use crate::cxxqt_object::upload_queue;
use crate::session::download::Cancel;
use crate::session::Session;

#[derive(Default)]
struct MockState {
    folder_names: Vec<String>,
    file_names: Vec<String>,
    fail_bad_once: bool,
    fail_file_once: Option<String>,
}

fn spawn_mock(state: Arc<Mutex<MockState>>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("mock 服务应能绑定本地端口");
    let addr = listener.local_addr().expect("mock 服务应有本地地址");
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else {
                continue;
            };
            handle(stream, &state);
        }
    });
    format!("http://{addr}")
}

fn handle(mut stream: TcpStream, state: &Arc<Mutex<MockState>>) {
    let Some((method, path, body)) = read_request(&mut stream) else {
        return;
    };
    let (status, body) = match (method.as_str(), path.as_str()) {
        ("GET", path) if path.starts_with("/drive/v1/files") => (
            "200 OK".to_owned(),
            r#"{"kind":"drive#fileList","files":[]}"#.to_owned(),
        ),
        ("POST", "/drive/v1/files") => create_entry(state, &body),
        _ => (
            "404 Not Found".to_owned(),
            r#"{"error":"not_found"}"#.to_owned(),
        ),
    };
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

fn create_entry(state: &Arc<Mutex<MockState>>, body: &[u8]) -> (String, String) {
    let value: serde_json::Value = serde_json::from_slice(body).unwrap_or_default();
    let kind = value["kind"].as_str().unwrap_or_default();
    let name = value["name"].as_str().unwrap_or_default();
    let mut state = state.lock().unwrap_or_else(|err| err.into_inner());
    if kind == "drive#folder" {
        state.folder_names.push(name.to_owned());
        if name == "bad" && state.fail_bad_once {
            state.fail_bad_once = false;
            return (
                "500 Internal Server Error".to_owned(),
                r#"{"error":"server_error","error_description":"mock failure"}"#.to_owned(),
            );
        }
        let id = format!("folder-{}", state.folder_names.len());
        return (
            "200 OK".to_owned(),
            serde_json::json!({"file": {"id": id}}).to_string(),
        );
    }
    if kind == "drive#file" {
        state.file_names.push(name.to_owned());
        if state.fail_file_once.as_deref() == Some(name) {
            state.fail_file_once = None;
            return (
                "500 Internal Server Error".to_owned(),
                r#"{"error":"server_error","error_description":"mock failure"}"#.to_owned(),
            );
        }
        let id = format!("file-{}", state.file_names.len());
        return (
            "200 OK".to_owned(),
            serde_json::json!({
                "file": {
                    "id": id,
                    "name": name,
                    "phase": "PHASE_TYPE_COMPLETE"
                }
            })
            .to_string(),
        );
    }
    (
        "400 Bad Request".to_owned(),
        r#"{"error":"bad_request"}"#.to_owned(),
    )
}

fn read_request(stream: &mut TcpStream) -> Option<(String, String, Vec<u8>)> {
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(position) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break position;
        }
    };
    let head = std::str::from_utf8(&buffer[..header_end]).ok()?;
    let mut lines = head.split("\r\n");
    let mut request = lines.next()?.split_whitespace();
    let method = request.next()?.to_owned();
    let path = request.next()?.to_owned();
    let content_length = lines
        .find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0);
    let body_start = header_end + 4;
    while buffer.len() < body_start + content_length {
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
    let body = buffer
        .get(body_start..body_start + content_length)?
        .to_vec();
    Some((method, path, body))
}

fn folder_job(local_path: PathBuf) -> TransferJob {
    TransferJob {
        id: "upload-folder:mock-root".to_owned(),
        name: "root".to_owned(),
        kind: EntryKind::Folder,
        size: 0,
        audit_flagged: false,
        sub_dir: Vec::new(),
        dir: PathBuf::new(),
        direction: TransferDirection::Upload,
        upload: Some(UploadJob {
            local_path,
            parent_id: "cloud-target".to_owned(),
            parent_name: "Test".to_owned(),
            ..UploadJob::default()
        }),
    }
}

fn file_job(name: &str, local_path: PathBuf, sub_dir: &[&str]) -> TransferJob {
    TransferJob {
        id: format!("upload:{}", local_path.display()),
        name: name.to_owned(),
        kind: EntryKind::File,
        size: 0,
        audit_flagged: false,
        sub_dir: sub_dir.iter().map(|part| (*part).to_owned()).collect(),
        dir: PathBuf::new(),
        direction: TransferDirection::Upload,
        upload: Some(UploadJob {
            local_path,
            parent_id: "cloud-target".to_owned(),
            parent_name: "Test".to_owned(),
            ..UploadJob::default()
        }),
    }
}

fn start(
    batch: u64,
    parent: TransferJob,
    files: Vec<(usize, TransferJob)>,
    dirs: Vec<Vec<String>>,
    upload_dirs: Vec<transfers::UploadDir>,
) -> UploadStart {
    UploadStart {
        batch,
        batch_cancel: Arc::new(Cancel::default()),
        parent: Some(parent),
        files: files
            .into_iter()
            .map(|(index, job)| UploadFile {
                index,
                job,
                cancel: Arc::new(Cancel::default()),
            })
            .collect::<Vec<_>>()
            .into(),
        dirs: dirs.into(),
        upload_dirs: upload_dirs.into(),
    }
}

#[test]
fn directory_failure_retry_only_touches_failed_subtree() {
    let state = Arc::new(Mutex::new(MockState {
        fail_bad_once: true,
        ..MockState::default()
    }));
    let base = spawn_mock(Arc::clone(&state));
    let session = Arc::new(Session::for_test(
        Config {
            drive_base: base,
            ..Config::default()
        },
        "mock-token",
    ));

    let root = std::env::temp_dir().join(format!("kichi-upload-subtree-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("root/good")).expect("创建 good 目录");
    std::fs::create_dir_all(root.join("root/bad")).expect("创建 bad 目录");
    let good_path = root.join("root/good/good.bin");
    let bad_path = root.join("root/bad/bad.bin");
    std::fs::File::create(&good_path).expect("创建 good 文件");
    std::fs::File::create(&bad_path).expect("创建 bad 文件");

    let parent = folder_job(root.join("root"));
    let good = file_job("good.bin", good_path, &["root", "good"]);
    let bad = file_job("bad.bin", bad_path, &["root", "bad"]);
    let dirs = vec![
        vec!["root".to_owned()],
        vec!["root".to_owned(), "good".to_owned()],
        vec!["root".to_owned(), "bad".to_owned()],
    ];

    let batch = transfers::new_batch();
    transfers::start_batch(batch, &parent.id, &parent);
    transfers::expand_folder(batch, &[good.clone(), bad.clone()], &dirs);
    clear_notice_group();

    let _ = run_batch(
        &session,
        &start(
            batch,
            parent.clone(),
            vec![(0, good.clone()), (1, bad.clone())],
            dirs.clone(),
            Vec::new(),
        ),
        upload_queue::current_epoch(),
    );

    let (parent_state, good_state, bad_state, upload_dirs, failed_dirs) =
        transfers::with_records(|records| {
            let parent = records
                .iter()
                .find(|record| record.batch == batch && !record.child)
                .expect("父行存在");
            let good = records
                .iter()
                .find(|record| record.batch == batch && record.index == 0 && record.child)
                .expect("good 子行存在");
            let bad = records
                .iter()
                .find(|record| record.batch == batch && record.index == 1 && record.child)
                .expect("bad 子行存在");
            (
                parent.state,
                good.state,
                bad.state,
                parent.upload_dirs.clone(),
                parent.upload_failed_dirs.clone(),
            )
        });
    assert_eq!(parent_state, TransferState::Failed);
    assert_eq!(good_state, TransferState::Done, "兄弟子树应照常完成");
    assert_eq!(bad_state, TransferState::Failed);
    assert_eq!(failed_dirs, vec![vec!["root".to_owned(), "bad".to_owned()]]);
    assert!(upload_dirs.iter().any(|dir| dir.path == ["root", "good"]));
    {
        let state = state.lock().unwrap_or_else(|err| err.into_inner());
        assert_eq!(state.folder_names, vec!["root", "bad", "good"]);
        assert_eq!(state.file_names, vec!["good.bin"]);
    }

    assert!(transfers::retry_upload_folder(batch));
    let upload_dirs = transfers::with_records(|records| {
        records
            .iter()
            .find(|record| record.batch == batch && !record.child)
            .expect("父行仍在")
            .upload_dirs
            .clone()
    });
    let _ = run_batch(
        &session,
        &start(batch, parent, vec![(1, bad)], dirs, upload_dirs),
        upload_queue::current_epoch(),
    );

    transfers::with_records(|records| {
        let parent = records
            .iter()
            .find(|record| record.batch == batch && !record.child)
            .expect("父行仍在");
        let good = records
            .iter()
            .find(|record| record.batch == batch && record.index == 0 && record.child)
            .expect("good 子行仍在");
        let bad = records
            .iter()
            .find(|record| record.batch == batch && record.index == 1 && record.child)
            .expect("bad 子行仍在");
        assert_eq!(parent.state, TransferState::Done);
        assert_eq!(good.state, TransferState::Done, "已完成文件不能重传");
        assert_eq!(bad.state, TransferState::Done);
        assert!(parent.upload_failed_dirs.is_empty());
    });
    {
        let state = state.lock().unwrap_or_else(|err| err.into_inner());
        assert_eq!(
            &state.folder_names[3..],
            &["bad".to_owned()],
            "重试只补建失败目录"
        );
        assert_eq!(
            &state.file_names[1..],
            &["bad.bin".to_owned()],
            "重试只重传失败文件"
        );
    }

    transfers::with_records(|records| records.retain(|record| record.batch != batch));
    transfers::with_trees(|trees| {
        trees.remove(&batch);
    });
    clear_notice_group();
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn child_retry_only_uploads_failed_file() {
    let state = Arc::new(Mutex::new(MockState {
        fail_file_once: Some("bad.bin".to_owned()),
        ..MockState::default()
    }));
    let base = spawn_mock(Arc::clone(&state));
    let session = Arc::new(Session::for_test(
        Config {
            drive_base: base,
            ..Config::default()
        },
        "mock-token",
    ));

    let root =
        std::env::temp_dir().join(format!("kichi-upload-child-retry-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("root/good")).expect("创建 good 目录");
    std::fs::create_dir_all(root.join("root/bad")).expect("创建 bad 目录");
    let good_path = root.join("root/good/good.bin");
    let bad_path = root.join("root/bad/bad.bin");
    std::fs::File::create(&good_path).expect("创建 good 文件");
    std::fs::File::create(&bad_path).expect("创建 bad 文件");

    let parent = folder_job(root.join("root"));
    let good = file_job("good.bin", good_path, &["root", "good"]);
    let bad = file_job("bad.bin", bad_path, &["root", "bad"]);
    let dirs = vec![
        vec!["root".to_owned()],
        vec!["root".to_owned(), "good".to_owned()],
        vec!["root".to_owned(), "bad".to_owned()],
    ];

    let batch = transfers::new_batch();
    transfers::start_batch(batch, &parent.id, &parent);
    transfers::expand_folder(batch, &[good.clone(), bad.clone()], &dirs);
    clear_notice_group();

    let _ = run_batch(
        &session,
        &start(
            batch,
            parent.clone(),
            vec![(0, good.clone()), (1, bad.clone())],
            dirs.clone(),
            Vec::new(),
        ),
        upload_queue::current_epoch(),
    );

    let (parent_state, good_state, bad_state, upload_dirs) = transfers::with_records(|records| {
        let parent = records
            .iter()
            .find(|record| record.batch == batch && !record.child)
            .expect("父行存在");
        let good = records
            .iter()
            .find(|record| record.batch == batch && record.index == 0 && record.child)
            .expect("good 子行存在");
        let bad = records
            .iter()
            .find(|record| record.batch == batch && record.index == 1 && record.child)
            .expect("bad 子行存在");
        (
            parent.state,
            good.state,
            bad.state,
            parent.upload_dirs.clone(),
        )
    });
    assert_eq!(parent_state, TransferState::Failed);
    assert_eq!(good_state, TransferState::Done, "成功兄弟文件应保持完成");
    assert_eq!(bad_state, TransferState::Failed);
    {
        let state = state.lock().unwrap_or_else(|err| err.into_inner());
        assert_eq!(state.folder_names, vec!["root", "bad", "good"]);
        assert_eq!(
            state
                .file_names
                .iter()
                .filter(|name| name.as_str() == "good.bin")
                .count(),
            1
        );
        assert_eq!(
            state
                .file_names
                .iter()
                .filter(|name| name.as_str() == "bad.bin")
                .count(),
            1
        );
    }

    assert!(transfers::retry_upload_child(batch, 1));
    let parent = transfers::with_records(|records| {
        records
            .iter()
            .find(|record| record.batch == batch && !record.child)
            .expect("父行仍在")
            .job
            .clone()
    });
    let _ = run_batch(
        &session,
        &start(batch, parent, vec![(1, bad)], dirs, upload_dirs),
        upload_queue::current_epoch(),
    );

    transfers::with_records(|records| {
        let parent = records
            .iter()
            .find(|record| record.batch == batch && !record.child)
            .expect("父行仍在");
        let good = records
            .iter()
            .find(|record| record.batch == batch && record.index == 0 && record.child)
            .expect("good 子行仍在");
        let bad = records
            .iter()
            .find(|record| record.batch == batch && record.index == 1 && record.child)
            .expect("bad 子行仍在");
        assert_eq!(parent.state, TransferState::Done);
        assert_eq!(good.state, TransferState::Done, "已完成文件不能重传");
        assert_eq!(bad.state, TransferState::Done);
    });
    {
        let state = state.lock().unwrap_or_else(|err| err.into_inner());
        assert_eq!(
            state
                .file_names
                .iter()
                .filter(|name| name.as_str() == "good.bin")
                .count(),
            1,
            "已完成的 good.bin 不能重传"
        );
        assert_eq!(
            state
                .file_names
                .iter()
                .filter(|name| name.as_str() == "bad.bin")
                .count(),
            2,
            "子文件重试只重传失败的 bad.bin"
        );
    }

    transfers::with_records(|records| records.retain(|record| record.batch != batch));
    transfers::with_trees(|trees| {
        trees.remove(&batch);
    });
    clear_notice_group();
    let _ = std::fs::remove_dir_all(&root);
}
