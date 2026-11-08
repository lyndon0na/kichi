//! 文件浏览：目录请求的生命周期与本地索引占位。
//!
//! 目录栈只存在 Rust 侧：QML 传文件夹 id，不自己拼路径。
//! 排序（文件夹优先）属于展示策略，见 [`super::files_view`]；导航见
//! [`super::files_nav`]。本文件只负责「什么时候发请求、结果怎么落到当前目录」。

use std::pin::Pin;
use std::time::Instant;

use cxx_qt::CxxQtType;

use super::actions::run_in_background;
use super::file_action_lock::FileActionDomain;
use super::qobject;

/// 根目录在面包屑里的名字。
pub(crate) const ROOT_NAME: &str = "全部文件";

/// 面包屑的一项：目录 id（根为空串）+ 显示名。
#[derive(Debug, Clone)]
pub(crate) struct Crumb {
    pub(crate) id: String,
    pub(crate) name: String,
}

impl Crumb {
    pub(crate) fn root() -> Self {
        Self {
            id: String::new(),
            name: ROOT_NAME.to_owned(),
        }
    }
}

impl qobject::Kichi {
    /// 用户点击刷新：绕过 TTL，并在已有列表时显示手动刷新反馈。
    pub fn refresh_files(mut self: Pin<&mut Self>) {
        // 根目录手动刷新顺带校准本地索引；探针自己带冷却与单飞
        if self.crumbs.last().is_some_and(|crumb| crumb.id.is_empty()) {
            self.as_mut().probe_tree_index();
        }
        self.as_mut().refresh_files_inner(true, true);
    }

    /// 登录、写操作收尾等内部强制校准：有索引旧行时先占位，再静默校准。
    pub fn refresh_files_background(self: Pin<&mut Self>) {
        self.refresh_files_inner(true, false);
    }

    /// 写操作结束后补发期间收到的刷新请求，并保留它是否来自用户按钮。
    pub(crate) fn resume_pending_files_request(mut self: Pin<&mut Self>) {
        let (pending, manual) = {
            let mut rust = self.as_mut().rust_mut();
            let pending = rust.refresh_pending;
            let manual = rust.refresh_pending_manual;
            rust.refresh_pending = false;
            rust.refresh_pending_manual = false;
            (pending, manual)
        };
        if pending {
            self.as_mut().refresh_files_inner(true, manual);
        }
    }

    /// 页面重新可见：缓存仍新鲜就不发请求，过期也只做静默后台校准。
    pub fn ensure_files_fresh(self: Pin<&mut Self>) {
        self.refresh_files_inner(false, false);
    }

    fn refresh_files_inner(mut self: Pin<&mut Self>, force: bool, manual: bool) {
        if self.files_request_running || self.listing {
            // 列表在途时又有人要刷新（例如移动完成）：记下来，本次读完再补一次。
            // 直接 return 会让源目录停留在旧数据上。
            if force {
                let show_manual = {
                    let mut rust = self.as_mut().rust_mut();
                    // 手动刷新直接并入当前请求；内部强制校准仍需排队补一次。
                    if manual {
                        rust.refresh_pending_manual |= rust.refresh_pending;
                    } else {
                        rust.refresh_pending = true;
                    }
                    let show = manual
                        && !rust.files_manual_refreshing
                        && rust.files_cache.contains(&rust.current_dir_id.to_string());
                    if show {
                        rust.files_manual_refreshing = true;
                    }
                    show
                };
                if show_manual {
                    self.as_mut().notify();
                }
            }
            return;
        }
        if self
            .as_ref()
            .rust()
            .has_file_action_domain(FileActionDomain::Files)
        {
            // 乐观状态还没提交时不能让服务端旧快照覆盖它。
            if force {
                let show_manual = {
                    let mut rust = self.as_mut().rust_mut();
                    rust.refresh_pending = true;
                    rust.refresh_pending_manual |= manual;
                    let show = manual
                        && !rust.files_manual_refreshing
                        && rust.files_cache.contains(&rust.current_dir_id.to_string());
                    if show {
                        rust.files_manual_refreshing = true;
                    }
                    show
                };
                if show_manual {
                    self.as_mut().notify();
                }
            }
            return;
        }
        let Some(current) = self.crumbs.last().cloned() else {
            return;
        };
        let now = Instant::now();
        let has_snapshot = self.files_cache.contains(&current.id);
        if !force && !self.files_stale && self.files_cache.is_fresh(&current.id, now) {
            return;
        }
        // 本地索引可用时（建库期间不碰索引，避免主线程等建库事务的锁）：
        // - 非强制刷新且完整新鲜 → 直接渲染，0 请求；
        // - 过期 / 脏 / 未补全，以及启动 / 登录后的后台强制校准 → 先用索引
        //   里的旧行占位（不闪骨架），再后台校准；校准失败时旧行原地保留；
        // - 手动刷新仍直连网络，不拿索引旧行冒充。
        if !self.index_building && !self.search_active {
            // 首屏可能在 warmUp 打开索引之前就触发刷新，这里补一次打开；
            // 打不开就静默回落网络，用户可见的提示由建库流程统一负责。
            let _ = self.backend.session.ensure_tree_index_open();
            if !force
                && !self.files_stale
                && !has_snapshot
                && self.as_mut().show_local_index_listing(&current.id)
            {
                return;
            }
            if !manual && !has_snapshot && self.as_mut().show_stale_local_index_listing(&current.id)
            {
                self.as_mut().start_files_request(current.id, true, manual);
                return;
            }
        }
        self.as_mut()
            .start_files_request(current.id, has_snapshot, manual);
    }

