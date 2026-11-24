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
use std::time::Duration;
use tokio::sync::{mpsc, watch};
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
    /// Counts successful reloads; subscribers are woken after each one.
    reload_tx: Arc<watch::Sender<u64>>,
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
            reload_tx: Arc::new(watch::channel(0).0),
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
            reload_tx: Arc::new(watch::channel(0).0),
        }
    }

    /// Subscribes to successful configuration reloads.
    ///
    /// The receiver's value is a reload counter; `changed().await` resolves after the
    /// new config has been stored, so listeners can re-evaluate state against it.
    #[must_use]
    pub fn subscribe_reloads(&self) -> watch::Receiver<u64> {
        self.reload_tx.subscribe()
    }

    /// Re-reads the config file now. On success the new config is swapped in and
    /// reload subscribers are notified; on failure the previous config is kept.
    ///
    /// # Errors
    /// Returns the read/parse error; the active configuration is left untouched.
    pub fn reload(&self) -> BwpResult<()> {
        let new_config = Self::load_file(&self.config_path)?;
        self.current.store(Arc::new(new_config));
        self.reload_tx.send_modify(|generation| *generation += 1);
        Ok(())
    }

    /// Read the `AppConfig` from a TOML file.
    fn load_file(path: &Path) -> BwpResult<AppConfig> {
        let content = std::fs::read_to_string(path).map_err(|e| {
            BwpError::Config(format!("Could not read file {}: {}", path.display(), e))
        })?;

        let config: AppConfig = toml::from_str(&content)
            .map_err(|e| BwpError::Config(format!("Invalid TOML in {}: {}", path.display(), e)))?;

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
    /// The `notify` crate will listen for filesystem changes. Change events are
    /// debounced with a trailing edge: the config is re-parsed once, after the
    /// events have settled for [`RELOAD_QUIET_PERIOD`], so the final edit is never
    /// missed. If parsing fails the previous config is kept; if it succeeds the
    /// new config pointer is swapped in atomically.
    ///
    /// Must be called from within a Tokio runtime.
    pub fn spawn_hot_reload_task(&self) {
        let (tx, mut rx) = mpsc::channel::<()>(1);
        let path_clone = self.config_path.clone();
        let reloader = ReloadHandle {
            current: self.current.clone(),
            reload_tx: self.reload_tx.clone(),
        };

        // Standard library thread because `notify::RecommendedWatcher` is synchronous/blocking in its callback.
        // We bridge it to the async world via a tokio channel.
        let watched_file_name = self
            .config_path
            .file_name()
            .map(std::ffi::OsStr::to_os_string);
        let _watcher_thread = std::thread::spawn(move || {
            let mut watcher =
                match notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
                    if let Ok(event) = res {
                        let relevant_kind = event.kind.is_modify() || event.kind.is_create();
                        let relevant_path = event.paths.iter().any(|p| {
                            p.file_name().map(std::ffi::OsStr::to_os_string) == watched_file_name
                        });
                        if relevant_kind && relevant_path {
                            // A full channel already holds a pending signal, so dropping is fine.
                            let _ = tx.try_send(());
                        }
                    }
                }) {
                    Ok(w) => w,
                    Err(e) => {
                        error!("Failed to initialize file watcher: {e}");
                        return;
                    }
                };

            // Attempt to watch the directory of the config file. A relative path such as
            // `config.toml` has an empty parent, which means the current directory.
            let parent_dir = match path_clone.parent() {
                Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
                _ => PathBuf::from("."),
            };

            if let Err(e) = watcher.watch(&parent_dir, RecursiveMode::NonRecursive) {
                error!("File watcher failed to attach to directory: {e}");
                return;
            }

            // Keep the thread (and thus the watcher) alive.
            loop {
                std::thread::park();
            }
        });

        // Async task to consume the debounced watch events and perform the reload.
        let file_path = self.config_path.clone();
        let _reload_task = tokio::spawn(async move {
            info!("Hot-reload watcher started on {:?}", file_path);

            while wait_for_settled(&mut rx, RELOAD_QUIET_PERIOD).await {
                match reloader.reload(&file_path) {
                    Ok(()) => {
                        info!("Configuration hot-reloaded successfully.");
                    }
                    Err(e) => {
                        error!("Hot-reload failed, retaining old config. Error: {e}");
                    }
                }
            }
        });
    }
}

/// The pieces of a [`ConfigManager`] the hot-reload task needs.
struct ReloadHandle {
    current: Arc<ArcSwap<AppConfig>>,
    reload_tx: Arc<watch::Sender<u64>>,
}

impl ReloadHandle {
    fn reload(&self, path: &Path) -> BwpResult<()> {
        let new_config = ConfigManager::load_file(path)?;
        // Atomic pointer replacement, then wake the subscribers.
        self.current.store(Arc::new(new_config));
        self.reload_tx.send_modify(|generation| *generation += 1);
        Ok(())
    }
}

/// How long the config file must stay quiet before a hot-reload is performed.
const RELOAD_QUIET_PERIOD: Duration = Duration::from_millis(300);

