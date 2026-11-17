use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use tokio::sync::{OwnedRwLockReadGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

const UPDATE_PENDING: &str =
    "An application update is being installed. Restart xFalcon before starting new work.";

#[derive(Default)]
pub struct WorkBarrier {
    gate: Arc<RwLock<()>>,
    blocked: AtomicBool,
}

impl WorkBarrier {
    pub async fn admit(&self) -> Result<RwLockReadGuard<'_, ()>, &'static str> {
        let guard = self.gate.read().await;
        if self.blocked.load(Ordering::SeqCst) {
            return Err(UPDATE_PENDING);
        }
        Ok(guard)
    }

    pub async fn admit_owned(&self) -> Result<OwnedRwLockReadGuard<()>, &'static str> {
        let guard = self.gate.clone().read_owned().await;
        if self.blocked.load(Ordering::SeqCst) {
            return Err(UPDATE_PENDING);
        }
        Ok(guard)
    }

    pub async fn begin_update(&self) -> RwLockWriteGuard<'_, ()> {
        let guard = self.gate.write().await;
        self.blocked.store(true, Ordering::SeqCst);
        guard
    }

    pub fn unblock(&self) {
        self.blocked.store(false, Ordering::SeqCst);
    }

    pub fn is_blocked(&self) -> bool {
        self.blocked.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::WorkBarrier;

    #[tokio::test]
    async fn update_excludes_new_work_until_unblocked() {
        let barrier = WorkBarrier::default();
        let update = barrier.begin_update().await;
        drop(update);
        assert!(barrier.admit().await.is_err());
        barrier.unblock();
        assert!(barrier.admit().await.is_ok());
    }

    #[tokio::test]
    async fn update_waits_for_an_admission_already_in_progress() {
        let barrier = Arc::new(WorkBarrier::default());
        let admitted = barrier.admit().await.expect("initial work admitted");
        let waiting = barrier.clone();
        let update = tokio::spawn(async move {
            let guard = waiting.begin_update().await;
            drop(guard);
        });

        tokio::task::yield_now().await;
        assert!(!update.is_finished());
        drop(admitted);
        update.await.expect("update task");
        assert!(barrier.admit().await.is_err());
    }

    #[tokio::test]
    async fn owned_admission_keeps_a_spawned_operation_inside_the_barrier() {
        let barrier = Arc::new(WorkBarrier::default());
        let admitted = barrier.admit_owned().await.expect("work admitted");
        let waiting = barrier.clone();
        let update = tokio::spawn(async move {
            let guard = waiting.begin_update().await;
            drop(guard);
        });

        tokio::task::yield_now().await;
        assert!(!update.is_finished());
        let spawned = tokio::spawn(async move {
            let _admission = admitted;
            tokio::task::yield_now().await;
        });
        spawned.await.expect("spawned operation");
        update.await.expect("update task");
        assert!(barrier.admit().await.is_err());
    }
}
