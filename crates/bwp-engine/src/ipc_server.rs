use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};
use tracing::{error, info};

use crate::state_machine::EngineEvent;
use bwp_core::ipc::{IpcCommand, IpcConnection, IpcResponse};

#[cfg(target_os = "windows")]
use bwp_core::ipc::windows::{bind_server, loop_server};

#[cfg(target_os = "linux")]
use bwp_core::ipc::linux::bind_server as platform_bind;

/// Start the background IPC Server that listens for CLI connections,
/// and delegates commands to the main Engine event queue.
pub async fn start_ipc_server(engine_tx: mpsc::Sender<EngineEvent>) {
    #[cfg(target_os = "windows")]
    run_windows_ipc_server(engine_tx).await;

    #[cfg(target_os = "linux")]
    run_linux_ipc_server(engine_tx).await;
}

#[cfg(target_os = "windows")]
async fn run_windows_ipc_server(engine_tx: mpsc::Sender<EngineEvent>) {
    info!("Starting Windows Named Pipe IPC Server...");

    // First instance binding
    let mut server = match bind_server() {
        Ok(s) => s,
        Err(e) => {
            error!("Failed to bind Named Pipe: {}", e);
            return;
        }
    };

    loop {
        // Wait for a client to connect
        if let Err(e) = server.connect().await {
            error!("Named Pipe connection error: {}", e);
            // Recreate server to prevent burning CPU
            server = loop {
                match loop_server() {
                    Ok(s) => break s,
                    Err(_) => tokio::time::sleep(std::time::Duration::from_secs(1)).await,
                }
            };
            continue;
        }

        // We have a connected client!
        let connected_client = server;
        server = loop {
            match loop_server() {
                Ok(s) => break s,
                Err(e) => {
                    error!("Failed to create next Named Pipe instance: {}", e);
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
            }
        };

        let tx = engine_tx.clone();
        tokio::spawn(async move {
            handle_ipc_connection(IpcConnection::new(connected_client), tx).await;
        });
    }
}

#[cfg(target_os = "linux")]
async fn run_linux_ipc_server(engine_tx: mpsc::Sender<EngineEvent>) {
    info!("Starting Linux Unix Socket IPC Server...");

    let listener = match platform_bind() {
        Ok(l) => l,
        Err(e) => {
            error!("Failed to bind Unix Socket: {}", e);
            return;
        }
    };

    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let tx = engine_tx.clone();
                tokio::spawn(async move {
                    handle_ipc_connection(IpcConnection::new(stream), tx).await;
                });
            }
            Err(e) => {
                error!("Failed to accept Unix Socket connection: {}", e);
            }
        }
    }
}

async fn handle_ipc_connection<S>(mut conn: IpcConnection<S>, engine_tx: mpsc::Sender<EngineEvent>)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    // Wait for the payload
    match conn.recv_command().await {
        Ok(Some(command)) => {
            // Forward it to the Engine
            let (resp_tx, resp_rx) = oneshot::channel();
            if engine_tx
                .send(EngineEvent::Ipc(command, resp_tx))
                .await
                .is_err()
            {
                error!("Engine event queue is closed.");
                return;
            }

            // Wait for Engine to process and respond
            if let Ok(response) = resp_rx.await {
                if let Err(e) = conn.send_response(&response).await {
                    error!("Failed to send IPC response back to client: {}", e);
                }
            }
        }
        Ok(None) => {}
        Err(e) => error!("Malformed IPC command from client: {}", e),
    }
}
