use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tysel_capability::CapabilityId;
use tysel_engine::IsolateConfig;
use tysel_manifest::Manifest;
use tysel_package::SourceMap;
#[cfg(unix)]
use tysel_runtime::ModuleTaskService;
use tysel_runtime::{AppIsolate, DurablePlane, SharedPool, handle_stream, spawn_app_isolate};
use tysel_task_rpc::TaskOutcome;

const IGNORED_DIRS: &[&str] = &["node_modules", "target", "dist", ".git", "data"];

struct Watch {
    rx: mpsc::UnboundedReceiver<notify::Result<Event>>,
    _watcher: RecommendedWatcher,
}

struct Loaded {
    isolate: AppIsolate,
    max_request_bytes: usize,
    addr: std::net::SocketAddr,
    websocket: bool,
    http1: bool,
    http2: bool,
    source_map: Arc<SourceMap>,
    task: Option<TaskSpec>,
    durable: Option<DurableSpec>,
}

#[derive(Clone)]
struct DurableSpec {
    source: String,
    config: IsolateConfig,
    sqlite_path: String,
    execution_profile: String,
    root: PathBuf,
}

#[derive(Clone)]
struct TaskSpec {
    application_id: String,
    source: String,
    config: IsolateConfig,
    execution_profile: String,
    secret_names: Vec<String>,
}

pub async fn run(manifest_path: PathBuf, entry: Option<PathBuf>) -> Result<()> {
    if component_entry(&manifest_path, entry.as_deref())?.is_some() {
        anyhow::bail!("Wasm Components are one-shot tasks; use `tysel run` instead of `tysel dev`");
    }
    serve(manifest_path, entry, true).await
}

/// Serve JavaScript without reload, or execute a Component once over stdio.
pub async fn run_once(manifest_path: PathBuf, entry: Option<PathBuf>) -> Result<()> {
    if let Some((manifest, root, entry)) = component_entry(&manifest_path, entry.as_deref())? {
        let source = fs::read(&entry)
            .with_context(|| format!("failed to read Component {}", entry.display()))?;
        let tap =
            tysel_build::tap_from_component_portable(&manifest, env!("CARGO_PKG_VERSION"), source)
                .with_context(|| format!("failed to compile Component {}", entry.display()))?;
        let mut grants = Vec::new();
        if !manifest.permissions.fs_read.is_empty() || !manifest.permissions.fs_write.is_empty() {
            grants.push(CapabilityId("tysel:fs".into()));
        }
        let policy = tysel_runtime::ComponentRuntimePolicy::new(grants).with_filesystem_root(root);
        tysel_runtime::run_component_tap_with_policy(&tap, &policy)?;
        return Ok(());
    }
    serve(manifest_path, entry, false).await
}

fn component_entry(
    manifest_path: &Path,
    entry: Option<&Path>,
) -> Result<Option<(Manifest, PathBuf, PathBuf)>> {
    let manifest = Manifest::from_path(manifest_path)
        .with_context(|| format!("failed to read {}", manifest_path.display()))?;
    let root = manifest_path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let entry = entry.map(Path::to_path_buf).unwrap_or_else(|| root.join(&manifest.app.entry));
    if entry.extension().and_then(|extension| extension.to_str()) == Some("wasm") {
        Ok(Some((manifest, root, entry)))
    } else {
        Ok(None)
    }
}

pub async fn run_mcp(manifest_path: PathBuf, entry: Option<PathBuf>) -> Result<()> {
    #[cfg(not(unix))]
    {
        let _ = (manifest_path, entry);
        anyhow::bail!("MCP stdio currently requires a Unix host")
    }
    #[cfg(unix)]
    {
        let loaded = load(&manifest_path, entry.as_deref())?;
        let spec = loaded.task.ok_or_else(|| anyhow!("application registers no module tasks"))?;
        let service = start_task_service(Some(spec), 1)
            .await?
            .ok_or_else(|| anyhow!("application registers no module tasks"))?;
        if service.ingress().registry().mcp_tools().next().is_none() {
            service.shutdown().await?;
            anyhow::bail!("application registers no MCP tools");
        }
        let endpoint = service.mcp_endpoint()?;
        loop {
            let message = tokio::task::spawn_blocking(|| {
                let input = io::stdin();
                tysel_cap_mcp::read_stdio_message(&mut input.lock())
            })
            .await??;
            let Some(message) = message else {
                break;
            };
            if let Some(response) = endpoint.handle_bytes(&message).await? {
                let output = io::stdout();
                tysel_cap_mcp::write_stdio_message(&mut output.lock(), &response)?;
            }
        }
        service.shutdown().await?;
        Ok(())
    }
}

