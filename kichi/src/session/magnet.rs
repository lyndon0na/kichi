//! 磁链秒传：解析文件清单、按选择建目录并逐个秒传。
//!
//! 本模块只做「一次操作的编排」，全部函数阻塞，调用方负责丢到后台线程。
//! 与普通离线任务不同：这里不创建云下载任务，而是把云端已缓存的内容
//! 直接挂进网盘，因此不会消耗云下载配额。

use std::collections::{BTreeMap, BTreeSet};

use kichi_core::{sanitize_drive_name, FolderCreateCache, InstantSave, MagnetFile, MagnetListing};

use super::{Failure, Session};

/// 一次秒传的统计。
#[derive(Debug, Default)]
pub struct MagnetSaveOutcome {
    pub saved: usize,
    /// 内容不在云端缓存里，无法秒传
    pub unavailable: usize,
    /// 请求失败（含中途因登录态 / 限流停止后未尝试的文件）
    pub failed: usize,
    /// 第一条硬失败原因；全部成功时为 `None`
    pub failure: Option<Failure>,
}

impl MagnetSaveOutcome {
    pub fn total(&self) -> usize {
        self.saved + self.unavailable + self.failed
    }
}

/// 一批磁链的解析结果。
#[derive(Debug, Default)]
pub struct MagnetResolution {
    /// 至少有一个可秒传文件的清单，保持输入顺序。
    pub listings: Vec<MagnetListing>,
    /// 解析失败或一个可秒传文件都没有的链接，交给普通云下载。
    pub fallback_links: Vec<String>,
}

impl Session {
    /// 解析磁力链接里的文件清单。阻塞。
    pub fn resolve_magnet(&self, link: &str) -> Result<MagnetListing, Failure> {
        let client = self.drive()?;
        client.list_magnet_resources(link).map_err(Failure::from)
    }

    /// 逐条解析一批磁链。阻塞。
    ///
    /// 解析失败或清单里没有可秒传文件的链接收进 `fallback_links`；登录态 /
    /// 人机验证 / 限流会中止整批，剩余链接不再请求。
    pub fn resolve_magnets(&self, links: &[String]) -> Result<MagnetResolution, Failure> {
        let mut resolution = MagnetResolution::default();
        for link in links {
            match self.resolve_magnet(link) {
                Ok(listing) if listing.available_count() > 0 => resolution.listings.push(listing),
                Ok(_) => resolution.fallback_links.push(link.clone()),
                Err(err) if err.is_session_failure() => return Err(err),
                Err(_) => resolution.fallback_links.push(link.clone()),
            }
        }
        Ok(resolution)
    }

    /// 把选中的文件秒传到 `parent_id`（空串 = 根目录）。阻塞。
    ///
    /// `groups` 的每个元素是「清单下标 + 该清单内的文件下标」，顺序不影响结果。
    /// `keep_structure` 为 false 时忽略磁链里的目录层级，所有文件直接放进
    /// `parent_id`，也不建容器目录。`progress(done, total)` 在每个文件处理完后调用。
    pub fn save_magnet_files(
        &self,
        listings: &[MagnetListing],
        groups: &[(usize, Vec<usize>)],
        parent_id: &str,
        keep_structure: bool,
        mut progress: impl FnMut(usize, usize),
    ) -> Result<MagnetSaveOutcome, Failure> {
        let client = self.drive()?;
        let parent = parent_id.trim();
        let base = (!parent.is_empty()).then_some(parent);
        let mut cache = FolderCreateCache::new();

        let total: usize = groups
            .iter()
            .filter_map(|(index, selection)| {
                listings.get(*index).map(|listing| (listing, selection))
            })
            .map(|(listing, selection)| ready_count(listing, selection))
            .sum();

        let mut outcome = MagnetSaveOutcome::default();
        let mut done = 0usize;
        let mut stopped = false;
        for (index, selection) in groups {
            let Some(listing) = listings.get(*index) else {
                continue;
            };
            let chosen = selected_files(listing, selection);
            let ready: Vec<&MagnetFile> = chosen
                .iter()
                .copied()
                .filter(|file| file.available())
                .collect();
            outcome.unavailable += chosen.len() - ready.len();
            if stopped {
                // 前面的清单遇到登录态 / 限流：剩余选中文件记为失败。
                outcome.failed += ready.len();
                continue;
            }
            if ready.is_empty() {
                continue;
            }
            save_listing(
                &client,
                listing,
                &ready,
                base,
                keep_structure,
                &mut cache,
                total,
                &mut done,
                &mut outcome,
                &mut stopped,
                &mut progress,
            );
        }

        Ok(outcome)
    }
}

