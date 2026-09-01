use std::io::{BufRead, BufReader, Read};
use std::ops::{Deref, DerefMut};
use std::process::{Child, Command};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

pub(crate) type CapturedLog = Arc<Mutex<String>>;

/// Owns a long-running test child and guarantees cleanup during assertion
/// unwinding as well as on the successful path.
pub(crate) struct ManagedChild(Child);

impl ManagedChild {
    pub(crate) fn spawn(command: &mut Command, description: &str) -> Self {
        Self(command.spawn().unwrap_or_else(|error| panic!("spawn {description}: {error}")))
    }
}

impl Deref for ManagedChild {
    type Target = Child;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for ManagedChild {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Drop for ManagedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

enum ListenEvent {
    Ready(String),
    Exited,
    ReadError(String),
}

/// Waits for the standard `tysel listen` announcement while continuously
/// draining both output pipes. Failures include exit status and captured output
/// instead of reducing every early process exit to an opaque `EOF`.
pub(crate) fn wait_listen(child: &mut Child, timeout: Duration) -> (String, CapturedLog) {
    let stdout = child.stdout.take().expect("child stdout must be piped");
    let stderr = child.stderr.take().expect("child stderr must be piped");
    let stderr_capture = capture(stderr);
    let stdout_log = Arc::new(Mutex::new(String::new()));
    let captured_stdout = stdout_log.clone();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        let mut announced = false;
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => {
                    if !announced {
                        let _ = tx.send(ListenEvent::Exited);
                    }
                    return;
                }
                Ok(_) => {
                    captured_stdout.lock().expect("stdout log").push_str(&line);
                    if !announced && let Some(address) = line.trim().strip_prefix("tysel listen ") {
                        announced = true;
                        let _ = tx.send(ListenEvent::Ready(address.to_owned()));
                    }
                }
                Err(error) => {
                    if !announced {
                        let _ = tx.send(ListenEvent::ReadError(error.to_string()));
                    }
                    return;
                }
            }
        }
    });

    let failure = match rx.recv_timeout(timeout) {
        Ok(ListenEvent::Ready(address)) => return (address, stderr_capture.log),
        Ok(ListenEvent::Exited) => "process exited before announcing readiness".to_owned(),
        Ok(ListenEvent::ReadError(error)) => {
            format!("failed to read readiness announcement: {error}")
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            format!("timed out after {timeout:?} waiting for readiness")
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            "readiness reader stopped without a result".to_owned()
        }
    };

    let status_before_cleanup = wait_for_exit(child, Duration::from_millis(100));
    let was_running = status_before_cleanup.is_none();
    if was_running {
        let _ = child.kill();
        let _ = child.wait();
    }
    let _ = stderr_capture.done.recv_timeout(Duration::from_millis(250));
    let stdout = stdout_log.lock().expect("stdout log").clone();
    let stderr = stderr_capture.log.lock().expect("stderr log").clone();
    let status = status_before_cleanup
        .map(|value| value.to_string())
        .unwrap_or_else(|| "still running (terminated by test)".to_owned());
    panic!("{failure}; status={status}; stdout={stdout:?}; stderr={stderr:?}");
}

fn wait_for_exit(child: &mut Child, timeout: Duration) -> Option<std::process::ExitStatus> {
    let started = Instant::now();
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            return Some(status);
        }
        if started.elapsed() >= timeout {
            return None;
        }
        thread::sleep(Duration::from_millis(5));
    }
}

struct Capture {
    log: CapturedLog,
    done: mpsc::Receiver<()>,
}

fn capture(mut reader: impl Read + Send + 'static) -> Capture {
    let log = Arc::new(Mutex::new(String::new()));
    let captured = log.clone();
    let (done_tx, done) = mpsc::channel();
    thread::spawn(move || {
        let mut buffer = [0u8; 512];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(length) => captured
                    .lock()
                    .expect("captured log")
                    .push_str(&String::from_utf8_lossy(&buffer[..length])),
            }
        }
        let _ = done_tx.send(());
    });
    Capture { log, done }
}
