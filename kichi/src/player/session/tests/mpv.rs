use super::super::*;
use super::*;

#[test]
fn mpv_reports_ready_when_the_window_appears() {
    let _guard = serial();
    shutdown_all();
    let config = fake_mpv_config("ready");
    let cancel = Arc::new(Cancel::default());
    let (sender, receiver) = std::sync::mpsc::channel();
    start(
        &config,
        "https://cdn.invalid/v?sign=S",
        "测试视频",
        cancel,
        PlaybackExtras::default(),
        move |event| {
            let _ = sender.send(event);
        },
    )
    .expect("启动假 mpv 失败");
    let event = receiver
        .recv_timeout(Duration::from_secs(10))
        .expect("应当回投事件");
    assert_eq!(
        event,
        StartEvent::Ready {
            subtitle_note: None
        }
    );
    // Ready 之后监控线程还在等它退出（这里它会一直挂着直到退出收尾）
    shutdown_all();
    assert_eq!(
        receiver.recv_timeout(Duration::from_secs(3)).ok(),
        Some(StartEvent::Stopped {
            progress: None,
            end: EndReason::Stopped,
        }),
        "主动关闭已经开播的播放器也要回投 Stopped"
    );
    assert_eq!(active_count(), 0, "退出收尾要把会话清干净");
}

#[test]
fn mpv_seeks_to_resume_and_reports_progress_on_stop() {
    let _guard = serial();
    shutdown_all();
    let dir = std::env::temp_dir().join(format!("kichi-progress-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let log = dir.join("mpv.log");
    let _ = std::fs::remove_file(&log);
    let config = fake_mpv_config_with_log("progress", &log);
    let cancel = Arc::new(Cancel::default());
    let (sender, receiver) = std::sync::mpsc::channel();
    start(
        &config,
        "https://cdn.invalid/v?sign=S",
        "续播",
        cancel,
        PlaybackExtras {
            resume: Some(PlaybackProgress {
                seconds: 40,
                duration: 120,
            }),
            ..PlaybackExtras::default()
        },
        move |event| {
            let _ = sender.send(event);
        },
    )
    .expect("启动假 mpv 失败");
    assert_eq!(
        receiver.recv_timeout(Duration::from_secs(10)).ok(),
        Some(StartEvent::Ready {
            subtitle_note: None
        })
    );
    assert_eq!(
        receiver.recv_timeout(Duration::from_secs(3)).ok(),
        Some(StartEvent::Progress {
            progress: PlaybackProgress {
                seconds: 42,
                duration: 120
            }
        })
    );
    shutdown_all();
    assert_eq!(
        receiver.recv_timeout(Duration::from_secs(3)).ok(),
        Some(StartEvent::Stopped {
            progress: Some(PlaybackProgress {
                seconds: 42,
                duration: 120
            }),
            end: EndReason::Stopped,
        })
    );
    let text = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(text.contains("seek\t40"), "应当跳到历史位置：{text:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn mpv_natural_eof_reports_last_non_full_second() {
    let _guard = serial();
    shutdown_all();
    let config = fake_mpv_config("complete");
    let cancel = Arc::new(Cancel::default());
    let (sender, receiver) = std::sync::mpsc::channel();
    start(
        &config,
        "https://cdn.invalid/v?sign=S",
        "播完",
        cancel,
        PlaybackExtras::default(),
        move |event| {
            let _ = sender.send(event);
        },
    )
    .expect("启动假 mpv 失败");
    assert_eq!(
        receiver.recv_timeout(Duration::from_secs(10)).ok(),
        Some(StartEvent::Ready {
            subtitle_note: None
        })
    );
    let final_progress = loop {
        match receiver.recv_timeout(Duration::from_secs(3)) {
            Ok(StartEvent::Progress { .. }) => continue,
            Ok(StartEvent::Stopped { progress, .. }) => break progress,
            other => panic!("意外的播放事件：{other:?}"),
        }
    };
    assert_eq!(
        final_progress,
        Some(PlaybackProgress {
            seconds: 119,
            duration: 120
        }),
        "自然播完必须上报片尾前一秒，避免服务端把满进度归零"
    );
    wait_for_no_sessions();
    assert_eq!(active_count(), 0);
}
