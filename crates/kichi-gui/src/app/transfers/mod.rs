//! 「传输任务」域: 上传 / 下载两条任务列表的状态(筛选 / 选中 / 展开)、任务生命周期
//! (入队 / 重试 / 移除 / 历史记录)与两条分栏的渲染。
//!
//! 状态与生命周期见 [`TransfersPage`]; 纯数据逻辑在 [`model`], 渲染按分栏落在
//! `download` / `upload`。

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::Receiver;

use eframe::egui::{self, vec2, Frame, Margin, RichText, Stroke};

use crate::icons::{self, Glyph};
use crate::msg::{Cmd, FolderItem};
use crate::settings::{
    self, DownloadChildRecord, DownloadRecord, DownloadRecordStatus, UploadRecord,
    UploadRecordStatus,
};
use crate::theme::Theme;

use super::global::Global;
use super::helpers;
use super::types::{
    DlFilter, DlJob, DlNode, DlOp, DlRow, DlStatus, TransferTab, UlFilter, UlJob, UlStatus,
    UploadPick,
};

mod download;
pub(crate) mod model;
mod upload;

/// 传输任务域状态: 上传 / 下载两条任务列表与它们的筛选、选中、展开, 以及
/// 本地上传的异步选择框。
pub(crate) struct TransfersPage {
    /// 传输任务页当前分栏(上传 / 下载)。
    pub(crate) transfer_tab: TransferTab,
    /// 本地下载任务(含从历史恢复的行与目录任务的子文件), 按 req_id 索引。
    pub(crate) jobs: BTreeMap<u64, DlJob>,
    pub(crate) selected_dl: HashSet<u64>,
    pub(crate) dl_filter: DlFilter,
    /// Shift+Click 范围选择锚点。
    pub(crate) last_clicked_dl: Option<u64>,

    /// 本地上传任务(含从历史恢复的行), 按 req_id 索引。
    pub(crate) ul_jobs: BTreeMap<u64, UlJob>,
    pub(crate) selected_ul: HashSet<u64>,
    pub(crate) ul_filter: UlFilter,
    pub(crate) ul_last_clicked: Option<u64>,
    /// 进行中的异步选择 (是否目录, 目标目录, 目标路径展示, 结果通道), 避免阻塞 UI 线程。
    pub(crate) upload_pick: Option<UploadPick>,
}

/// 传输任务页本帧产生的跨域动作(页面不持 `&mut App`, 由 `App` 在本帧渲染后执行)。
pub(crate) enum TransfersAction {
    /// 用系统默认程序打开本地路径(目录 / 已下载文件); `quiet_ok` 为真时成功不提示。
    OpenPath(PathBuf, String, bool),
    /// 打开本地文件选择框上传文件。
    PickFiles,
    /// 打开本地目录选择框递归上传。
    PickFolder,
    /// 在「我的文件」中打开指定层级(stack 为 (id, label) 列表)。
    NavigateTo(Vec<(Option<String>, String)>),
    /// 打开本地下载目录(优先设置里的路径, 其次系统下载目录; 都不存在时提示)。
    OpenDownloadDir,
}

