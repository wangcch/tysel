use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Counts QuickJS execution time and pauses while the isolate waits on host I/O.
pub(crate) struct CpuBudget {
    limit_ns: u64,
    origin: Instant,
    used_ns: AtomicU64,
    slice_start_ns: AtomicU64,
}

impl CpuBudget {
    pub(crate) fn new(limit: Duration) -> Arc<Self> {
        let budget = Arc::new(Self {
            limit_ns: u64::try_from(limit.as_nanos()).unwrap_or(u64::MAX).max(1),
            origin: Instant::now(),
            used_ns: AtomicU64::new(0),
            slice_start_ns: AtomicU64::new(0),
        });
        budget.resume();
        budget
    }

    fn now_ns(&self) -> u64 {
        u64::try_from(self.origin.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }

    pub(crate) fn resume(&self) {
        let start = self.now_ns().max(1);
        let _ = self.slice_start_ns.compare_exchange(0, start, Ordering::SeqCst, Ordering::SeqCst);
    }

    pub(crate) fn pause(&self) {
        let start = self.slice_start_ns.swap(0, Ordering::SeqCst);
        if start != 0 {
            self.used_ns.fetch_add(self.now_ns().saturating_sub(start), Ordering::SeqCst);
        }
    }

    pub(crate) fn exhausted(&self) -> bool {
        let extra = match self.slice_start_ns.load(Ordering::SeqCst) {
            0 => 0,
            start => self.now_ns().saturating_sub(start),
        };
        self.used_ns.load(Ordering::SeqCst).saturating_add(extra) >= self.limit_ns
    }
}
