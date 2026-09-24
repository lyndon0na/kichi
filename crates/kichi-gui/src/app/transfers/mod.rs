//! 「传输任务」域: 上传 / 下载两条任务列表的状态(筛选 / 选中 / 展开)、任务生命周期
//! (入队 / 重试 / 移除 / 历史记录)与两条分栏的渲染。
//!
//! 状态与生命周期见 [`TransfersPage`]; 纯数据逻辑在 [`model`], 渲染按分栏落在
//! `download` / `upload`。

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::Receiver;

use eframe::egui::{
    self, vec2, Align, FontId, Frame, Layout, Margin, Pos2, Rect, RichText, Stroke,
};

use crate::format;
use crate::icons::{self, Glyph};
use crate::msg::{Cmd, FolderItem};
use crate::settings::{
    self, DownloadChildRecord, DownloadRecord, DownloadRecordStatus, UploadRecord,
    UploadRecordStatus,
};
use crate::theme::{mix, Theme};

use super::global::Global;
use super::helpers::{self, card_shell, icon_action, paint_checkbox, truncate_text, CheckState};
use super::types::{
    DlFilter, DlJob, DlNode, DlOp, DlRow, DlSel, DlStatus, TransferTab, UlFilter, UlJob, UlStatus,
    UploadPick,
};

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

/// 某个层级节点的名称左边界(相对行矩形)。父级目录标题在 CB_W+18 处,
/// 因此 depth=1 的子项会比父标题再右移一档, 层级才看得出区别。
fn node_left(inner_min_x: f32, depth: u32) -> f32 {
    inner_min_x + CB_W + NODE_BASE + depth as f32 * NODE_STEP
}

/// 画层级竖线: 每个祖先层级一条, 形成目录树的分组导轨。`extend` 用于跨行间距相接。
fn paint_rails(
    painter: &egui::Painter,
    th: &Theme,
    rect: Rect,
    inner_min_x: f32,
    depth: u32,
    extend: f32,
) {
    let col = mix(th.text_faint, th.bg, 0.45);
    for k in 1..depth {
        let x = node_left(inner_min_x, k) - 30.0;
        painter.line_segment(
            [
                Pos2::new(x, rect.min.y + 2.0),
                Pos2::new(x, rect.max.y + extend),
            ],
            Stroke::new(1.0, col),
        );
    }
}

/// 下载行状态文案(目录任务的「文件 k/N」由卡片自行追加)。
fn status_line(job: &DlJob) -> (egui::Color32, String) {
    if job.is_folder() {
        return match &job.status {
            DlStatus::Queued => (egui::Color32::from_gray(150), "扫描目录中…".into()),
            DlStatus::Running => (egui::Color32::from_rgb(60, 130, 200), "下载中".into()),
            DlStatus::Done => (egui::Color32::from_rgb(70, 150, 90), "已完成".into()),
            DlStatus::Failed(what) => (egui::Color32::from_rgb(217, 70, 60), what.clone()),
        };
    }
    match &job.status {
        DlStatus::Queued => (egui::Color32::from_gray(150), "排队中".into()),
        DlStatus::Running => (egui::Color32::from_rgb(60, 130, 200), "下载中".into()),
        DlStatus::Done => (egui::Color32::from_rgb(70, 150, 90), "已完成".into()),
        DlStatus::Failed(what) => (egui::Color32::from_rgb(217, 70, 60), what.clone()),
    }
}

/// 画目录行的展开/收起三角(展开朝下, 收起朝右)。
fn paint_disclosure(painter: &egui::Painter, rect: Rect, expanded: bool, color: egui::Color32) {
    let c = rect.center();
    let r = rect.width().min(rect.height()) * 0.45;
    let pts = if expanded {
        vec![
            Pos2::new(c.x - r, c.y - r * 0.5),
            Pos2::new(c.x + r, c.y - r * 0.5),
            Pos2::new(c.x, c.y + r * 0.7),
        ]
    } else {
        vec![
            Pos2::new(c.x - r * 0.5, c.y - r),
            Pos2::new(c.x + r * 0.7, c.y),
            Pos2::new(c.x - r * 0.5, c.y + r),
        ]
    };
    painter.add(egui::Shape::convex_polygon(pts, color, Stroke::NONE));
}