pub async fn run_queue(
    manifest_path: PathBuf,
    entry: Option<PathBuf>,
    name: String,
    message_id: Option<String>,
    input: String,
) -> Result<()> {
    #[cfg(not(unix))]
    {
        let _ = (manifest_path, entry, name, message_id, input);
        anyhow::bail!("Queue execution currently requires a Unix host")
    }
    #[cfg(unix)]
    {
        let input: serde_json::Value =
            serde_json::from_str(&input).context("queue input must be JSON")?;
        let loaded = load(&manifest_path, entry.as_deref())?;
        let spec = loaded.task.ok_or_else(|| anyhow!("application registers no module tasks"))?;
        let service = start_task_service(Some(spec), 1)
            .await?
            .ok_or_else(|| anyhow!("application registers no module tasks"))?;
        let ingress = service.ingress();
        let now_ms = u64::try_from(
            SystemTime::now().duration_since(UNIX_EPOCH).context("system clock")?.as_millis(),
        )
        .context("system clock overflow")?;
        let submitted = ingress.enqueue_queue(&name, message_id, input, now_ms).await;
        let outcome = match submitted {
            Ok(id) => tokio::time::timeout(
                Duration::from_millis(ingress.request_timeout_ms().saturating_add(1_000)),
                async {
                    loop {
                        if let Some(outcome) = ingress.outcome(id).await {
                            break outcome;
                        }
                        tokio::time::sleep(Duration::from_millis(5)).await;
                    }
                },
            )
            .await
            .map_err(|_| anyhow!("queue task timed out")),
            Err(error) => Err(error.into()),
        };
        service.shutdown().await?;
        match outcome? {
            TaskOutcome::Completed { result } => {
                println!("{}", serde_json::to_string(&result)?);
                Ok(())
            }
            TaskOutcome::Failed { error, .. } => anyhow::bail!("queue task failed: {error}"),
            TaskOutcome::Canceled {} => anyhow::bail!("queue task was canceled"),
            TaskOutcome::TimedOut {} => anyhow::bail!("queue task timed out"),
            TaskOutcome::Suspended {} => anyhow::bail!("queue task suspended"),
        }
    }
}

async fn serve(manifest_path: PathBuf, entry: Option<PathBuf>, reload: bool) -> Result<()> {
    let loaded = load(&manifest_path, entry.as_deref())?;
    let pool = SharedPool::with_server_options(
        loaded.isolate,
        loaded.max_request_bytes,
        loaded.websocket,
        loaded.http1,
        loaded.http2,
        Some(loaded.source_map),
    );
    let listener =
        TcpListener::bind(loaded.addr).await.with_context(|| format!("bind {}", loaded.addr))?;
    let mut task_generation = 1u64;
    let mut task_service = start_task_service(loaded.task, task_generation).await?;
    let mut durable = start_dev_durable(loaded.durable).await?;
    let bound = listener.local_addr()?;
    if durable.is_some() {
        println!("tysel durable on");
    }
    println!("tysel listen {bound}");
    io::stdout().flush()?;
    if reload {
        let mut changes = watch(manifest_path.parent().unwrap_or(Path::new(".")))?;
        loop {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => break,
                error = task_service_failure(task_service.as_ref()) => {
                    shutdown_durable(durable.take()).await?;
                    shutdown_task_service(task_service).await?;
                    return Err(error);
                }
                _ = wait_change(&mut changes.rx) => match load(&manifest_path, entry.as_deref()) {
                    Ok(next) => {
                        task_generation = task_generation.saturating_add(1);
                        match start_task_service(next.task.clone(), task_generation).await {
                            Ok(next_tasks) => {
                                match start_dev_durable(next.durable.clone()).await {
                                    Ok(next_durable) => {
                                        eprintln!("tysel reload");
                                        pool.replace_with_server_options(
                                            next.isolate,
                                            next.max_request_bytes,
                                            next.websocket,
                                            next.http1,
                                            next.http2,
                                            Some(next.source_map),
                                        );
                                        shutdown_task_service(task_service).await?;
                                        shutdown_durable(durable.take()).await?;
                                        task_service = next_tasks;
                                        durable = next_durable;
                                    }
                                    Err(err) => eprintln!("error: {err:#}"),
                                }
                            }
                            Err(err) => eprintln!("error: {err:#}"),
                        }
                    }
                    Err(err) => eprintln!("error: {err:#}"),
                },
                accepted = listener.accept() => {
                    let (stream, _) = accepted.context("accept")?;
                    handle_stream(stream, pool.clone());
                }
            }
        }
        shutdown_durable(durable.take()).await?;
        shutdown_task_service(task_service).await?;
        return Ok(());
    }
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            error = task_service_failure(task_service.as_ref()) => {
                shutdown_durable(durable.take()).await?;
                shutdown_task_service(task_service).await?;
                return Err(error);
            }
            accepted = listener.accept() => {
                let (stream, _) = accepted.context("accept")?;
                handle_stream(stream, pool.clone());
            }
        }
    }
    shutdown_durable(durable.take()).await?;
    shutdown_task_service(task_service).await?;
    Ok(())
}

