//! 下载到本地：QML 入口与状态属性。
//!
//! 队列节奏与后台细节在 [`super::queue`]：
//! - **一次只下一个文件**（串行）：并行会互相抢带宽，官方也视为压力；
//! - 文件夹下载先递归列目录再排队，落盘时保留层级；
//! - 进度经主线程队列回投（`pump()` 消费），后台线程不碰 Qt 对象；
//! - 登出时在途下载整体作废。
//!
//! 本文件只做：入口校验、起线程、把状态刷到 QML 属性。

use std::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt_lib::QString;
use kichi_core::EntryKind;

use super::actions::run_in_background;
use super::qobject;
use super::queue::{
    self, collect_folder, is_busy, progress_text, run_queue, set_busy, with_batch, Job,
};
use crate::session::download::Cancel;
use crate::session::Failure;

impl qobject::Kichi {
    /// 下载一个条目（文件直接下；文件夹递归排队）。界面点「⬇」走这里。
    pub fn download_entry(mut self: Pin<&mut Self>, id: QString) {
        let id = id.to_string();
        // 搜索结果里的条目也能直接下：先在当前目录找，再找搜索命中
        let Some(entry) = self.as_ref().rust().find_entry(&id) else {
            self.as_mut()
                .set_message("该条目已不在当前列表中，请刷新后重试", false);
            return;
        };
        if is_busy() {
            self.as_mut().set_message(
                "已有下载在进行，等它结束再试（当前版本一次只下一个）",
                false,
            );
            return;
        }
        let dir = self.settings.download_dir_path();
        if entry.kind == EntryKind::Folder {
            let session = self.backend.session.clone();
            self.as_mut()
                .set_message(&format!("正在读取「{}」的目录结构……", entry.name), true);
            run_in_background(
                move || collect_folder(&session, &entry),
                move |mut qichi, result| match result {
                    Ok(jobs) => qichi.as_mut().start_batch(dir, jobs),
                    Err(err) => qichi.as_mut().on_download_failure(err),
                },
            );
            return;
        }
        let jobs = vec![Job {
            id: entry.id.clone(),
            sub_dir: Vec::new(),
            name: entry.name.clone(),
        }];
        self.as_mut().start_batch(dir, jobs);
    }

    /// 取消当前下载（整批停掉；已下完的文件保留）。
    pub fn cancel_downloads(mut self: Pin<&mut Self>) {
        with_batch(|state| {
            state.cancel.cancel();
            state.running = false;
        });
        self.as_mut().set_message("正在取消下载……", false);
        self.as_mut().sync_download_state();
    }

    /// 在文件管理器里打开下载目录（没建就顺手建好）。
    pub fn open_download_dir(&self) {
        let dir = self.settings.download_dir_path();
        if let Err(err) = std::fs::create_dir_all(&dir) {
            eprintln!("[kichi] 创建下载目录失败：{err}");
        }
        let spawned = std::process::Command::new("xdg-open")
            .arg(&dir)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        if let Err(err) = spawned {
            eprintln!("[kichi] 打开下载目录失败：{err}");
        }
    }

    /// 启动一批下载（主线程调用；队列线程从这里接手）。
    pub(crate) fn start_batch(mut self: Pin<&mut Self>, dir: std::path::PathBuf, jobs: Vec<Job>) {
        if jobs.is_empty() {
            self.as_mut().set_message("这个位置没有可下载的文件", false);
            return;
        }
        let total = jobs.len();
        with_batch(|state| {
            *state = queue::Batch {
                running: true,
                remaining: total,
                cancel: std::sync::Arc::new(Cancel::default()),
                ..queue::Batch::default()
            };
        });
        set_busy(true);
        self.as_mut()
            .set_message(&format!("开始下载 {total} 个文件……"), false);
        self.as_mut().sync_download_state();

        let session = self.backend.session.clone();
        let spawned = std::thread::Builder::new()
            .name("kichi-dl".into())
            .spawn(move || run_queue(session, dir, jobs));
        if let Err(err) = spawned {
            set_busy(false);
            with_batch(|state| state.running = false);
            eprintln!("[kichi] 无法创建下载线程：{err}");
            self.as_mut()
                .set_message("无法创建下载线程，下载未开始", false);
            self.as_mut().sync_download_state();
        }
    }

    /// 把队列状态刷到 QML 属性（主线程调用）。
    pub(crate) fn sync_download_state(mut self: Pin<&mut Self>) {
        let (current, bytes, total, remaining, done, failed, running) = with_batch(|state| {
            (
                state.current.clone(),
                state.current_bytes,
                state.current_total,
                state.remaining,
                state.done,
                state.failed,
                state.running,
            )
        });
        let text = progress_text(&current, bytes, total);
        {
            let mut rust = self.as_mut().rust_mut();
            rust.download_current = QString::from(text.as_str());
            rust.download_running = running;
            rust.download_remaining = i32::try_from(remaining).unwrap_or(i32::MAX);
            rust.download_done = i32::try_from(done).unwrap_or(i32::MAX);
            rust.download_failed = i32::try_from(failed).unwrap_or(i32::MAX);
        }
        self.as_mut().notify();
    }

    /// 下载相关错误的统一提示（令牌失效仍走登录失效分支）。
    fn on_download_failure(mut self: Pin<&mut Self>, failure: Failure) {
        if matches!(failure, Failure::Auth(_)) {
            self.as_mut().on_failure(failure);
            return;
        }
        self.as_mut().set_message(&failure.to_string(), false);
    }

    /// 登出时作废下载：取消在途任务并清空状态。
    pub(crate) fn reset_downloads(mut self: Pin<&mut Self>) {
        queue::bump_epoch();
        with_batch(|state| {
            state.cancel.cancel();
            *state = queue::Batch::default();
        });
        set_busy(false);
        self.as_mut().sync_download_state();
    }
}
