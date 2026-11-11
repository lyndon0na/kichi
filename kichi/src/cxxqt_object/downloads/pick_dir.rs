//! 「下载到…」：每次下载临时选一个本地目录。
//!
//! 只对**本次下载**生效：选中的目录作为批次快照交给调度器，不写回全局
//! `downloadDir` 设置。目录选择器走 XDG Portal（见 [`super::folder_picker`]），
//! 在后台线程里等结果，回投主线程后才登记批次，不阻塞界面。
//!
//! 已经在排队 / 准备 / 下载 / 暂停的条目沿用「下载」项的语义：单条只跳去
//! 传输页（像「已在下载队列中」），多选跳过并在汇总里说明，绝不重复入队。
//! 多选只弹一次选择器，整个选择共用该目录；每个条目仍各自一批。

use std::path::{Path, PathBuf};
use std::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QString, QStringList};

use kichi_core::FileEntry;

use super::file_optimistic::is_unconfirmed_create;
use super::qobject;
use super::queue;
use crate::session::Failure;

/// 弹选择器之前对选中条目的分类。
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct PickedTargets {
    /// 真正会入队的条目：保留打开菜单时的快照，选择器开着时用户翻目录也不丢
    pub(crate) entries: Vec<FileEntry>,
    /// 已在排队 / 准备 / 下载 / 暂停
    pub(crate) already: u32,
    /// 已经不在当前列表 / 搜索结果里
    pub(crate) missing: u32,
    /// 临时文件夹还没有真实云端 id
    pub(crate) syncing: u32,
}

/// 登记之后的结果（与 [`PickedTargets`] 的含义一致，`added` 是新建的批次数）。
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct PickedOutcome {
    pub(crate) added: u32,
    pub(crate) already: u32,
    pub(crate) missing: u32,
    pub(crate) syncing: u32,
}

/// 把右侧菜单交来的 id 分成「可入队 / 已在队列 / 不在了 / 等真实 id」。
///
/// `find` / `is_pending` / `is_syncing` 由调用方提供，纯函数便于单测。
pub(crate) fn classify_targets(
    ids: impl IntoIterator<Item = String>,
    mut find: impl FnMut(&str) -> Option<FileEntry>,
    mut is_pending: impl FnMut(&str) -> bool,
    mut is_syncing: impl FnMut(&str) -> bool,
) -> PickedTargets {
    let mut targets = PickedTargets::default();
    for raw in ids {
        let id = raw.trim();
        if id.is_empty() {
            continue;
        }
        if is_syncing(id) {
            targets.syncing += 1;
            continue;
        }
        let Some(entry) = find(id) else {
            targets.missing += 1;
            continue;
        };
        if is_pending(id) {
            targets.already += 1;
            continue;
        }
        targets.entries.push(entry);
    }
    targets
}

/// 按同一个目录登记所有条目：每个条目各自一批。
///
/// `reserve` 返回 `false` 表示登记时发现它已经在队列里（选择器开着期间
/// 被别的入口排上了），计进 `already`，不重复入队。
pub(crate) fn enqueue_targets(
    targets: PickedTargets,
    dir: &Path,
    mut reserve: impl FnMut(FileEntry, &Path) -> bool,
) -> PickedOutcome {
    let PickedTargets {
        entries,
        already,
        missing,
        syncing,
    } = targets;
    let mut outcome = PickedOutcome {
        added: 0,
        already,
        missing,
        syncing,
    };
    for entry in entries {
        if reserve(entry, dir) {
            outcome.added += 1;
        } else {
            outcome.already += 1;
        }
    }
    outcome
}

/// 「下载到…」入队后的底栏汇总。
///
/// 一到情况一句话：新建几项、跳过几项（已在队列 / 不在了 / 等真实 id）。
pub(crate) fn summary_message(added: u32, already: u32, missing: u32, syncing: u32) -> String {
    let syncing_text = format!("{syncing} 个文件夹正在同步真实 id，请稍后再下载");
    if added == 0 {
        let mut parts = Vec::new();
        if already > 0 {
            parts.push("这些条目已在下载队列中".to_string());
        }
        if missing > 0 {
            parts.push(format!("选中的 {missing} 项已不在当前列表中，请刷新后重试"));
        }
        if syncing > 0 {
            parts.push(syncing_text);
        }
        return parts.join("；");
    }
    let mut skipped = Vec::new();
    if already > 0 {
        skipped.push(format!("{already} 项已在队列中"));
    }
    if missing > 0 {
        skipped.push(format!("{missing} 项已不在当前列表中"));
    }
    if syncing > 0 {
        skipped.push(syncing_text);
    }
    if skipped.is_empty() {
        format!("已加入下载队列：{added} 项")
    } else {
        format!("已加入下载队列：{added} 项（{}）", skipped.join("；"))
    }
}

