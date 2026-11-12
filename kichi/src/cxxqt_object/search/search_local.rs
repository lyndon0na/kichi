//! 搜索桥接：输入阶段只筛当前目录，回车 / 搜索按钮才全盘。
//!
//! 输入阶段和没有索引时完全一致：只筛当前目录已加载条目（0 请求）。
//! `searchAll` 才发起全盘搜索：索引就绪读本地库（0 请求），未就绪逐页扫网盘。
//!
//! 线程纪律：
//! - 单飞：最多一个索引查询在途，期间的新关键词只保留最后一个；
//! - 结果经 `run_in_background` 回主线程；`search_epoch` 让退出 / 登出后的
//!   旧结果直接作废；
//! - 后台线程不碰 Qt，也不拿索引以外的锁。

use std::pin::Pin;

use chrono::Utc;
use cxx_qt::CxxQtType;
use cxx_qt_lib::{QString, QVariant};

use super::actions::run_in_background;
use super::qobject;
use super::search::{hit_map, SEARCH_LIST_LIMIT};
use super::QVariantList;
use crate::session::download::Cancel;
use crate::session::search::filter_entries;
use crate::session::{Failure, IndexSearchResult, Session};

/// 本地索引是否可以承担全盘搜索：已打开、未关闭、且成功建过库。
pub(crate) fn local_index_ready(session: &Session) -> bool {
    session.local_index_ready()
}

/// 输入阶段的状态行：索引就绪也仍然只筛当前目录，并提示回车可搜全盘；
/// 未就绪 / 关闭时沿用原来的原因说明。
fn local_scope_hint(session: &Session) -> &'static str {
    if local_index_ready(session) {
        "只筛了当前目录（回车可搜整个网盘）"
    } else if session.local_index_enabled() {
        "本地索引尚未就绪，只筛了当前目录"
    } else {
        "本地索引已关闭，只筛了当前目录（可在设置中开启）"
    }
}

impl qobject::Kichi {
    /// 搜索框每次有效输入都会来这里。
    pub fn search_local(mut self: Pin<&mut Self>, query: QString) {
        let raw = query.to_string();
        let trimmed = raw.trim().to_owned();
        if trimmed.is_empty() {
            // 清空关键词 = 回到当前目录（Esc 的第一段就是这么来的）
            self.as_mut().exit_search();
            return;
        }
        // 已经看完全盘结果、只是改了筛选（关键词没变）：按当前条件重筛，
        // 不重新起一轮扫描。网络结果直接用已收集的命中（ADR 0004）；
        // 索引结果因为筛选下推到了 SQL，必须重跑查询。
        let source = self.search_source.to_string();
        let refilter_full = self.search_active
            && self.search_query.to_string().trim() == trimmed
            && (source == "network" || source == "index");
        {
            let mut rust = self.as_mut().rust_mut();
            rust.search_active = true;
            rust.search_query = QString::from(raw.as_str());
        }
        if refilter_full {
            if source == "index" && local_index_ready(&self.backend.session) {
                self.as_mut().start_local_index_search(trimmed);
            } else if source == "network" {
                self.as_mut().refilter_network_hits();
            } else {
                let hint = local_scope_hint(&self.backend.session);
                self.as_mut().run_local_fallback(&trimmed, hint);
            }
            return;
        }
        // 改关键词 = 和没有索引时一样：只筛当前目录，不碰网盘。
        let hint = local_scope_hint(&self.backend.session);
        self.as_mut().run_local_fallback(&trimmed, hint);
    }

    /// 起一轮本地索引全盘搜索（`searchAll` 在索引就绪时走这里）。
    pub(crate) fn start_local_index_search(mut self: Pin<&mut Self>, query: String) {
        let session = self.backend.session.clone();
        if !local_index_ready(&session) {
            let hint = local_scope_hint(&session);
            self.as_mut().run_local_fallback(&query, hint);
            return;
        }

        // 后台用发起时的筛选与排序列；完成时若已经变了，会按新条件重跑。
        let filter = self.search_filter.clone();
        let sort_field = self.sort_field;
        let sort_ascending = self.sort_ascending;
        let mut pending_only = false;
        let epoch = {
            let mut rust = self.as_mut().rust_mut();
            rust.search_active = true;
            rust.search_query = QString::from(query.as_str());
            rust.search_source = QString::from("index");
            rust.search_status = QString::from("正在本地索引里搜索……");
            rust.status_message = QString::default();
            if rust.search_local_running {
                // 单飞：只保留最后一个关键词，本轮结束后接着搜
                pending_only = true;
                rust.search_pending = Some(query.clone());
                rust.search_epoch
            } else {
                rust.search_local_running = true;
                rust.search_pending = None;
                rust.search_settled = false;
                rust.search_scanned = 0;
                rust.search_epoch = rust.search_epoch.wrapping_add(1);
                if let Some(previous) = rust.search_cancel.take() {
                    previous.cancel();
                }
                rust.search_cancel = Some(std::sync::Arc::new(Cancel::default()));
                rust.search_epoch
            }
        };
        self.as_mut().notify();
        if pending_only {
            return;
        }
        // 搜索路径不阻塞探针：超过 5 分钟没对账就在后台补一次
        self.as_mut().maybe_probe_from_search();

        let worker_query = query.clone();
        let worker_filter = filter.clone();
        run_in_background(
            move || {
                session.search_index_local(
                    &worker_query,
                    &worker_filter,
                    sort_field,
                    sort_ascending,
                )
            },
            move |mut qichi, result| {
                qichi.as_mut().finish_local_search(
                    epoch,
                    query,
                    filter,
                    (sort_field, sort_ascending),
                    result,
                )
            },
        );
    }

