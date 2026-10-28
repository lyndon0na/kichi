//! 下载到本地：QML 入口与状态属性。
//!
//! 队列节奏与后台细节在 [`super::queue`]：
//! - **一次只跑一批、批内串行**：并行的下载会互相抢带宽，官方也视为压力；
//!   不同条目可以排队，轮到才读目录；
//! - 判重键是云端条目 id：同一条目在排队 / 准备 / 下载中时点下载 = 跳转定位；
//! - 文件夹下载轮到时才递归列目录，落盘时保留层级，并先建出完整目录树；
//! - **空文件夹算成功**：建出目录即完成，不弹通知；读不到的子目录按跳过处理
//!   （边界见 `docs/adr/0012-folder-download-boundaries.md`）；
//! - 进度经主线程队列回投（`pump()` 消费），后台线程不碰 Qt 对象；
//! - 登出时在途下载停止并**保留断点**，绝不落成品；旧线程退出前新批次只排队。
//!
//! 逐条记录在 [`super::transfers`]；本文件只做：入口校验、发定位请求、
//! 调度器推进、把记录表刷到 QML 属性。

use std::pin::Pin;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::Instant;

use cxx_qt::CxxQtType;
use cxx_qt_lib::QString;

use super::qobject;
use super::queue;
use super::queue_collect;
use super::reveal;
use super::transfers;
use super::transfers_actions;
use super::transfers_detail;
use super::transfers_history::{self, StoreTarget};
use super::transfers_rate::heartbeat_due;
use super::transfers_view::HISTORY_DAY_ALL;
use crate::session::Failure;

/// 上一次推 `transferLive` 的时刻（心跳节流；只有主线程读写）。
fn last_live_push() -> &'static Mutex<Option<Instant>> {
    static LAST: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();
    LAST.get_or_init(|| Mutex::new(None))
}

/// 记下这次推送：结构同步也会顺带刷新 `transferLive`，心跳不必马上再来一次。
fn mark_live_pushed() {
    *last_live_push()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Instant::now());
}

/// 这一轮运行的历史文件目标（启动时加载定下，之后不变）。
fn history_target() -> &'static Mutex<Option<StoreTarget>> {
    static TARGET: OnceLock<Mutex<Option<StoreTarget>>> = OnceLock::new();
    TARGET.get_or_init(|| Mutex::new(None))
}

fn with_target<T>(f: impl FnOnce(&mut Option<StoreTarget>) -> T) -> T {
    let mut guard: MutexGuard<'static, Option<StoreTarget>> = history_target()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    f(&mut guard)
}

/// 启动时加载一次下载历史（首次 `sync_transfers()` 之前由构造器调用）。
///
/// 走不进 `impl Kichi`：构造器里 `self` 还没成形，而读盘之后紧接着就要把
/// 记录表刷到属性上。
pub(crate) fn load_history(object: &mut crate::cxxqt_object::KichiRust) {
    let target = transfers_history::boot(&StoreTarget::from_env());
    transfers::set_session(target.session);
    with_target(|slot| *slot = Some(target));
    let (list, active) = transfers_history::snapshot();
    object.transfers = list;
    object.transfer_revision = object.transfer_revision.wrapping_add(1);
    object.download_active = active;
    object.transfer_has_finished = transfers_history::has_finished();
}

/// 强制把记录表写盘（退出 / 登出前调用）。
pub(crate) fn flush_history() {
    let target = with_target(|slot| slot.clone());
    if let Some(target) = target {
        transfers_history::flush_now(&target);
    }
}

/// 节流写盘：结构同步时调用，最多 2 秒写一次。
fn maybe_write_history() {
    let target = with_target(|slot| slot.clone());
    if let Some(target) = target {
        transfers_history::maybe_flush(&target);
    }
}

