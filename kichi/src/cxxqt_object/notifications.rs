//! 系统通知的统一出口（走 XDG 门户，不依赖额外守护进程）。
//!
//! 使用处：下载批次结束（每批最多一条）、离线任务结束、触发官方配额/限速。
//! 是否弹由「任务结束通知」设置开关控制，调用方负责判断；这里只管拼文案与发送。

/// 一条拼好的系统通知。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Notice {
    pub(crate) summary: &'static str,
    pub(crate) body: String,
    pub(crate) icon: &'static str,
}

/// 一批下载结束时的汇总数字。
pub(crate) struct BatchSummary<'a> {
    pub(crate) name: &'a str,
    pub(crate) folder: bool,
    pub(crate) done: usize,
    pub(crate) failed: usize,
    pub(crate) cancelled: usize,
    /// 被跳过的目录数（过深 / 读失败 / 本地建目录失败）
    pub(crate) skipped: usize,
    /// 单文件批次最后一条失败原因
    pub(crate) failure: Option<&'a str>,
}

/// 正常收尾（跑完、取消之外的结束）→ 一条通知。
pub(crate) fn batch_finished(summary: BatchSummary<'_>) -> Notice {
    let BatchSummary {
        name,
        folder,
        done,
        failed,
        cancelled,
        skipped,
        failure,
    } = summary;
    // 跳过数只往「有成功」的通知里塞：全失败时数字已经够多，别再加噪音
    let skip_note = if skipped > 0 {
        format!(" · 跳过 {skipped} 个目录")
    } else {
        String::new()
    };

    // 一个文件都没动成（空批次兜底）：不给「下载完成」，也别假装有失败数字。
    // 实际上空文件夹已在 `end_notice` 直接返回 None，正常不会走到这里。
    if done == 0 && failed == 0 && cancelled == 0 {
        return Notice {
            summary: "下载结束",
            body: format!("{name}\n未下载任何文件"),
            icon: "dialog-warning",
        };
    }

    if folder {
        if failed == 0 && cancelled == 0 {
            return Notice {
                summary: "下载完成",
                body: format!("「{name}」\n{done} 个文件{skip_note}"),
                icon: "folder-download",
            };
        }
        if done == 0 && failed > 0 {
            let result = if cancelled > 0 {
                format!("失败 {failed} 个，取消 {cancelled} 个")
            } else {
                format!("{failed} 个文件全部失败")
            };
            return Notice {
                summary: "下载失败",
                body: format!("「{name}」\n{result}\n详情见传输任务页"),
                icon: "dialog-error",
            };
        }
        if done == 0 {
            return Notice {
                summary: "下载结束",
                body: format!("「{name}」\n取消 {cancelled} 个文件\n详情见传输任务页"),
                icon: "dialog-information",
            };
        }
        let cancel_note = if cancelled > 0 {
            format!("，取消 {cancelled} 个")
        } else {
            String::new()
        };
        return Notice {
            summary: "下载结束",
            body: format!(
                "「{name}」\n成功 {done} 个，失败 {failed} 个{cancel_note}{skip_note}\n详情见传输任务页"
            ),
            icon: "dialog-warning",
        };
    }

    if failed > 0 {
        let reason = failure
            .filter(|reason| !reason.is_empty())
            .unwrap_or("下载失败，详情见传输任务页");
        return Notice {
            summary: "下载失败",
            body: format!("{name}\n{reason}"),
            icon: "dialog-error",
        };
    }

    // 单文件成功：只报文件名，落盘路径去传输任务页看
    Notice {
        summary: "下载完成",
        body: name.to_owned(),
        icon: "folder-download",
    }
}

/// 批次还没开始就失败（读目录出错 / 线程起不来 / 根目录建不出来）→ 一条通知。
pub(crate) fn batch_failed(name: &str, folder: bool, reason: &str) -> Notice {
    let body = if folder {
        format!("「{name}」\n{reason}")
    } else {
        format!("{name}\n{reason}")
    };
    Notice {
        summary: "下载失败",
        body,
        icon: "dialog-error",
    }
}

/// 发一条拼好的通知。
pub(crate) fn show_notice(notice: &Notice) {
    show(notice.summary, &notice.body, notice.icon);
}

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

#[cfg(test)]
#[path = "notifications_tests.rs"]
mod tests;
