//! Configuration models for Bandwidth-Prioritizer.
//!
//! Defines the strongly-typed structures that map to `config.toml`.
//! All configuration is validated at parse time via `serde`.

use crate::types::BandwidthLimit;
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{HashMap, HashSet};

/// Helper to deserialize a sequence of strings into a `HashSet` of lowercased strings.
fn deserialize_lowercase_set<'de, D>(deserializer: D) -> Result<HashSet<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let vec: Vec<String> = Vec::deserialize(deserializer)?;
    Ok(vec.into_iter().map(|s| s.to_ascii_lowercase()).collect())
}

/// Root configuration structure, mirrors `config.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    /// General application settings.
    #[serde(default)]
    pub general: GeneralConfig,

    /// List of process names considered "critical" (whitelisted).
    /// When any of these are active, non-critical processes get throttled.
    #[serde(default, deserialize_with = "deserialize_lowercase_set")]
    pub critical_apps: HashSet<String>,

    /// Default throttle limit applied to non-critical processes.
    #[serde(default = "default_throttle_limit")]
    pub default_throttle_limit_kbps: u64,

    /// Named profiles for quick switching (e.g. "gaming", "meeting").
    #[serde(default)]
    pub profiles: HashMap<String, ProfileConfig>,
}

/// General daemon/runtime settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralConfig {
    /// Polling interval for the process monitor, in milliseconds.
    #[serde(default = "default_poll_interval_ms")]
    pub poll_interval_ms: u64,

    /// Path to the log file. `None` means stdout only.
    pub log_file: Option<String>,

    /// Log verbosity level (`trace`, `debug`, `info`, `warn`, `error`).
    #[serde(default = "default_log_level")]
    pub log_level: String,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            poll_interval_ms: default_poll_interval_ms(),
            log_file: None,
            log_level: default_log_level(),
        }
    }
}

/// A named profile containing a set of critical apps and throttle rules.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileConfig {
    /// Optional description for the profile.
    pub description: Option<String>,
    /// Critical apps specific to this profile (merged with global list).
    #[serde(default, deserialize_with = "deserialize_lowercase_set")]
    pub critical_apps: HashSet<String>,
    /// Throttle limit override for this profile (in Kbps).
    pub throttle_limit_kbps: Option<u64>,
}

impl AppConfig {
    /// Returns the effective [`BandwidthLimit`] for the current configuration.
    #[must_use]
    pub fn effective_throttle_limit(&self) -> BandwidthLimit {
        BandwidthLimit::from_kbps(self.default_throttle_limit_kbps)
    }

    /// Returns the effective throttle limit for a specific profile,
    /// falling back to the global default.
    #[must_use]
    pub fn throttle_limit_for_profile(&self, profile_name: &str) -> BandwidthLimit {
        self.profiles
            .get(profile_name)
            .and_then(|p| p.throttle_limit_kbps)
            .map_or_else(
                || self.effective_throttle_limit(),
                BandwidthLimit::from_kbps,
            )
    }

    /// Returns the combined critical app list for a given profile.
    /// Merges global `critical_apps` with profile-specific ones.
    #[must_use]
    pub fn critical_apps_for_profile(&self, profile_name: &str) -> HashSet<String> {
        let mut apps = self.critical_apps.clone();
        if let Some(profile) = self.profiles.get(profile_name) {
            for app in &profile.critical_apps {
                apps.insert(app.clone());
            }
        }
        apps
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            general: GeneralConfig::default(),
            critical_apps: HashSet::from([
                "discord".to_string(),
                "zoom".to_string(),
                "code".to_string(),
            ]),
            default_throttle_limit_kbps: default_throttle_limit(),
            profiles: HashMap::new(),
        }
    }
}

// ─── Defaults ───────────────────────────────────────────────────────

const fn default_poll_interval_ms() -> u64 {
    500
}

const fn default_throttle_limit() -> u64 {
    1_000 // 1 Mbps default
}

fn default_log_level() -> String {
    "info".to_string()
}

// ─── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_sane_values() {
        let config = AppConfig::default();
        assert_eq!(config.general.poll_interval_ms, 500);
        assert_eq!(config.default_throttle_limit_kbps, 1_000);
        assert!(!config.critical_apps.is_empty());
    }

    #[test]
    fn effective_limit_matches_default() {
        let config = AppConfig::default();
        let limit = config.effective_throttle_limit();
        assert_eq!(limit.as_kbps(), 1_000);
    }

    #[test]
    fn profile_override_works() {
        let mut config = AppConfig::default();
        config.profiles.insert(
            "gaming".to_string(),
            ProfileConfig {
                description: Some("Low-latency gaming profile".to_string()),
                critical_apps: HashSet::from(["steam.exe".to_string()]),
                throttle_limit_kbps: Some(500),
            },
        );

        let limit = config.throttle_limit_for_profile("gaming");
        assert_eq!(limit.as_kbps(), 500);

        let apps = config.critical_apps_for_profile("gaming");
        assert!(apps.contains("steam.exe"));
        assert!(apps.contains("discord"));
    }

    #[test]
    fn missing_profile_uses_defaults() {
        let config = AppConfig::default();
        let limit = config.throttle_limit_for_profile("nonexistent");
        assert_eq!(limit.as_kbps(), 1_000);
    }

    #[test]
    fn config_roundtrip_toml() {
        let config = AppConfig::default();
        let toml_str = toml::to_string_pretty(&config).expect("serialize");
        let parsed: AppConfig = toml::from_str(&toml_str).expect("deserialize");
        assert_eq!(parsed.general.poll_interval_ms, config.general.poll_interval_ms);
        assert_eq!(parsed.default_throttle_limit_kbps, config.default_throttle_limit_kbps);
    }
}
