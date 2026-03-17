//! # Bandwidth-Prioritizer (bwp)
//!
//! System-level network traffic shaper that dynamically throttles
//! non-critical applications when priority apps are active.

use anyhow::Result;
use clap::Parser;
use tracing_subscriber::EnvFilter;

/// Bandwidth-Prioritizer — Dynamic network traffic shaper.
#[derive(Parser, Debug)]
#[command(
    name = "bwp",
    version,
    author,
    about = "Dynamically throttle non-critical apps when priority applications are active."
)]
enum Cli {
    /// Start the bandwidth prioritizer daemon.
    Start {
        /// Path to the configuration file.
        #[arg(short, long, default_value = "config.toml")]
        config: String,

        /// Run in foreground (don't daemonize).
        #[arg(short, long)]
        foreground: bool,
    },

    /// Stop the running daemon.
    Stop,

    /// Show current status (active rules, throttle state).
    Status,

    /// Add a process to the critical (priority) application list.
    Add {
        /// Process name to add (e.g. "discord", "zoom").
        #[arg(required = true)]
        app: String,
    },

    /// Switch to a named profile.
    Profile {
        /// Profile name (e.g. "gaming", "meeting").
        #[arg(required = true)]
        name: String,
    },
}

fn main() -> Result<()> {
    // Initialize tracing subscriber with environment filter
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(true)
        .with_thread_ids(true)
        .with_file(true)
        .with_line_number(true)
        .init();

    let cli = Cli::parse();

    match cli {
        Cli::Start { config, foreground } => {
            tracing::info!(
                config_path = %config,
                foreground,
                "Starting Bandwidth-Prioritizer daemon…"
            );
            // TODO(faz-1): Initialize process monitor
            // TODO(faz-2): Load configuration
            // TODO(faz-3): Initialize traffic controller
            // TODO(faz-4): Start throttle engine event loop
            println!("🚀 Bandwidth-Prioritizer v{}", env!("CARGO_PKG_VERSION"));
            println!("   Config: {config}");
            println!(
                "   Mode:   {}",
                if foreground {
                    "foreground"
                } else {
                    "daemon"
                }
            );
            println!("\n⚙️  Engine not yet implemented — see FAZ 1-4 in progress.md");
        }
        Cli::Stop => {
            tracing::info!("Stopping daemon…");
            println!("🛑 Stop command received (daemon IPC not yet implemented)");
        }
        Cli::Status => {
            tracing::info!("Querying status…");
            println!("📊 Status query (not yet implemented)");
        }
        Cli::Add { app } => {
            tracing::info!(app = %app, "Adding critical app");
            println!("➕ Adding '{app}' to critical app list (persistence not yet implemented)");
        }
        Cli::Profile { name } => {
            tracing::info!(profile = %name, "Switching profile");
            println!("🔄 Switching to profile '{name}' (not yet implemented)");
        }
    }

    Ok(())
}