    /// 本地索引里有完整且新鲜的目录快照：直接渲染（不显示加载态）。
    ///
    /// 返回 `false` 表示索引不可用 / 缺目录 / 过期，调用方继续走网络。
    fn show_local_index_listing(mut self: Pin<&mut Self>, dir_id: &str) -> bool {
        let Some(index) = self.backend.session.tree_index() else {
            return false;
        };
        let now = chrono::Utc::now().timestamp();
        let entries =
            match index.listing_if_fresh(dir_id, now, crate::session::tree_index::FRESH_TTL_SECS) {
                Ok(Some(entries)) => entries,
                Ok(None) => return false,
                Err(err) => {
                    self.backend
                        .session
                        .record_tree_index_error(format!("本地索引读取失败：{err}"));
                    return false;
                }
            };
        {
            let mut rust = self.as_mut().rust_mut();
            rust.crumb_backup = None;
            rust.files_stale = false;
            rust.files_cache
                .store(dir_id.to_owned(), &entries, Instant::now());
        }
        self.as_mut().show_cached_entries(entries);
        true
    }

    /// 索引里有该目录的旧行（过期 / 脏 / 未补全）时先渲染，随后仍会后台校准。
    ///
    /// 返回 `false` 表示索引没有可显示的条目，调用方照旧显示加载态。
    fn show_stale_local_index_listing(mut self: Pin<&mut Self>, dir_id: &str) -> bool {
        let Some(index) = self.backend.session.tree_index() else {
            return false;
        };
        if index.is_closed() {
            return false;
        }
        let entries = match index.listing_any(dir_id) {
            Ok(Some(entries)) => entries,
            Ok(None) => return false,
            Err(err) => {
                self.backend
                    .session
                    .record_tree_index_error(format!("本地索引读取失败：{err}"));
                return false;
            }
        };
        {
            let mut rust = self.as_mut().rust_mut();
            rust.crumb_backup = None;
            rust.files_stale = false;
            rust.files_cache.store_stale(dir_id.to_owned(), &entries);
        }
        self.as_mut().show_cached_entries(entries);
        true
    }

