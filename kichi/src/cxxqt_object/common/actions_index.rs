//! 本地树索引：建库、canary 探针与索引错误提示。

use std::pin::Pin;

use cxx_qt::CxxQtType;

use super::actions::{enqueue_ui, run_in_background};
use super::qobject;
use crate::session::tree_index::sync::ProbeOutcome;
use crate::session::Failure;

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
        let changed = {
            let mut rust = self.as_mut().rust_mut();
            let changed = rust.local_index_ready != ready;
            rust.local_index_ready = ready;
            changed
        };
        if changed {
            self.as_mut().notify();
        }
    }

    /// 起一次全量建库；调用方已确认需要建库（首次 / 探针判为 suspect）。
    fn start_tree_index_build(
        mut self: Pin<&mut Self>,
        session: std::sync::Arc<crate::session::Session>,
        index: std::sync::Arc<crate::session::tree_index::TreeIndex>,
    ) {
        if self.index_building {
            return;
        }
        let cancel = std::sync::Arc::new(crate::session::download::Cancel::default());
        index.begin_build_tracking();
        let epoch = {
            let mut rust = self.as_mut().rust_mut();
            rust.index_build_epoch = rust.index_build_epoch.wrapping_add(1);
            rust.index_building = true;
            rust.index_build_cancel = Some(std::sync::Arc::clone(&cancel));
            rust.index_build_epoch
        };
        self.as_mut().set_message("正在建立本地索引……");
        let work_session = session.clone();
        let done_session = session.clone();
        let tracking_index = std::sync::Arc::clone(&index);
        run_in_background(
            move || {
                let client = work_session.drive()?;
                let mut last_reported = 0usize;
                crate::session::tree_index::sync::full_build(
                    &index,
                    &client,
                    &cancel,
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
                            qichi
                                .as_mut()
                                .set_message(&format!("正在建立本地索引（{scanned}/?）……"));
                        });
                    },
                )
            },
            move |mut qichi, result| {
                // 建库成功会用扫描快照整库替换：先把建库期间的用户写操作
                // 重放到新快照上，否则扫描后才删除 / 改名的条目会复活。
                let succeeded = matches!(
                    &result,
                    Ok(crate::session::tree_index::sync::BuildOutcome::Built(_))
                );
                let replay = tracking_index.finish_build_tracking(succeeded);
                if qichi.as_ref().rust().index_build_epoch != epoch {
                    return;
                }
                if let Err(err) = replay {
                    if !tracking_index.is_closed() {
                        done_session.record_tree_index_error(format!("本地索引写入失败：{err}"));
                    }
                }
                {
                    let mut rust = qichi.as_mut().rust_mut();
                    rust.index_building = false;
                    rust.index_build_cancel = None;
                    // 建库失败 / 取消时旧库可能仍然可用（重建场景），统一按
                    // 库的真实状态重算，而不是固定写成 true / false。
                    rust.local_index_ready = done_session.local_index_ready();
                }
                match result {
                    Ok(crate::session::tree_index::sync::BuildOutcome::Built(count)) => {
                        qichi
                            .as_mut()
                            .set_message(&format!("本地索引已建立（{count} 条）"));
                    }
                    Ok(crate::session::tree_index::sync::BuildOutcome::Cancelled) => {
                        // 登出 / 关闭开关会推进 epoch，走不到这里；能到这里的
                        // 只有用户点了底栏的「取消」。
                        qichi
                            .as_mut()
                            .set_message("已取消建立本地索引（可在设置里关闭再开启重试）");
                    }
                    Err(err) => {
                        qichi
                            .as_mut()
                            .set_message(&format!("本地索引建立失败：{err}"));
                    }
                }
                qichi.as_mut().report_index_error();
            },
        );
    }

    /// 底栏进度条上的「取消」：中止在途全量建库。
    ///
    /// 只置取消信号、不推进 epoch，让后台线程正常收尾并走完成回调
    /// （清 index_building、把提示换成「已取消」）；旧库保持原样，开关仍是
    /// 开启状态，用户之后在设置里关闭再开启即可重试。
    pub fn cancel_local_index_build(mut self: Pin<&mut Self>) {
        let cancel = self.as_mut().rust_mut().index_build_cancel.take();
        let Some(cancel) = cancel else {
            return;
        };
        cancel.cancel();
        self.as_mut().set_message("正在取消建立本地索引……");
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
