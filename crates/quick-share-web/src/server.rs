use crate::{EndpointInfo, PreparedWebSecurity, SecurityError, endpoint_info};
use axum::Router;
use axum_server::Handle;
use std::{
    io,
    net::{IpAddr, SocketAddr},
    time::Duration,
};
use thiserror::Error;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

pub struct RunningWebServer {
    pub endpoint: EndpointInfo,
    local_addr: SocketAddr,
    handle: Handle<SocketAddr>,
    server_task: Option<JoinHandle<Result<(), io::Error>>>,
    shutdown_task: Option<JoinHandle<()>>,
}

impl RunningWebServer {
    #[must_use]
    pub const fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub fn shutdown(&self) {
        self.handle.graceful_shutdown(Some(Duration::from_secs(5)));
    }

    pub async fn wait(mut self) -> Result<(), ServerError> {
        let server_task = self.server_task.take().ok_or(ServerError::Internal)?;
        server_task.await.map_err(|_| ServerError::Task)??;
        if let Some(task) = self.shutdown_task.take() {
            task.abort();
        }
        Ok(())
    }
}

impl std::fmt::Debug for RunningWebServer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RunningWebServer")
            .field("endpoint", &self.endpoint)
            .field("local_addr", &self.local_addr)
            .finish_non_exhaustive()
    }
}

impl Drop for RunningWebServer {
    fn drop(&mut self) {
        self.handle
            .graceful_shutdown(Some(Duration::from_millis(100)));
        if let Some(task) = self.shutdown_task.take() {
            task.abort();
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn start_web_server(
    router: Router,
    bind: SocketAddr,
    advertise_ip: IpAddr,
    security: PreparedWebSecurity,
    token: &str,
    ttl: Duration,
    max_downloads: Option<u32>,
    cancellation: CancellationToken,
    quota_shutdown: CancellationToken,
) -> Result<RunningWebServer, ServerError> {
    if ttl.is_zero() {
        return Err(ServerError::InvalidTimeout);
    }
    let listener = std::net::TcpListener::bind(bind)?;
    listener.set_nonblocking(true)?;
    let local_addr = listener.local_addr()?;
    let endpoint = endpoint_info(
        local_addr,
        advertise_ip,
        &security,
        token,
        ttl,
        max_downloads,
    )?;
    let handle = Handle::new();
    let server_handle = handle.clone();
    let server_task = if let Some(config) = security.rustls_config() {
        let server = axum_server::from_tcp_rustls(listener, config)?
            .handle(server_handle)
            .serve(router.into_make_service());
        tokio::spawn(server)
    } else {
        let server = axum_server::from_tcp(listener)?
            .handle(server_handle)
            .serve(router.into_make_service());
        tokio::spawn(server)
    };
    let shutdown_handle = handle.clone();
    let shutdown_task = tokio::spawn(async move {
        let deadline = tokio::time::Instant::now() + ttl;
        let grace = tokio::select! {
            _ = tokio::time::sleep_until(deadline) => Duration::from_secs(5),
            _ = cancellation.cancelled() => Duration::from_secs(5),
            _ = quota_shutdown.cancelled() => deadline.saturating_duration_since(tokio::time::Instant::now()),
        };
        shutdown_handle.graceful_shutdown(Some(grace));
    });
    Ok(RunningWebServer {
        endpoint,
        local_addr,
        handle,
        server_task: Some(server_task),
        shutdown_task: Some(shutdown_task),
    })
}

#[derive(Debug, Error)]
pub enum ServerError {
    #[error("Web server timeout must be greater than zero")]
    InvalidTimeout,
    #[error("Web server socket failed")]
    Io(#[from] io::Error),
    #[error("Web endpoint security metadata failed")]
    Security(#[from] SecurityError),
    #[error("Web server task failed")]
    Task,
    #[error("Web server internal state is unavailable")]
    Internal,
}