    /// 一轮索引查询结束：应用结果，或接着跑在途期间积压的关键词 /
    /// 在途期间改过的筛选、排序。
    fn finish_local_search(
        mut self: Pin<&mut Self>,
        epoch: u64,
        query: String,
        used_filter: kichi_core::SearchFilter,
        used_sort: (crate::settings::SortField, bool),
        result: Result<IndexSearchResult, Failure>,
    ) {
        {
            let this = self.as_ref();
            let rust = this.rust();
            // 停止 / 退出搜索 / 登出后的旧结果直接丢弃
            if rust.search_epoch != epoch || !rust.search_active {
                return;
            }
        }
        let next = {
            let mut rust = self.as_mut().rust_mut();
            rust.search_local_running = false;
            rust.search_cancel = None;
            rust.search_pending
                .take()
                .filter(|pending| pending != &query)
        };
        if let Some(next) = next {
            self.as_mut().start_local_index_search(next);
            return;
        }
        // 关键词没变但筛选 / 排序变了：按当前条件重跑，别把旧结果留在界面上。
        if self.search_filter != used_filter || (self.sort_field, self.sort_ascending) != used_sort
        {
            self.as_mut().start_local_index_search(query);
            return;
        }
        match result {
            Ok(result) => self.as_mut().apply_index_results(&query, result),
            Err(err) => {
                let reason = format!("本地索引读取失败（{err}），只筛了当前目录");
                self.as_mut().run_local_fallback(&query, &reason);
            }
        }
    }

    /// 把索引结果铺到界面：全部命中留在 `search_index`，只显示前 500 行。
    fn apply_index_results(mut self: Pin<&mut Self>, query: &str, result: IndexSearchResult) {
        // 输入框已经换了关键词、防抖还没触发时，旧结果不要再闪一下
        if self.search_query.to_string().trim() != query {
            return;
        }
        let total = result.hits.len();
        let mut rows = QVariantList::default();
        for hit in result.hits.iter().take(SEARCH_LIST_LIMIT) {
            rows.append(QVariant::from(&hit_map(hit)));
        }
        let built = built_time_suffix(result.built_at);
        let status = if result.trash_hits > 0 {
            format!(
                "本地索引命中 {total} 条（含回收站 {} 条）{built}",
                result.trash_hits
            )
        } else {
            format!("本地索引命中 {total} 条{built}")
        };
        {
            let mut rust = self.as_mut().rust_mut();
            rust.search_active = true;
            rust.search_source = QString::from("index");
            rust.search_settled = true;
            rust.search_scanned = 0;
            rust.search_hits = i32::try_from(total).unwrap_or(i32::MAX);
            rust.search_displayed = total.min(SEARCH_LIST_LIMIT);
            rust.search_truncated = total > SEARCH_LIST_LIMIT;
            rust.search_status = QString::from(status.as_str());
            rust.search_index = result
                .hits
                .into_iter()
                .map(|hit| (hit.id().to_owned(), hit))
                .collect();
            rust.search_results = rows;
            rust.status_message = QString::default();
        }
        self.as_mut().notify();
    }

    /// 索引不可用时的降级：只筛当前目录已加载的条目（0 请求）。
    fn run_local_fallback(mut self: Pin<&mut Self>, query: &str, reason: &str) {
        let now = Utc::now().timestamp();
        let matched = filter_entries(&self.entries, query, &self.search_filter, now);
        let count = matched.len();
        let current = self.entries.len();
        let filtered = self.search_filter.is_active();
        let status = if count == 0 {
            let suffix = if filtered {
                "；当前筛选条件下没有匹配"
            } else {
                ""
            };
            format!(
                "{reason}：本目录已加载的 {current} 项里没有名称包含「{query}」的条目{suffix}（没有请求网盘）"
            )
        } else {
            let suffix = if filtered {
                "，已按筛选过滤"
            } else {
                ""
            };
            format!("{reason}：命中 {count} 项{suffix}（本目录已加载 {current} 项，没有请求网盘）")
        };
        {
            let mut rust = self.as_mut().rust_mut();
            // 在途的索引查询 / 旧的全盘扫描整轮作废
            rust.search_epoch = rust.search_epoch.wrapping_add(1);
            if let Some(cancel) = rust.search_cancel.take() {
                cancel.cancel();
            }
            rust.search_active = true;
            rust.search_query = QString::from(query);
            rust.search_local_running = false;
            rust.search_pending = None;
            rust.search_index.clear();
            rust.search_source = QString::from("fallback");
        }
        self.as_mut().apply_local_filter(matched, &status);
    }
}

/// 「；索引更新于 2026-11-04 12:34」；没有建库时间时为空串。
fn built_time_suffix(built_at: Option<i64>) -> String {
    let Some(seconds) = built_at else {
        return String::new();
    };
    match chrono::DateTime::from_timestamp(seconds, 0) {
        Some(time) => format!(
            "；索引更新于 {}",
            time.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M")
        ),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_time_is_optional_and_formatted() {
        assert_eq!(built_time_suffix(None), "");
        assert_eq!(built_time_suffix(Some(i64::MAX)), "", "越界时间不编造日期");
        let text = built_time_suffix(Some(1_700_000_000));
        assert!(
            text.starts_with("；索引更新于 20"),
            "状态行要说明索引有多新：{text}"
        );
    }
}
