use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tysel_engine::IsolateConfig;
use tysel_engine_qjs::IsolatePool;
use tysel_manifest::Manifest;
use tysel_runtime::{SharedPool, handle_stream};

const IGNORED_DIRS: &[&str] = &["node_modules", "target", "dist", ".git", "data"];

struct Watch {
    rx: mpsc::UnboundedReceiver<notify::Result<Event>>,
    _watcher: RecommendedWatcher,
}

pub async fn run(manifest_path: PathBuf, entry: Option<PathBuf>) -> Result<()> {
    let (isolate, max_request_bytes, addr) = load(&manifest_path, entry.as_deref())?;
    let pool = SharedPool::new(isolate, max_request_bytes);
    let listener = TcpListener::bind(addr).await.with_context(|| format!("bind {addr}"))?;
    let bound = listener.local_addr()?;
    println!("tysel listen {bound}");
    io::stdout().flush()?;
    let mut changes = watch(manifest_path.parent().unwrap_or(Path::new(".")))?;

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => return Ok(()),
            _ = wait_change(&mut changes.rx) => match load(&manifest_path, entry.as_deref()) {
                Ok((next, max_bytes, _)) => {
                    eprintln!("tysel reload");
                    pool.replace(next, max_bytes);
                }
                Err(err) => eprintln!("error: {err:#}"),
            },
            accepted = listener.accept() => {
                let (stream, _) = accepted.context("accept")?;
                handle_stream(stream, pool.clone());
            }
        }
    }
}

fn load(
    manifest_path: &Path,
    entry: Option<&Path>,
) -> Result<(Arc<IsolatePool>, usize, std::net::SocketAddr)> {
    let manifest = Manifest::from_path(manifest_path)
        .with_context(|| format!("failed to read {}", manifest_path.display()))?;
    let root = manifest_path.parent().unwrap_or(Path::new("."));
    let entry = match entry {
        Some(path) => path.to_path_buf(),
        None => root.join(&manifest.app.entry),
    };
    let (bundle, source_map) = tysel_build::read_bundle(&entry)
        .with_context(|| format!("failed to bundle {}", entry.display()))?;
    let tap = tysel_build::tap_from_app(&manifest, env!("CARGO_PKG_VERSION"), bundle, source_map);
    let source = tap.bundle_source()?.to_owned();
    let config = IsolateConfig {
        memory_limit_bytes: tap.manifest.memory_limit_bytes,
        cpu_ms_per_turn: tap.manifest.cpu_ms_per_turn,
        request_timeout_ms: tap.manifest.request_timeout_ms,
    };
    let pool = IsolatePool::spawn(1, &source, config)?;
    let addr = tap
        .manifest
        .listen
        .parse()
        .map_err(|_| anyhow!("invalid listen address '{}'", tap.manifest.listen))?;
    Ok((Arc::new(pool), tap.manifest.max_request_bytes, addr))
}

fn watch(root: &Path) -> Result<Watch> {
    let (tx, rx) = mpsc::unbounded_channel();
    let mut watcher = RecommendedWatcher::new(
        move |event| {
            let _ = tx.send(event);
        },
        Config::default(),
    )?;
    watch_dir(&mut watcher, root)?;
    Ok(Watch { rx, _watcher: watcher })
}

fn watch_dir(watcher: &mut RecommendedWatcher, dir: &Path) -> Result<()> {
    watcher.watch(dir, RecursiveMode::NonRecursive)?;
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return Ok(()),
    };
    for entry in entries {
        let Ok(entry) = entry else {
            continue;
        };
        let path = entry.path();
        if path.is_dir() && !ignored_dir(&path) {
            watch_dir(watcher, &path)?;
        }
    }
    Ok(())
}

async fn wait_change(rx: &mut mpsc::UnboundedReceiver<notify::Result<Event>>) {
    loop {
        let Some(event) = rx.recv().await else {
            std::future::pending::<()>().await;
            return;
        };
        if !relevant(event) {
            continue;
        }
        let debounce = Instant::now() + Duration::from_millis(80);
        while Instant::now() < debounce {
            let remain = debounce.saturating_duration_since(Instant::now());
            tokio::select! {
                maybe = rx.recv() => {
                    let Some(event) = maybe else {
                        return;
                    };
                    let _ = relevant(event);
                }
                _ = tokio::time::sleep(remain) => break,
            }
        }
        return;
    }
}

fn relevant(event: notify::Result<Event>) -> bool {
    let Ok(event) = event else {
        return false;
    };
    if !matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)) {
        return false;
    }
    event.paths.iter().any(|path| {
        if ignored(path) {
            return false;
        }
        matches!(
            path.extension().and_then(|ext| ext.to_str()),
            Some("ts" | "tsx" | "mts" | "cts" | "js" | "mjs" | "cjs" | "json" | "toml")
        )
    })
}

fn ignored(path: &Path) -> bool {
    path.components().any(|component| {
        matches!(
            component.as_os_str().to_str(),
            Some("node_modules" | "target" | "dist" | ".git" | "data")
        )
    })
}

fn ignored_dir(path: &Path) -> bool {
    matches!(path.file_name().and_then(|name| name.to_str()), Some(name) if IGNORED_DIRS.contains(&name))
}
