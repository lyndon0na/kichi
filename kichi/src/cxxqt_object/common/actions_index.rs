//! 本地树索引：建库、canary 探针与索引错误提示。

use std::pin::Pin;
use std::sync::{Arc, Mutex};

use cxx_qt::CxxQtType;
use cxx_qt_lib::QString;

use super::actions::{enqueue_ui, run_in_background};
use super::qobject;
use crate::session::search::{ScanBuildReport, ScanTapSnapshot};
use crate::session::tree_index::sync::ProbeOutcome;
use crate::session::{Failure, ScanEnd};
use kichi_core::SearchFilter;

/// 全盘扫描期间共享给后台线程的搜索条件。
///
/// 后台每页读一次；`revision` 变了就重新匹配已收集的整份数据。界面退出搜索
/// 时只把 `active` 关掉，建库继续跑。
pub(crate) struct ScanTap {
    pub active: bool,
    pub query: String,
    pub filter: SearchFilter,
    pub revision: u64,
}

impl ScanTap {
    pub(crate) fn inactive() -> Self {
        Self {
            active: false,
            query: String::new(),
            filter: SearchFilter::default(),
            revision: 0,
        }
    }

    pub(crate) fn snapshot(&self) -> ScanTapSnapshot {
        ScanTapSnapshot {
            active: self.active,
            query: self.query.clone(),
            filter: self.filter.clone(),
            revision: self.revision,
        }
    }

    pub(crate) fn activate(&mut self, query: String, filter: SearchFilter) {
        if self.active && self.query == query && self.filter == filter {
            return;
        }
        self.active = true;
        self.query = query;
        self.filter = filter;
        self.revision = self.revision.wrapping_add(1);
    }

    pub(crate) fn deactivate(&mut self) {
        if self.active {
            self.active = false;
            self.revision = self.revision.wrapping_add(1);
        }
    }
}

impl qobject::Kichi {
    pub(crate) fn report_index_error(mut self: Pin<&mut Self>) {
        if self.index_error_reported {
            return;
        }
        let Some(message) = self.backend.session.take_tree_index_error() else {
            return;
        };
        self.as_mut().rust_mut().index_error_reported = true;
        self.as_mut().set_message(&message);
    }

    /// 登录 / 启动恢复后调用：库为空或版本不认识时后台建一次全量索引；
    /// 已建好则顺手做一次 canary 探针。
    ///
    /// 建库失败只在底栏提示一次，不影响目录浏览（索引只是缓存，网络路径照常）。
    pub(crate) fn ensure_tree_index_build(mut self: Pin<&mut Self>) {
        if self.index_building {
            return;
        }
        self.as_mut().report_index_error();
        let session = self.backend.session.clone();
        let index = match session.ensure_tree_index_open() {
            Ok(index) => index,
            Err(_) => {
                self.as_mut().refresh_local_index_ready();
                self.as_mut().report_index_error();
                return;
            }
        };
        match index.needs_full_build() {
            Ok(true) => {}
            Ok(false) => {
                self.as_mut().refresh_local_index_ready();
                // 索引已建好：登录 / 启动恢复顺手对一次账（探针自带冷却与单飞）
                self.as_mut().probe_tree_index();
                return;
            }
            Err(err) => {
                session.record_tree_index_error(format!("本地索引读取失败：{err}"));
                self.as_mut().refresh_local_index_ready();
                self.as_mut().report_index_error();
                return;
            }
        }
        self.as_mut().refresh_local_index_ready();
        self.as_mut().start_tree_index_build(session, index);
    }

    /// 把「本地索引是否就绪」同步到 QML 可见属性；值没变就不发通知。
    ///
    /// 就绪是派生状态（库是否成功建过），只在打开 / 建库收尾 / 登出 / 开关
    /// 切换这些时点重算，避免每次 `notify` 都去查一次库。
    pub(crate) fn refresh_local_index_ready(mut self: Pin<&mut Self>) {
        let ready = self.backend.session.local_index_ready();
        let built_at = self
            .backend
            .session
            .index_built_at()
            .and_then(|seconds| chrono::DateTime::from_timestamp(seconds, 0))
            .map(|time| {
                time.with_timezone(&chrono::Local)
                    .format("%Y-%m-%d %H:%M")
                    .to_string()
            })
            .unwrap_or_default();
        let changed = {
            let mut rust = self.as_mut().rust_mut();
            let changed = rust.local_index_ready != ready
                || rust.local_index_built_at.to_string() != built_at;
            rust.local_index_ready = ready;
            rust.local_index_built_at = QString::from(built_at.as_str());
            changed
        };
        if changed {
            self.as_mut().notify();
        }
    }

