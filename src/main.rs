mod config;
mod failed_logs;
mod http;
mod model;
mod store;
mod worker;

use std::future::IntoFuture;
use std::io::IsTerminal;
use std::process::ExitCode;

use anyhow::{Context, Result, anyhow};
use tokio::net::TcpListener;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::mpsc;
use tracing::{error, info};

use crate::config::Config;
use crate::failed_logs::FailedLogs;
use crate::store::PgStore;
use crate::worker::Worker;

const QUEUE_SIZE: usize = 1000;

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .with_target(false)
        .init();

    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            error!("{err:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<()> {
    let config = Config::from_env()?;
    let mut shutdown = Box::pin(shutdown_signal()?);

    let store = tokio::select! {
        store = PgStore::connect(config.postgres) => store?,
        () = &mut shutdown => {
            info!("Shutting down before the database was reached");
            return Ok(());
        }
    };
    let (queue, mut receiver) = mpsc::channel(QUEUE_SIZE);
    let mut worker = tokio::spawn(async move {
        Worker::new(store, FailedLogs::new(config.failed_logs_file))
            .run(&mut receiver)
            .await
    });

    let listener = TcpListener::bind(("0.0.0.0", config.listen_port))
        .await
        .with_context(|| format!("Unable to listen on port {}", config.listen_port))?;
    if config.app_env == "prod" {
        info!("Running api server in production mode");
    } else {
        info!("Running api server in dev mode");
    }
    info!("Listening on port {}", config.listen_port);

    // The server holds the only sender: once it has stopped, the worker drains
    // the queue and returns.
    let server = axum::serve(listener, http::router(queue))
        .with_graceful_shutdown(shutdown)
        .into_future();

    tokio::select! {
        served = server => served.context("HTTP server failed")?,
        stopped = &mut worker => {
            stopped??;
            return Err(anyhow!("Log worker stopped unexpectedly"));
        }
    }

    info!("Shutting down, draining the log queue");
    worker.await?
}

/// Registers the handlers right away: as the container's PID 1, the process
/// ignores SIGTERM and SIGINT as long as it has none.
fn shutdown_signal() -> Result<impl Future<Output = ()> + Send + 'static> {
    let mut interrupt = signal(SignalKind::interrupt()).context("Unable to listen for SIGINT")?;
    let mut terminate = signal(SignalKind::terminate()).context("Unable to listen for SIGTERM")?;
    Ok(async move {
        tokio::select! {
            _ = interrupt.recv() => {}
            _ = terminate.recv() => {}
        }
    })
}
