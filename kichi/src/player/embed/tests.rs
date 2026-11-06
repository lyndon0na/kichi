//! 内嵌链路运行期自检（默认 ignored）。
//!
//! `cargo test -p kichi embed::tests -- --ignored --nocapture`
//!
//! 需要当前会话有 `WAYLAND_DISPLAY` 与 ffmpeg。mpv 连的是进程内代理，
//! 不会在桌面上开窗口；片源无音轨，不会出声。

use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::{
    available, frame_pending, is_active, set_viewport, start, stop, take_frame, SessionSettings,
    SubtitleStyle,
};
use crate::player::{PlaybackExtras, StartEvent};
use crate::session::download::Cancel;

#[test]
#[ignore = "需要 Wayland 会话与 ffmpeg；手动运行"]
fn embedded_pipeline_delivers_a_frame() {
    if let Err(reason) = available() {
        eprintln!("跳过：{reason}");
        return;
    }

    let dir = std::env::temp_dir().join(format!("kichi-embed-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("创建测试目录失败");
    let clip = dir.join("clip.mp4");
    let status = std::process::Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "color=c=blue:s=320x180:d=2",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-an",
        ])
        .arg(&clip)
        .status()
        .expect("无法运行 ffmpeg");
    assert!(status.success(), "生成测试片失败");

    let cancel = Arc::new(Cancel::default());
    let (tx, rx) = mpsc::channel();
    let url = format!("file://{}", clip.display());
    start(
        &url,
        "embed-test",
        cancel.clone(),
        PlaybackExtras::default(),
        SessionSettings::new(
            0,
            true,
            1.0,
            SubtitleStyle::default(),
            String::new(),
            String::new(),
            Vec::new(),
        ),
        move |event| {
            let _ = tx.send(event);
        },
    )
    .expect("内嵌会话启动失败");
    // 没有界面时手动给一个视口，代理才知道该让 mpv 画多大。
    set_viewport(640, 360, 1.0);

    let deadline = Instant::now() + Duration::from_secs(20);
    let mut ready = false;
    let mut frame = None;
    while Instant::now() < deadline {
        while let Ok(event) = rx.try_recv() {
            match event {
                StartEvent::Ready { .. } => ready = true,
                StartEvent::Failed(reason) => panic!("播放失败：{reason}"),
                StartEvent::Cancelled => panic!("会话被意外取消"),
                StartEvent::Progress { .. } | StartEvent::Stopped { .. } => {}
            }
        }
        if ready && frame_pending() {
            frame = take_frame();
            if frame.is_some() {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    let mut frame = frame.expect("20 秒内没有收到画面帧");
    assert!(frame.width > 0 && frame.height > 0, "画面尺寸为 0");
    assert!(
        frame.pixels.is_some() || !frame.planes.is_empty(),
        "帧既没有像素也没有 dmabuf"
    );
    frame.mark_displayed();
    drop(frame);

    stop();
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if !is_active() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!is_active(), "停止后会话仍留在登记表");
    let _ = std::fs::remove_dir_all(&dir);
}