fn load(manifest_path: &Path, entry: Option<&Path>) -> Result<Loaded> {
    let manifest = Manifest::from_path(manifest_path)
        .with_context(|| format!("failed to read {}", manifest_path.display()))?;
    let root = manifest_path.parent().unwrap_or(Path::new("."));
    let entry = match entry {
        Some(path) => path.to_path_buf(),
        None => root.join(&manifest.app.entry),
    };
    let (bundle, source_map) = tysel_build::read_bundle(&entry)
        .with_context(|| format!("failed to bundle {}", entry.display()))?;
    let parsed_source_map =
        Arc::new(SourceMap::parse(&source_map).context("failed to parse generated source map")?);
    let tap = tysel_build::tap_from_app(&manifest, env!("CARGO_PKG_VERSION"), bundle, source_map);
    tysel_engine_qjs::configure_sqlite_path(&tap.manifest.sqlite_path, Some(root));
    tysel_engine_qjs::configure_fs(
        tap.manifest.fs_read.clone(),
        tap.manifest.fs_write.clone(),
        Some(root),
    );
    let file_values = fs::read_to_string(root.join(".env"))
        .ok()
        .map(|text| tysel_engine_qjs::parse_dotenv(&text))
        .unwrap_or_default();
    let postgres = tysel_manifest::resolve_postgres(&tap.manifest.postgres, &file_values);
    tysel_engine_qjs::configure_postgres(
        postgres.as_ref().map(|config| config.url.clone()),
        postgres.is_some_and(|config| config.read_only),
    );
    tysel_engine_qjs::configure_secrets(tysel_engine_qjs::load_declared(
        &tap.manifest.secret_names,
        &file_values,
    ));
    tysel_runtime::configure_llm_from_env(tap.manifest.request_timeout_ms)?;
    tysel_engine_qjs::configure_fetch_hosts(tap.manifest.fetch_hosts.clone());
    tysel_engine_qjs::configure_execution_profile(&tap.manifest.execution_profile);
    tysel_observability::configure_http_log(&tap.manifest.application_id, tap.manifest.json_logs);
    let source = tap.bundle_source()?.to_owned();
    let config = IsolateConfig {
        memory_limit_bytes: tap.manifest.memory_limit_bytes,
        cpu_ms_per_turn: tap.manifest.cpu_ms_per_turn,
        request_timeout_ms: tap.manifest.request_timeout_ms,
    };
    let pool = spawn_app_isolate(
        &tap.manifest.execution_profile,
        &source,
        config,
        tap.manifest.secret_names.clone(),
    )?;
    let addr = tap
        .manifest
        .listen
        .parse()
        .map_err(|_| anyhow!("invalid listen address '{}'", tap.manifest.listen))?;
    let websocket = tap.manifest.websocket && !matches!(pool, AppIsolate::Isolated(_));
    // Inspection of isolated modules must happen inside tysel-worker. Keep the
    // candidate here; start_task_service will discard an empty registry after
    // performing profile-correct inspection.
    let has_tasks = tap.manifest.execution_profile.eq_ignore_ascii_case("isolated")
        || !tysel_engine_qjs::inspect_task_module(&source, config)?.is_empty();
    let task = has_tasks.then(|| TaskSpec {
        application_id: tap.manifest.application_id.clone(),
        source: source.clone(),
        config,
        execution_profile: tap.manifest.execution_profile.clone(),
        secret_names: tap.manifest.secret_names.clone(),
    });
    Ok(Loaded {
        isolate: pool,
        max_request_bytes: tap.manifest.max_request_bytes,
        addr,
        websocket,
        http1: tap.manifest.http1,
        http2: tap.manifest.http2,
        source_map: parsed_source_map,
        task,
        durable: Some(DurableSpec {
            source,
            config,
            sqlite_path: tap.manifest.sqlite_path.clone(),
            execution_profile: tap.manifest.execution_profile.clone(),
            root: root.to_path_buf(),
        }),
    })
}

