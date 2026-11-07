//! `transfers.rs` 的单元测试：记录表的状态迁移、父行聚合、排序、上限与显示字段。
//!
//! 拆出来只为守住行数软上限（实现本身约 350 行）。文件夹批次在统一列表
//! 只占一条总账行，子行的字段与内联展开快照也在这里覆盖。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

use cxx_qt_lib::QString;
use kichi_core::{EntryKind, UploadStage};

use super::*;
use crate::cxxqt_object::transfers_view::{
    build_detail_snapshot, build_snapshot, live_map, notice_map, ordered, parent_live_map,
    percent_of, record_map, QVariantMap, MAX_ROWS,
};
use crate::cxxqt_object::QVariantList;

/// 一条记录的种子（名字就是裸名字，单文件层级）。
fn job(name: &str) -> TransferJob {
    TransferJob {
        id: format!("id-{name}"),
        name: name.to_owned(),
        kind: EntryKind::File,
        size: 0,
        audit_flagged: false,
        sub_dir: Vec::new(),
        dir: PathBuf::from("/tmp"),
        direction: TransferDirection::Download,
        upload: None,
    }
}

fn folder_job(name: &str) -> TransferJob {
    TransferJob {
        kind: EntryKind::Folder,
        ..job(name)
    }
}

fn upload_job(name: &str, size: u64) -> TransferJob {
    TransferJob {
        id: format!("upload:{name}"),
        name: name.to_owned(),
        kind: EntryKind::File,
        size,
        audit_flagged: false,
        sub_dir: Vec::new(),
        dir: PathBuf::new(),
        direction: TransferDirection::Upload,
        upload: Some(UploadJob {
            local_path: PathBuf::from("/home/me").join(name),
            parent_id: "cloud-folder".to_owned(),
            parent_name: "动画".to_owned(),
            ..UploadJob::default()
        }),
    }
}

fn add_batch(records: &mut Vec<Transfer>, seq: u64, names: &[&str]) {
    for (index, name) in names.iter().enumerate() {
        records.push(row_from(seq, index, "source", &job(name), 1_790_000_000, 1));
    }
}

/// 一个文件夹批次：父行 + 读目录后挂上的子行。
fn folder_batch(records: &mut Vec<Transfer>, batch: u64, name: &str, children: &[(&str, u64)]) {
    records.push(row_from(
        batch,
        0,
        "source",
        &folder_job(name),
        1_790_000_000,
        1,
    ));
    mark_preparing_in(records, batch);
    let base = job("seed");
    let seeds: Vec<TransferJob> = children
        .iter()
        .map(|(child, size)| TransferJob {
            id: format!("{name}-{child}"),
            name: (*child).to_owned(),
            sub_dir: vec![name.to_owned()],
            size: *size,
            ..base.clone()
        })
        .collect();
    expand_folder_in(records, batch, &seeds, 1_790_000_000, 1);
}

fn child_at(records: &[Transfer], batch: u64, index: usize) -> usize {
    records
        .iter()
        .position(|record| record.batch == batch && record.child && record.index == index)
        .expect("子行存在")
}

fn child_mut(records: &mut [Transfer], batch: u64, index: usize) -> &mut Transfer {
    let at = records
        .iter()
        .position(|record| record.batch == batch && record.child && record.index == index)
        .expect("子行存在");
    &mut records[at]
}

fn set(records: &mut [Transfer], batch: u64, index: usize, state: TransferState) {
    if let Some(record) = find_file(records, batch, index) {
        record.state = state;
    }
}

fn set_child(records: &mut [Transfer], batch: u64, index: usize, state: TransferState) {
    let at = child_at(records, batch, index);
    records[at].state = state;
    rebuild_batch_stats(records, batch);
}

fn text_of(map: &QVariantMap, key: &str) -> String {
    map.get(&QString::from(key))
        .and_then(|value| value.value::<QString>())
        .map(|value| value.to_string())
        .unwrap_or_default()
}

fn int_of(map: &QVariantMap, key: &str) -> i32 {
    map.get(&QString::from(key))
        .and_then(|value| value.value::<i32>())
        .unwrap_or_default()
}

fn bool_of(map: &QVariantMap, key: &str) -> bool {
    map.get(&QString::from(key))
        .and_then(|value| value.value::<bool>())
        .unwrap_or_default()
}

fn map_at(list: &QVariantList, index: usize) -> QVariantMap {
    list.at(index)
        .and_then(|value| value.value::<QVariantMap>())
        .expect("快照的每一行都是 map")
}

#[path = "transfers_tests/folder.rs"]
mod folder;
#[path = "transfers_tests/lifecycle.rs"]
mod lifecycle;
#[path = "transfers_tests/skip.rs"]
mod skip;
#[path = "transfers_tests/state.rs"]
mod state;
#[path = "transfers_tests/upload.rs"]
mod upload;
#[path = "transfers_tests/view.rs"]
mod view;
