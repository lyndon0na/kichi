//! 「全局搜索」域: 搜索模式状态、结果收敛与分页。
//!
//! 搜索走服务端 `Cmd::SearchFiles`, 结果不落目录缓存, 而是单独一份列表;
//! 搜索模式下文件页以这份列表为数据源(见 `mod.rs::visible_rows`)。

use std::collections::HashSet;

use kichi_core::types::{File, FileList};

use crate::msg::Cmd;

use super::global::Global;

/// 搜索状态(box 关键词 / 结果 / 分页 / 请求防串)。
#[derive(Default)]
pub(crate) struct SearchPage {
    /// 是否处于搜索模式(搜索模式下显示搜索结果而非当前目录)。
    mode: bool,
    /// 当前搜索关键词。
    keyword: String,
    /// 搜索结果。
    results: Vec<File>,
    /// 搜索结果分页 token; None = 已到底。
    next: Option<String>,
    /// 搜索是否进行中(首页或加载更多)。
    loading: bool,
    /// 当前搜索请求 id, 用于丢弃乱序的旧响应。
    req_id: u64,
}

impl SearchPage {
    /// 是否处于搜索模式。
    pub(crate) fn is_active(&self) -> bool {
        self.mode
    }

    /// 当前搜索关键词。
    pub(crate) fn keyword(&self) -> &str {
        &self.keyword
    }

    /// 搜索结果。
    pub(crate) fn results(&self) -> &[File] {
        &self.results
    }

    /// 搜索是否进行中。
    pub(crate) fn is_loading(&self) -> bool {
        self.loading
    }

    /// 是否还有下一页。
    pub(crate) fn has_more(&self) -> bool {
        self.next.is_some()
    }

    /// 搜索结果是否为空。
    pub(crate) fn is_empty(&self) -> bool {
        self.results.is_empty()
    }

    /// 以 `keyword` 触发一次新搜索(调用方保证关键词非空)。
    pub(crate) fn start(&mut self, g: &mut Global, keyword: String) {
        self.mode = true;
        self.keyword = keyword.clone();
        self.results.clear();
        self.next = None;
        self.loading = true;
        self.req_id += 1;
        tracing::info!("发送搜索命令: req_id={}", self.req_id);
        g.send(Cmd::SearchFiles {
            keyword,
            token: None,
            append: false,
            req_id: self.req_id,
        });
    }

    /// 退出搜索模式。
    pub(crate) fn exit(&mut self) {
        self.mode = false;
        self.keyword.clear();
        self.results.clear();
        self.next = None;
        self.loading = false;
    }

    /// 加载更多搜索结果(正在加载或已到底时忽略)。
    pub(crate) fn load_more(&mut self, g: &mut Global) {
        if self.loading || self.next.is_none() {
            return;
        }
        self.loading = true;
        let token = self.next.clone().unwrap();
        g.send(Cmd::SearchFiles {
            keyword: self.keyword.clone(),
            token: Some(token),
            append: true,
            req_id: self.req_id,
        });
    }

    /// 搜索结果响应(req_id 不匹配则丢弃)。返回 true 表示这是一次新搜索的首页,
    /// 调用方需据此重置缩略图缓存。
    pub(crate) fn on_results(&mut self, req_id: u64, append: bool, list: FileList) -> bool {
        if req_id != self.req_id {
            return false;
        }
        self.loading = false;
        self.next = list.next_page_token;
        if append {
            let known: HashSet<String> = self.results.iter().map(|f| f.id.clone()).collect();
            for f in list.files {
                if !known.contains(&f.id) {
                    self.results.push(f);
                }
            }
            false
        } else {
            self.results = list.files;
            true
        }
    }

    /// 搜索失败。
    pub(crate) fn on_failed(&mut self, g: &mut Global, what: String) {
        self.loading = false;
        g.toast_err(&what);
    }
}
