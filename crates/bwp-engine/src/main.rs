//! `bwp-engine`: the Bandwidth-Prioritizer daemon.

#[cfg(not(windows))]
fn main() -> anyhow::Result<()> {
    anyhow::bail!(
        "bwp-engine only runs on Windows: the Linux build exists for development and CI, \
         there is no connection monitor or traffic shaper for Linux yet"
    )
}

#[cfg(windows)]
mod service;

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    windows_main::main()
}

#[cfg(windows)]
mod windows_main {
    use std::sync::Arc;

    use anyhow::Result;
    use bwp_controller::windows::WindowsController;
    use bwp_core::config::resolve_config_path;
    use bwp_core::manager::ConfigManager;
    use bwp_core::pid::{remove_pid, write_pid};
    use bwp_engine::ipc_server::start_ipc_server;
    use bwp_engine::{Engine, EngineEvent};
    use bwp_monitor::ProcessMonitor;
    use bwp_monitor::WindowsMonitor as Monitor;
    use tokio::sync::mpsc;
    use tracing::{error, info, warn};

    pub fn main() -> Result<()> {
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
                info!("Starting as Windows Service...");
                return crate::service::run_as_service();
            }
        }

        // Default mode: run normally in console
        info!("Starting bwp-engine core locally...");
        run_engine_core()
    }

    /// Runs the engine until it is stopped through IPC.
    #[tokio::main]
    pub async fn run_engine_core() -> Result<()> {
        info!("Starting Bandwidth-Prioritizer Engine Daemon...");

        // Write PID file
        if let Err(e) = write_pid() {
            error!("Failed to write PID file: {}", e);
            return Err(e.into());
        }

        // 1. Initialize Config Manager. The path is resolved relative to the executable
        // (a service starts in System32), and defaults are used if it cannot be loaded.
        let config_path = resolve_config_path();
        info!("Using configuration file {}", config_path.display());
        let config_manager = Arc::new(ConfigManager::new_or_default(&config_path));
        if !config_path.exists() {
            warn!(
                "Configuration file {} not found, running with built-in defaults. \
                 Set BWP_CONFIG or place config.toml next to bwp-engine.exe.",
                config_path.display()
            );
        }
        config_manager.spawn_hot_reload_task();

        // 2. Initialize Monitor (poll interval from the config)
        let poll_interval_ms = config_manager
            .get_shared_config()
            .load()
            .general
            .poll_interval_ms;
        let monitor =
            Monitor::with_poll_interval(std::time::Duration::from_millis(poll_interval_ms));
        let mut monitor_rx = monitor.watch_connections().await?;

        // Convert monitor events to engine events
        let (engine_tx, engine_rx) = mpsc::channel(1024);
        let engine_tx_for_monitor = engine_tx.clone();

        let _monitor_bridge = tokio::spawn(async move {
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

        // 3. Initialize Controller. Packet shaping is opt-in: it needs the
        // `experimental-windivert` build feature and `[shaping] experimental_windivert`.
        let experimental_windivert = config_manager
            .get_shared_config()
            .load()
            .shaping
            .experimental_windivert;
        let controller = Arc::new(WindowsController::with_shaping(experimental_windivert)?);

        // 4. Start IPC Server
        let ipc_engine_tx = engine_tx.clone();
        let _ipc_task = tokio::spawn(async move {
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
}
