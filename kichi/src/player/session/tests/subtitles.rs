use super::super::*;
use super::*;

fn test_assets() -> (SubtitleAssets, std::path::PathBuf) {
    let mut assets = SubtitleAssets::create().expect("建字幕临时目录");
    assets
        .push(
            "srt",
            b"1\n00:00:01,000 --> 00:00:02,000\nhi\n",
            "测试.zh.srt",
        )
        .expect("写字幕");
    let dir = assets
        .tracks()
        .first()
        .and_then(|track| track.path.parent())
        .map(std::path::Path::to_path_buf)
        .expect("字幕有临时目录");
    (assets, dir)
}

#[test]
fn subtitles_are_attached_before_ready_and_cleaned_up() {
    let _guard = serial();
    shutdown_all();
    let config = fake_mpv_config("ready");
    let cancel = Arc::new(Cancel::default());
    let (assets, dir) = test_assets();
    let (sender, receiver) = std::sync::mpsc::channel();
    start(
        &config,
        "https://cdn.invalid/v?sign=S",
        "带字幕",
        cancel,
        PlaybackExtras {
            subtitles: Some(assets),
            ..PlaybackExtras::default()
        },
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
    shutdown_all();
    let deadline = Instant::now() + Duration::from_secs(3);
    while dir.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!dir.exists(), "mpv 退出后字幕临时目录要删掉");
}

#[test]
fn a_rejected_sub_add_becomes_a_ready_note() {
    let _guard = serial();
    shutdown_all();
    let config = fake_mpv_config("sub-fail");
    let cancel = Arc::new(Cancel::default());
    let (assets, _dir) = test_assets();
    let (sender, receiver) = std::sync::mpsc::channel();
    start(
        &config,
        "https://cdn.invalid/v?sign=S",
        "字幕被拒",
        cancel,
        PlaybackExtras {
            subtitles: Some(assets),
            ..PlaybackExtras::default()
        },
        move |event| {
            let _ = sender.send(event);
        },
    )
    .expect("启动假 mpv 失败");
    let event = receiver
        .recv_timeout(Duration::from_secs(10))
        .expect("应当回投事件");
    match event {
        StartEvent::Ready { subtitle_note } => {
            let note = subtitle_note.expect("回执失败要带说明");
            assert!(note.contains("字幕未能加载"), "{note}");
        }
        other => panic!("字幕失败不能拖垮播放：{other:?}"),
    }
    shutdown_all();
}

#[test]
fn late_subtitles_are_attached_while_playing() {
    // 预等 2 秒没下完的字幕：播放中放进共享槽，监控线程要自己补挂
    let _guard = serial();
    shutdown_all();
    let dir = std::env::temp_dir().join(format!("kichi-late-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let log = dir.join("sub-add.log");
    let _ = std::fs::remove_file(&log);
    let config = fake_mpv_config_with_log("ready", &log);
    let slot: crate::player::subtitles::SubtitleSlot = Arc::new(Mutex::new(None));
    let late_cancel = Arc::new(Cancel::default());
    let cancel = Arc::new(Cancel::default());
    let (sender, receiver) = std::sync::mpsc::channel();
    start(
        &config,
        "https://cdn.invalid/v?sign=S",
        "补挂",
        cancel,
        PlaybackExtras {
            late: Some(LateSubtitles::new(slot.clone(), late_cancel.clone())),
            ..PlaybackExtras::default()
        },
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
    // 预等阶段没有字幕，播放开始后才“下完”
    let (assets, sub_dir) = test_assets();
    *slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(assets);
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut text = String::new();
    while Instant::now() < deadline {
        text = std::fs::read_to_string(&log).unwrap_or_default();
        if !text.is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(text.contains("测试.zh.srt"), "播放中应当补挂：{text:?}");
    assert!(
        text.starts_with("select\t"),
        "一条都没挂过时，补挂的第一条要选中：{text:?}"
    );
    shutdown_all();
    let deadline = Instant::now() + Duration::from_secs(3);
    while sub_dir.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!sub_dir.exists(), "补挂的字幕临时目录也要清理");
    let _ = std::fs::remove_dir_all(&dir);
}
