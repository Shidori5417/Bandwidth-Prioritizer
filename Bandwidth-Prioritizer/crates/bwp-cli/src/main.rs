//! Bandwidth-Prioritizer CLI Frontend
//! 
//! Provides a `clap`-based CLI and `ratatui` UI for managing the `bwp-engine` daemon via IPC.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

use bwp_core::ipc::{IpcCommand, IpcResponse};

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
    
    /// Remove an application from the priority whitelist dynamically.
    Remove {
        /// Process name to remove.
        app: String,
    },
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
            println!("🚀 Starting bwp-engine daemon...");
            // TODO: Use std::process::Command to spawn engine binary and detach.
            println!("(Daemon spawn logic to be implemented. Currently, please run the engine manually.)");
        }
        CliCommand::Stop => {
            send_ipc_and_print(IpcCommand::Stop).await?;
            println!("🛑 Daemon stop command sent.");
        }
        CliCommand::Status => {
            println!("📊 Querying engine status...");
            send_ipc_and_print(IpcCommand::Status).await?;
        }
        CliCommand::Top => {
            println!("💻 TUI Dashboard (Top) is not yet implemented.");
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
        Some(IpcResponse::StatusPayload { state, active_profile, throttled_pids_count, priority_pids_count, current_limit_kbps }) => {
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