    /// 起一次全量建库；调用方已确认需要建库（首次 / 手动重建 / 探针 suspect）。
    ///
    /// 如果搜索已经把关键词挂上来，这一次遍历同时回投命中；否则只建库。
    /// 没有「取消」入口：登出、关闭索引、退出应用会通过 `index_build_cancel`
    /// 中止，旧库保持原样。
    pub(crate) fn start_tree_index_build(
        mut self: Pin<&mut Self>,
        session: Arc<crate::session::Session>,
        index: Arc<crate::session::tree_index::TreeIndex>,
    ) {
        if self.index_building {
            return;
        }
        let tap = self
            .index_scan_tap
            .clone()
            .unwrap_or_else(|| Arc::new(Mutex::new(ScanTap::inactive())));
        let cancel = Arc::new(crate::session::download::Cancel::default());
        let epoch = {
            let mut rust = self.as_mut().rust_mut();
            rust.index_scan_tap = Some(Arc::clone(&tap));
            rust.index_build_epoch = rust.index_build_epoch.wrapping_add(1);
            rust.index_building = true;
            rust.index_build_cancel = Some(Arc::clone(&cancel));
            rust.index_build_scanned = 0;
            rust.index_build_epoch
        };
        self.as_mut().set_message("正在建立本地索引……");
        let work_session = session.clone();
        run_in_background(
            move || {
                let mut last_reported = 0usize;
                work_session.scan_and_build(
                    &index,
                    &cancel,
                    &mut || tap.lock().ok().map(|tap| tap.snapshot()),
                    &mut |scanned| {
                        if scanned == last_reported {
                            return;
                        }
                        last_reported = scanned;
                        // 全盘列举不给总数，进度只能报已扫描条数。
                        enqueue_ui(move |mut qichi| {
                            if qichi.as_ref().rust().index_build_epoch != epoch {
                                return;
                            }
                            qichi.as_mut().rust_mut().index_build_scanned =
                                i32::try_from(scanned).unwrap_or(i32::MAX);
                            qichi
                                .as_mut()
                                .set_message(&format!("正在扫描并建立本地索引（{scanned}/?）……"));
                        });
                    },
                    &mut |progress| {
                        enqueue_ui(move |mut qichi| {
                            qichi.as_mut().apply_scan_progress(epoch, progress);
                        });
                    },
                )
            },
            move |mut qichi, result| qichi.as_mut().finish_index_scan(epoch, result),
        );
    }

    /// 设置页的「建立 / 重建索引」：开关没开就先打开，再从空库或断点建。
    pub fn build_local_index(mut self: Pin<&mut Self>, rebuild: bool) {
        if self.index_building {
            return;
        }
        if !self.backend.session.local_index_enabled() {
            self.as_mut().enable_local_index_for_search();
        }
        let session = self.backend.session.clone();
        let index = match session.ensure_tree_index_open() {
            Ok(index) => index,
            Err(_) => {
                self.as_mut().refresh_local_index_ready();
                self.as_mut().report_index_error();
                return;
            }
        };
        if !rebuild {
            match index.needs_full_build() {
                Ok(true) => {}
                Ok(false) => {
                    self.as_mut().refresh_local_index_ready();
                    self.as_mut().probe_tree_index();
                    return;
                }
                Err(err) => {
                    session.record_tree_index_error(format!("本地索引读取失败：{err}"));
                    self.as_mut().refresh_local_index_ready();
                    self.as_mut().report_index_error();
                    return;
                }
            }
        }
        self.as_mut().refresh_local_index_ready();
        self.as_mut().start_tree_index_build(session, index);
    }