/// 系统选择器的结果 → 可入队的目录。
///
/// `Ok(None)` = 用户取消（什么都不做）。非 UTF-8、不是文件夹、无法访问
/// 都返回底栏能直接显示的错误文案；不把无效目录带进队列。
fn resolve_picked_dir(picked: Result<Option<PathBuf>, String>) -> Result<Option<PathBuf>, Failure> {
    let Some(path) = picked.map_err(Failure::Other)? else {
        return Ok(None);
    };
    if path.to_str().is_none() {
        return Err(Failure::Other(format!(
            "选择的下载目录不是有效 UTF-8，无法使用：{}",
            path.display()
        )));
    }
    match std::fs::metadata(&path) {
        Ok(meta) if meta.is_dir() => Ok(Some(path)),
        Ok(_) => Err(Failure::Other(format!(
            "选择的路径不是文件夹：{}",
            path.display()
        ))),
        Err(err) => Err(Failure::Other(format!(
            "选择的下载目录无法访问：{}（{err}）",
            path.display()
        ))),
    }
}

impl qobject::Kichi {
    /// 下载到…：先弹系统目录选择器，选中后按所选目录入队。
    ///
    /// 选择器在后台线程里等结果；用户取消什么都不做，无效路径写底栏。
    /// 全选都已在队列里时不弹选择器，直接走「去查看」/ 汇总提示。
    pub fn choose_download_dir_for_entries(mut self: Pin<&mut Self>, ids: QStringList) {
        let ids: Vec<String> = ids
            .iter()
            .map(|id| id.to_string())
            .filter(|id| !id.trim().is_empty())
            .collect();
        if ids.is_empty() {
            self.as_mut().set_message("没有可下载的条目");
            return;
        }
        let targets = classify_targets(
            ids.iter().cloned(),
            |id| self.rust().find_entry(id),
            queue::is_pending,
            |id| is_unconfirmed_create(self.rust(), id),
        );
        if targets.entries.is_empty() {
            // 单条已在队列里：沿用「下载」项的语义，带用户去看那一行，
            // 不再让人白选一次目录。
            if ids.len() == 1
                && targets.already == 1
                && targets.missing == 0
                && targets.syncing == 0
            {
                self.as_mut()
                    .locate_transfer(QString::from(ids[0].as_str()));
                return;
            }
            let message = summary_message(0, targets.already, targets.missing, targets.syncing);
            self.as_mut().set_message(&message);
            return;
        }

        let start = self.settings.download_dir_path();
        super::actions::run_in_background(
            move || resolve_picked_dir(super::folder_picker::choose(&start)),
            move |mut qichi, picked| match picked {
                // 用户取消：什么都不做，也不改全局下载目录
                Ok(None) => {}
                Ok(Some(dir)) => qichi.as_mut().enqueue_picked_downloads(targets, dir),
                Err(failure) => qichi.as_mut().set_message(&failure.to_string()),
            },
        );
    }

    /// 选择器返回目录后的入队（主线程）：每个条目各自一批，共用该目录快照。
    fn enqueue_picked_downloads(mut self: Pin<&mut Self>, targets: PickedTargets, dir: PathBuf) {
        let outcome = enqueue_targets(targets, &dir, |entry, dir| {
            queue::reserve(entry, dir.to_path_buf()).is_some()
        });
        if outcome.added > 0 {
            self.as_mut().advance_downloads();
        }
        if outcome.added > 0 && outcome.already == 0 && outcome.missing == 0 && outcome.syncing == 0
        {
            self.as_mut().clear_status_message();
        } else {
            let message = summary_message(
                outcome.added,
                outcome.already,
                outcome.missing,
                outcome.syncing,
            );
            self.as_mut().set_message(&message);
        }
    }
}

#[cfg(test)]
#[path = "pick_dir_tests.rs"]
mod tests;
