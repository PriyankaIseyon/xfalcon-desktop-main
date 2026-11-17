//! Progress observed by the live driver and its tool journal.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use xfalcon_contracts::inference::InferenceEvent;

pub(crate) struct RunActivity {
    base: Instant,
    last_ms: AtomicU64,
    ended: AtomicBool,
}

impl RunActivity {
    pub(crate) fn new() -> Self {
        Self {
            base: Instant::now(),
            last_ms: AtomicU64::new(0),
            ended: AtomicBool::new(false),
        }
    }

    pub(crate) fn saw(&self, event: &InferenceEvent) {
        if matches!(
            event,
            InferenceEvent::Failed { .. } | InferenceEvent::Completed { .. }
        ) {
            self.ended.store(true, Ordering::Relaxed);
        }
        self.touch();
    }

    pub(crate) fn touch(&self) {
        let now = u64::try_from(self.base.elapsed().as_millis()).unwrap_or(u64::MAX);
        self.last_ms.fetch_max(now, Ordering::Relaxed);
    }

    pub(crate) fn idle(&self) -> Duration {
        self.base
            .elapsed()
            .saturating_sub(Duration::from_millis(self.last_ms.load(Ordering::Relaxed)))
    }

    pub(crate) fn ended(&self) -> bool {
        self.ended.load(Ordering::Relaxed)
    }
}