impl TransfersPage {
    /// 装配页面: 下载 / 上传任务从历史记录恢复。历史文件按最新在前存储, 倒序分配
    /// id 使 id 随时间递增(旧的 id 小)。
    pub(crate) fn from_settings() -> Self {
        let mut jobs = BTreeMap::new();
        let mut next_id = 0u64;
        for record in settings::load_download_history().into_iter().rev() {
            if record.is_folder {
                let folder_status = match record.status {
                    DownloadRecordStatus::Done => DlStatus::Done,
                    DownloadRecordStatus::Cancelled => continue,
                    DownloadRecordStatus::Failed(what) => DlStatus::Failed(what),
                };
                let cloud_id = record.file_id.clone();
                next_id += 1;
                let folder_id = next_id;
                let mut nodes: Vec<DlNode> = Vec::with_capacity(record.children.len());
                for ch in &record.children {
                    if ch.is_dir {
                        nodes.push(DlNode {
                            is_dir: true,
                            name: ch.name.clone(),
                            depth: ch.depth,
                            rid: None,
                            expanded: true,
                            files_done: 0,
                            files_total: 0,
                            done: false,
                        });
                        continue;
                    }
                    let cstatus = match &ch.status {
                        DownloadRecordStatus::Done => DlStatus::Done,
                        DownloadRecordStatus::Cancelled => DlStatus::Failed("已取消".into()),
                        DownloadRecordStatus::Failed(w) => DlStatus::Failed(w.clone()),
                    };
                    let done = cstatus == DlStatus::Done;
                    next_id += 1;
                    jobs.insert(
                        next_id,
                        DlJob {
                            file_id: ch.file_id.clone(),
                            record_id: String::new(),
                            name: ch.name.clone(),
                            dir: ch.dir.clone(),
                            total: ch.total,
                            done: ch.done,
                            status: cstatus,
                            speed: 0,
                            last_done: 0,
                            last_at: None,
                            at: (ch.at != 0).then_some(ch.at),
                            folder_id: None,
                            parent: Some(folder_id),
                            nodes: Vec::new(),
                            expanded: false,
                            files_done: 0,
                            files_total: 0,
                        },
                    );
                    nodes.push(DlNode {
                        is_dir: false,
                        name: ch.name.clone(),
                        depth: ch.depth,
                        rid: Some(next_id),
                        expanded: false,
                        files_done: 0,
                        files_total: 0,
                        done,
                    });
                }
                model::compute_dir_counts(&mut nodes);
                let files_total = nodes.iter().filter(|n| !n.is_dir).count() as u32;
                let files_done = nodes.iter().filter(|n| !n.is_dir && n.done).count() as u32;
                jobs.insert(
                    folder_id,
                    DlJob {
                        file_id: cloud_id.clone(),
                        record_id: record.timestamp,
                        name: record.name,
                        dir: record.dir,
                        total: record.total,
                        done: record.done,
                        status: folder_status,
                        speed: 0,
                        last_done: 0,
                        last_at: None,
                        at: (record.at != 0).then_some(record.at),
                        folder_id: Some(cloud_id),
                        parent: None,
                        nodes,
                        expanded: false,
                        files_done,
                        files_total,
                    },
                );
                continue;
            }
            let status = match record.status {
                DownloadRecordStatus::Done => DlStatus::Done,
                // 取消的任务不再保留在列表中(旧版本可能写入过取消记录)。
                DownloadRecordStatus::Cancelled => continue,
                DownloadRecordStatus::Failed(what) => DlStatus::Failed(what),
            };
            next_id += 1;
            jobs.insert(
                next_id,
                DlJob {
                    file_id: record.file_id,
                    record_id: record.timestamp,
                    name: record.name,
                    dir: record.dir,
                    total: record.total,
                    done: record.done,
                    status,
                    speed: 0,
                    last_done: 0,
                    last_at: None,
                    at: (record.at != 0).then_some(record.at),
                    folder_id: None,
                    parent: None,
                    nodes: Vec::new(),
                    expanded: false,
                    files_done: 0,
                    files_total: 0,
                },
            );
        }

        let mut ul_jobs = BTreeMap::new();
        let mut next_id = 0u64;
        // 历史按最新在前存储, 倒序分配 id 使 id 随时间递增。
        for record in settings::load_upload_history().into_iter().rev() {
            let status = match record.status {
                UploadRecordStatus::Done => UlStatus::Done,
                UploadRecordStatus::Failed(what) => UlStatus::Failed(what),
            };
            next_id += 1;
            ul_jobs.insert(
                next_id,
                UlJob {
                    local_path: record.local_path,
                    name: record.name,
                    parent: record.parent,
                    dest_stack: record.dest_stack,
                    total: record.total,
                    done: record.done,
                    status,
                    speed: 0,
                    last_done: 0,
                    last_at: None,
                    record_id: record.timestamp,
                    is_dir: record.is_dir,
                    files_done: 0,
                    files_total: 0,
                    current: String::new(),
                    at: (record.at != 0).then_some(record.at),
                },
            );
        }

        Self {
            transfer_tab: TransferTab::Download,
            jobs,
            selected_dl: HashSet::new(),
            dl_filter: DlFilter::All,
            last_clicked_dl: None,
            ul_jobs,
            selected_ul: HashSet::new(),
            ul_filter: UlFilter::All,
            ul_last_clicked: None,
            upload_pick: None,
        }
    }

    /// 历史任务里最大的 req_id(供 `App::restore_req_id` 修正发号起点, 避免与
    /// 恢复出来的历史任务 id 冲突)。
    pub(crate) fn max_job_id(&self) -> u64 {
        self.jobs
            .keys()
            .chain(self.ul_jobs.keys())
            .max()
            .copied()
            .unwrap_or(0)
    }

    // ---------------- 下载: 入队 ----------------

    /// 提交单个下载任务并登记任务行, 返回 req_id。parent 为所属目录任务的 req_id。
    pub(crate) fn enqueue_download_item(
        &mut self,
        g: &mut Global,
        file_id: String,
        name: String,
        dir: PathBuf,
        parent: Option<u64>,
    ) -> u64 {
        let req_id = g.alloc_req_id();
        let job = match parent {
            Some(p) => DlJob::child(file_id.clone(), name.clone(), dir.clone(), p),
            None => DlJob::queued(file_id.clone(), name.clone(), dir.clone()),
        };
        self.jobs.insert(req_id, job);
        g.send(Cmd::StartDownload {
            req_id,
            file_id,
            name,
            dest_dir: dir,
        });
        req_id
    }

    /// 逐个提交下载任务(共享同一个已选目录)。
    pub(crate) fn enqueue_downloads(
        &mut self,
        g: &mut Global,
        items: Vec<(String, String)>,
        dir: PathBuf,
    ) {
        let n = items.len();
        if n == 0 {
            return;
        }
        for (id, name) in items {
            self.enqueue_download_item(g, id, name, dir.clone(), None);
        }
        g.toast_ok(&format!("已加入下载队列 ({n} 个文件)"));
    }