fn ready_count(listing: &MagnetListing, selection: &[usize]) -> usize {
    selected_files(listing, selection)
        .iter()
        .filter(|file| file.available())
        .count()
}

/// 秒传一份清单里已确认可秒传的文件；成果累计进 `outcome`。
#[allow(clippy::too_many_arguments)]
fn save_listing(
    client: &kichi_core::DriveClient,
    listing: &MagnetListing,
    ready: &[&MagnetFile],
    base: Option<&str>,
    keep_structure: bool,
    cache: &mut FolderCreateCache,
    total: usize,
    done: &mut usize,
    outcome: &mut MagnetSaveOutcome,
    stopped: &mut bool,
    progress: &mut impl FnMut(usize, usize),
) {
    // 容器名沿用单根目录名 / 磁链 dn=；平铺模式不建容器与子目录。
    let container_id = if needs_container(ready, keep_structure) {
        let name = sanitize_drive_name(&listing.root_name, "磁力任务");
        match client.create_folder_entry_cached(base, &name, cache) {
            Ok(id) => Some(id),
            Err(err) => {
                record_listing_failure(Failure::from(err), ready.len(), outcome, stopped);
                *done += ready.len();
                progress(*done, total);
                return;
            }
        }
    } else {
        None
    };
    let root_id = container_id.as_deref().or(base).unwrap_or("");

    let folder_ids = if keep_structure {
        match create_folders(client, ready, root_id, cache) {
            Ok(ids) => ids,
            Err(err) => {
                record_listing_failure(err, ready.len(), outcome, stopped);
                if let Some(id) = container_id.as_deref() {
                    // 目录建到一半失败：别把半截空目录留在网盘里。
                    let _ = client.trash(&[id]);
                }
                *done += ready.len();
                progress(*done, total);
                return;
            }
        }
    } else {
        BTreeMap::new()
    };

    let mut saved = 0;
    for (index, file) in ready.iter().enumerate() {
        *done += 1;
        let target = if keep_structure {
            let path = file.folders.join("/");
            folder_ids.get(&path).map(String::as_str).unwrap_or(root_id)
        } else {
            root_id
        };
        match client.instant_save_file(target, file) {
            Ok(InstantSave::Saved) => saved += 1,
            Ok(InstantSave::Unavailable) => outcome.unavailable += 1,
            Err(err) => {
                let current = Failure::from(err);
                let session_failure = current.is_session_failure();
                outcome.failed += 1;
                if outcome.failure.is_none() {
                    outcome.failure = Some(current);
                }
                // 登录态 / 验证 / 限流类错误继续发只会重复失败，剩余文件记为失败。
                if session_failure {
                    let remaining = ready.len() - index - 1;
                    outcome.failed += remaining;
                    *done += remaining;
                    *stopped = true;
                    progress(*done, total);
                    break;
                }
            }
        }
        progress(*done, total);
    }
    outcome.saved += saved;

    if saved == 0 {
        if let Some(id) = container_id.as_deref() {
            // 整体失败时别留下一个空容器；清理失败不盖住真正的原因。
            let _ = client.trash(&[id]);
        }
    }
}

fn record_listing_failure(
    failure: Failure,
    count: usize,
    outcome: &mut MagnetSaveOutcome,
    stopped: &mut bool,
) {
    *stopped = failure.is_session_failure();
    outcome.failed += count;
    if outcome.failure.is_none() {
        outcome.failure = Some(failure);
    }
}

fn selected_files<'a>(listing: &'a MagnetListing, selection: &[usize]) -> Vec<&'a MagnetFile> {
    let selected: BTreeSet<usize> = selection.iter().copied().collect();
    listing
        .files
        .iter()
        .enumerate()
        .filter(|(index, _)| selected.contains(index))
        .map(|(_, file)| file)
        .collect()
}

