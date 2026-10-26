//! 系统通知的统一出口（走 XDG 门户，不依赖额外守护进程）。
//!
//! 两处使用：离线任务结束、触发官方配额/限速。是否弹由「任务完成通知」
//! 设置开关控制，调用方负责判断；这里只管发。

/// 发一条桌面通知。失败只进日志：通知发不出去不该打断用户手头的事。
pub(crate) fn show(summary: &str, body: &str, icon: &str) {
    let outcome = notify_rust::Notification::new()
        .appname("kichi")
        .summary(summary)
        .body(body.trim_end())
        .icon(icon)
        .timeout(notify_rust::Timeout::Milliseconds(8000))
        .show();
    if let Err(err) = outcome {
        eprintln!("[kichi] 系统通知发送失败：{err}");
    }
}