    /// 提交整目录下载: 后台先扫描目录树, 回 `Msg::FolderScanned` 后逐个入队。
    pub(crate) fn enqueue_download_folder(
        &mut self,
        g: &mut Global,
        folder_id: String,
        name: String,
        dir: PathBuf,
    ) -> u64 {
        let req_id = g.alloc_req_id();
        self.jobs.insert(
            req_id,
            DlJob::folder(folder_id.clone(), name.clone(), dir.clone()),
        );
        g.send(Cmd::StartDownloadFolder {
            req_id,
            folder_id,
            name,
            dest_dir: dir,
        });
        req_id
    }

    // ---------------- 下载: 进度与终态 ----------------

    /// 子文件进度回传后刷新所属目录任务的聚合进度。
    pub(crate) fn on_dl_progress(&mut self, req_id: u64, total: u64, done: u64) {
        if let Some(j) = self.jobs.get_mut(&req_id) {
            if j.status == DlStatus::Queued {
                j.status = DlStatus::Running;
            }
            if total > 0 {
                j.total = total;
            }
            model::sample_speed(&mut j.speed, &mut j.last_done, &mut j.last_at, done);
            if done > j.done {
                j.done = done;
            }
        }
        if let Some(p) = self.jobs.get(&req_id).and_then(|j| j.parent) {
            self.recompute_folder(p);
        }
    }

    pub(crate) fn on_dl_finished(&mut self, req_id: u64, bytes: u64) {
        if let Some(j) = self.jobs.get_mut(&req_id) {
            j.status = DlStatus::Done;
            j.done = bytes;
            if bytes > 0 && j.total == 0 {
                j.total = bytes;
            }
        }
        match self.jobs.get(&req_id).and_then(|j| j.parent) {
            Some(p) => self.recompute_folder(p),
            None => self.persist_download_job(req_id, DownloadRecordStatus::Done),
        }
    }

    /// 取消后从列表移除且不保留记录; 未完成的 .part 已由下载线程删除。
    pub(crate) fn on_dl_cancelled(&mut self, req_id: u64) {
        let parent = self.jobs.get(&req_id).and_then(|j| j.parent);
        self.jobs.remove(&req_id);
        self.selected_dl.remove(&req_id);
        if self.last_clicked_dl == Some(req_id) {
            self.last_clicked_dl = None;
        }
        if let Some(p) = parent {
            if let Some(pj) = self.jobs.get_mut(&p) {
                pj.nodes.retain(|n| n.rid != Some(req_id));
            }
            self.recompute_folder(p);
        }
    }

    pub(crate) fn on_dl_failed(&mut self, req_id: u64, what: String) {
        if let Some(j) = self.jobs.get_mut(&req_id) {
            j.status = DlStatus::Failed(what.clone());
        }
        match self.jobs.get(&req_id).and_then(|j| j.parent) {
            Some(p) => self.recompute_folder(p),
            None => self.persist_download_job(req_id, DownloadRecordStatus::Failed(what)),
        }
    }

    /// 目录扫描完成: 登记目录树并逐个入队子文件; 空目录直接完成。
    pub(crate) fn on_folder_scanned(
        &mut self,
        g: &mut Global,
        req_id: u64,
        items: Vec<FolderItem>,
        total_bytes: u64,
    ) {
        if !self
            .jobs
            .get(&req_id)
            .map(|j| j.is_folder())
            .unwrap_or(false)
        {
            return;
        }
        // 没有文件(空目录或仅空子目录): 直接完成。
        if items.iter().all(|it| it.is_dir) {
            let nodes: Vec<DlNode> = items
                .iter()
                .map(|it| DlNode {
                    is_dir: true,
                    name: it.name.clone(),
                    depth: it.depth,
                    rid: None,
                    expanded: true,
                    files_done: 0,
                    files_total: 0,
                    done: false,
                })
                .collect();
            if let Some(j) = self.jobs.get_mut(&req_id) {
                j.status = DlStatus::Done;
                j.total = 0;
                j.done = 0;
                j.nodes = nodes;
                j.files_done = 0;
                j.files_total = 0;
            }
            self.persist_download_job(req_id, DownloadRecordStatus::Done);
            g.toast_ok("空目录已创建");
            return;
        }
        let mut nodes: Vec<DlNode> = Vec::with_capacity(items.len());
        let mut files_total = 0u32;
        for it in items {
            if it.is_dir {
                nodes.push(DlNode {
                    is_dir: true,
                    name: it.name,
                    depth: it.depth,
                    rid: None,
                    expanded: true,
                    files_done: 0,
                    files_total: 0,
                    done: false,
                });
            } else {
                let cid = self.enqueue_download_item(
                    g,
                    it.file_id,
                    it.name.clone(),
                    it.dir,
                    Some(req_id),
                );
                nodes.push(DlNode {
                    is_dir: false,
                    name: it.name,
                    depth: it.depth,
                    rid: Some(cid),
                    expanded: false,
                    files_done: 0,
                    files_total: 0,
                    done: false,
                });
                files_total += 1;
            }
        }
        model::compute_dir_counts(&mut nodes);
        if let Some(j) = self.jobs.get_mut(&req_id) {
            j.nodes = nodes;
            j.total = total_bytes;
            j.done = 0;
            j.files_done = 0;
            j.files_total = files_total;
            j.status = DlStatus::Running;
        }
        g.toast_ok(&format!("已加入下载队列 ({files_total} 个文件)"));
    }

