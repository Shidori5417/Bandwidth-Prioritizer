//! Bandwidth-Prioritizer CLI Frontend
//! 
//! Provides a `clap`-based CLI and `ratatui` UI for managing the `bwp-engine` daemon via IPC.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

use bwp_core::ipc::{IpcCommand, IpcResponse};

mod tui;
mod service;

#[cfg(target_os = "windows")]
use bwp_core::ipc::windows::connect_client;

#[cfg(target_os = "linux")]
use bwp_core::ipc::linux::connect_client;

#[derive(Parser, Debug)]
#[command(
    name = "bwp",
    version,
    author,
    about = "Bandwidth-Prioritizer - Dynamic network traffic shaper.\nManages the background throttle engine via IPC."
)]
struct CliArgs {
    #[command(subcommand)]
    command: CliCommand,
}

#[derive(Subcommand, Debug)]
enum CliCommand {
    /// Start the bandwidth prioritizer daemon in the background.
    Start,
    
    /// Stop the running bandwidth prioritizer daemon.
    Stop,
    
    /// Show current status (active rules, throttle state, throttled PIDs).
    Status,
    
    /// Open the real-time TUI dashboard.
    Top,
    
    /// Switch to a named profile from the config.
    Set {
        /// Profile name (e.g. "gaming", "meeting").
        profile: String,
    },
    
    /// Add an application to the priority whitelist dynamically.
    Add {
        /// Process name to add (e.g. "discord.exe").
        app: String,
    },
    
    Remove {
        /// Process name to remove.
        app: String,
    },
    
    /// Manage the background daemon as a persistent OS service.
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },
}

#[derive(Subcommand, Debug)]
enum ServiceAction {
    /// Register the background engine as an OS service (Requires Admin)
    Install,
    /// Unregister the OS service (Requires Admin)
    Uninstall,
    /// Start the registered OS service (Requires Admin)
    Start,
    /// Stop the running OS service (Requires Admin)
    Stop,
    /// Check the status of the OS service (Requires Admin)
    Status,
}

cfg_if::cfg_if! {
    if #[cfg(target_os = "windows")] {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")))
        .with_target(false)
        .without_time()
        .init();

    let args = CliArgs::parse();

    match args.command {
        CliCommand::Start => {
            if let Ok(Some(pid)) = bwp_core::pid::read_pid() {
                if bwp_core::pid::is_process_running(pid) {
                    println!("Engine is already running (PID: {})", pid);
                    return Ok(());
                } else {
                    println!("Found stale PID file. Starting fresh engine...");
                }
            } else {
                println!("🚀 Starting bwp-engine daemon...");
            }

            // Locate engine binary relative to CLI binary (same build directory)
            let engine_path = std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|d| d.join("bwp-engine.exe")))
                .unwrap_or_else(|| std::path::PathBuf::from("bwp-engine"));

            let mut command = std::process::Command::new(&engine_path);
            let _ = command.stdin(std::process::Stdio::null())
                   .stdout(std::process::Stdio::null())
                   .stderr(std::process::Stdio::null());

            #[cfg(target_os = "windows")]
            {
                let _ = command.creation_flags(CREATE_NO_WINDOW);
            }

            match command.spawn() {
                Ok(child) => {
                    println!("✅ Daemon spawned successfully (PID: {}).", child.id());
                }
                Err(e) => {
                    println!("❌ Failed to spawn daemon: {}", e);
                    println!("Make sure 'bwp-engine.exe' is in your PATH or current directory.");
                }
            }
        }
        CliCommand::Stop => {
            println!("🛑 Sending stop command to engine...");
            match stop_engine().await {
                Ok(_) => {
                    bwp_core::pid::remove_pid().unwrap_or(());
                    println!("✅ Engine stopped.");
                }
                Err(_) => {
                    if let Ok(Some(pid)) = bwp_core::pid::read_pid() {
                        println!("Could not connect to stop gracefully. You may need to manually kill PID {}.", pid);
                        bwp_core::pid::remove_pid().unwrap_or(());
                    } else {
                        println!("Engine does not appear to be running.");
                    }
                }
            }
        }
        CliCommand::Status => {
            println!("📊 Querying engine status...");
            send_ipc_and_print(IpcCommand::Status).await?;
        }
        CliCommand::Top => {
            tui::run_tui().await?;
        }
        CliCommand::Set { profile } => {
            println!("🔄 Setting profile to '{}'...", profile);
            send_ipc_and_print(IpcCommand::ApplyProfile(profile)).await?;
        }
        CliCommand::Add { app } => {
            println!("➕ Adding '{}' to priority whitelist...", app);
            send_ipc_and_print(IpcCommand::AddApp(app)).await?;
        }
        CliCommand::Remove { app } => {
            println!("➖ Removing '{}' from priority whitelist...", app);
            send_ipc_and_print(IpcCommand::RemoveApp(app)).await?;
        }
        CliCommand::Service { action } => {
            match action {
                ServiceAction::Install => service::install()?,
                ServiceAction::Uninstall => service::uninstall()?,
                ServiceAction::Start => service::start()?,
                ServiceAction::Stop => service::stop()?,
                ServiceAction::Status => service::status()?,
            }
        }
    }

    Ok(())
}

async fn send_ipc_and_print(command: IpcCommand) -> Result<()> {
    // Attempt connection
    let mut conn = connect_client().await.context("Failed to connect to bwp-engine. Is the daemon running?")?;
    
    conn.send_command(&command).await.context("Failed to send command to daemon")?;
    
    match conn.recv_response().await.context("Failed to receive response from daemon")? {
        Some(IpcResponse::Ok) => {
            println!("✅ Success.");
        }
        Some(IpcResponse::Error(err)) => {
            println!("❌ Engine Error: {}", err);
        }
        Some(IpcResponse::StatusPayload { state, active_profile, throttled_pids_count, priority_pids_count, current_limit_kbps, .. }) => {
            println!("\n--- Engine Status ---");
            println!("State Mode        : {}", state);
            println!("Active Profile    : {}", active_profile.as_deref().unwrap_or("None"));
            println!("Priority PIDs     : {}", priority_pids_count);
            println!("Throttled PIDs    : {}", throttled_pids_count);
            println!("Enforced Limit    : {} Kbps", current_limit_kbps);
            println!("---------------------");
        }
        None => {
            println!("⚠️ Connection closed by daemon before response.");
        }
    }
    
    Ok(())
}

/// Sends Stop to engine; treats both Ok and EOF (pipe close) as success.
/// Uses a short timeout so CLI doesn't hang if engine is unresponsive.
async fn stop_engine() -> Result<()> {
    let mut conn = connect_client().await.context("Failed to connect to bwp-engine. Is the daemon running?")?;
    conn.send_command(&IpcCommand::Stop).await.context("Failed to send Stop command")?;
    
    // Engine may close the pipe before sending a response (race between break + flush).
    // Either Ok(Some(Ok)) or Ok(None) [EOF] is fine — engine is shutting down.
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        conn.recv_response(),
    ).await;
    
    match result {
        Ok(Ok(_)) | Err(_) => Ok(()), // response received OR timeout (engine already dead)
        Ok(Err(e)) => Err(e.into()),
    }
}