impl qobject::Kichi {
    /// 下载一个条目（文件直接下；文件夹轮到时递归读目录）。界面右键菜单的「下载」走这里。
    ///
    /// 跳转只在「这次没建新批次」时发生：说明这个条目已经在队列里了，
    /// 用户想去看看它；第一次下载不打断用户当前所在页面。
    pub fn download_entry(mut self: Pin<&mut Self>, id: QString) {
        let id = id.to_string();
        // 搜索结果里的条目也能直接下：先在当前目录找，再找搜索命中
        let Some(entry) = self.as_ref().rust().find_entry(&id) else {
            self.as_mut()
                .set_message("该条目已不在当前列表中，请刷新后重试", false);
            return;
        };
        let dir = self.settings.download_dir_path();
        let source_id = entry.id.clone();
        // 登记成功后先推进队列：空闲时这一下就把线程起来了；有批次在跑就只是排队。
        // `advance_downloads` 里已经 sync 过一次，这里不再重复。
        if queue::reserve(entry, dir).is_some() {
            self.as_mut().advance_downloads();
            return;
        }
        // 已经在排队 / 准备 / 下载中：再点一次 = 去传输任务页看它
        self.as_mut()
            .locate_transfer(QString::from(source_id.as_str()));
    }

    /// 这个条目是否已在排队 / 准备 / 下载中（右键菜单文案用）。
    pub fn is_download_pending(&self, id: QString) -> bool {
        queue::is_pending(&id.to_string())
    }

    /// 这个文件是否有可续传的断点（右键菜单显示「继续下载」）。
    ///
    /// 只认当前下载目录里的断点：换了目录之后接着点下载会重新落一份，
    /// 菜单里不该假装「继续」。
    pub fn has_partial_download(&self, id: QString) -> bool {
        let dir = self.settings.download_dir_path();
        crate::session::partials::lookup(&id.to_string(), &dir).is_some()
    }

