//! 传输任务的纯数据逻辑: 状态映射 / 进度聚合 / 目录树计数 / 速率取样。
//! 不依赖 egui, 可直接单测。

use std::time::Instant;

use crate::settings::DownloadRecordStatus;

use super::types::{DlJob, DlNode, DlStatus};

/// 下载任务状态 -> 持久化记录状态(非终态仅在异常情况下出现, 兜底标记未完成)。
pub(super) fn dl_record_status(s: &DlStatus) -> DownloadRecordStatus {
    match s {
        DlStatus::Done => DownloadRecordStatus::Done,
        DlStatus::Failed(w) => DownloadRecordStatus::Failed(w.clone()),
        DlStatus::Queued | DlStatus::Running => DownloadRecordStatus::Failed("未完成".into()),
    }
}

/// 汇总目录任务下所有子文件的进度与状态: (合计大小, 已下载, 合计速率, 聚合状态)。
pub(super) fn aggregate_children<'a>(
    children: impl Iterator<Item = &'a DlJob>,
) -> (u64, u64, u64, DlStatus) {
    let mut total = 0u64;
    let mut done = 0u64;
    let mut speed = 0u64;
    let (mut n, mut done_n, mut fail_n) = (0u32, 0u32, 0u32);
    let mut active = false;
    for c in children {
        n += 1;
        total = total.saturating_add(c.total);
        done = done.saturating_add(c.done);
        match &c.status {
            DlStatus::Done => done_n += 1,
            DlStatus::Failed(_) => fail_n += 1,
            DlStatus::Running => {
                active = true;
                speed = speed.saturating_add(c.speed);
            }
            DlStatus::Queued => {}
        }
    }
    let status = if fail_n > 0 {
        DlStatus::Failed(format!("{fail_n} 个文件失败"))
    } else if n > 0 && done_n == n {
        DlStatus::Done
    } else if active {
        DlStatus::Running
    } else {
        DlStatus::Queued
    };
    (total, done, speed, status)
}

/// 依据文件节点的 `done` 标记, 自底向上累加每个子目录节点的子树文件计数。
/// `nodes` 必须按先序排列(父节点先于其子孙)。
pub(super) fn compute_dir_counts(nodes: &mut [DlNode]) {
    for n in nodes.iter_mut() {
        if n.is_dir {
            n.files_done = 0;
            n.files_total = 0;
        }
    }
    // 栈内为当前仍「开放」的祖先目录下标(按 depth 递增)。
    let mut stack: Vec<usize> = Vec::new();
    for i in 0..nodes.len() {
        while let Some(&top) = stack.last() {
            if nodes[top].depth >= nodes[i].depth {
                stack.pop();
            } else {
                break;
            }
        }
        if nodes[i].is_dir {
            stack.push(i);
        } else if nodes[i].rid.is_some() {
            let done = nodes[i].done;
            for &d in &stack {
                nodes[d].files_total += 1;
                if done {
                    nodes[d].files_done += 1;
                }
            }
        }
    }
}

