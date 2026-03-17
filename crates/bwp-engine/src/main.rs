use anyhow::Result;
use bwp_core::pid::{write_pid, remove_pid};
use bwp_core::manager::ConfigManager;
use bwp_engine::{Engine, EngineEvent};
use bwp_engine::ipc_server::start_ipc_server;
use bwp_controller::windows::WindowsController;
use bwp_monitor::WindowsMonitor as Monitor;
use bwp_monitor::ProcessMonitor; // Import trait to satisfy `run()` if needed
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{info, error};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    info!("Starting Bandwidth-Prioritizer Engine Daemon...");

    // Write PID file
    if let Err(e) = write_pid() {
        error!("Failed to write PID file: {}", e);
        return Err(e.into());
    }

    // 1. Initialize Monitor
    let monitor = Monitor::new();
    let mut monitor_rx = monitor.watch_connections().await?;
    
    // Convert monitor events to engine events
    let (engine_tx, engine_rx) = mpsc::channel(1024);
    let engine_tx_for_monitor = engine_tx.clone();

    tokio::spawn(async move {
        while let Some(event) = monitor_rx.recv().await {
            if engine_tx_for_monitor.send(EngineEvent::Monitor(event)).await.is_err() {
                break;
            }
        }
    });

    // 2. Initialize Controller
    let controller = Arc::new(WindowsController::new()?);

    // 3. Initialize Config Manager
    let config_manager = Arc::new(ConfigManager::new("config.toml")?);

    // 4. Start IPC Server
    let ipc_engine_tx = engine_tx.clone();
    tokio::spawn(async move {
        start_ipc_server(ipc_engine_tx).await;
    });

    // 5. Start Engine processing loop
    let mut engine = Engine::new(config_manager, controller);
    if let Err(e) = engine.run_event_loop(engine_rx).await {
        error!("Engine event loop terminated with error: {}", e);
    }

    // Clean up
    let _ = remove_pid();
    Ok(())
}
