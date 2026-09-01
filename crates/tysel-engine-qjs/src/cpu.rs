use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Counts QuickJS execution time and pauses while the isolate waits on host I/O.
pub(crate) struct CpuBudget {
    limit_ns: u64,
    clock: CpuClock,
    used_ns: AtomicU64,
    slice_start_ns: AtomicU64,
}

/// Uses the current thread's CPU clock where the platform exposes one. Unlike
/// wall time, this clock does not charge an isolate while the OS deschedules
/// its test or worker thread under load.
struct CpuClock {
    fallback_origin: Instant,
}

impl CpuClock {
    fn new() -> Self {
        Self { fallback_origin: Instant::now() }
    }

    fn now_ns(&self) -> u64 {
        thread_cpu_time_ns().unwrap_or_else(|| {
            u64::try_from(self.fallback_origin.elapsed().as_nanos()).unwrap_or(u64::MAX)
        })
    }
}

impl CpuBudget {
    pub(crate) fn new(limit: Duration) -> Arc<Self> {
        let budget = Arc::new(Self {
            limit_ns: u64::try_from(limit.as_nanos()).unwrap_or(u64::MAX).max(1),
            clock: CpuClock::new(),
            used_ns: AtomicU64::new(0),
            slice_start_ns: AtomicU64::new(0),
        });
        budget.resume();
        budget
    }

    fn now_ns(&self) -> u64 {
        self.clock.now_ns()
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

#[cfg(any(
    target_os = "android",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "ios",
    target_os = "linux",
    target_os = "macos",
    target_os = "netbsd",
    target_os = "openbsd"
))]
fn thread_cpu_time_ns() -> Option<u64> {
    let mut time = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    #[allow(unsafe_code)]
    let result = unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut time) };
    if result != 0 || time.tv_sec < 0 || time.tv_nsec < 0 {
        return None;
    }
    let seconds = u64::try_from(time.tv_sec).ok()?;
    let nanos = u64::try_from(time.tv_nsec).ok()?;
    seconds.checked_mul(1_000_000_000)?.checked_add(nanos)
}

#[cfg(not(any(
    target_os = "android",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "ios",
    target_os = "linux",
    target_os = "macos",
    target_os = "netbsd",
    target_os = "openbsd"
)))]
fn thread_cpu_time_ns() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(any(
        target_os = "android",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "ios",
        target_os = "linux",
        target_os = "macos",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    #[test]
    fn descheduled_time_does_not_exhaust_the_budget() {
        let budget = CpuBudget::new(Duration::from_millis(10));
        std::thread::sleep(Duration::from_millis(30));
        assert!(!budget.exhausted());
    }
}
