//! 文件夹批次的目录遍历：收集文件、成功列出的目录、与被跳过的目录数。
//!
//! 纯遍历 + 结果分类，不碰 Qt、不写盘；下载线程只消费 [`Collected`]。
//! 边界规则见 `docs/adr/0012-folder-download-boundaries.md`。
//!
//! 三条底线（`docs/pitfalls.md` 的「文件夹收集的三条边界」）：
//! - 只有 [`Failure::Other`] 能当作「跳过这个分支」；登录失效 / 限流 / 人机验证
//!   必须整批中止，否则会伪装成「完成了但跳过很多」；
//! - 空文件夹与「所有子目录都读失败」都可能 `files.is_empty()`，必须靠
//!   `dirs` / `skipped` 分开判定；
//! - 层级上限与已访问 id（防环）都不发请求，直接记一次跳过。

use std::collections::HashSet;
use std::sync::Arc;

use kichi_core::{EntryKind, FileEntry};

use super::queue::Job;
use crate::session::download::Cancel;
use crate::session::{Failure, Session};

/// 默认的最大层级：被点文件夹 = 0，其子目录 = 1。
pub(crate) const DEFAULT_MAX_DEPTH: usize = 16;
/// 覆盖层级上限的环境变量（0 / 空串 / 非法值回默认）。
pub(crate) const MAX_DEPTH_ENV: &str = "KICHI_DOWNLOAD_MAX_DEPTH";

/// 取消时统一用的失败原因（与 `session` 层的措辞一致）。
pub(crate) const CANCELLED: &str = "下载已取消";

/// 一次目录收集的结果。
pub(crate) struct Collected {
    /// 要下载的文件（保留层级）
    pub(crate) files: Vec<Job>,
    /// 相对下载根目录的目录路径；含根文件夹名；只含成功列出的目录
    pub(crate) dirs: Vec<Vec<String>>,
    /// 被跳过的目录数（过深 / 云端读失败；本地建目录失败由下载线程追加）
    pub(crate) skipped: usize,
}

/// 收集整棵目录树；顶层读不到直接返回 `Err`（整批失败）。
///
/// `list` 注入目录读取，单测用假树替换真实网络。
pub(crate) fn collect_tree(
    list: &mut dyn FnMut(&str) -> Result<Vec<FileEntry>, Failure>,
    root: &FileEntry,
    max_depth: usize,
    cancel: &Cancel,
) -> Result<Collected, Failure> {
    let mut walk = Walk {
        list,
        cancel,
        max_depth,
        visited: HashSet::from([root.id.clone()]),
        files: Vec::new(),
        dirs: Vec::new(),
        skipped: 0,
    };
    let mut path = vec![root.name.clone()];
    if let Err(err) = walk.visit(&root.id, &mut path, 0) {
        // 取消的优先级高于错误分类：网线被拔掉时按的取消也记「已取消」
        if cancel.is_cancelled() {
            return Err(Failure::Other(CANCELLED.to_owned()));
        }
        return Err(err);
    }

    // 根目录读到了、文件一个没有，且没有任何子目录成功列出：这不是空文件夹，
    // 而是整个子树都读失败（被删 / 层级过深）。
    if walk.files.is_empty() && walk.skipped > 0 && walk.dirs.len() == 1 {
        return Err(Failure::Other(format!(
            "所有子目录都无法读取（跳过 {} 个），可能已删除或层级过深",
            walk.skipped
        )));
    }
    Ok(Collected {
        files: walk.files,
        dirs: walk.dirs,
        skipped: walk.skipped,
    })
}

/// 真实会话的收集入口：每次收集读一次层级上限。
pub(crate) fn collect_folder(
    session: &Arc<Session>,
    folder: &FileEntry,
    cancel: &Cancel,
) -> Result<Collected, Failure> {
    let mut list = |folder_id: &str| session.list_dir_with_cancel(Some(folder_id), Some(cancel));
    collect_tree(&mut list, folder, max_depth(), cancel)
}

/// 当前配置的最大层级（`KICHI_DOWNLOAD_MAX_DEPTH`，非法 / 0 回默认）。
pub(crate) fn max_depth() -> usize {
    parse_max_depth(std::env::var(MAX_DEPTH_ENV).ok().as_deref())
}

/// 解析环境变量；`None` / 空串 / 非数字 / 0 都回默认。
fn parse_max_depth(raw: Option<&str>) -> usize {
    raw.and_then(|raw| raw.trim().parse::<usize>().ok())
        .filter(|depth| *depth > 0)
        .unwrap_or(DEFAULT_MAX_DEPTH)
}

/// 只有「普通失败」能跳过；登录 / 限流 / 验证码必须冒泡。
fn is_skippable(err: &Failure) -> bool {
    matches!(err, Failure::Other(_))
}

/// 递归遍历的中间状态（收集结果 + 防环表）。
struct Walk<'a> {
    list: &'a mut dyn FnMut(&str) -> Result<Vec<FileEntry>, Failure>,
    cancel: &'a Cancel,
    max_depth: usize,
    visited: HashSet<String>,
    files: Vec<Job>,
    dirs: Vec<Vec<String>>,
    skipped: usize,
}

impl Walk<'_> {
    /// 列出一个目录并把条目分类；成功列出的目录才进 `dirs`。
    fn visit(
        &mut self,
        folder_id: &str,
        path: &mut Vec<String>,
        depth: usize,
    ) -> Result<(), Failure> {
        if self.cancel.is_cancelled() {
            return Err(Failure::Other(CANCELLED.to_owned()));
        }
        let entries = (self.list)(folder_id)?;
        // 先序：父目录一定在子目录之前，建目录时按顺序创建即可
        self.dirs.push(path.clone());

        for entry in entries {
            if self.cancel.is_cancelled() {
                return Err(Failure::Other(CANCELLED.to_owned()));
            }
            match entry.kind {
                EntryKind::Folder => self.visit_folder(entry, path, depth)?,
                EntryKind::File => self.files.push(Job {
                    id: entry.id.clone(),
                    sub_dir: path.clone(),
                    name: entry.name.clone(),
                    size: entry.size,
                    audit_flagged: entry.audit_flagged,
                }),
            }
        }
        Ok(())
    }

    /// 子目录：过深 / 重复 id 直接记跳过；读取失败按类别决定跳过还是中止。
    fn visit_folder(
        &mut self,
        entry: FileEntry,
        path: &mut Vec<String>,
        depth: usize,
    ) -> Result<(), Failure> {
        let child_depth = depth + 1;
        if child_depth > self.max_depth {
            self.skipped += 1;
            return Ok(());
        }
        if !self.visited.insert(entry.id.clone()) {
            // 环形 / 重复引用：这个分支的内容已经在别处收集过
            self.skipped += 1;
            return Ok(());
        }
        path.push(entry.name);
        let result = self.visit(&entry.id, path, child_depth);
        path.pop();

        // 取消优先于错误分类：取消不是「跳过」
        if self.cancel.is_cancelled() {
            return Err(Failure::Other(CANCELLED.to_owned()));
        }
        match result {
            Ok(()) => Ok(()),
            Err(err) if is_skippable(&err) => {
                self.skipped += 1;
                Ok(())
            }
            Err(err) => Err(err),
        }
    }
}

#[cfg(test)]
#[path = "queue_collect_tests.rs"]
mod tests;