    pub(crate) fn on_folder_scan_failed(&mut self, g: &mut Global, req_id: u64, what: String) {
        if let Some(j) = self.jobs.get_mut(&req_id) {
            j.status = DlStatus::Failed(what.clone());
        }
        self.persist_download_job(req_id, DownloadRecordStatus::Failed(what.clone()));
        g.toast_err(&what);
    }

    /// 依据子文件状态重算目录任务的聚合进度、各子目录计数与状态, 并在首次进入终态时写历史。
    pub(crate) fn recompute_folder(&mut self, folder: u64) {
        // 先把节点列表移出, 便于同时读取各子任务的进度而不产生借用冲突。
        let Some(mut nodes) = self
            .jobs
            .get_mut(&folder)
            .filter(|j| j.is_folder())
            .map(|j| std::mem::take(&mut j.nodes))
        else {
            return;
        };
        let kids: Vec<&DlJob> = nodes
            .iter()
            .filter_map(|n| n.rid)
            .filter_map(|rid| self.jobs.get(&rid))
            .collect();
        let (total, done, speed, status) = model::aggregate_children(kids.into_iter());
        // 同步文件节点的完成标记, 再自底向上累加每个子目录的计数。
        for n in nodes.iter_mut() {
            if let Some(rid) = n.rid {
                n.done = self
                    .jobs
                    .get(&rid)
                    .map(|j| j.status == DlStatus::Done)
                    .unwrap_or(false);
            }
        }
        model::compute_dir_counts(&mut nodes);
        let files_total = nodes.iter().filter(|n| !n.is_dir).count() as u32;
        let files_done = nodes.iter().filter(|n| !n.is_dir && n.done).count() as u32;
        let terminal = matches!(status, DlStatus::Done | DlStatus::Failed(_));
        let mut write_record = false;
        if let Some(f) = self.jobs.get_mut(&folder) {
            // 扫描时已知合计大小, 优先保留; 未知(0)时用子文件汇总兜底。
            if f.total == 0 {
                f.total = total;
            }
            f.done = done;
            f.speed = speed;
            f.status = status.clone();
            f.files_done = files_done;
            f.files_total = files_total;
            f.nodes = nodes;
            write_record = terminal && f.record_id.is_empty();
        }
        if write_record {
            self.persist_download_job(folder, model::dl_record_status(&status));
        }
    }

    /// 写一条下载历史记录, 并回填任务行的 record_id / at。
    /// 目录任务额外内联其子文件快照。
    pub(crate) fn persist_download_job(&mut self, rid: u64, status: DownloadRecordStatus) {
        let children = if self.jobs.get(&rid).map(|j| j.is_folder()).unwrap_or(false) {
            self.snapshot_children(rid)
        } else {
            Vec::new()
        };
        let rec_id = model::record_id_now();
        let at = crate::format::now_unix();
        let Some(j) = self.jobs.get_mut(&rid) else {
            return;
        };
        j.record_id = rec_id.clone();
        j.at = Some(at);
        settings::append_download_record(DownloadRecord {
            file_id: j.file_id.clone(),
            name: j.name.clone(),
            dir: j.dir.clone(),
            total: j.total,
            done: j.done,
            status,
            at,
            timestamp: rec_id,
            is_folder: j.is_folder(),
            children,
        });
    }

    /// 目录任务的树节点记录快照(按先序, 含子目录)。
    fn snapshot_children(&self, folder: u64) -> Vec<DownloadChildRecord> {
        let nodes = self
            .jobs
            .get(&folder)
            .map(|j| j.nodes.clone())
            .unwrap_or_default();
        nodes
            .iter()
            .map(|n| {
                let c = n.rid.and_then(|rid| self.jobs.get(&rid));
                DownloadChildRecord {
                    file_id: c.map(|c| c.file_id.clone()).unwrap_or_default(),
                    name: n.name.clone(),
                    dir: c.map(|c| c.dir.clone()).unwrap_or_default(),
                    total: c.map(|c| c.total).unwrap_or(0),
                    done: c.map(|c| c.done).unwrap_or(0),
                    status: c
                        .map(|c| model::dl_record_status(&c.status))
                        .unwrap_or(DownloadRecordStatus::Done),
                    at: c.and_then(|c| c.at).unwrap_or(0),
                    is_dir: n.is_dir,
                    depth: n.depth,
                }
            })
            .collect()
    }

