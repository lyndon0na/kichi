use super::super::*;
use super::*;

#[test]
fn vlc_reports_ready_progress_resume_and_stopped() {
    let _guard = serial();
    shutdown_all();
    let log = std::env::temp_dir().join(format!("kichi-fake-vlc-{}.log", std::process::id()));
    let _ = std::fs::remove_file(&log);
    let config = fake_vlc_config("ready", &log);
    assert_eq!(config.protocol(), PlayerProtocol::Vlc);
    let (sender, receiver) = std::sync::mpsc::channel();
    let id = start(
        &config,
        "https://cdn.invalid/v?sign=S",
        "测试视频",
        Arc::new(Cancel::default()),
        PlaybackExtras {
            resume: Some(PlaybackProgress {
                seconds: 42,
                duration: 120,
            }),
            ..PlaybackExtras::default()
        },
        move |event| {
            let _ = sender.send(event);
        },
    )
    .expect("启动假 VLC 失败");
    assert_ne!(id, 0);
    assert_eq!(
        receiver.recv_timeout(Duration::from_secs(2)).ok(),
        Some(StartEvent::Ready {
            subtitle_note: None
        })
    );
    std::thread::sleep(Duration::from_millis(260));
    shutdown_all();

    let mut saw_progress = false;
    let final_progress = loop {
        match receiver.recv_timeout(Duration::from_secs(2)).ok() {
            Some(StartEvent::Progress { .. }) => saw_progress = true,
            Some(StartEvent::Stopped { progress, .. }) => break progress,
            Some(other) => panic!("意外事件：{other:?}"),
            None => break None,
        }
    };
    assert!(saw_progress, "VLC 轮询应该产生播放进度");
    assert!(final_progress.is_some(), "退出时要补最后一个 VLC 播放位置");
    let log = std::fs::read_to_string(&log).expect("读取假 VLC 日志");
    assert!(log.contains("seek\t42"), "续播位置要走 RC seek：{log}");
    let _ = std::fs::remove_file(log);
    assert_eq!(active_count(), 0);
}

#[test]
fn vlc_exit_before_ready_is_reported() {
    let _guard = serial();
    shutdown_all();
    let log = std::env::temp_dir().join(format!("kichi-fake-vlc-fail-{}.log", std::process::id()));
    let config = fake_vlc_config("fail", &log);
    let (sender, receiver) = std::sync::mpsc::channel();
    start(
        &config,
        "https://cdn.invalid/v?sign=S",
        "坏视频",
        Arc::new(Cancel::default()),
        PlaybackExtras::default(),
        move |event| {
            let _ = sender.send(event);
        },
    )
    .expect("启动假 VLC 失败");
    match receiver.recv_timeout(Duration::from_secs(2)).ok() {
        Some(StartEvent::Failed(text)) => assert!(text.contains("加载前退出"), "{text}"),
        other => panic!("应当报 VLC 加载失败，实际 {other:?}"),
    }
    // 失败事件是在监控线程里发出的；会话表要等回调返回后才移除。
    wait_for_no_sessions();
    assert_eq!(active_count(), 0, "加载失败的会话要从登记表移除");
    let _ = std::fs::remove_file(log);
}