/// Trailing-edge debounce: waits for the first event, then keeps waiting until no
/// further event arrives for `quiet`. Returns `true` when a reload should happen and
/// `false` when the channel has been closed with nothing pending.
async fn wait_for_settled(rx: &mut mpsc::Receiver<()>, quiet: Duration) -> bool {
    if rx.recv().await.is_none() {
        return false;
    }
    while let Ok(Some(())) = tokio::time::timeout(quiet, rx.recv()).await {}
    true
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
        file.write_all(b"[general\nmissing_bracket = true")
            .expect("Failed to write to temp file");

        // Attempt to load the file
        let result = ConfigManager::load_file(&file_path);

        // Verify it fails with a config error indicating invalid TOML
        assert!(
            result.is_err(),
            "Expected load_file to fail with invalid TOML"
        );
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

    #[tokio::test]
    async fn reload_swaps_config_and_notifies_subscribers() {
        let path = std::env::temp_dir().join(format!("bwp_reload_{}.toml", std::process::id()));
        std::fs::write(&path, "critical_apps = [\"a\"]").unwrap();
        let manager = ConfigManager::new(&path).unwrap();
        let mut rx = manager.subscribe_reloads();

        std::fs::write(&path, "critical_apps = [\"b\"]").unwrap();
        manager.reload().unwrap();
        rx.changed().await.unwrap();
        assert_eq!(*rx.borrow(), 1);
        assert_eq!(manager.get_shared_config().load().critical_apps, ["b"]);

        // A broken file keeps the old config and does not notify.
        std::fs::write(&path, "[general\n").unwrap();
        assert!(manager.reload().is_err());
        assert_eq!(manager.get_shared_config().load().critical_apps, ["b"]);
        assert!(!rx.has_changed().unwrap());
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn debounce_fires_once_after_events_settle() {
        let (tx, mut rx) = mpsc::channel::<()>(1);
        let quiet = Duration::from_millis(80);

        // Burst of events spaced shorter than the quiet period, with a trailing one.
        let sender = tokio::spawn(async move {
            for _ in 0..4 {
                let _ = tx.send(()).await;
                tokio::time::sleep(Duration::from_millis(30)).await;
            }
            // Keep the channel open a while so we can observe the settle.
            tokio::time::sleep(Duration::from_millis(400)).await;
        });

        let start = tokio::time::Instant::now();
        assert!(wait_for_settled(&mut rx, quiet).await);
        // Must have waited for the whole burst (~90ms) plus the quiet period.
        assert!(start.elapsed() >= Duration::from_millis(150));
        // The burst was collapsed into a single reload: nothing else is pending.
        assert!(rx.try_recv().is_err());
        sender.abort();
    }

    #[tokio::test]
    async fn debounce_reports_closed_channel() {
        let (tx, mut rx) = mpsc::channel::<()>(1);
        drop(tx);
        assert!(!wait_for_settled(&mut rx, Duration::from_millis(10)).await);
    }

    #[tokio::test]
    async fn debounce_reloads_pending_event_when_channel_closes() {
        let (tx, mut rx) = mpsc::channel::<()>(1);
        tx.send(()).await.unwrap();
        drop(tx);
        assert!(wait_for_settled(&mut rx, Duration::from_millis(10)).await);
        assert!(!wait_for_settled(&mut rx, Duration::from_millis(10)).await);
    }

    /// A fresh, empty directory under the system temp dir for one test.
    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bwp_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn new_or_default_falls_back_when_file_is_missing() {
        let dir = scratch_dir("missing");
        let manager = ConfigManager::new_or_default(dir.join("absent.toml"));
        let config = manager.get_shared_config().load();
        let default = AppConfig::default();

        assert_eq!(
            config.default_throttle_limit_kbps,
            default.default_throttle_limit_kbps
        );
        assert_eq!(config.critical_apps, default.critical_apps);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn new_or_default_falls_back_on_invalid_toml() {
        let dir = scratch_dir("invalid");
        let path = dir.join("config.toml");
        std::fs::write(&path, "invalid = [toml").unwrap();

        let manager = ConfigManager::new_or_default(&path);
        assert_eq!(
            manager
                .get_shared_config()
                .load()
                .default_throttle_limit_kbps,
            AppConfig::default().default_throttle_limit_kbps
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn new_or_default_loads_valid_file() {
        let dir = scratch_dir("valid");
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            "default_throttle_limit_kbps = 5000\ncritical_apps = [\"test_app\"]\n",
        )
        .unwrap();

        let manager = ConfigManager::new_or_default(&path);
        let config = manager.get_shared_config().load();
        assert_eq!(config.default_throttle_limit_kbps, 5000);
        assert_eq!(config.critical_apps, ["test_app"]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn hot_reload_picks_up_file_changes() {
        let dir = scratch_dir("hot_reload");
        let path = dir.join("config.toml");
        std::fs::write(&path, "default_throttle_limit_kbps = 1000\n").unwrap();

        let manager = ConfigManager::new_or_default(&path);
        assert_eq!(
            manager
                .get_shared_config()
                .load()
                .default_throttle_limit_kbps,
            1000
        );
        let mut reloads = manager.subscribe_reloads();
        manager.spawn_hot_reload_task();

        // Give the watcher thread time to attach before editing the file.
        tokio::time::sleep(Duration::from_millis(200)).await;
        std::fs::write(&path, "default_throttle_limit_kbps = 9999\n").unwrap();

        tokio::time::timeout(Duration::from_secs(5), reloads.changed())
            .await
            .expect("config was not hot-reloaded within 5 seconds")
            .unwrap();
        assert_eq!(
            manager
                .get_shared_config()
                .load()
                .default_throttle_limit_kbps,
            9999
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
