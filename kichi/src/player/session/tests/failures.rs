use super::super::*;
use super::*;

#[test]
fn cancel_before_the_window_appears_quits_mpv() {
    let _guard = serial();
    shutdown_all();
    // 一直不出窗口：正是用户按「取消」的那种状态
    let config = fake_mpv_config("never");
    let cancel = Arc::new(Cancel::default());
    let (sender, receiver) = std::sync::mpsc::channel();
    start(
        &config,
        "https://cdn.invalid/v?sign=S",
        "慢启动",
        cancel.clone(),
        PlaybackExtras::default(),
        move |event| {
            let _ = sender.send(event);
        },
    )
    .expect("启动假 mpv 失败");
    // 等监控线程真的连上并发出 loadfile（fake mpv 收到才会一直不吭声）
    std::thread::sleep(Duration::from_millis(400));
    let started = Instant::now();
    cancel.cancel();
    let event = receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("取消后应当回投事件");
    assert_eq!(event, StartEvent::Cancelled);
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "取消收尾不该拖太久：{:?}",
        started.elapsed()
    );
    // 监控线程收尾后会移除会话；mpv 已被 quit（800 ms 宽限内）
    let deadline = Instant::now() + Duration::from_secs(3);
    while active_count() > 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(active_count(), 0, "取消后会话要从登记表移除");
}

#[test]
fn mpv_reporting_no_ipc_but_exiting_is_a_failure() {
    let _guard = serial();
    shutdown_all();
    // 名字含 mpv、但根本不建 socket 就直接退出（相当于播放器启动失败）
    let config = PlayerConfig {
        program: "false".to_owned(),
        pre_args: vec!["fake_mpv".to_owned()],
        access_args: Vec::new(),
    };
    let cancel = Arc::new(Cancel::default());
    let (sender, receiver) = std::sync::mpsc::channel();
    start(
        &config,
        "https://cdn.invalid/v?sign=S",
        "起不来",
        cancel,
        PlaybackExtras::default(),
        move |event| {
            let _ = sender.send(event);
        },
    )
    .expect("spawn 本身能成功");
    let event = receiver
        .recv_timeout(Duration::from_secs(10))
        .expect("应当回投事件");
    match event {
        StartEvent::Failed(text) => assert!(text.contains("起不来"), "{text}"),
        other => panic!("播放器没起来应当报失败：{other:?}"),
    }
}

#[test]
fn a_rejected_link_is_retried_and_then_plays() {
    let _guard = serial();
    shutdown_all();
    // 第一次 403、第二次成功：用户看到的就是「等一下自己播起来」
    let config = fake_mpv_config("fail-once");
    let cancel = Arc::new(Cancel::default());
    let (sender, receiver) = std::sync::mpsc::channel();
    start(
        &config,
        "https://cdn.invalid/v?sign=S",
        "第一次被拒",
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
        },
        "重试后应当成功开播"
    );
    shutdown_all();
}

#[test]
fn mpv_reports_failure_for_a_bad_link() {
    let _guard = serial();
    shutdown_all();
    let config = fake_mpv_config("fail");
    let cancel = Arc::new(Cancel::default());
    let (sender, receiver) = std::sync::mpsc::channel();
    start(
        &config,
        "https://cdn.invalid/v?sign=S",
        "坏链接",
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
    match event {
        StartEvent::Failed(text) => assert!(text.contains("坏链接"), "{text}"),
        other => panic!("坏链接应当失败：{other:?}"),
    }
    shutdown_all();
    assert_eq!(active_count(), 0);
}
