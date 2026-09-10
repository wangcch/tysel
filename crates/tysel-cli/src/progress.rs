//! Human-only stderr progress. A worker keeps ticking during blocking I/O.
use std::io::{self, IsTerminal, Write};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

// Opt in at the CLI boundary; library-style unit tests remain silent.
static ENABLED: AtomicBool = AtomicBool::new(false);

pub(crate) fn configure(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
}

type ByteProgress = Option<(u64, Option<u64>)>;

pub(crate) struct Progress {
    label: String,
    started: Instant,
    bytes: Arc<Mutex<ByteProgress>>,
    worker: Option<(mpsc::Sender<()>, JoinHandle<()>)>,
    enabled: bool,
    terminal: bool,
    finished: bool,
}

impl Progress {
    pub(crate) fn start(label: impl Into<String>) -> Self {
        Self::start_with_animation(label, true)
    }

    /// Static boundaries for measured work and operations that emit their own logs.
    pub(crate) fn start_plain(label: impl Into<String>) -> Self {
        Self::start_with_animation(label, false)
    }

    fn start_with_animation(label: impl Into<String>, animate: bool) -> Self {
        let label = label.into().chars().filter(|c| !c.is_control()).collect::<String>();
        let enabled = ENABLED.load(Ordering::Relaxed);
        let terminal = animate
            && enabled
            && io::stderr().is_terminal()
            && std::env::var("TERM").as_deref() != Ok("dumb")
            && terminal_columns().is_some();
        let started = Instant::now();
        let bytes = Arc::new(Mutex::new(None));
        let worker = if terminal {
            let (stop, receiver) = mpsc::channel();
            let label = label.clone();
            let bytes = Arc::clone(&bytes);
            let handle = thread::spawn(move || {
                let mut frame = 0;
                loop {
                    let value = *bytes.lock().unwrap_or_else(|e| e.into_inner());
                    let text = status(&label, started.elapsed(), value);
                    let mut stderr = io::stderr().lock();
                    // Keep the cursor at the start of our final line. If the terminal
                    // shrinks and reflows it, the next clear removes the wrapped tail too.
                    let text = terminal_columns()
                        .map(|columns| {
                            fit_line(
                                &format!("{} {text}", ['|', '/', '-', '\\'][frame % 4]),
                                columns,
                            )
                        })
                        .unwrap_or_default();
                    let _ = write!(stderr, "\r\x1b[J{text}\r");
                    let _ = stderr.flush();
                    drop(stderr);
                    frame += 1;
                    if receiver.recv_timeout(Duration::from_millis(100))
                        != Err(mpsc::RecvTimeoutError::Timeout)
                    {
                        break;
                    }
                }
            });
            Some((stop, handle))
        } else {
            if enabled {
                let _ = writeln!(io::stderr().lock(), "{label}...");
            }
            None
        };
        Self { label, started, bytes, worker, enabled, terminal, finished: false }
    }

    pub(crate) fn run<T>(label: &str, operation: impl FnOnce() -> Result<T>) -> Result<T> {
        let progress = Self::start(label);
        let value = operation().with_context(|| label.to_owned())?;
        progress.finish();
        Ok(value)
    }

    pub(crate) fn run_plain<T>(label: &str, operation: impl FnOnce() -> Result<T>) -> Result<T> {
        let progress = Self::start_plain(label);
        let value = operation().with_context(|| label.to_owned())?;
        progress.finish();
        Ok(value)
    }

    pub(crate) fn bytes(&self, downloaded: u64, total: Option<u64>) {
        *self.bytes.lock().unwrap_or_else(|e| e.into_inner()) = Some((downloaded, total));
    }

    pub(crate) fn finish(mut self) {
        self.finished = true;
    }
}

fn terminal_columns() -> Option<usize> {
    console::Term::stderr()
        .size_checked()
        .map(|(_, columns)| usize::from(columns))
        .filter(|&columns| columns > 0)
}

fn fit_line(text: &str, columns: usize) -> String {
    // Leave the last column empty to avoid autowrap, counting display cells,
    // not UTF-8 bytes or characters (wide and combining characters differ).
    console::truncate_str(text, columns.saturating_sub(1), "").into_owned()
}