    /// 取消所有未结束的批次（排队中的直接放掉，正在跑的等线程收尾；已下完的文件保留）。
    pub fn cancel_downloads(mut self: Pin<&mut Self>) {
        queue::cancel_all();
        // 立刻反映到记录表：正在下的那条与排队中的都变「已取消」，
        // 不用等下载线程读到取消标志（网络卡住时可能要等当前读返回）。
        // `downloadRunning` 由调度器的收尾推进，不在这里清。
        transfers::cancel_active("");
        self.as_mut().set_message("正在取消下载……", false);
        self.as_mut().sync_transfers();
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

    /// 清空已结束（完成 / 失败 / 已取消）的记录，进行中 / 排队 / 已中断的保留。
    ///
    /// 清的是**全部历史**里所有已结束的整批，不按日期分组收窄（ADR 0014）。
    pub fn clear_finished_transfers(mut self: Pin<&mut Self>) {
        transfers::clear_all();
        self.as_mut().sync_transfers();
    }

    /// 分组行要列的日子（新 → 旧）：`[{ key, label }, …]`。
    ///
    /// 每条带 `key`：QML 的 Repeater 靠它定位到具体某天的按钮。
    pub fn transfer_history_days(&self) -> super::QVariantList {
        use cxx_qt_lib::{QMap, QMapPair_QString_QVariant, QVariant};

        let days = transfers_history::history_days_now();
        let mut list = super::QVariantList::default();
        for (key, label) in days {
            let mut map = QMap::<QMapPair_QString_QVariant>::default();
            let key_text = QString::from(key.as_str());
            map.insert_clone(&QString::from("key"), &QVariant::from(&key_text));
            let label_text = QString::from(label.as_str());
            map.insert_clone(&QString::from("label"), &QVariant::from(&label_text));
            list.append(QVariant::from(&map));
        }
        list
    }

    /// 选中一个分组（`today` / 具体日期 / `all`）；默认只看今天。
    pub fn set_transfer_history_day(mut self: Pin<&mut Self>, day: QString) {
        let day = day.to_string();
        let day = if day.trim().is_empty() {
            HISTORY_DAY_ALL.to_owned()
        } else {
            day
        };
        let changed = transfers_history::set_history_day(&day);
        if changed {
            self.as_mut().sync_transfers();
        } else {
            self.as_mut().notify();
        }
    }

    /// 重试一条失败 / 已中断的传输记录：按原 id、原下载目录、原子目录重新入队。
    ///
    /// 文件夹总账行（读目录前整体失败或整个中断）用它落盘的目录树与子行整批重来；
    /// 断点能否接上取决于 `partials.json`（改过下载目录 = 从头下）。
    ///
    /// 顺序：取目标（记录锁）→ 查重 + 登记（调度锁）→ 成功后才删旧行 →
    /// 先提示再加入、推进队列（`advance` 的启动失败原因可以覆盖提示）。
    pub fn retry_transfer(mut self: Pin<&mut Self>, key: QString) {
        let key = key.to_string();
        let Some(job) = transfers_actions::retry_target(&key) else {
            self.as_mut().set_message("这条记录已经不能重试了", false);
            return;
        };
        let name = job.display_name();
        let source_id = job.id.clone();
        // 读目录前整体失败 / 整个中断的文件夹：用它落盘时记下的目录树整批重来。
        // 先把旧行抽出来（`take_whole_folder` 删掉整批），登记失败时再挂回去，历史不丢。
        let whole_folder = transfers_actions::is_whole_folder(&key)
            .then(|| transfers_actions::take_whole_folder(&key))
            .flatten();
        let retry_job = whole_folder
            .as_ref()
            .map(|(_, job)| job.clone())
            .unwrap_or(job);
        if queue::reserve_retry(retry_job).is_some() {
            // 整批重试时旧行已经被抽走；丢掉暂存的目录树（新批次会重新读目录）
            if let Some((batch, _)) = &whole_folder {
                transfers_actions::forget_stash(*batch);
            } else {
                transfers_actions::remove(&key);
            }
            self.as_mut()
                .set_message(&format!("已重新加入下载队列：{name}"), false);
            // advance 会 sync；线程起不来时它会用失败原因覆盖上面的提示
            self.as_mut().advance_downloads();
        } else {
            if let Some((batch, restored)) = whole_folder {
                // 登记失败：把抽走的整批放回去，免得「已中断」行凭空消失
                transfers_actions::restore_folder(batch, &restored);
                self.as_mut().sync_transfers();
            }
            // 同一个文件 id 已在排队 / 下载中：旧行不删，带用户去看已有那一行
            self.as_mut().set_message("这个条目已在下载队列中", false);
            self.as_mut()
                .locate_transfer(QString::from(source_id.as_str()));
        }
    }

    /// 打开某个文件夹批次的详情页：只有「已有子文件」的文件夹行有效。
    ///
    /// 无效 key（排队中 / 准备中 / 空文件夹）不会激活详情，
    /// QML 侧看到 `transferDetailActive` 仍为 false 就留在主列表。
    pub fn open_transfer_folder(mut self: Pin<&mut Self>, key: QString) {
        transfers_detail::select_folder(&key.to_string());
        self.as_mut().sync_transfers();
    }

    /// 关闭文件夹详情页，回到传输任务主列表。
    pub fn close_transfer_folder(mut self: Pin<&mut Self>) {
        if transfers_detail::clear_selection() {
            self.as_mut().sync_transfers();
        }
    }

    /// 在文件管理器里显示一条已完成记录的落盘文件。
    pub fn reveal_transfer(mut self: Pin<&mut Self>, key: QString) {
        // 路径从 PathBuf 直接交给后台线程，非 UTF-8 文件名不会在往返中失真
        let Some(path) = transfers_actions::path_for(&key.to_string()) else {
            self.as_mut()
                .set_message("这条记录没有可定位的落盘路径", false);
            return;
        };
        if let Err(err) = reveal::spawn(path) {
            self.as_mut()
                .set_message(&format!("无法打开文件管理器：{err}"), false);
        }
    }

    /// 推进调度器：空闲且队列非空时起下一批的线程。主线程调用。
    pub(crate) fn advance_downloads(mut self: Pin<&mut Self>) {
        let session = self.backend.session.clone();
        let failures = queue::advance(&session);
        let had_failures = !failures.is_empty();
        for failure in &failures {
            self.as_mut()
                .notify_batch_failed(&failure.name, failure.folder, &failure.reason);
        }
        if had_failures {
            // 失败不进状态栏（用户选择），只清掉可能残留的「开始下载…」
            self.as_mut().set_message("", false);
        }
        self.as_mut().sync_transfers();
    }

    /// 文件夹批次读完目录后的收尾（主线程，由下载线程回投）。
    pub(crate) fn on_folder_collected(
        mut self: Pin<&mut Self>,
        batch: u64,
        result: Result<queue_collect::Collected, Failure>,
        cancelled: bool,
        my_epoch: u64,
    ) {
        // 期间退出过登录：记录表已清空，这一批属于上个会话，只还调度槽、不碰记录。
        // 旧线程真正退出后，重新登录排队的批次才能开始。
        if my_epoch != queue::current_epoch() {
            let _ = queue::release(batch);
            self.as_mut().advance_downloads();
            return;
        }
        if let Some(collected) = self.as_mut().settle_collected(batch, result, cancelled) {
            let total = collected.files.len();
            let session = self.backend.session.clone();
            match queue::start_download(&session, batch, collected) {
                // 空文件夹没有「开始下载 0 个文件」这种提示：
                // 建完目录后由收尾写「已创建空文件夹」
                queue::StartOutcome::Started if total > 0 => self
                    .as_mut()
                    .set_message(&format!("开始下载 {total} 个文件……"), false),
                queue::StartOutcome::Started => {}
                // 线程起不来：`start_download` 已把这一批标失败并释放
                queue::StartOutcome::Failed(failure) => {
                    self.as_mut().notify_batch_failed(
                        &failure.name,
                        failure.folder,
                        &failure.reason,
                    );
                    self.as_mut().set_message("", false);
                }
                // 这一批已不是当前批次 / 期间被取消：记录已收尾，下面照常推进
                queue::StartOutcome::NotCurrent | queue::StartOutcome::Cancelled => {}
            }
        }
        // 不管这一批是开跑、失败还是空文件夹，都要让后面排队的顶上
        self.as_mut().advance_downloads();
    }

    /// 收集结果 → 记录表状态。返回收集到的目录树；`None` = 这一批到此为止。
    fn settle_collected(
        mut self: Pin<&mut Self>,
        batch: u64,
        result: Result<queue_collect::Collected, Failure>,
        cancelled: bool,
    ) -> Option<queue_collect::Collected> {
        let failure = match result {
            _ if cancelled => {
                // 用户主动取消：不弹系统通知
                transfers::cancel_batch(batch, "");
                let _ = queue::release(batch);
                self.as_mut().sync_transfers();
                return None;
            }
            // 空文件夹也是成功：目录由下载线程建好后标完成
            Ok(collected) => return Some(collected),
            Err(err) => err,
        };
        transfers::fail_batch(batch, &failure.to_string());
        let report = queue::release(batch);
        self.as_mut().on_download_failure(failure, report);
        self.as_mut().sync_transfers();
        None
    }

    /// 把记录表刷到 QML 属性（主线程调用）。
    ///
    /// 结构变化（排队 / 准备 / 开始 / 完成 / 失败 / 取消 / 清空）走这里，整个模型重建；
    /// 顺手按 2 秒节流写一次下载历史（`docs/adr/0014-download-history-on-disk.md`）。
    pub(crate) fn sync_transfers(mut self: Pin<&mut Self>) {
        let (list, active) = transfers_history::snapshot();
        let has_finished = transfers_history::has_finished();
        let live = transfers::live_snapshot();
        let (detail, detail_active) = transfers_detail::detail_snapshot();
        let (running, queued) = queue::counts();
        {
            let mut rust = self.as_mut().rust_mut();
            rust.transfers = list;
            rust.transfer_revision = rust.transfer_revision.wrapping_add(1);
            rust.transfer_live = live;
            rust.transfer_detail = detail;
            rust.transfer_detail_active = detail_active;
            rust.download_active = active;
            rust.transfer_has_finished = has_finished;
            rust.download_running = running;
            rust.download_queued = queued;
        }
        mark_live_pushed();
        maybe_write_history();
        self.as_mut().notify();
    }

    /// 只刷新进度（进度节拍用）。
    ///
    /// `transfers` 模型不动：给 ListView 的 model 赋新值会整列表重灌、
    /// 滚动位置回到顶部（2026-10-27 实测），下载期间就没法往下翻。
    pub(crate) fn sync_transfer_live(mut self: Pin<&mut Self>) {
        let live = transfers::live_snapshot();
        mark_live_pushed();
        self.as_mut().rust_mut().transfer_live = live;
        self.as_mut().notify();
    }

    /// 主线程 1Hz 心跳：没有进度回调时也让「已卡住 N 秒」继续走。
    ///
    /// QML 的 `pump()` 每 100ms 调一次，这里按 `LIVE_HEARTBEAT` 节流。
    /// 卡住时下载线程一个字节都收不到、自然没有回调；只靠 `progress` 推送的话
    /// 秒数会停在卡住的那一刻（见 `docs/pitfalls.md`）。
    ///
    /// 只碰 `transferLive`：`transfers` / `transferRevision` 一概不动，
    /// 否则整个列表会被重建、滚动位置被冲回顶部。
    pub(crate) fn maybe_heartbeat_live(mut self: Pin<&mut Self>) {
        // 进度只在下载中推送，空闲时 transferLive 是空的；用记录表兜底，
        // 免得残留值让心跳每秒空转一次。
        if self.transfer_live.is_empty() || !transfers::has_running_file() {
            return;
        }
        let now = Instant::now();
        let due = {
            let mut last = last_live_push()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let due = heartbeat_due(*last, now);
            if due {
                *last = Some(now);
            }
            due
        };
        if due {
            self.as_mut().sync_transfer_live();
        }
    }

    /// 下载相关错误的统一提示（令牌失效仍走登录失效分支）。
    fn on_download_failure(
        mut self: Pin<&mut Self>,
        failure: Failure,
        report: Option<queue::BatchReport>,
    ) {
        if matches!(&failure, Failure::Auth(_) | Failure::RateLimited { .. }) {
            // 这两类已有专门提示：限流弹配额通知，登录失效直接回登录页
            self.as_mut().on_failure(failure);
            return;
        }
        if let Some(report) = report {
            self.as_mut()
                .notify_batch_failed(&report.name, report.folder, &failure.to_string());
        }
        // 失败不进状态栏（用户选择）
        self.as_mut().set_message("", false);
    }

    /// 批次级失败通知（准备阶段失败 / 线程起不来）；失败原因不写状态栏。
    fn notify_batch_failed(self: Pin<&mut Self>, name: &str, folder: bool, reason: &str) {
        if !self.notify_enabled {
            return;
        }
        super::notifications::show_notice(&super::notifications::batch_failed(
            name, folder, reason,
        ));
    }

    /// 登出时作废在途下载；**下载历史保留**（ADR 0014）。
    ///
    /// 在途线程只置 `abandon`（停止 + 保留 part + 绝不 rename），调度槽留到
    /// 线程自己收尾；重新登录后同一文件可以入队，但旧线程退出前只显示「排队中」。
    ///
    /// 记录表**不清**：在途行先落成「已中断」（进度与重试入口都留着），再强制写盘。
    /// 旧线程退出后 `post_finished` 的 epoch 检查保证它们不再改记录。
    pub(crate) fn reset_downloads(mut self: Pin<&mut Self>) {
        queue::reset();
        // 在途的批次已经作废：记录落成「已中断」（保留重试入口），再强制写盘。
        // 旧线程退出后 `post_finished` 的 epoch 检查保证它们不再改记录。
        transfers::interrupt_active();
        flush_history();
        // 文件夹详情页的选中不跨会话；记录与「打开位置」都保留
        transfers_detail::clear_selection();
        self.as_mut().sync_transfers();
    }
}
