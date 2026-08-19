use std::io::{self, Write};
use std::net::SocketAddr;
use std::sync::Arc;

use tokio::net::TcpListener;
use tysel_engine::IsolateConfig;
use tysel_engine_qjs::IsolatePool;
use tysel_package::Tap;

use crate::http::{HttpError, serve_with_websocket};

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
    let pool = IsolatePool::spawn(1, &bundle, config)?;
    let listener = TcpListener::bind(addr).await?;
    let bound = listener.local_addr()?;
    println!("tysel listen {bound}");
    io::stdout().flush()?;
    serve_with_websocket(
        listener,
        Arc::new(pool),
        tap.manifest.max_request_bytes,
        tap.manifest.websocket,
    )
    .await?;
    Ok(())
}
