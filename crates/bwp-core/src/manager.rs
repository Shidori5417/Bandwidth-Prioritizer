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
            let parent_dir = match path_clone.parent() {
                Some(p) if !p.as_os_str().is_empty() => p,
                _ => {
                    error!("Cannot determine parent directory for config file {:?}", path_clone);
                    return;
                }
            };

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_load_file_invalid_toml() {
        let temp_dir = std::env::temp_dir();
        let file_path = temp_dir.join(format!("invalid_config_{}.toml", std::process::id()));

        // Write invalid TOML content
        let mut file = std::fs::File::create(&file_path).expect("Failed to create temp file");
        file.write_all(b"[general\nmissing_bracket = true").expect("Failed to write to temp file");

        // Attempt to load the file
        let result = ConfigManager::load_file(&file_path);

        // Verify it fails with a config error indicating invalid TOML
        assert!(result.is_err(), "Expected load_file to fail with invalid TOML");
        match result.unwrap_err() {
            BwpError::Config(msg) => {
                assert!(
                    msg.contains("Invalid TOML in"),
                    "Error message should mention 'Invalid TOML in', got: {}",
                    msg
                );
            }
            err => panic!("Expected BwpError::Config, got {:?}", err),
        }

        // Cleanup
        let _ = std::fs::remove_file(file_path);
    }

    #[test]
    fn test_new_or_default_non_existent_file() {
        let temp_dir = std::env::temp_dir();
        let file_path = temp_dir.join(format!("non_existent_{}.toml", std::process::id()));

        // Ensure file does not exist
        if file_path.exists() {
            std::fs::remove_file(&file_path).expect("Failed to remove existing file");
        }

        let manager = ConfigManager::new_or_default(&file_path);
        let config = manager.get_shared_config().load();

        // Should match default
        assert_eq!(config.default_throttle_limit_kbps, AppConfig::default().default_throttle_limit_kbps);
        assert_eq!(config.critical_apps, AppConfig::default().critical_apps);
    }

    #[test]
    fn test_new_or_default_invalid_toml() {
        let temp_dir = std::env::temp_dir();
        let file_path = temp_dir.join(format!("invalid_or_default_{}.toml", std::process::id()));

        // Write invalid TOML
        std::fs::write(&file_path, "invalid = [toml").expect("Failed to write temp file");

        let manager = ConfigManager::new_or_default(&file_path);
        let config = manager.get_shared_config().load();

        // Should fall back to default
        assert_eq!(config.default_throttle_limit_kbps, AppConfig::default().default_throttle_limit_kbps);

        let _ = std::fs::remove_file(file_path);
    }

    #[test]
    fn test_new_or_default_valid_file() {
        let temp_dir = std::env::temp_dir();
        let file_path = temp_dir.join(format!("valid_config_{}.toml", std::process::id()));

        let custom_config = r#"
            default_throttle_limit_kbps = 5000
            critical_apps = ["test_app"]
        "#;
        std::fs::write(&file_path, custom_config).expect("Failed to write temp file");

        let manager = ConfigManager::new_or_default(&file_path);
        let config = manager.get_shared_config().load();

        assert_eq!(config.default_throttle_limit_kbps, 5000);
        assert!(config.critical_apps.contains(&"test_app".to_string()));

        let _ = std::fs::remove_file(file_path);
    }

    #[tokio::test]
    async fn test_hot_reload_success() {
        let temp_dir = std::env::temp_dir();
        let file_path = temp_dir.join(format!("hot_reload_{}.toml", std::process::id()));

        // Initial config
        let initial_config = "default_throttle_limit_kbps = 1000";
        std::fs::write(&file_path, initial_config).expect("Failed to write temp file");

        let manager = ConfigManager::new_or_default(&file_path);
        assert_eq!(manager.get_shared_config().load().default_throttle_limit_kbps, 1000);

        // Start hot-reload task
        manager.spawn_hot_reload_task();

        // Update config
        let updated_config = "default_throttle_limit_kbps = 9999";
        // Small delay to ensure the watcher is ready (though notify might be slow to attach)
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        std::fs::write(&file_path, updated_config).expect("Failed to write updated file");

        // Wait for hot-reload: 500ms debounce + 50ms sleep + some buffer
        let mut success = false;
        for _ in 0..20 {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            if manager.get_shared_config().load().default_throttle_limit_kbps == 9999 {
                success = true;
                break;
            }
        }

        assert!(success, "Configuration was not hot-reloaded within 2 seconds");

        let _ = std::fs::remove_file(file_path);
    }
}
