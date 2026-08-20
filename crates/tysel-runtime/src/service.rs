use std::io::{self, Write};
use std::net::SocketAddr;

use tokio::net::TcpListener;
use tysel_engine::IsolateConfig;
use tysel_package::Tap;

use crate::http::{AppIsolate, HttpError, serve_with_websocket, spawn_app_isolate};

#[derive(Debug, thiserror::Error)]
pub enum StubError {
    #[error(transparent)]
    Package(#[from] tysel_package::PackageError),
    #[error(transparent)]
    Http(#[from] HttpError),
    #[error(transparent)]
    Engine(#[from] tysel_engine::EngineError),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("invalid listen address '{0}'")]
    Listen(String),
}

pub async fn run_stub() -> Result<(), StubError> {
    run_tap(Tap::from_current_exe()?).await
}

pub async fn run_tap(tap: Tap) -> Result<(), StubError> {
    let addr: SocketAddr =
        tap.manifest.listen.parse().map_err(|_| StubError::Listen(tap.manifest.listen.clone()))?;
    let config = IsolateConfig {
        memory_limit_bytes: tap.manifest.memory_limit_bytes,
        cpu_ms_per_turn: tap.manifest.cpu_ms_per_turn,
        request_timeout_ms: tap.manifest.request_timeout_ms,
    };
    let bundle = tap.bundle_source()?.to_owned();
    tysel_engine_qjs::configure_sqlite_path(&tap.manifest.sqlite_path, None);
    tysel_engine_qjs::configure_fs(
        tap.manifest.fs_read.clone(),
        tap.manifest.fs_write.clone(),
        None,
    );
    let postgres =
        tysel_manifest::resolve_postgres(&tap.manifest.postgres, &std::collections::HashMap::new());
    tysel_engine_qjs::configure_postgres(
        postgres.as_ref().map(|config| config.url.clone()),
        postgres.is_some_and(|config| config.read_only),
    );
    tysel_engine_qjs::configure_secrets(tysel_engine_qjs::load_declared(
        &tap.manifest.secret_names,
        &std::collections::HashMap::new(),
    ));
    tysel_engine_qjs::configure_fetch_hosts(tap.manifest.fetch_hosts.clone());
    tysel_engine_qjs::configure_execution_profile(&tap.manifest.execution_profile);
    tysel_observability::configure_http_log(&tap.manifest.application_id, tap.manifest.json_logs);
    let pool = spawn_app_isolate(
        &tap.manifest.execution_profile,
        &bundle,
        config,
        tap.manifest.secret_names.clone(),
    )?;
    let websocket = tap.manifest.websocket && !matches!(pool, AppIsolate::Isolated(_));
    let listener = TcpListener::bind(addr).await?;
    let bound = listener.local_addr()?;
    println!("tysel listen {bound}");
    io::stdout().flush()?;
    serve_with_websocket(listener, pool, tap.manifest.max_request_bytes, websocket).await?;
    Ok(())
}