fn status(label: &str, elapsed: Duration, bytes: ByteProgress) -> String {
    let seconds = elapsed.as_secs_f64();
    let Some((done, total)) = bytes else {
        return format!("{label} ({seconds:.1}s)");
    };
    let mib = done as f64 / 1_048_576.0;
    let rate = mib / seconds.max(0.001);
    match total.filter(|&total| total > 0) {
        Some(total) => format!(
            "{label} {:.0}% {mib:.1}/{:.1} MiB {rate:.1} MiB/s ({seconds:.1}s)",
            (done as f64 / total as f64 * 100.0).min(100.0).floor(),
            total as f64 / 1_048_576.0
        ),
        None => format!("{label} {mib:.1} MiB {rate:.1} MiB/s ({seconds:.1}s)"),
    }
}

impl Drop for Progress {
    fn drop(&mut self) {
        if let Some((stop, handle)) = self.worker.take() {
            let _ = stop.send(());
            let _ = handle.join();
        }
        if self.enabled {
            let value = *self.bytes.lock().unwrap_or_else(|e| e.into_inner());
            let text = status(&self.label, self.started.elapsed(), value);
            let prefix = if self.terminal { "\r\x1b[J" } else { "" };
            let outcome = if self.finished { "Done" } else { "Failed" };
            let _ = writeln!(io::stderr().lock(), "{prefix}{outcome}: {text}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Also usable from a PTY to exercise live rendering without a real upgrade.
    #[test]
    fn output_probe() {
        let Ok(mode) = std::env::var("TYSEL_TEST_PROGRESS") else {
            return;
        };
        configure(mode != "silent");
        let progress = if mode == "plain" {
            let progress = Progress::start_plain("Download test archive");
            assert!(progress.worker.is_none());
            progress
        } else {
            Progress::start("Download test archive")
        };
        std::thread::sleep(Duration::from_millis(350));
        progress.bytes(1_048_576, Some(2_097_152));
        std::thread::sleep(Duration::from_millis(150));
        if mode != "failed" {
            progress.finish();
        }
    }

    #[test]
    fn redirected_output_is_plain_and_machine_mode_is_silent() {
        for mode in ["human", "silent", "failed", "plain"] {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "progress::tests::output_probe", "--nocapture"])
                .env("TYSEL_TEST_PROGRESS", mode)
                .output()
                .unwrap();
            assert!(output.status.success());
            let stderr = String::from_utf8(output.stderr).unwrap();
            if mode == "silent" {
                assert!(stderr.is_empty());
            } else {
                assert!(!stderr.contains('\x1b'));
                assert!(!stderr.contains('\r'));
                assert!(stderr.contains("50% 1.0/2.0 MiB"));
                assert!(stderr.contains(if mode == "failed" { "Failed:" } else { "Done:" }));
            }
        }
    }

    #[test]
    fn live_lines_fit_display_cells_even_after_resizing() {
        for columns in [80, 40, 12, 1, 0, 120] {
            for text in ["| Download archive 50% 12.0/24.0 MiB", "| 下载 e\u{301} 🙂 数据"] {
                let line = fit_line(text, columns);
                assert!(console::measure_text_width(&line) <= columns.saturating_sub(1));
            }
        }
    }

    #[test]
    fn unknown_or_zero_length_never_invents_a_percentage() {
        for total in [None, Some(0)] {
            let text = status("Downloading", Duration::from_secs(2), Some((1_048_576, total)));
            assert!(!text.contains('%'));
            assert!(text.contains("1.0 MiB 0.5 MiB/s"));
        }
        let text =
            status("Downloading", Duration::from_secs(2), Some((1_048_576, Some(2_097_152))));
        assert!(text.contains("50% 1.0/2.0 MiB"));
    }

    #[test]
    fn failed_stage_preserves_the_underlying_error() {
        let error = Progress::run::<()>("Verify release", || anyhow::bail!("signature mismatch"))
            .unwrap_err();
        assert_eq!(format!("{error:#}"), "Verify release: signature mismatch");
    }
}
