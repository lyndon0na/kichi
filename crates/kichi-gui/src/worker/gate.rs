use std::sync::atomic::{AtomicUsize, Ordering};

use tokio::sync::Notify;

/// 动态并发闸: 上限可随时调整(扩容立刻放行排队任务, 缩容只影响后续
/// acquire、不打断在传任务)。tokio Semaphore 的 set_capacity 未稳定,
/// 而 forget_permits 缩容后会被任务释放的许可回填, 故自行实现。
pub(super) struct Gate {
    limit: AtomicUsize,
    active: AtomicUsize,
    woken: Notify,
}

/// 占用一个并发槽位的守卫; 释放时唤醒排队任务。
pub(super) struct GateGuard<'a> {
    gate: &'a Gate,
}

impl Gate {
    pub(super) fn new(limit: usize) -> Self {
        Self {
            limit: AtomicUsize::new(limit),
            active: AtomicUsize::new(0),
            woken: Notify::new(),
        }
    }

    pub(super) fn set_limit(&self, limit: usize) {
        self.limit.store(limit.max(1), Ordering::Relaxed);
        self.woken.notify_waiters();
    }

    pub(super) async fn acquire(&self) -> GateGuard<'_> {
        loop {
            // enable() 先注册唤醒、再判定槽位, 避免判定与挂起之间错过 notify_waiters。
            let notified = self.woken.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let cur = self.active.load(Ordering::Acquire);
            if cur < self.limit.load(Ordering::Relaxed)
                && self
                    .active
                    .compare_exchange_weak(cur, cur + 1, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
            {
                return GateGuard { gate: self };
            }
            notified.await;
        }
    }
}

impl Drop for GateGuard<'_> {
    fn drop(&mut self) {
        self.gate.active.fetch_sub(1, Ordering::AcqRel);
        self.gate.woken.notify_waiters();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;

    #[tokio::test(flavor = "current_thread")]
    async fn gate_limit_changes_take_effect_immediately() {
        let gate = Arc::new(Gate::new(1));
        let held = gate.acquire().await; // 占满唯一槽位

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let g2 = gate.clone();
        let waiter = tokio::spawn(async move {
            let _g = g2.acquire().await;
            let _ = tx.send(());
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(rx.try_recv().is_err(), "满载时新任务应排队");

        // 扩容立刻放行排队任务。
        gate.set_limit(2);
        tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("扩容后未放行")
            .expect("排队任务 panic");

        // 缩容不打断已在传的任务(held 仍持有槽位), 但超额后的新 acquire 需排队。
        gate.set_limit(1);
        let g3 = gate.clone();
        let blocked = tokio::spawn(async move {
            let _g = g3.acquire().await;
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!blocked.is_finished(), "缩容后超额时新任务应排队");

        // 槽位释放后排队任务自动获准。
        drop(held);
        tokio::time::timeout(Duration::from_secs(1), blocked)
            .await
            .expect("槽位释放后未放行排队任务")
            .expect("排队任务 panic");
    }
}