    /// 从列表移除一个下载任务(运行中的普通文件先取消; 目录任务连同子文件一起移除)。
    pub(crate) fn remove_download_job(&mut self, g: &mut Global, rid: u64) {
        let Some((is_folder, kids, rec, running, parent)) = self.jobs.get(&rid).map(|j| {
            (
                j.is_folder(),
                j.file_rids().collect::<Vec<_>>(),
                (
                    j.record_id.clone(),
                    j.file_id.clone(),
                    j.name.clone(),
                    j.dir.clone(),
                ),
                matches!(j.status, DlStatus::Queued | DlStatus::Running),
                j.parent,
            )
        }) else {
            return;
        };
        if is_folder {
            // 目录: 停止运行中的子任务并连同子行一起移除, 同时删除目录历史记录。
            for cid in kids {
                let c_running = self
                    .jobs
                    .get(&cid)
                    .map(|j| matches!(j.status, DlStatus::Queued | DlStatus::Running))
                    .unwrap_or(false);
                if c_running {
                    g.send(Cmd::CancelDownload { req_id: cid });
                }
                self.jobs.remove(&cid);
                self.selected_dl.remove(&cid);
            }
            settings::remove_download_record(&rec.0, &rec.1, &rec.2, &rec.3);
            self.jobs.remove(&rid);
            self.selected_dl.remove(&rid);
            if self.last_clicked_dl == Some(rid) {
                self.last_clicked_dl = None;
            }
            return;
        }
        if let Some(p) = parent {
            // 子文件行(兜底路径): 从父目录的目录树中摘除后刷新聚合。
            if let Some(pj) = self.jobs.get_mut(&p) {
                pj.nodes.retain(|n| n.rid != Some(rid));
            }
            self.jobs.remove(&rid);
            self.selected_dl.remove(&rid);
            if self.last_clicked_dl == Some(rid) {
                self.last_clicked_dl = None;
            }
            self.recompute_folder(p);
            return;
        }
        if running {
            g.send(Cmd::CancelDownload { req_id: rid });
            return;
        }
        settings::remove_download_record(&rec.0, &rec.1, &rec.2, &rec.3);
        self.jobs.remove(&rid);
        self.selected_dl.remove(&rid);
        if self.last_clicked_dl == Some(rid) {
            self.last_clicked_dl = None;
        }
    }