/// 用时间加权 EMA 刷新任务速率。
///
/// `drain()` 会在单帧内一次性消费积压的多条进度消息, 若逐条按 `Instant::now()`
/// 取样会出现 `dt≈0` 而使瞬时速率爆炸(截图里的 389 MB/s)。这里仅当距上次取样
/// 满 `MIN_SAMPLE` 秒才计算一次, 短间隔消息只推进 `done` 不动速率。
pub(super) fn sample_speed(
    speed: &mut u64,
    last_done: &mut u64,
    last_at: &mut Option<Instant>,
    done: u64,
) {
    const MIN_SAMPLE: f64 = 0.25;
    let now = Instant::now();
    match *last_at {
        None => {
            *last_at = Some(now);
            *last_done = done;
        }
        Some(at) => {
            let dt = now.duration_since(at).as_secs_f64();
            if dt < MIN_SAMPLE {
                return;
            }
            if done >= *last_done {
                let inst = ((done - *last_done) as f64 / dt) as u64;
                *speed = if *speed == 0 {
                    inst
                } else {
                    ((*speed as f64) * 0.6 + (inst as f64) * 0.4) as u64
                };
            }
            *last_at = Some(now);
            *last_done = done;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn dl_job(status: DlStatus, total: u64, done: u64) -> DlJob {
        let mut j = DlJob::queued("id".into(), "n".into(), std::path::PathBuf::from("/tmp"));
        j.status = status;
        j.total = total;
        j.done = done;
        j
    }

    #[test]
    fn record_status_maps_terminal_states_and_falls_back() {
        assert_eq!(
            dl_record_status(&DlStatus::Done),
            DownloadRecordStatus::Done
        );
        assert_eq!(
            dl_record_status(&DlStatus::Failed("权限不足".into())),
            DownloadRecordStatus::Failed("权限不足".into())
        );
        // 非终态不该出现在持久化路径上, 兜底记为未完成。
        for s in [DlStatus::Queued, DlStatus::Running] {
            assert_eq!(
                dl_record_status(&s),
                DownloadRecordStatus::Failed("未完成".into())
            );
        }
    }

    #[test]
    fn aggregate_sums_and_running_status() {
        let a = dl_job(DlStatus::Running, 100, 40);
        let b = dl_job(DlStatus::Done, 50, 50);
        let (total, done, _speed, status) = aggregate_children([&a, &b].into_iter());
        assert_eq!((total, done), (150, 90));
        assert_eq!(status, DlStatus::Running);
    }

    #[test]
    fn aggregate_all_done_is_done() {
        let a = dl_job(DlStatus::Done, 10, 10);
        let b = dl_job(DlStatus::Done, 5, 5);
        let (total, done, _speed, status) = aggregate_children([&a, &b].into_iter());
        assert_eq!((total, done), (15, 15));
        assert_eq!(status, DlStatus::Done);
    }

    #[test]
    fn aggregate_any_failure_wins() {
        let a = dl_job(DlStatus::Done, 10, 10);
        let f = dl_job(DlStatus::Failed("x".into()), 0, 0);
        let (_t, _d, _s, status) = aggregate_children([&a, &f].into_iter());
        assert!(matches!(status, DlStatus::Failed(_)));
    }

    #[test]
    fn aggregate_queued_before_any_activity() {
        let a = dl_job(DlStatus::Queued, 10, 0);
        let (_t, _d, _s, status) = aggregate_children([&a].into_iter());
        assert_eq!(status, DlStatus::Queued);
    }

    fn dir_node(name: &str, depth: u32) -> DlNode {
        DlNode {
            is_dir: true,
            name: name.into(),
            depth,
            rid: None,
            expanded: true,
            files_done: 0,
            files_total: 0,
            done: false,
        }
    }

    fn file_node(depth: u32, done: bool) -> DlNode {
        DlNode {
            is_dir: false,
            name: "f".into(),
            depth,
            rid: Some(1),
            expanded: false,
            files_done: 0,
            files_total: 0,
            done,
        }
    }

    #[test]
    fn dir_counts_roll_up_subtree() {
        // A/ (1): 自身 2 个文件(1 完成) + 子目录 B/ (2): 1 个文件(完成); C/ (1): 空。
        let mut nodes = vec![
            dir_node("A", 1),
            file_node(2, true),
            file_node(2, false),
            dir_node("B", 2),
            file_node(3, true),
            dir_node("C", 1),
        ];
        compute_dir_counts(&mut nodes);
        // A 汇总其整个子树: 3 个文件、2 个完成。
        assert_eq!((nodes[0].files_done, nodes[0].files_total), (2, 3));
        assert_eq!((nodes[3].files_done, nodes[3].files_total), (1, 1));
        assert_eq!((nodes[5].files_done, nodes[5].files_total), (0, 0));
    }

    #[test]
    fn sample_speed_short_interval_keeps_rate() {
        // last_at 置于未来使 dt 饱和为 0, 必然走「间隔不足」分支: 短间隔只记基线、
        // 不取样, 避免单帧积压多条进度时 dt≈0 把瞬时速率算爆。
        let mut speed = 500u64;
        let mut last_done = 0u64;
        let mut last_at = Some(Instant::now() + Duration::from_secs(10));
        sample_speed(&mut speed, &mut last_done, &mut last_at, 100_000_000);
        assert_eq!(speed, 500);
        assert_eq!(last_done, 0);
    }
}