    /// 全盘搜索首次确认开启索引：只打开开关并落盘，不另起一次建库；
    /// 调用方紧接着会启动「扫描兼建库」，同一次遍历负责回投结果和建库。
    pub(crate) fn enable_local_index_for_search(mut self: Pin<&mut Self>) {
        if self.backend.session.local_index_enabled() {
            return;
        }
        self.backend.session.set_local_index_enabled(true);
        self.as_mut().update_settings(|settings| {
            settings.local_index_enabled = true;
        });
        self.as_mut().rust_mut().index_error_reported = false;
        self.as_mut().refresh_local_index_ready();
    }

    /// 一轮「扫描兼建库」收尾：提交状态、处理失败，搜索还在时做一次本地查询
    /// 把最终结果（含扫描末尾才变化的筛选 / 排序）收敛成同一份。
    pub(crate) fn finish_index_scan(
        mut self: Pin<&mut Self>,
        epoch: u64,
        result: Result<ScanBuildReport, Failure>,
    ) {
        if self.index_build_epoch != epoch {
            return;
        }
        let (search_active, query) = {
            let this = self.as_ref();
            let rust = this.rust();
            (rust.search_active, rust.search_query.to_string())
        };
        {
            let mut rust = self.as_mut().rust_mut();
            rust.index_building = false;
            rust.index_build_cancel = None;
            rust.index_scan_tap = None;
            rust.index_build_scanned = 0;
        }
        self.as_mut().refresh_local_index_ready();

        match result {
            Ok(report) => match report.end {
                ScanEnd::Finished => match report.built {
                    Some(Ok(count)) => {
                        if search_active {
                            self.as_mut().start_local_index_search(query);
                        }
                        self.as_mut()
                            .set_message(&format!("本地索引已建立（{count} 条）"));
                    }
                    Some(Err(err)) => {
                        let message = format!("本地索引建立失败：{err}");
                        self.as_mut().set_message(&message);
                        self.as_mut().settle_search(format!(
                            "全盘扫描完成，但{message}；当前结果只来自已扫条目"
                        ));
                    }
                    None => {
                        self.as_mut()
                            .set_message("全盘扫描完成，但本次没有提交本地索引");
                        self.as_mut()
                            .settle_search("全盘扫描完成，本次没有提交本地索引".to_owned());
                    }
                },
                ScanEnd::Stopped => {
                    self.as_mut()
                        .settle_search("扫描已取消，本次没有提交本地索引".to_owned());
                }
                ScanEnd::BrokenPaging => {
                    let message = "官方分页异常（下一页标记原地打转），扫描已停止，结果可能不全";
                    self.as_mut().set_message(message);
                    self.as_mut().settle_search(message.to_owned());
                }
            },
            Err(err) => {
                let reason = err.to_string();
                let message = format!("全盘扫描中断：{reason}");
                self.as_mut().settle_search(format!(
                    "扫描中断：{reason}（已扫出的结果保留，结果可能不全）"
                ));
                if matches!(err, Failure::Auth(_)) {
                    self.as_mut().on_failure(err);
                } else {
                    self.as_mut().set_message(&message);
                }
            }
        }
        self.as_mut().report_index_error();
        self.as_mut().notify();
    }

    /// 窗口重新获得焦点：触发一次 canary 探针（冷却 / 单飞都在探针入口挡）。
    pub fn on_window_activated(mut self: Pin<&mut Self>) {
        if self.auth_state.to_string() != "signedIn" {
            return;
        }
        self.as_mut().probe_tree_index();
    }