/// 是否需要建容器目录：平铺模式从不建；保留结构时多文件或带子目录才建。
fn needs_container(files: &[&MagnetFile], keep_structure: bool) -> bool {
    keep_structure && (files.len() > 1 || files.iter().any(|file| !file.folders.is_empty()))
}

/// 按相对路径建出所有需要的子目录，返回「路径 → 目录 id」。
fn create_folders(
    client: &kichi_core::DriveClient,
    files: &[&MagnetFile],
    root_id: &str,
    cache: &mut FolderCreateCache,
) -> Result<BTreeMap<String, String>, Failure> {
    let ordered = folder_paths(files);
    let mut ids = BTreeMap::new();
    for path in ordered {
        let (parent_path, raw_name) = match path.rsplit_once('/') {
            Some((parent, name)) => (parent, name),
            None => ("", path.as_str()),
        };
        let parent_id = ids.get(parent_path).map(String::as_str).unwrap_or(root_id);
        let name = sanitize_drive_name(raw_name, "未命名文件夹");
        let id = client
            .create_folder_entry_cached(Some(parent_id), &name, cache)
            .map_err(Failure::from)?;
        ids.insert(path, id);
    }
    Ok(ids)
}

/// 需要的子目录路径：去重后按层数从浅到深排列，父目录一定排在子目录之前。
fn folder_paths(files: &[&MagnetFile]) -> Vec<String> {
    let mut paths: BTreeSet<String> = BTreeSet::new();
    for file in files {
        for depth in 1..=file.folders.len() {
            paths.insert(file.folders[..depth].join("/"));
        }
    }
    let mut ordered: Vec<String> = paths.into_iter().collect();
    ordered.sort_by(|a, b| {
        a.matches('/')
            .count()
            .cmp(&b.matches('/').count())
            .then_with(|| a.cmp(b))
    });
    ordered
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listing(files: Vec<MagnetFile>) -> MagnetListing {
        MagnetListing {
            root_name: "示例".to_owned(),
            total_size: files.iter().map(|file| file.size).sum(),
            files,
        }
    }

    fn file(name: &str, folders: &[&str], gcid: Option<&str>) -> MagnetFile {
        MagnetFile {
            name: name.to_owned(),
            folders: folders.iter().map(|part| (*part).to_owned()).collect(),
            size: 1,
            gcid: gcid.map(str::to_owned),
        }
    }

    #[test]
    fn selection_keeps_listing_order_and_ignores_unknown_indices() {
        let listing = listing(vec![
            file("a", &[], Some("x")),
            file("b", &[], Some("y")),
            file("c", &[], None),
        ]);
        let chosen = selected_files(&listing, &[2, 0, 9, 0]);
        let names: Vec<&str> = chosen.iter().map(|file| file.name.as_str()).collect();
        assert_eq!(names, vec!["a", "c"]);
    }

    #[test]
    fn flat_mode_never_creates_container() {
        let nested = file("a", &["合集"], Some("x"));
        assert!(!needs_container(&[&nested], false));
        assert!(needs_container(&[&nested], true));

        let lone = file("a", &[], Some("x"));
        assert!(!needs_container(&[&lone], true), "单文件无目录不建容器");

        let second = file("b", &[], Some("y"));
        assert!(
            needs_container(&[&lone, &second], true),
            "多文件保留结构时建容器"
        );
        assert!(!needs_container(&[&lone, &second], false));
    }

    #[test]
    fn folder_paths_are_deduped_and_parents_come_first() {
        let first = file("a", &["合集", "第一季"], Some("x"));
        let second = file("b", &["合集"], Some("y"));
        let third = file("c", &["合集", "第一季", "花絮"], Some("z"));
        let paths = folder_paths(&[&first, &second, &third]);
        assert_eq!(paths, vec!["合集", "合集/第一季", "合集/第一季/花絮"]);
    }

    #[test]
    fn ready_count_only_counts_cached_files() {
        let listing = listing(vec![file("a", &[], Some("x")), file("b", &[], None)]);
        assert_eq!(ready_count(&listing, &[0, 1]), 1);
        assert_eq!(ready_count(&listing, &[1]), 0);
    }
}
