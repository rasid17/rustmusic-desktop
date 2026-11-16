//! Lifecycle management for the Mobile Remote HTTP server.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

use sqlx::SqlitePool;
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

use crate::core::dlna_server::net::detect_local_ipv4;
use crate::core::mobile_remote::router::{build_router, RouterState};
use crate::core::mobile_remote::state::MobileRemoteSharedState;

pub struct MobileRemoteServer {
    port: u16,
    shutdown_tx: Option<broadcast::Sender<()>>,
    handle: Option<JoinHandle<()>>,
}

impl MobileRemoteServer {
    pub fn new(port: u16) -> Self {
        Self {
            port,
            shutdown_tx: None,
            handle: None,
        }
    }

    pub fn is_running(&self) -> bool {
        self.shutdown_tx.is_some()
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub async fn start(
        &mut self,
        pool: SqlitePool,
        shared: Arc<MobileRemoteSharedState>,
        app_handle: tauri::AppHandle,
    ) -> Result<(), String> {
        if self.is_running() {
            return Ok(());
        }

        let (shutdown_tx, _) = broadcast::channel::<()>(1);
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), self.port);
        let listener = TcpListener::bind(addr)
            .await
            .map_err(|e| format!("Failed to bind Mobile Remote on port {}: {e}", self.port))?;

        let local_ip = detect_local_ipv4();
        log::info!(
            "📱 Mobile Remote server listening on http://{}:{}",
            local_ip,
            self.port
        );

        let router = build_router(RouterState {
            pool,
            shared,
            app_handle,
        });

        let mut shutdown_rx = shutdown_tx.subscribe();
        let handle = tokio::spawn(async move {
            let _ = axum::serve(listener, router)
                .with_graceful_shutdown(async move {
                    let _ = shutdown_rx.recv().await;
                })
                .await;
            log::debug!("📱 Mobile Remote HTTP server stopped");
        });

        self.shutdown_tx = Some(shutdown_tx);
        self.handle = Some(handle);
        Ok(())
    }

    pub async fn stop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.await;
        }
    }
}