    /// canary 探针入口：60 秒冷却 + 单飞，建库期间绝不读索引。
    ///
    /// 网络 / 接口失败只记录一次尝试（让冷却照样生效），不动节点、不触发重建。
    pub(crate) fn probe_tree_index(mut self: Pin<&mut Self>) {
        if self.index_building || self.index_probing {
            return;
        }
        if self.auth_state.to_string() != "signedIn" {
            return;
        }
        let session = self.backend.session.clone();
        let index = match session.tree_index() {
            Some(index) if !index.is_closed() => index,
            _ => return,
        };
        match index.needs_full_build() {
            Ok(false) => {}
            // 没建过库 / 读失败：交给 ensure_tree_index_build，探针不抢活
            _ => return,
        }
        let now = chrono::Utc::now().timestamp();
        match index.last_probe_at() {
            Ok(Some(last))
                if now.saturating_sub(last) < crate::session::tree_index::PROBE_COOLDOWN_SECS =>
            {
                return;
            }
            Ok(_) => {}
            Err(_) => return,
        }
        let cancel = std::sync::Arc::new(crate::session::download::Cancel::default());
        let epoch = {
            let mut rust = self.as_mut().rust_mut();
            // 冷却检查期间可能正好开始建库：起线程前再确认一次
            if rust.index_building {
                return;
            }
            rust.index_probe_epoch = rust.index_probe_epoch.wrapping_add(1);
            rust.index_probing = true;
            rust.index_probe_cancel = Some(std::sync::Arc::clone(&cancel));
            rust.index_probe_epoch
        };
        let done_session = session.clone();
        run_in_background(
            move || {
                let client = session.drive()?;
                crate::session::tree_index::sync::run_canary(&index, &client, &cancel)
            },
            move |mut qichi, result| {
                if qichi.as_ref().rust().index_probe_epoch != epoch {
                    return;
                }
                {
                    let mut rust = qichi.as_mut().rust_mut();
                    rust.index_probing = false;
                    rust.index_probe_cancel = None;
                }
                match result {
                    Ok(outcome) => qichi.as_mut().apply_probe_outcome(outcome),
                    Err(err) => {
                        // 失败只记录尝试（冷却生效），节点与目录快照都不动
                        if let Ok(index) = done_session.ensure_tree_index_open() {
                            let now = chrono::Utc::now().timestamp();
                            if let Err(write) = index.note_probe_attempt(now) {
                                if !index.is_closed() {
                                    done_session.record_tree_index_error(format!(
                                        "本地索引写入失败：{write}"
                                    ));
                                }
                            }
                        }
                        // 登录态失效要切回登录页；其余失败不打断浏览
                        if matches!(err, Failure::Auth(_)) {
                            qichi.as_mut().on_failure(err);
                        }
                    }
                }
            },
        );
    }

    /// 搜索路径：探针超过 5 分钟没跑过就在后台补一次，不阻塞本次搜索。
    pub(crate) fn maybe_probe_from_search(mut self: Pin<&mut Self>) {
        if self.index_building || self.index_probing {
            return;
        }
        let Some(index) = self.backend.session.tree_index() else {
            return;
        };
        if index.is_closed() {
            return;
        }
        let stale = match index.last_probe_at() {
            Ok(Some(last)) => {
                chrono::Utc::now().timestamp().saturating_sub(last)
                    >= crate::session::tree_index::PROBE_SEARCH_STALE_SECS
            }
            Ok(None) => true,
            Err(_) => false,
        };
        if stale {
            self.as_mut().probe_tree_index();
        }
    }

    /// 应用探针结论：只有 suspect 且过了冷却才启动全量重建。
    fn apply_probe_outcome(mut self: Pin<&mut Self>, outcome: ProbeOutcome) {
        match outcome {
            ProbeOutcome::Fresh { .. } | ProbeOutcome::Patched { .. } | ProbeOutcome::Cancelled => {
            }
            ProbeOutcome::Unavailable { first_time, .. } => {
                // 只提示一次，避免每次窗口获得焦点都弹同一句
                if first_time {
                    self.as_mut().set_message(
                        "网盘「最近修改」页的排序与预期不符，本地索引暂不做自动校准（目录仍会按需刷新）",
                    );
                }
            }
            ProbeOutcome::Suspect { reason, rebuild } => {
                if !rebuild {
                    return;
                }
                let message = format!(
                    "检测到网盘内容与本地索引不一致（{}），正在后台重建本地索引……",
                    reason.as_str()
                );
                self.as_mut().set_message(&message);
                let session = self.backend.session.clone();
                let index = match session.tree_index() {
                    Some(index) if !index.is_closed() => index,
                    _ => return,
                };
                self.as_mut().start_tree_index_build(session, index);
            }
        }
    }
}