#[cfg(unix)]
async fn start_task_service(
    spec: Option<TaskSpec>,
    generation: u64,
) -> Result<Option<ModuleTaskService>> {
    let Some(spec) = spec else {
        return Ok(None);
    };
    let socket = std::env::temp_dir()
        .join(format!("tysel-dev-task-{}-{generation}.sock", std::process::id()));
    let service = ModuleTaskService::start(
        socket,
        spec.application_id,
        spec.source,
        spec.config,
        spec.execution_profile,
        spec.secret_names,
    )
    .await?;
    if service.ingress().registry().is_empty() {
        service.shutdown().await?;
        return Ok(None);
    }
    Ok(Some(service))
}

#[cfg(unix)]
async fn task_service_failure(service: Option<&ModuleTaskService>) -> anyhow::Error {
    match service {
        Some(service) => anyhow!(service.failed().await),
        None => std::future::pending().await,
    }
}

#[cfg(not(unix))]
async fn start_task_service(spec: Option<TaskSpec>, _generation: u64) -> Result<Option<()>> {
    if spec.is_some() {
        anyhow::bail!("module tasks currently require a Unix host")
    }
    Ok(None)
}

#[cfg(unix)]
async fn shutdown_task_service(service: Option<ModuleTaskService>) -> Result<()> {
    if let Some(service) = service {
        service.shutdown().await?;
    }
    Ok(())
}

#[cfg(not(unix))]
async fn shutdown_task_service(_service: Option<()>) -> Result<()> {
    Ok(())
}

async fn start_dev_durable(spec: Option<DurableSpec>) -> Result<Option<Arc<DurablePlane>>> {
    let Some(spec) = spec else {
        return Ok(None);
    };
    if spec.execution_profile.eq_ignore_ascii_case("isolated") {
        return Ok(None);
    }
    if !DurablePlane::requested(&spec.sqlite_path, Some(&spec.root), &spec.source, spec.config)? {
        return Ok(None);
    }
    let Some(store) = DurablePlane::open_store(&spec.sqlite_path, Some(&spec.root))? else {
        return Ok(None);
    };
    if !DurablePlane::should_start(store.as_ref(), &spec.source, spec.config)? {
        return Ok(None);
    }
    let owner = format!("tysel-dev-{}", std::process::id());
    Ok(Some(DurablePlane::start(store, spec.source, spec.config, owner)?))
}

async fn shutdown_durable(plane: Option<Arc<DurablePlane>>) -> Result<()> {
    if let Some(plane) = plane {
        plane.shutdown().await?;
    }
    Ok(())
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
    event.paths.iter().any(|path| is_watched(path))
}

fn is_watched(path: &Path) -> bool {
    if ignored(path) {
        return false;
    }
    if path.file_name().and_then(|name| name.to_str()) == Some(".env") {
        return true;
    }
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("ts" | "tsx" | "mts" | "cts" | "js" | "mjs" | "cjs" | "json" | "toml")
    )
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watches_dotenv_and_source_but_not_ignored_paths() {
        assert!(is_watched(Path::new("/app/.env")));
        assert!(is_watched(Path::new("/app/src/index.ts")));
        assert!(is_watched(Path::new("/app/tysel.toml")));
        assert!(!is_watched(Path::new("/app/README.md")));
        assert!(!is_watched(Path::new("/app/node_modules/pkg/.env")));
        assert!(!is_watched(Path::new("/app/node_modules/pkg/index.js")));
    }
}
