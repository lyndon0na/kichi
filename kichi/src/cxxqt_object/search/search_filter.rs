//! 搜索筛选的桥接状态：三组已勾选项 + 摘要 / 激活条件数 + 目录列表的筛选结果。
//!
//! 纯逻辑在 `kichi_core::search_filter`；这里只负责：
//! - 把 QML 传来的线名列表解析成 [`SearchFilter`] 并写回属性；
//! - 立即按新条件重算 `filteredFiles`（文件页列表 / 网格用它），不联网；
//! - 给状态行提供可直接显示的中文摘要。
//!
//! 筛选是文件页的常驻视图条件：浏览目录时直接裁剪列表，搜索时同样作用于命中；
//! 只由用户「清除筛选」或退出登录清空，切目录 / 退出搜索都不动它。
//! 每组可多选，同组取并集（见 [`SearchFilter`]）。
//!
//! **不在这里起搜索**：筛选变化后由界面复用现有防抖再跑当前关键词
//! （索引就绪走本地索引，未就绪时网络结果按已有命中重筛，不触发新的全盘扫描）。

use std::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QString, QStringList};
use kichi_core::SearchFilter;

use super::qobject;

/// 把一组筛选写进桥接属性（不 notify，调用方负责）。
pub(crate) fn store_filter(rust: &mut crate::cxxqt_object::KichiRust, filter: SearchFilter) {
    // 先按新条件裁一遍当前目录列表；列表本身（`files`）保持完整。
    let filtered = super::files::list_filtered(
        &rust.entries,
        &rust.pending_entry_ids,
        &filter,
        chrono::Utc::now().timestamp(),
    );
    rust.search_filter_types = wire_list(filter.types.iter().map(|kind| kind.as_str()));
    rust.search_filter_modifieds = wire_list(filter.modified.iter().map(|window| window.as_str()));
    rust.search_filter_sizes = wire_list(filter.sizes.iter().map(|range| range.as_str()));
    rust.search_filter_active_count = i32::try_from(filter.active_count()).unwrap_or(i32::MAX);
    rust.search_filter_summary = QString::from(filter.summary().as_str());
    rust.search_filter = filter;
    rust.filtered_files = filtered;
}

/// 清回默认筛选（清除筛选 / 退出登录时）。
pub(crate) fn reset_filter(rust: &mut crate::cxxqt_object::KichiRust) {
    store_filter(rust, SearchFilter::default());
}

fn wire_list<'a>(values: impl Iterator<Item = &'a str>) -> QStringList {
    let mut list = QStringList::default();
    for value in values {
        list.append(QString::from(value));
    }
    list
}

fn to_strings(list: &QStringList) -> Vec<String> {
    list.iter().map(|value| value.to_string()).collect()
}

impl qobject::Kichi {
    /// 设置筛选：三组都传「已勾选项」的线名列表，空列表表示该组不限制。
    ///
    /// 类型：`folder|file|video|audio|image|document|archive|other`；
    /// 修改时间：`7d|30d|1y`；大小：`lt10m|10m-100m|100m-1g|gt1g`。
    /// 同组多选取并集。认不出的值忽略，绝不因为一个错误线名让筛选不可用。
    pub fn set_search_filter(
        mut self: Pin<&mut Self>,
        types: QStringList,
        modified: QStringList,
        sizes: QStringList,
    ) {
        let filter = SearchFilter::from_wire(
            &to_strings(&types),
            &to_strings(&modified),
            &to_strings(&sizes),
        );
        {
            let mut rust = self.as_mut().rust_mut();
            store_filter(&mut rust, filter);
        }
        self.as_mut().notify();
    }

    /// 清除全部筛选（空态里的「清除筛选」）。只改状态，界面随后复用防抖重跑。
    pub fn clear_search_filters(mut self: Pin<&mut Self>) {
        {
            let mut rust = self.as_mut().rust_mut();
            reset_filter(&mut rust);
        }
        self.as_mut().notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cxxqt_object::KichiRust;
    use kichi_core::{ModifiedFilter, SizeFilter, TypeFilter};

    fn list(values: &[&str]) -> QStringList {
        wire_list(values.iter().copied())
    }

    #[test]
    fn store_filter_exposes_wire_lists_and_summary() {
        let mut rust = KichiRust::default();
        store_filter(
            &mut rust,
            SearchFilter {
                types: vec![TypeFilter::Video, TypeFilter::Document],
                modified: vec![ModifiedFilter::Last7Days],
                sizes: vec![SizeFilter::Lt10MiB],
            },
        );
        assert_eq!(
            to_strings(&rust.search_filter_types),
            vec!["video", "document"]
        );
        assert_eq!(to_strings(&rust.search_filter_modifieds), vec!["7d"]);
        assert_eq!(to_strings(&rust.search_filter_sizes), vec!["lt10m"]);
        assert_eq!(rust.search_filter_active_count, 3);
        assert_eq!(
            rust.search_filter_summary.to_string(),
            "视频、文档 · 最近 7 天 · <10 MB"
        );
    }

    #[test]
    fn reset_filter_clears_lists_count_and_summary() {
        let mut rust = KichiRust::default();
        store_filter(
            &mut rust,
            SearchFilter {
                types: vec![TypeFilter::Folder],
                modified: vec![],
                sizes: vec![SizeFilter::GiBPlus],
            },
        );
        assert_eq!(rust.search_filter_active_count, 2);
        reset_filter(&mut rust);
        assert!(rust.search_filter_types.is_empty());
        assert!(rust.search_filter_modifieds.is_empty());
        assert!(rust.search_filter_sizes.is_empty());
        assert_eq!(rust.search_filter_active_count, 0);
        assert_eq!(rust.search_filter_summary.to_string(), "");
        assert!(!rust.search_filter.is_active());
    }

    #[test]
    fn from_wire_drops_the_all_and_any_placeholders() {
        let filter = SearchFilter::from_wire(
            &to_strings(&list(&["video", "all"])),
            &to_strings(&list(&["any"])),
            &[],
        );
        assert_eq!(filter.types, vec![TypeFilter::Video]);
        assert!(filter.modified.is_empty());
        assert!(filter.sizes.is_empty());
        assert_eq!(filter.active_count(), 1);
    }
}