/// 渲染单个顶层下载任务卡片, 返回操作和选择请求。
/// `shell=false` 时不自绘卡片底(用于展开目录: 底由整块面板统一绘制)。
#[allow(clippy::too_many_arguments)]
fn dl_card(
    ui: &mut egui::Ui,
    th: &Theme,
    rid: u64,
    job: &DlJob,
    is_sel: bool,
    ctrl: bool,
    shift: bool,
    now: u64,
    shell: bool,
) -> (Option<DlOp>, Option<DlSel>) {
    let mut op: Option<DlOp> = None;
    let mut sel: Option<DlSel> = None;
    let w = ui.available_width().max(320.0);
    let h = DL_CARD_H;
    let (rect, resp) = ui.allocate_exact_size(vec2(w, h), egui::Sense::click());
    let painter = ui.painter().clone();

    let inner = rect.shrink2(vec2(12.0, 10.0));
    let is_folder = job.is_folder();

    // 背景 / 选中态
    if shell {
        card_shell(&painter, th, rect, resp.hovered(), is_sel);
    }

    // 复选框
    let cb_rect = Rect::from_center_size(
        Pos2::new(inner.min.x + 8.0, inner.center().y),
        vec2(16.0, 16.0),
    );
    let cb_resp = ui.interact(
        cb_rect,
        ui.id().with(("dl_check", rid)),
        egui::Sense::click(),
    );
    if cb_resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    paint_checkbox(
        &painter,
        th,
        cb_rect,
        if is_sel {
            CheckState::Checked
        } else {
            CheckState::Unchecked
        },
        resp.hovered() || cb_resp.hovered(),
    );
    let cb_clicked = cb_resp.clicked();

    // 布局: [复选框] [展开箭头] [名称/状态/目录] [进度] [按钮]
    let mut content_x = inner.min.x + CB_W;
    // 目录行: 展开/收起箭头
    if is_folder {
        let ch_rect = Rect::from_center_size(
            Pos2::new(content_x + 7.0, inner.center().y),
            vec2(16.0, 16.0),
        );
        let ch_resp = ui.interact(
            ch_rect,
            ui.id().with(("dl_expand", rid)),
            egui::Sense::click(),
        );
        if ch_resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let ch_col = if ch_resp.hovered() {
            th.text
        } else {
            th.text_weak
        };
        paint_disclosure(&painter, ch_rect.shrink(4.0), job.expanded, ch_col);
        if ch_resp.clicked() {
            op = Some(DlOp::Expand);
        }
        content_x += 18.0;
    }
    let right_start = inner.max.x - BTN_W;
    let left_w = ((right_start - content_x - 16.0) * 0.46).max(120.0);

    let name_g = truncate_text(
        &painter,
        &job.name,
        left_w,
        FontId::proportional(13.5),
        th.text,
    );
    painter.galley(Pos2::new(content_x, inner.min.y + 1.0), name_g, th.text);
    let (col, mut txt) = status_line(job);
    // 目录任务: 追加「文件 已完成/全部」。
    if is_folder && job.files_total > 0 {
        txt = format!("{txt} · 文件 {}/{}", job.files_done, job.files_total);
    }
    if let Some(at) = job.at {
        let rel = format::fmt_rel(now, at);
        if !rel.is_empty() {
            txt = format!("{txt} · {rel}");
        }
    }
    let status_g = truncate_text(&painter, &txt, left_w, FontId::proportional(11.5), col);
    painter.galley(Pos2::new(content_x, inner.min.y + 18.0), status_g, col);
    let dest = job.dir.to_string_lossy();
    if !dest.is_empty() {
        let dg = truncate_text(
            &painter,
            &format!("→ {dest}"),
            left_w,
            FontId::proportional(10.5),
            th.text_faint,
        );
        painter.galley(Pos2::new(content_x, inner.min.y + 35.0), dg, th.text_faint);
    }

    // 中间: 进度 / 速度 / ETA
    let mid_x = content_x + left_w + 14.0;
    let mid_w = (right_start - 14.0 - mid_x).max(0.0);
    if mid_w > 40.0 {
        match &job.status {
            DlStatus::Running if job.total > 0 => {
                let frac = (job.done as f32 / job.total as f32).clamp(0.0, 1.0);
                let bar_rect = Rect::from_min_max(
                    Pos2::new(mid_x, inner.center().y - 10.0),
                    Pos2::new(mid_x + mid_w, inner.center().y + 2.0),
                );
                painter.rect_filled(bar_rect, th.cr(3), mix(th.text_faint, th.bg, 0.7));
                let fill_w = mid_w * frac;
                if fill_w > 0.0 {
                    let fill_rect = Rect::from_min_max(
                        bar_rect.min,
                        Pos2::new(bar_rect.min.x + fill_w, bar_rect.max.y),
                    );
                    painter.rect_filled(fill_rect, th.cr(3), th.accent);
                }
                let mut info = format!(
                    "{} / {}",
                    format::fmt_bytes(job.done as i64),
                    format::fmt_bytes(job.total as i64)
                );
                if job.speed > 0 {
                    info.push_str(&format!("   {}/s", format::fmt_bytes(job.speed as i64)));
                    let eta = format::fmt_eta(job.total.saturating_sub(job.done), job.speed);
                    if !eta.is_empty() {
                        info.push_str(&format!("   剩余 {eta}"));
                    }
                }
                painter.text(
                    Pos2::new(mid_x, inner.center().y + 6.0),
                    egui::Align2::LEFT_TOP,
                    info,
                    FontId::proportional(10.5),
                    th.text_weak,
                );
            }
            DlStatus::Running => {
                let mut info = if job.done > 0 {
                    format!("已接收 {}", format::fmt_bytes(job.done as i64))
                } else {
                    "连接中…".to_string()
                };
                if job.speed > 0 {
                    info.push_str(&format!("   {}/s", format::fmt_bytes(job.speed as i64)));
                }
                painter.text(
                    Pos2::new(mid_x, inner.center().y - 7.0),
                    egui::Align2::LEFT_TOP,
                    info,
                    FontId::proportional(11.5),
                    th.text_weak,
                );
            }
            _ => {
                if job.done > 0 {
                    painter.text(
                        Pos2::new(mid_x, inner.center().y - 7.0),
                        egui::Align2::LEFT_TOP,
                        format!("已下载 {}", format::fmt_bytes(job.done as i64)),
                        FontId::proportional(11.5),
                        th.text_weak,
                    );
                }
            }
        }
    }

    // 右侧: 图标操作按钮(右对齐, 统一预留宽度保证各卡片列对齐)
    let mut btns: Vec<(Glyph, &str, DlOp)> = Vec::new();
    match &job.status {
        DlStatus::Queued | DlStatus::Running => btns.push((Glyph::Close, "取消下载", DlOp::Cancel)),
        DlStatus::Done => {
            // 目录没有单一文件可打开, 仅提供「打开所在目录」。
            if !is_folder {
                btns.push((Glyph::OpenExternal, "打开文件", DlOp::OpenFile));
            }
            btns.push((Glyph::Folder, "打开所在目录", DlOp::OpenDir));
            btns.push((Glyph::Trash, "从列表移除", DlOp::Remove));
        }
        DlStatus::Failed(_) => {
            if !job.file_id.is_empty() {
                btns.push((Glyph::Refresh, "重试下载", DlOp::Retry));
            }
            btns.push((Glyph::Folder, "打开所在目录", DlOp::OpenDir));
            btns.push((Glyph::Trash, "从列表移除", DlOp::Remove));
        }
    }

    let btn_sz = 28.0;
    let btn_gap = 4.0;
    let btn_y = inner.center().y - btn_sz / 2.0;
    let mut bx = inner.max.x;
    for (glyph, tip, dop) in btns.into_iter().rev() {
        let rect = Rect::from_min_max(Pos2::new(bx - btn_sz, btn_y), Pos2::new(bx, btn_y + btn_sz));
        bx -= btn_sz + btn_gap;
        let id = ui.id().with(("dl_btn", rid, tip));
        if icon_action(
            ui,
            &painter,
            th,
            rect,
            id,
            glyph,
            tip,
            glyph == Glyph::Trash,
        ) {
            op = Some(dop);
        }
    }

    // 点击处理(按钮已消费的操作不改变选中)
    if cb_clicked {
        sel = Some(DlSel::Toggle(rid));
    } else if op.is_none() && resp.clicked() {
        if shift {
            sel = Some(DlSel::Range(rid));
        } else if ctrl {
            sel = Some(DlSel::Toggle(rid));
        } else {
            sel = Some(DlSel::Replace(rid));
        }
    }

    (op, sel)
}