    fn start_files_request(
        mut self: Pin<&mut Self>,
        dir_id: String,
        has_snapshot: bool,
        manual: bool,
    ) {
        let parent_id = (!dir_id.is_empty()).then(|| dir_id.clone());
        let session = self.backend.session.clone();
        let (epoch, visible) = {
            let mut rust = self.as_mut().rust_mut();
            rust.files_request_running = true;
            rust.listing = !has_snapshot;
            rust.files_manual_refreshing = manual && has_snapshot;
            rust.list_epoch = rust.list_epoch.wrapping_add(1);
            (
                rust.list_epoch,
                rust.listing || rust.files_manual_refreshing,
            )
        };
        if visible {
            self.as_mut().notify();
        }

        let index_dir = dir_id.clone();
        run_in_background(
            move || {
                let entries = session.list_dir(parent_id.as_deref())?;
                // 网络成功才更新索引；索引失败不覆盖这次已经拿到的列表。
                if let Some(index) = session.tree_index() {
                    if let Err(err) = index.put_listing(&index_dir, &entries, true) {
                        // 登出关闭索引属于正常流程，不当作错误提示
                        if !index.is_closed() {
                            session.record_tree_index_error(format!("本地索引写入失败：{err}"));
                        }
                    }
                }
                Ok(entries)
            },
            move |mut qichi, result| {
                // 退出登录 / 重新登录后旧请求作废，避免把上个会话的列表贴上来
                if qichi.as_ref().rust().list_epoch != epoch {
                    return;
                }
                match result {
                    Ok(entries) => {
                        // 读到了：这一跳算成功，不再需要回滚快照
                        {
                            let mut rust = qichi.as_mut().rust_mut();
                            rust.crumb_backup = None;
                        }
                        qichi.as_mut().show_entries(entries);
                        qichi.as_mut().report_index_error();
                    }
                    Err(err) => {
                        qichi.as_mut().finish_files_request();
                        {
                            let mut rust = qichi.as_mut().rust_mut();
                            rust.files_stale = false;
                            if has_snapshot {
                                // 旧快照仍可继续显示；30 秒内不再追打失败目录。
                                rust.files_cache.defer_refresh(&dir_id, Instant::now());
                            }
                        }
                        if !has_snapshot {
                            // 首次进入都没读到：面包屑退回跳转前，别指向空路径。
                            qichi.as_mut().restore_crumbs_on_failure();
                        }
                        qichi.as_mut().on_refresh_failure(err);
                    }
                }
                // 在途期间被丢弃的刷新请求，现在补上（只补一次）
                let (again, again_manual) = {
                    let mut rust = qichi.as_mut().rust_mut();
                    let pending = rust.refresh_pending;
                    let manual = rust.refresh_pending_manual;
                    rust.refresh_pending = false;
                    rust.refresh_pending_manual = false;
                    (pending, manual)
                };
                if again {
                    qichi.as_mut().refresh_files_inner(true, again_manual);
                }
            },
        );
    }

    /// 进入当前面包屑所指目录：有快照先恢复，再按 TTL 决定是否后台校准。
    pub(crate) fn enter_current_dir(mut self: Pin<&mut Self>) {
        self.as_mut().interrupt_files_request();
        let Some(current) = self.crumbs.last().cloned() else {
            return;
        };
        let now = Instant::now();
        if let Some(entries) = self.files_cache.snapshot(&current.id) {
            {
                self.as_mut().rust_mut().crumb_backup = None;
            }
            self.as_mut().show_cached_entries(entries);
            if !self.files_stale && self.files_cache.is_fresh(&current.id, now) {
                return;
            }
        }
        self.refresh_files_inner(false, false);
    }

    /// 作废所有目录请求；导航、登录切换和本地乐观写入前调用。
    pub(crate) fn interrupt_files_request(mut self: Pin<&mut Self>) {
        let changed = {
            let mut rust = self.as_mut().rust_mut();
            rust.list_epoch = rust.list_epoch.wrapping_add(1);
            rust.refresh_pending = false;
            rust.refresh_pending_manual = false;
            let changed =
                rust.files_request_running || rust.listing || rust.files_manual_refreshing;
            rust.files_request_running = false;
            rust.listing = false;
            rust.files_manual_refreshing = false;
            changed
        };
        if changed {
            self.as_mut().notify();
        }
    }

    pub(crate) fn cancel_files_refresh_for_mutation(self: Pin<&mut Self>) {
        self.interrupt_files_request();
    }

    /// 目标目录被其它目录的写操作改变：保留旧行用于先显示，下次进入立即校准。
    ///
    /// 新建文件夹 / 复制 / 上传 / 离线任务完成没有可确定的本地行，只把目录
    /// 标脏；索引写入放后台，失败只记录错误。
    pub(crate) fn mark_file_dir_stale(mut self: Pin<&mut Self>, dir_id: &str) {
        self.as_mut().rust_mut().files_cache.mark_stale(dir_id);
        let session = self.backend.session.clone();
        let dir_id = dir_id.to_owned();
        run_in_background(
            move || {
                session.write_index(|index| index.mark_dir_dirty(&dir_id));
                Ok(())
            },
            |_, _| {},
        );
    }

    /// 回收站还原 / 资料库星标等跨目录变化：所有目录下次进入都先校准。
    pub(crate) fn invalidate_file_caches(mut self: Pin<&mut Self>) {
        {
            let mut rust = self.as_mut().rust_mut();
            rust.files_cache.mark_all_stale();
            rust.files_stale = true;
        }
        self.as_mut().notify();
    }

    fn finish_files_request(mut self: Pin<&mut Self>) {
        {
            let mut rust = self.as_mut().rust_mut();
            rust.files_request_running = false;
            rust.listing = false;
            rust.files_manual_refreshing = false;
        }
    }
}

// 条目映射与排序的实现在 [`super::files_view`]，这里重导出以保持
// `files::entry_map` / `files::sort_entries` 等调用点不变。
pub(crate) use super::files_view::{entry_map, list_from_with_pending, sort_entries};
