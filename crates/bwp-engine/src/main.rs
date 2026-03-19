use anyhow::Result;
use bwp_controller::windows::WindowsController;
use bwp_core::manager::ConfigManager;
use bwp_core::pid::{remove_pid, write_pid};
use bwp_engine::ipc_server::start_ipc_server;
use bwp_engine::{Engine, EngineEvent};
use bwp_monitor::ProcessMonitor; // Import trait to satisfy `run()` if needed
use bwp_monitor::WindowsMonitor as Monitor;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{error, info};

mod service;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let mut args = std::env::args();
    let _ = args.next(); // skip binary name

    // Internal hack: when service gets `Stop`, it calls its own binary with `--internal-stop`
    // to trigger graceful IPC shutdown.
    if let Some(arg) = args.next() {
        if arg == "--internal-stop" {
            // Spawn a dummy tokio to send the stop IPC signal
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(async {
                if let Ok(mut conn) = bwp_core::ipc::windows::connect_client().await {
                    let _ = conn.send_command(&bwp_core::ipc::IpcCommand::Stop).await;
                }
            });
            return Ok(());
        } else if arg == "--service" {
            #[cfg(target_os = "windows")]
            {
                info!("Starting as Windows Service...");
                return service::run_as_service();
            }
            #[cfg(not(target_os = "windows"))]
            {
                return Err(anyhow::anyhow!("Service mode only supported on Windows"));
            }
        }
    }

    // Default mode: run normally in console
    info!("Starting bwp-engine core locally...");
    run_engine_core()
}

#[tokio::main]
pub async fn run_engine_core() -> Result<()> {
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
            if engine_tx_for_monitor
                .send(EngineEvent::Monitor(event))
                .await
                .is_err()
            {
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
