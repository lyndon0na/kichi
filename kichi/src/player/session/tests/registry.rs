use super::super::*;
use super::*;

#[test]
fn cancel_marks_only_that_session() {
    let _guard = serial();
    shutdown_all();
    let first = Arc::new(Cancel::default());
    let second = Arc::new(Cancel::default());
    let ids = with_sessions(|map| {
        let mut ids = Vec::new();
        for cancel in [&first, &second] {
            let id = next_id();
            map.insert(
                id,
                Session {
                    cancel: cancel.clone(),
                    child: Arc::new(Mutex::new(Some(sleeper()))),
                    cleanup: Vec::new(),
                },
            );
            ids.push(id);
        }
        ids
    });
    assert_eq!(active_count(), 2);

    // 取消不存在的 id 不报错，也不影响别的会话
    cancel(0);
    assert!(!first.is_cancelled() && !second.is_cancelled());

    cancel(ids[0]);
    assert!(first.is_cancelled(), "要取消的是这一条");
    assert!(!second.is_cancelled(), "别的 id 不能受到影响");
    shutdown_all();
}

#[test]
fn shutdown_all_is_idempotent_and_kills_processes() {
    let _guard = serial();
    shutdown_all();
    let cancel = Arc::new(Cancel::default());
    let child = Arc::new(Mutex::new(Some(sleeper())));
    with_sessions(|map| {
        map.insert(
            next_id(),
            Session {
                cancel: cancel.clone(),
                child: child.clone(),
                cleanup: Vec::new(),
            },
        );
    });
    shutdown_all();
    assert!(
        cancel.is_cancelled(),
        "退出时置取消，避免监控线程继续等窗口"
    );
    assert_eq!(active_count(), 0);
    assert!(child.lock().unwrap_or_else(|e| e.into_inner()).is_none());
    // 再来一次不能 panic
    shutdown_all();
    assert_eq!(active_count(), 0);
}

#[test]
fn shutdown_all_removes_the_ipc_socket() {
    let _guard = serial();
    shutdown_all();
    let dir = std::env::temp_dir().join(format!("kichi-sock-clean-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let socket = dir.join("s.sock");
    std::fs::write(&socket, b"").expect("造假 socket 文件");

    let cancel = Arc::new(Cancel::default());
    with_sessions(|map| {
        map.insert(
            next_id(),
            Session {
                cancel: cancel.clone(),
                child: Arc::new(Mutex::new(Some(sleeper()))),
                cleanup: vec![socket.clone()],
            },
        );
    });
    shutdown_all();
    assert!(
        !socket.exists(),
        "退出收尾要删掉 socket，不能让 /tmp/kichi-play 越攒越多"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn spawn_failure_is_reported() {
    let _guard = serial();
    shutdown_all();
    let config = PlayerConfig {
        program: "kichi-no-such-mpv".to_owned(),
        pre_args: Vec::new(),
        access_args: Vec::new(),
    };
    assert!(config.is_mpv(), "测试命令要按 mpv 路径处理");
    let cancel = Arc::new(Cancel::default());
    let result = start(
        &config,
        "https://cdn.invalid/v?sign=S",
        "t",
        cancel,
        PlaybackExtras::default(),
        |_| {},
    );
    assert!(result.is_err(), "spawn 失败必须冒泡给提示条");
    assert_eq!(active_count(), 0, "失败不能留下登记项");
}

#[test]
fn mpv_input_placeholders_are_rejected() {
    let _guard = serial();
    shutdown_all();
    let config = PlayerConfig {
        program: "mpv".to_owned(),
        pre_args: Vec::new(),
        access_args: vec!["{url}".to_owned()],
    };
    let result = start(
        &config,
        "https://cdn.invalid/v?sign=S",
        "t",
        Arc::new(Cancel::default()),
        PlaybackExtras::default(),
        |_| {},
    );
    assert!(
        result
            .err()
            .is_some_and(|reason| reason.contains("不能使用 {url}")),
        "mpv 的输入必须由 IPC 下发"
    );
    assert_eq!(active_count(), 0);
}

#[test]
fn non_mpv_launch_reports_ready_immediately() {
    let _guard = serial();
    shutdown_all();
    // 非 mpv 路径不建会话、不等窗口：spawn 成功就回投 Ready
    let config = PlayerConfig {
        program: "true".to_owned(),
        pre_args: Vec::new(),
        access_args: Vec::new(),
    };
    let cancel = Arc::new(Cancel::default());
    let (sender, receiver) = std::sync::mpsc::channel();
    let id = start(
        &config,
        "https://cdn.invalid/v?sign=S",
        "t",
        cancel,
        PlaybackExtras::default(),
        move |event| {
            let _ = sender.send(event);
        },
    )
    .expect("启动应当成功");
    assert_eq!(id, 0);
    assert_eq!(
        receiver.recv_timeout(Duration::from_secs(2)).ok(),
        Some(StartEvent::Ready {
            subtitle_note: None
        })
    );
    assert_eq!(
        receiver.recv_timeout(Duration::from_secs(2)).ok(),
        Some(StartEvent::Stopped {
            progress: None,
            end: EndReason::Unknown,
        }),
        "播放器进程退出后要清掉「播放中」"
    );
    assert_eq!(active_count(), 0);
}