/// 渲染目录任务下的一个子目录行(紧凑单行: 缩进 + 箭头 + 文件夹图标 + 子树文件计数),
/// 返回是否点击了展开箭头。`h` 为该行高度。
fn dl_dir_node(
    ui: &mut egui::Ui,
    th: &Theme,
    node: &DlNode,
    folder: u64,
    idx: usize,
    h: f32,
) -> bool {
    let w = ui.available_width().max(320.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, h), egui::Sense::click());
    let painter = ui.painter().clone();
    let inner = rect.shrink2(vec2(12.0, 0.0));
    paint_rails(&painter, th, rect, inner.min.x, node.depth, NODE_GAP);

    // 子目录行用略深的底色作为分组标题。
    let base = mix(th.card, th.text_faint, 0.12);
    let bg = if resp.hovered() {
        mix(base, th.text, if th.dark { 0.05 } else { 0.03 })
    } else {
        base
    };
    let bar = Rect::from_min_max(
        Pos2::new(inner.min.x, rect.min.y),
        Pos2::new(inner.max.x, rect.max.y),
    );
    painter.rect_filled(bar, th.cr(6), bg);

    // 名称对齐到该层级的缩进位; 箭头与图标放在名称左侧的缩进区。
    let left = node_left(inner.min.x, node.depth);
    let ch_rect =
        Rect::from_center_size(Pos2::new(left - 32.0, inner.center().y), vec2(16.0, 16.0));
    let ch_resp = ui.interact(
        ch_rect,
        ui.id().with(("dl_dir_expand", folder, idx)),
        egui::Sense::click(),
    );
    if ch_resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let ch_col = if ch_resp.hovered() {
        th.text
    } else {
        th.text_weak
    };
    paint_disclosure(&painter, ch_rect.shrink(4.0), node.expanded, ch_col);

    let icon_rect =
        Rect::from_center_size(Pos2::new(left - 15.0, inner.center().y), vec2(14.0, 14.0));
    icons::paint(&painter, icon_rect, Glyph::Folder, th.text_weak);

    let right_start = inner.max.x - 10.0;
    let left_w = ((right_start - left - 16.0) * 0.55).max(100.0);
    let name_g = truncate_text(
        &painter,
        &node.name,
        left_w,
        FontId::proportional(12.5),
        th.text,
    );
    painter.galley(
        Pos2::new(left, inner.center().y - name_g.size().y / 2.0),
        name_g,
        th.text,
    );

    let info = if node.files_total == 0 {
        "空".to_string()
    } else {
        format!("{} / {} 个文件", node.files_done, node.files_total)
    };
    painter.text(
        Pos2::new(right_start, inner.center().y),
        egui::Align2::RIGHT_CENTER,
        info,
        FontId::proportional(11.0),
        th.text_faint,
    );

    // 点击整行也可切换展开。
    ch_resp.clicked() || resp.clicked()
}

