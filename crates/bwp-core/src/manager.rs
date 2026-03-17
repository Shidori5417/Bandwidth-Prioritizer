//! Configuration Hot-Reloading Manager
//!
//! Watches the `config.toml` file for changes, deserializes it,
//! and atomically updates the shared configuration state without locking.
//!
//! We use `ArcSwap` to provide true zero-cost (lock-free) reads in the fast path
//! (such as packet inspection loops or ETW event callbacks).

use crate::config::AppConfig;
use crate::{BwpError, BwpResult};
use arc_swap::ArcSwap;
use notify::{Event, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::mpsc;
use std::fs::OpenOptions;
use fs2::FileExt;
use std::io::{Seek, Write};
use tracing::{error, info, warn};

/// Thread-safe, lock-free configuration registry.
///
/// It holds the latest active configuration in an `ArcSwap`. Reader threads (like
/// the throttle engine) can load the config instantly without taking locks.
pub struct ConfigManager {
    /// The actual configuration wrapped in ArcSwap for atomic pointer swaps.
    current: Arc<ArcSwap<AppConfig>>,
    /// Path to the config file being watched.
    config_path: PathBuf,
}

impl ConfigManager {
    /// Initialize a new ConfigManager by reading the file from disk once.
    /// Does not start the background watcher yet.
    pub fn new<P: AsRef<Path>>(path: P) -> BwpResult<Self> {
        let config_path = path.as_ref().to_path_buf();
        let config = Self::load_file(&config_path)?;

        Ok(Self {
            current: Arc::new(ArcSwap::from_pointee(config)),
            config_path,
        })
    }

    /// Default initialization. If the file is missing, it creates a default configuration.
    pub fn new_or_default<P: AsRef<Path>>(path: P) -> Self {
        let config_path = path.as_ref().to_path_buf();
        let config = match Self::load_file(&config_path) {
            Ok(c) => c,
            Err(e) => {
                warn!(%e, "Failed to load config file. Using defaults.");
                AppConfig::default()
            }
        };

        Self {
            current: Arc::new(ArcSwap::from_pointee(config)),
            config_path,
        }
    }

    /// Read the `AppConfig` from a TOML file.
    fn load_file(path: &Path) -> BwpResult<AppConfig> {
        let content = std::fs::read_to_string(path).map_err(|e| {
            BwpError::Config(format!("Could not read file {}: {}", path.display(), e))
        })?;

        let config: AppConfig = toml::from_str(&content).map_err(|e| {
            BwpError::Config(format!("Invalid TOML in {}: {}", path.display(), e))
        })?;

        Ok(config)
    }

    /// Loads the current configuration from disk, applies the given closure to modify it,
    /// and writes the changes back to disk. This triggers the hot-reload task.
    pub fn update_config<F>(&self, modifier: F) -> BwpResult<()>
    where
        F: FnOnce(&mut AppConfig),
    {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&self.config_path)
            .map_err(BwpError::Io)?;

        file.lock_exclusive().map_err(BwpError::Io)?;

        // Read the config safely by reading from the already opened locked file.
        use std::io::Read;
        let mut content = String::new();
        file.read_to_string(&mut content).map_err(BwpError::Io)?;

        let mut config: AppConfig = toml::from_str(&content).map_err(|e| {
            BwpError::Config(format!("Invalid TOML in {}: {}", self.config_path.display(), e))
        })?;

        modifier(&mut config);

        let new_content = toml::to_string_pretty(&config)
            .map_err(|e| BwpError::Config(format!("Failed to serialize config: {}", e)))?;

        file.set_len(0).map_err(BwpError::Io)?;
        file.rewind().map_err(BwpError::Io)?;
        file.write_all(new_content.as_bytes()).map_err(BwpError::Io)?;

        file.unlock().map_err(BwpError::Io)?;
        Ok(())
    }

    /// Returns a shared pointer to the `ArcSwap` representing the current configuration.
    /// Fast-path reads can call `.load()` on this continuously without overhead.
    #[must_use]
    pub fn get_shared_config(&self) -> Arc<ArcSwap<AppConfig>> {
        self.current.clone()
    }

    /// Spawns an asynchronous hot-reload watcher.
    ///
    /// The `notify` crate will listen for filesystem changes. Upon modification,
    /// the watcher attempts to re-parse the config file. If successful, it atomically
    /// swaps the new config pointer.
    pub fn spawn_hot_reload_task(&self) {
        let (tx, mut rx) = mpsc::channel(1);
        let path_clone = self.config_path.clone();
        let shared_config = self.current.clone();

        // Standard library thread because `notify::RecommendedWatcher` is synchronous/blocking in its callback.
        // We bridge it to the async world via a tokio channel.
        std::thread::spawn(move || {
            let mut watcher = notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    if event.kind.is_modify() || event.kind.is_create() {
                        // Send a signal into the sync->async bridge
                        let _ = tx.blocking_send(());
                    }
                }
            })
            .expect("Failed to initialize file watcher");

            // Attempt to watch the directory of the config file.
            let parent_dir = path_clone.parent().unwrap_or(Path::new("."));
            if let Err(e) = watcher.watch(parent_dir, RecursiveMode::NonRecursive) {
                error!("File watcher failed to attach to directory: {e}");
                return;
            }

            // Keep the thread alive, pumping events into the channel.
            loop {
                std::thread::park();
            }
        });

        // Async task to consume the debounced watch events and perform the reload.
        let file_path = self.config_path.clone();
        tokio::spawn(async move {
            info!("Hot-reload watcher started on {:?}", file_path);
            let mut last_reload_time = tokio::time::Instant::now();

            while rx.recv().await.is_some() {
                // Debounce simple: only process one reload per 500ms
                if last_reload_time.elapsed().as_millis() < 500 {
                    continue;
                }
                last_reload_time = tokio::time::Instant::now();

                // Yield to allow the OS to complete file write buffers
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;

                match Self::load_file(&file_path) {
                    Ok(new_config) => {
                        info!("Configuration hot-reloaded successfully.");
                        // The magic happens here: atomic pointer replacement!
                        shared_config.store(Arc::new(new_config));
                    }
                    Err(e) => {
                        error!("Hot-reload failed, retaining old config. Error: {e}");
                    }
                }
            }
        });
    }
}