    /// 重试目录下载: 仅重下失败的子文件; 若尚未扫描出子文件(扫描阶段失败)则重新扫描。
    pub(crate) fn retry_folder(&mut self, g: &mut Global, rid: u64) {
        let Some((folder_id, name, dir, rec_id, file_id, has_files)) =
            self.jobs.get(&rid).map(|j| {
                (
                    j.folder_id.clone(),
                    j.name.clone(),
                    j.dir.clone(),
                    j.record_id.clone(),
                    j.file_id.clone(),
                    j.nodes.iter().any(|n| !n.is_dir),
                )
            })
        else {
            return;
        };
        if !has_files {
            // 扫描阶段失败(尚无文件节点): 重新扫描, 沿用同一 req_id。
            settings::remove_download_record(&rec_id, &file_id, &name, &dir);
            if let Some(j) = self.jobs.get_mut(&rid) {
                j.record_id.clear();
                j.at = None;
                j.status = DlStatus::Queued;
                j.expanded = true;
            }
            if let Some(fid) = folder_id {
                g.send(Cmd::StartDownloadFolder {
                    req_id: rid,
                    folder_id: fid,
                    name,
                    dest_dir: dir,
                });
                g.toast_ok("正在重新扫描目录…");
            }
            return;
        }
        // 收集失败文件所在的节点下标, 只重下这些文件。
        let failed: Vec<(usize, String, String, PathBuf)> = self
            .jobs
            .get(&rid)
            .map(|j| {
                j.nodes
                    .iter()
                    .enumerate()
                    .filter_map(|(i, n)| {
                        let c = self.jobs.get(&n.rid?)?;
                        match &c.status {
                            DlStatus::Failed(_) => {
                                Some((i, c.file_id.clone(), c.name.clone(), c.dir.clone()))
                            }
                            _ => None,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        if failed.is_empty() {
            return;
        }
        for (idx, c_file_id, c_name, c_dir) in failed {
            let new_cid = self.enqueue_download_item(g, c_file_id, c_name, c_dir, Some(rid));
            let old = {
                let Some(j) = self.jobs.get_mut(&rid) else {
                    continue;
                };
                j.nodes.get_mut(idx).and_then(|n| n.rid.replace(new_cid))
            };
            if let Some(old) = old {
                self.jobs.remove(&old);
                self.selected_dl.remove(&old);
            }
        }
        settings::remove_download_record(&rec_id, &file_id, &name, &dir);
        if let Some(j) = self.jobs.get_mut(&rid) {
            j.record_id.clear();
            j.at = None;
            j.expanded = true;
        }
        self.recompute_folder(rid);
    }

    // ---------------- 上传 ----------------

    /// 逐个提交上传任务到给定网盘目录(None = 根目录)。
    pub(crate) fn enqueue_upload(
        &mut self,
        g: &mut Global,
        paths: Vec<PathBuf>,
        parent: Option<String>,
        dest_stack: Vec<(Option<String>, String)>,
    ) {
        let mut n = 0usize;
        for path in paths {
            let name = path
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            if name.is_empty() {
                continue;
            }
            let req_id = g.alloc_req_id();
            self.ul_jobs.insert(
                req_id,
                UlJob::queued(path.clone(), name, parent.clone(), dest_stack.clone()),
            );
            g.send(Cmd::StartUpload {
                req_id,
                path,
                parent: parent.clone(),
            });
            n += 1;
        }
        if n > 0 {
            g.toast_ok(&format!("已加入上传队列 ({n} 个文件)"));
        }
    }

    /// 提交目录递归上传任务。
    pub(crate) fn enqueue_upload_dir(
        &mut self,
        g: &mut Global,
        path: PathBuf,
        parent: Option<String>,
        dest_stack: Vec<(Option<String>, String)>,
    ) {
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        if name.is_empty() {
            return;
        }
        let req_id = g.alloc_req_id();
        self.ul_jobs.insert(
            req_id,
            UlJob::queued_dir(path.clone(), name, parent.clone(), dest_stack),
        );
        g.send(Cmd::StartUploadDir {
            req_id,
            path,
            parent,
        });
        g.toast_ok("已加入上传队列 (文件夹)");
    }

    /// 打开本地文件 / 目录选择框(已在选择中时忽略重复点击)。
    pub(crate) fn start_pick(
        &mut self,
        is_dir: bool,
        parent: Option<String>,
        dest_stack: Vec<(Option<String>, String)>,
        rx: Receiver<Vec<PathBuf>>,
    ) {
        if self.upload_pick.is_some() {
            return;
        }
        self.upload_pick = Some((is_dir, parent, dest_stack, rx));
    }

    /// 每帧检查异步选择结果; 选好后按当时的目标目录入队。返回本次选中的目录,
    /// 由 `App` 更新 `last_dir` 并落盘(那是 App 的持久化设置)。
    pub(crate) fn poll_file_picker(&mut self, g: &mut Global) -> Option<String> {
        let Some((is_dir, parent, stack, rx)) = &self.upload_pick else {
            return None;
        };
        let is_dir = *is_dir;
        match rx.try_recv() {
            Ok(paths) => {
                let parent = parent.clone();
                let stack = stack.clone();
                self.upload_pick = None;
                if paths.is_empty() {
                    return None;
                }
                let start = helpers::picked_dir(&paths, is_dir)
                    .and_then(|p| p.to_str().map(str::to_string));
                if is_dir {
                    for p in paths {
                        self.enqueue_upload_dir(g, p, parent.clone(), stack.clone());
                    }
                } else {
                    self.enqueue_upload(g, paths, parent, stack);
                }
                start
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.upload_pick = None;
                None
            }
        }
    }

    pub(crate) fn on_ul_progress(&mut self, req_id: u64, total: u64, done: u64) {
        if let Some(j) = self.ul_jobs.get_mut(&req_id) {
            if j.status == UlStatus::Queued {
                j.status = UlStatus::Running;
            }
            if total > 0 {
                j.total = total;
            }
            model::sample_speed(&mut j.speed, &mut j.last_done, &mut j.last_at, done);
            if done > j.done {
                j.done = done;
            }
        }
    }

    pub(crate) fn on_ul_files(&mut self, req_id: u64, done: u32, total: u32, current: String) {
        if let Some(j) = self.ul_jobs.get_mut(&req_id) {
            j.files_done = done;
            j.files_total = total;
            j.current = current;
            if j.status == UlStatus::Queued {
                j.status = UlStatus::Running;
            }
        }
    }

    /// 上传完成: 写历史并返回目标目录(外层=任务是否存在, 内层 None=根目录),
    /// 供 `App` 作废该目录缓存并刷新配额。
    pub(crate) fn on_ul_finished(&mut self, g: &mut Global, req_id: u64) -> Option<Option<String>> {
        let parent = self.ul_jobs.get_mut(&req_id).map(|j| {
            j.status = UlStatus::Done;
            if j.total > 0 {
                j.done = j.total;
            }
            // 写入上传历史。
            let rec_id = model::record_id_now();
            j.record_id = rec_id.clone();
            j.at = Some(crate::format::now_unix());
            settings::append_upload_record(UploadRecord {
                local_path: j.local_path.clone(),
                name: j.name.clone(),
                parent: j.parent.clone(),
                dest_stack: j.dest_stack.clone(),
                total: j.total,
                done: j.done,
                status: UploadRecordStatus::Done,
                is_dir: j.is_dir,
                at: j.at.unwrap_or(0),
                timestamp: rec_id,
            });
            j.parent.clone()
        });
        g.toast_ok("上传完成");
        parent
    }

    pub(crate) fn on_ul_cancelled(&mut self, req_id: u64) {
        self.ul_jobs.remove(&req_id);
        self.selected_ul.remove(&req_id);
        if self.ul_last_clicked == Some(req_id) {
            self.ul_last_clicked = None;
        }
    }

    pub(crate) fn on_ul_failed(&mut self, g: &mut Global, req_id: u64, what: String) {
        if let Some(j) = self.ul_jobs.get_mut(&req_id) {
            j.status = UlStatus::Failed(what.clone());
            // 写入上传历史。
            let rec_id = model::record_id_now();
            j.record_id = rec_id.clone();
            j.at = Some(crate::format::now_unix());
            settings::append_upload_record(UploadRecord {
                local_path: j.local_path.clone(),
                name: j.name.clone(),
                parent: j.parent.clone(),
                dest_stack: j.dest_stack.clone(),
                total: j.total,
                done: j.done,
                status: UploadRecordStatus::Failed(what.clone()),
                is_dir: j.is_dir,
                at: j.at.unwrap_or(0),
                timestamp: rec_id,
            });
        }
        g.toast_err(&format!("上传失败: {what}"));
    }

    /// 重试一个上传任务(移除旧行与历史后重新入队)。
    pub(crate) fn retry_upload_job(&mut self, g: &mut Global, rid: u64) {
        let info = self.ul_jobs.get(&rid).map(|j| {
            (
                j.local_path.clone(),
                j.parent.clone(),
                j.record_id.clone(),
                j.name.clone(),
                j.is_dir,
                j.dest_stack.clone(),
            )
        });
        if let Some((path, parent, rec_id, name, is_dir, stack)) = info {
            settings::remove_upload_record(&rec_id, &path, &name);
            self.ul_jobs.remove(&rid);
            self.selected_ul.remove(&rid);
            if self.ul_last_clicked == Some(rid) {
                self.ul_last_clicked = None;
            }
            if is_dir {
                self.enqueue_upload_dir(g, path, parent, stack);
            } else {
                self.enqueue_upload(g, vec![path], parent, stack);
            }
        }
    }

    /// 从列表移除一个上传任务(运行中的取消; 其余删除行与历史记录)。
    pub(crate) fn remove_upload_job(&mut self, g: &mut Global, rid: u64) {
        let running = self
            .ul_jobs
            .get(&rid)
            .map(|j| matches!(j.status, UlStatus::Queued | UlStatus::Running))
            .unwrap_or(false);
        if running {
            g.send(Cmd::CancelUpload { req_id: rid });
            return;
        }
        if let Some(job) = self.ul_jobs.get(&rid) {
            settings::remove_upload_record(&job.record_id, &job.local_path, &job.name);
        }
        self.ul_jobs.remove(&rid);
        self.selected_ul.remove(&rid);
        if self.ul_last_clicked == Some(rid) {
            self.ul_last_clicked = None;
        }
    }

    // ---------------- 列表查询 ----------------

    /// 当前上传筛选下可见的任务 id(按 map 顺序)。
    pub(crate) fn visible_ul_ids(&self) -> Vec<u64> {
        self.ul_jobs
            .iter()
            .filter(|(_, j)| self.ul_filter.matches(j))
            .map(|(id, _)| *id)
            .collect()
    }

    /// 当前筛选下可见的顶层任务 id(不含目录的子文件), 按最新在前排序。
    pub(crate) fn visible_dl_ids(&self) -> Vec<u64> {
        let mut ids: Vec<u64> = self
            .jobs
            .iter()
            .filter(|(_, j)| j.parent.is_none() && self.dl_filter.matches(j))
            .map(|(id, _)| *id)
            .collect();
        ids.sort_unstable_by(|a, b| b.cmp(a));
        ids
    }

    /// 传输任务页当前要渲染的行(顶层任务 + 已展开目录的子文件)。
    pub(crate) fn dl_rows(&self) -> Vec<DlRow> {
        let mut rows = Vec::new();
        for id in self.visible_dl_ids() {
            let Some(j) = self.jobs.get(&id) else {
                continue;
            };
            if !j.is_folder() || !j.expanded {
                rows.push(DlRow::Job(id));
                continue;
            }
            // 先序遍历, 跳过被收起子目录的子孙。
            let mut visible: Vec<usize> = Vec::new();
            let mut skip_below: Option<u32> = None;
            for (i, n) in j.nodes.iter().enumerate() {
                if let Some(d) = skip_below {
                    if n.depth > d {
                        continue;
                    }
                    skip_below = None;
                }
                visible.push(i);
                if n.is_dir && !n.expanded {
                    skip_below = Some(n.depth);
                }
            }
            rows.push(DlRow::Tree(id, visible));
        }
        rows
    }

    /// 是否有进行中的下载任务(排队或下载中), 用于加快进度轮询。
    pub(crate) fn has_active_downloads(&self) -> bool {
        self.jobs
            .values()
            .any(|j| j.status == DlStatus::Queued || j.status == DlStatus::Running)
    }

    /// 是否有进行中的上传任务(排队或上传中), 用于加快进度轮询。
    pub(crate) fn has_active_uploads(&self) -> bool {
        self.ul_jobs
            .values()
            .any(|j| j.status == UlStatus::Queued || j.status == UlStatus::Running)
    }

    /// 清空下载列表与选中(退出登录 / 会话失效)。
    pub(crate) fn clear(&mut self) {
        self.jobs.clear();
        self.selected_dl.clear();
        self.last_clicked_dl = None;
    }
}

/// 顶层任务卡片高度, 以及其后的间距。
const DL_CARD_H: f32 = 72.0;
const DL_CARD_GAP: f32 = 8.0;
/// 目录树子行(紧凑单行)的高度与间距: 子目录略矮、子文件稍高。
const NODE_DIR_H: f32 = 30.0;
const NODE_FILE_H: f32 = 38.0;
const NODE_GAP: f32 = 2.0;
/// 展开目录卡片底部的留白。
const TREE_PAD: f32 = 8.0;
/// 单个展开目录最多渲染的子行数与截断提示行高(超出部分只提示)。
const TREE_CHILD_CAP: usize = 1000;
const TREE_HINT_H: f32 = 26.0;

/// 左侧复选框列的宽度, 卡片内容统一从这里起排。
const CB_W: f32 = 26.0;
/// 右侧图标按钮组预留宽度(保证各卡片列对齐)。
const BTN_W: f32 = 92.0;
/// 子项的基准缩进(相对复选框列)与每层递增量。
const NODE_BASE: f32 = 22.0;
const NODE_STEP: f32 = 18.0;

impl TransfersPage {
    /// 传输任务页顶部的「上传 / 下载」分栏按钮。
    fn transfer_tab_button(
        &mut self,
        ui: &mut egui::Ui,
        th: &Theme,
        tab: TransferTab,
        label: &str,
    ) {
        let selected = self.transfer_tab == tab;
        let text = RichText::new(label).size(13.0).color(if selected {
            th.on_accent
        } else {
            th.text_weak
        });
        let btn = egui::Button::new(text)
            .fill(if selected {
                th.accent
            } else {
                egui::Color32::TRANSPARENT
            })
            .stroke(Stroke::new(
                1.0,
                if selected { th.accent } else { th.border },
            ))
            .corner_radius(th.cr(8));
        if ui.add(btn).clicked() {
            self.transfer_tab = tab;
        }
    }

    /// 下载列表的状态筛选按钮。
    fn dl_filter_button(&mut self, ui: &mut egui::Ui, th: &Theme, filter: DlFilter, label: &str) {
        let selected = self.dl_filter == filter;
        let text = RichText::new(label).size(12.0).color(if selected {
            th.on_accent
        } else {
            th.text_weak
        });
        let btn = egui::Button::new(text)
            .fill(if selected {
                th.accent
            } else {
                egui::Color32::TRANSPARENT
            })
            .stroke(Stroke::new(
                1.0,
                if selected { th.accent } else { th.border },
            ))
            .corner_radius(th.cr(8));
        if ui.add(btn).clicked() && !selected {
            self.dl_filter = filter;
            // 切换筛选时清空选择, 避免被筛掉的项仍处于选中状态
            self.selected_dl.clear();
            self.last_clicked_dl = None;
        }
    }

    /// 上传列表的状态筛选按钮。
    fn ul_filter_button(&mut self, ui: &mut egui::Ui, th: &Theme, filter: UlFilter, label: &str) {
        let selected = self.ul_filter == filter;
        let text = RichText::new(label).size(12.0).color(if selected {
            th.on_accent
        } else {
            th.text_weak
        });
        let btn = egui::Button::new(text)
            .fill(if selected {
                th.accent
            } else {
                egui::Color32::TRANSPARENT
            })
            .stroke(Stroke::new(
                1.0,
                if selected { th.accent } else { th.border },
            ))
            .corner_radius(th.cr(8));
        if ui.add(btn).clicked() && !selected {
            self.ul_filter = filter;
            self.selected_ul.clear();
            self.ul_last_clicked = None;
        }
    }

    /// 请求在「我的文件」中打开指定层级(stack 为 (id, label) 列表)。
    fn navigate_to_stack(actions: &mut Vec<TransfersAction>, stack: Vec<(Option<String>, String)>) {
        if stack.is_empty() {
            return;
        }
        actions.push(TransfersAction::NavigateTo(stack));
    }

    /// 渲染传输任务页; 跨域副作用(打开本地路径 / 选择框 / 跳转文件页)以动作返回,
    /// 由 `App` 在本帧渲染后执行。
    pub(crate) fn show(
        &mut self,
        ctx: &egui::Context,
        th: &Theme,
        g: &mut Global,
    ) -> Vec<TransfersAction> {
        let mut actions: Vec<TransfersAction> = Vec::new();

        // 底部批量操作条: 底部面板保证布局高度正确; 用页面同色铺底避免露出窗口
        // 底色, 顶部加一条分隔线, 整体是单层扁平工具条, 不再有嵌套盒子。
        // 须在正文之前声明; 下载侧的选中操作先收进 `bar_ops`, 与正文操作一起按序应用。
        let mut bar_ops: Vec<(u64, DlOp)> = Vec::new();
        match self.transfer_tab {
            TransferTab::Download => self.download_action_bar(ctx, th, &mut bar_ops),
            TransferTab::Upload => self.upload_action_bar(ctx, th, g),
        }

        egui::CentralPanel::default()
            .frame(Frame::new().fill(th.bg).inner_margin(Margin {
                left: 20,
                right: 20,
                top: 16,
                bottom: 12,
            }))
            .show(ctx, |ui| {
                // 标题行
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(22.0, 22.0), egui::Sense::hover());
                    icons::paint(ui.painter(), r, Glyph::Transfer, th.accent);
                    ui.label(RichText::new("传输任务").size(19.0).strong().color(th.text));
                });
                ui.add_space(10.0);

                // 上传 / 下载 分栏
                ui.horizontal(|ui| {
                    self.transfer_tab_button(ui, th, TransferTab::Upload, "上传");
                    self.transfer_tab_button(ui, th, TransferTab::Download, "下载");
                });
                ui.add_space(10.0);

                match self.transfer_tab {
                    TransferTab::Upload => self.upload_tab(ui, th, g, &mut actions),
                    TransferTab::Download => self.download_tab(ui, th, g, &mut actions, bar_ops),
                }
            });

        actions
    }
}