/// 渲染目录任务下的一个文件行(紧凑单行: 名称 + 状态 + 大小), 只读。
fn dl_file_node(ui: &mut egui::Ui, th: &Theme, node: &DlNode, job: &DlJob, h: f32) {
    let w = ui.available_width().max(320.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, h), egui::Sense::click());
    let painter = ui.painter().clone();
    let inner = rect.shrink2(vec2(12.0, 0.0));
    paint_rails(&painter, th, rect, inner.min.x, node.depth, NODE_GAP);
    if resp.hovered() {
        let hover = mix(th.card, th.text, if th.dark { 0.05 } else { 0.03 });
        painter.rect_filled(rect.shrink2(vec2(8.0, 0.0)), th.cr(6), hover);
    }

    let left = node_left(inner.min.x, node.depth);
    let right_start = inner.max.x - 10.0;
    let left_w = (right_start - left - 150.0).max(100.0);
    let name_g = truncate_text(
        &painter,
        &job.name,
        left_w,
        FontId::proportional(12.5),
        th.text,
    );
    painter.galley(
        Pos2::new(left, inner.center().y - name_g.size().y / 2.0),
        name_g,
        th.text,
    );

    let (col, txt) = file_node_status(job);
    painter.text(
        Pos2::new(right_start, inner.center().y),
        egui::Align2::RIGHT_CENTER,
        txt,
        FontId::proportional(11.0),
        col,
    );
}

/// 子文件行的右侧文案与颜色: 状态 + 大小/速率。
fn file_node_status(job: &DlJob) -> (egui::Color32, String) {
    let green = egui::Color32::from_rgb(70, 150, 90);
    let blue = egui::Color32::from_rgb(60, 130, 200);
    let gray = egui::Color32::from_gray(150);
    let red = egui::Color32::from_rgb(217, 70, 60);
    match &job.status {
        DlStatus::Done => (
            green,
            format!("已完成 · {}", format::fmt_bytes(job.done as i64)),
        ),
        DlStatus::Running => {
            let mut s = String::from("下载中");
            if job.total > 0 {
                let pct = (job.done as f32 / job.total as f32 * 100.0).round() as u32;
                s.push_str(&format!(" · {pct}%"));
            } else if job.done > 0 {
                s.push_str(&format!(" · {}", format::fmt_bytes(job.done as i64)));
            }
            if job.speed > 0 {
                s.push_str(&format!(" · {}/s", format::fmt_bytes(job.speed as i64)));
            }
            (blue, s)
        }
        DlStatus::Queued => (gray, "排队中".to_string()),
        DlStatus::Failed(_) => (red, "失败".to_string()),
    }
}

impl TransfersPage {
    /// 目录树节点的行高。
    fn node_h(&self, folder: &u64, idx: usize) -> f32 {
        let is_dir = self
            .jobs
            .get(folder)
            .and_then(|j| j.nodes.get(idx))
            .map(|n| n.is_dir)
            .unwrap_or(false);
        if is_dir {
            NODE_DIR_H
        } else {
            NODE_FILE_H
        }
    }

    /// 展开目录整块面板的高度(标题行 + 子行 + 可能的截断提示 + 底部留白)。
    fn tree_block_height(&self, folder: &u64, nodes: &[usize]) -> f32 {
        let shown = nodes.len().min(TREE_CHILD_CAP);
        let mut h = DL_CARD_H + TREE_PAD;
        for idx in &nodes[..shown] {
            h += self.node_h(folder, *idx) + NODE_GAP;
        }
        if nodes.len() > shown {
            h += TREE_HINT_H;
        }
        h
    }
}

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
        let mut ops: Vec<(u64, DlOp)> = Vec::new();
        let mut sel_reqs: Vec<DlSel> = Vec::new();
        let mut clear_done = false;
        let mut actions: Vec<TransfersAction> = Vec::new();

        // 底部批量操作条: 底部面板保证布局高度正确; 用页面同色铺底避免露出窗口
        // 底色, 顶部加一条分隔线, 整体是单层扁平工具条, 不再有嵌套盒子。
        if self.transfer_tab == TransferTab::Download
            && !self.jobs.is_empty()
            && !self.selected_dl.is_empty()
        {
            egui::TopBottomPanel::bottom("dl_action_bar")
                .frame(Frame::new().fill(th.bg).inner_margin(Margin {
                    left: 20,
                    right: 20,
                    top: 0,
                    bottom: 0,
                }))
                .show(ctx, |ui| {
                    let top = ui.max_rect().min.y;
                    ui.painter()
                        .hline(ui.max_rect().x_range(), top, Stroke::new(1.0, th.border));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("已选择 {} 项", self.selected_dl.len()))
                                .size(13.0)
                                .color(th.text),
                        );
                        // 选中项里可重试(已知云端 id 的取消/失败任务)的数量
                        let retryable: Vec<u64> = self
                            .selected_dl
                            .iter()
                            .copied()
                            .filter(|rid| {
                                self.jobs
                                    .get(rid)
                                    .map(|j| {
                                        !j.file_id.is_empty()
                                            && matches!(j.status, DlStatus::Failed(_))
                                    })
                                    .unwrap_or(false)
                            })
                            .collect();
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui
                                .add(
                                    egui::Button::new(RichText::new("移除选中").color(th.danger))
                                        .stroke(Stroke::new(1.0, mix(th.danger, th.bg, 0.35)))
                                        .fill(egui::Color32::TRANSPARENT)
                                        .corner_radius(th.cr(8)),
                                )
                                .clicked()
                            {
                                for rid in &self.selected_dl {
                                    let running = self
                                        .jobs
                                        .get(rid)
                                        .map(|j| {
                                            matches!(j.status, DlStatus::Queued | DlStatus::Running)
                                        })
                                        .unwrap_or(false);
                                    ops.push((
                                        *rid,
                                        if running { DlOp::Cancel } else { DlOp::Remove },
                                    ));
                                }
                            }
                            if !retryable.is_empty()
                                && ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new("重试选中").color(th.on_accent),
                                        )
                                        .fill(th.accent)
                                        .stroke(Stroke::NONE)
                                        .corner_radius(th.cr(8)),
                                    )
                                    .clicked()
                            {
                                for rid in &retryable {
                                    ops.push((*rid, DlOp::Retry));
                                }
                            }
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("取消选中").color(th.text_weak),
                                    )
                                    .frame(false),
                                )
                                .clicked()
                            {
                                self.selected_dl.clear();
                            }
                        });
                    });
                    ui.add_space(8.0);
                });
        }

        // 上传页底部批量操作条(与下载页一致)
        self.upload_action_bar(ctx, th, g);

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

                // -------- 上传 --------
                if self.transfer_tab == TransferTab::Upload {
                    self.upload_tab(ui, th, g, &mut actions);
                    return;
                }

                // -------- 下载 --------
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("文件保存在本地下载目录, 可前往「设置」修改。")
                            .color(th.text_weak)
                            .size(12.5),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("打开下载目录").color(th.text_weak),
                                )
                                .stroke(Stroke::new(1.0, th.border))
                                .fill(egui::Color32::TRANSPARENT)
                                .corner_radius(th.cr(8)),
                            )
                            .clicked()
                        {
                            actions.push(TransfersAction::OpenDownloadDir);
                        }
                    });
                });
                ui.add_space(8.0);

                // 筛选栏: 状态分段 + 主复选框(全选/全不选) + 计数 (仅有任务时显示)
                let mut dl_ids: Vec<u64> = Vec::new();
                if !self.jobs.is_empty() {
                    ui.horizontal(|ui| {
                        // 仅统计顶层任务(目录已聚合其子文件, 避免重复计数)。
                        let top = self.jobs.values().filter(|j| j.parent.is_none());
                        let total = top.clone().count();
                        let active = top
                            .clone()
                            .filter(|j| matches!(j.status, DlStatus::Queued | DlStatus::Running))
                            .count();
                        let done_c = top.clone().filter(|j| j.status == DlStatus::Done).count();
                        let failed = top
                            .filter(|j| matches!(j.status, DlStatus::Failed(_)))
                            .count();

                        self.dl_filter_button(ui, th, DlFilter::All, &format!("全部 {total}"));
                        self.dl_filter_button(
                            ui,
                            th,
                            DlFilter::Active,
                            &format!("进行中 {active}"),
                        );
                        self.dl_filter_button(ui, th, DlFilter::Done, &format!("已完成 {done_c}"));
                        self.dl_filter_button(ui, th, DlFilter::Failed, &format!("失败 {failed}"));

                        // 当前筛选下可见项
                        dl_ids = self.visible_dl_ids();
                        // 剔除已不在当前筛选中的选中项(如进行中任务完成后被筛掉)
                        self.selected_dl.retain(|id| dl_ids.contains(id));
                        let vis_total = dl_ids.len();
                        let vis_selected = dl_ids
                            .iter()
                            .filter(|id| self.selected_dl.contains(id))
                            .count();
                        let master = if vis_total == 0 || vis_selected == 0 {
                            CheckState::Unchecked
                        } else if vis_selected >= vis_total {
                            CheckState::Checked
                        } else {
                            CheckState::Partial
                        };

                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if done_c > 0
                                && ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new("清除已完成").color(th.text_weak),
                                        )
                                        .frame(false),
                                    )
                                    .clicked()
                            {
                                clear_done = true;
                            }
                            ui.add_space(10.0);
                            let (cb_rect, cb_resp) =
                                ui.allocate_exact_size(vec2(16.0, 16.0), egui::Sense::click());
                            paint_checkbox(ui.painter(), th, cb_rect, master, cb_resp.hovered());
                            if cb_resp.clicked() {
                                if master == CheckState::Checked {
                                    for id in &dl_ids {
                                        self.selected_dl.remove(id);
                                    }
                                } else {
                                    for id in &dl_ids {
                                        self.selected_dl.insert(*id);
                                    }
                                }
                            }
                            ui.add_space(6.0);
                            ui.label(RichText::new("全选").color(th.text_weak).size(12.5));
                            ui.add_space(12.0);
                            if vis_selected > 0 {
                                ui.label(
                                    RichText::new(format!("已选 {vis_selected}/{vis_total}"))
                                        .color(th.accent)
                                        .size(12.5),
                                );
                            } else {
                                ui.label(
                                    RichText::new(format!("共 {vis_total} 个"))
                                        .color(th.text_faint)
                                        .size(12.5),
                                );
                            }
                            ui.add_space(12.0);
                            let (sum_done, sum_total, speed) = self
                                .jobs
                                .values()
                                .filter(|j| j.parent.is_none())
                                .fold((0u64, 0u64, 0u64), |(d, t, s), j| {
                                    let sp = if matches!(j.status, DlStatus::Running) {
                                        j.speed
                                    } else {
                                        0
                                    };
                                    (d + j.done, t + j.total, s + sp)
                                });
                            if sum_total > 0 {
                                let pct =
                                    (sum_done as f32 / sum_total as f32 * 100.0).round() as u32;
                                let mut agg = format!(
                                    "整体 {pct}% · {}/{}",
                                    format::fmt_bytes(sum_done as i64),
                                    format::fmt_bytes(sum_total as i64)
                                );
                                if speed > 0 {
                                    agg.push_str(&format!(
                                        " · {}/s",
                                        format::fmt_bytes(speed as i64)
                                    ));
                                }
                                ui.label(RichText::new(agg).color(th.text_faint).size(12.0));
                            }
                        });
                    });
                    ui.add_space(4.0);
                }

                if self.jobs.is_empty() {
                    ui.centered_and_justified(|ui| {
                        ui.add_space(60.0);
                        let (r, _) = ui.allocate_exact_size(vec2(64.0, 64.0), egui::Sense::hover());
                        icons::paint(ui.painter(), r, Glyph::Download, th.text_faint);
                        ui.add_space(10.0);
                        ui.label(RichText::new("暂无下载任务").color(th.text_weak).size(14.0));
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new("在「我的文件」中选择文件后下载到本地")
                                .color(th.text_faint)
                                .size(12.0),
                        );
                        ui.add_space(60.0);
                    });
                    return;
                }

                // 当前筛选下没有任务
                if dl_ids.is_empty() {
                    ui.add_space(40.0);
                    ui.vertical_centered(|ui| {
                        ui.label(
                            RichText::new("该筛选下暂无任务")
                                .color(th.text_weak)
                                .size(13.0),
                        );
                    });
                    return;
                }

                let ctrl = ui.input(|i| i.modifiers.ctrl);
                let shift = ui.input(|i| i.modifiers.shift);
                let now = format::now_unix();

                // 顺序布局: 由 egui 负责滚动范围与排版(不再手写虚拟滚动, 避免坐标/裁剪问题)。
                // 展开目录把子树包在同一块面板里; 单个目录最多渲染 TREE_CHILD_CAP 个子行。
                let dl_rows = self.dl_rows();
                let scroll_h = ui.available_height();
                egui::ScrollArea::vertical()
                    .id_salt("downloads_scroll")
                    .auto_shrink([false, false])
                    .max_height(scroll_h.max(60.0))
                    .show(ui, |ui| {
                        // 行间距完全由下面的显式 gap 控制。
                        ui.spacing_mut().item_spacing.y = 0.0;
                        let width = ui.available_width().max(320.0);
                        for row in &dl_rows {
                            match row {
                                DlRow::Job(id) => {
                                    if let Some(job) = self.jobs.get(id) {
                                        let is_sel = self.selected_dl.contains(id);
                                        let (op, sel) = dl_card(
                                            ui, th, *id, job, is_sel, ctrl, shift, now, true,
                                        );
                                        if let Some(op) = op {
                                            ops.push((*id, op));
                                        }
                                        if let Some(sel) = sel {
                                            sel_reqs.push(sel);
                                        }
                                    }
                                    ui.add_space(DL_CARD_GAP);
                                }
                                DlRow::Tree(folder, nodes) => {
                                    let Some(job) = self.jobs.get(folder) else {
                                        continue;
                                    };
                                    let shown = nodes.len().min(TREE_CHILD_CAP);
                                    let truncated = nodes.len() > shown;
                                    // 面板高度已知, 先在当前光标处铺底, 再顺序画内容。
                                    let block_h = self.tree_block_height(folder, nodes);
                                    let top = ui.cursor().min;
                                    let block = Rect::from_min_size(
                                        Pos2::new(top.x, top.y),
                                        vec2(width, block_h),
                                    );
                                    let header =
                                        Rect::from_min_size(block.min, vec2(width, DL_CARD_H));
                                    let hovered = ui.rect_contains_pointer(header);
                                    card_shell(
                                        ui.painter(),
                                        th,
                                        block,
                                        hovered,
                                        self.selected_dl.contains(folder),
                                    );
                                    let is_sel = self.selected_dl.contains(folder);
                                    let (op, sel) = dl_card(
                                        ui, th, *folder, job, is_sel, ctrl, shift, now, false,
                                    );
                                    if let Some(op) = op {
                                        ops.push((*folder, op));
                                    }
                                    if let Some(sel) = sel {
                                        sel_reqs.push(sel);
                                    }
                                    for idx in &nodes[..shown] {
                                        let idx = *idx;
                                        let h = self.node_h(folder, idx);
                                        let Some(node) =
                                            self.jobs.get(folder).and_then(|j| j.nodes.get(idx))
                                        else {
                                            ui.add_space(h + NODE_GAP);
                                            continue;
                                        };
                                        if node.is_dir {
                                            if dl_dir_node(ui, th, node, *folder, idx, h) {
                                                ops.push((*folder, DlOp::ToggleDir(idx)));
                                            }
                                        } else if let Some(cid) = node.rid {
                                            if let Some(cjob) = self.jobs.get(&cid) {
                                                dl_file_node(ui, th, node, cjob, h);
                                            }
                                        }
                                        ui.add_space(NODE_GAP);
                                    }
                                    if truncated {
                                        let (r, _) = ui.allocate_exact_size(
                                            vec2(width, TREE_HINT_H),
                                            egui::Sense::hover(),
                                        );
                                        ui.painter().text(
                                            Pos2::new(r.min.x + CB_W + NODE_BASE, r.center().y),
                                            egui::Align2::LEFT_CENTER,
                                            format!("… 仅显示前 {shown} 项(共 {} 项)", nodes.len()),
                                            FontId::proportional(11.0),
                                            th.text_faint,
                                        );
                                    }
                                    ui.add_space(TREE_PAD);
                                    ui.add_space(DL_CARD_GAP);
                                }
                            }
                        }
                    });

                // 处理选择请求（在 CentralPanel 闭包内，确保 dl_ids 和 ctrl 可见）
                for sel in sel_reqs {
                    match sel {
                        DlSel::Replace(rid) => {
                            self.selected_dl.clear();
                            self.selected_dl.insert(rid);
                            self.last_clicked_dl = Some(rid);
                        }
                        DlSel::Toggle(rid) => {
                            if self.selected_dl.contains(&rid) {
                                self.selected_dl.remove(&rid);
                            } else {
                                self.selected_dl.insert(rid);
                            }
                            self.last_clicked_dl = Some(rid);
                        }
                        DlSel::Range(rid) => {
                            if let Some(anchor) = self.last_clicked_dl {
                                let start_idx =
                                    dl_ids.iter().position(|x| *x == anchor).unwrap_or(0);
                                let end_idx = dl_ids.iter().position(|x| *x == rid).unwrap_or(0);
                                let (from, to) = if start_idx <= end_idx {
                                    (start_idx, end_idx)
                                } else {
                                    (end_idx, start_idx)
                                };
                                if !ctrl {
                                    self.selected_dl.clear();
                                }
                                for id in &dl_ids[from..=to] {
                                    self.selected_dl.insert(*id);
                                }
                            } else {
                                self.selected_dl.clear();
                                self.selected_dl.insert(rid);
                            }
                            self.last_clicked_dl = Some(rid);
                        }
                    }
                }
            });

        // 处理操作
        if clear_done {
            let done_ids: Vec<u64> = self
                .jobs
                .iter()
                .filter(|(_, j)| j.parent.is_none() && j.status == DlStatus::Done)
                .map(|(id, _)| *id)
                .collect();
            for rid in done_ids {
                self.remove_download_job(g, rid);
            }
        }
        for (rid, op) in ops {
            let is_folder = self.jobs.get(&rid).map(|j| j.is_folder()).unwrap_or(false);
            match op {
                DlOp::Expand => {
                    if let Some(j) = self.jobs.get_mut(&rid) {
                        j.expanded = !j.expanded;
                    }
                }
                DlOp::ToggleDir(idx) => {
                    if let Some(j) = self.jobs.get_mut(&rid) {
                        if let Some(n) = j.nodes.get_mut(idx) {
                            n.expanded = !n.expanded;
                        }
                    }
                }
                DlOp::Cancel => {
                    if is_folder {
                        // 目录: 通知子任务停止并连同目录一起移除(不记历史)。
                        self.remove_download_job(g, rid);
                    } else {
                        g.send(Cmd::CancelDownload { req_id: rid });
                    }
                }
                DlOp::OpenDir => {
                    let dir = self.jobs.get(&rid).map(|job| job.dir.clone());
                    if let Some(dir) = dir {
                        actions.push(TransfersAction::OpenPath(
                            dir.clone(),
                            dir.display().to_string(),
                            true,
                        ));
                    }
                }
                DlOp::OpenFile => {
                    let opened = self
                        .jobs
                        .get(&rid)
                        .map(|job| (job.dir.join(&job.name), job.name.clone()));
                    if let Some((path, label)) = opened {
                        actions.push(TransfersAction::OpenPath(path, label, false));
                    }
                }
                DlOp::Retry => {
                    if is_folder {
                        self.retry_folder(g, rid);
                    } else {
                        // 取出旧条目信息后移除旧行, 再重新入队, 避免同一文件被重复重试。
                        let info = self.jobs.get(&rid).and_then(|j| {
                            if j.file_id.is_empty() {
                                None
                            } else {
                                Some((
                                    j.file_id.clone(),
                                    j.name.clone(),
                                    j.dir.clone(),
                                    j.record_id.clone(),
                                ))
                            }
                        });
                        if let Some((file_id, name, dir, rec_id)) = info {
                            self.enqueue_downloads(
                                g,
                                vec![(file_id.clone(), name.clone())],
                                dir.clone(),
                            );
                            self.remove_download_job(g, rid);
                            settings::remove_download_record(&rec_id, &file_id, &name, &dir);
                        }
                    }
                }
                DlOp::Remove => {
                    // 仅从列表/历史记录中移除, 不删除本地已下载的文件。
                    self.remove_download_job(g, rid);
                }
            }
        }

        actions
    }
}
