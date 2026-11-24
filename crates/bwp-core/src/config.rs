//! Configuration models for Bandwidth-Prioritizer.
//!
//! Defines the strongly-typed structures that map to `config.toml`.
//! All configuration is validated at parse time via `serde`.

use crate::types::BandwidthLimit;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// File name of the configuration file looked up next to the executable.
pub const CONFIG_FILE_NAME: &str = "config.toml";

/// Environment variable that overrides the configuration file location.
pub const CONFIG_ENV_VAR: &str = "BWP_CONFIG";

/// Resolves which `config.toml` to use.
///
/// Order: the `BWP_CONFIG` environment variable, then `config.toml` next to the
/// running executable (a Windows service starts with `System32` as its working
/// directory, so the current directory is not reliable), then `config.toml` in the
/// current directory as a last resort.
#[must_use]
pub fn resolve_config_path() -> std::path::PathBuf {
    if let Some(p) = std::env::var_os(CONFIG_ENV_VAR).filter(|p| !p.is_empty()) {
        return p.into();
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(CONFIG_FILE_NAME)))
        .filter(|p| p.exists())
        .unwrap_or_else(|| CONFIG_FILE_NAME.into())
}

/// Root configuration structure, mirrors `config.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    /// General application settings.
    #[serde(default)]
    pub general: GeneralConfig,

    /// List of process names considered "critical" (whitelisted).
    /// When any of these are active, non-critical processes get throttled.
    #[serde(default)]
    pub critical_apps: Vec<String>,

    /// Default throttle limit applied to non-critical processes.
    #[serde(default = "default_throttle_limit")]
    pub default_throttle_limit_kbps: u64,

    /// Named profiles for quick switching (e.g. "gaming", "meeting").
    #[serde(default)]
    pub profiles: HashMap<String, ProfileConfig>,

    /// Packet-shaping settings (experimental features are opt-in).
    #[serde(default)]
    pub shaping: ShapingConfig,
}

/// Settings for the packet-shaping backend.
///
/// Everything here defaults to *off*: the daemon never intercepts packets unless
/// the user explicitly opts in.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShapingConfig {
    /// Enables the experimental WinDivert packet shaper (Windows only).
    ///
    /// Only honoured by a `bwp-engine` built with the `experimental-windivert` Cargo
    /// feature, and only read at startup (changing it needs an engine restart). When
    /// `false` (the default) no packet is ever captured.
    #[serde(default)]
    pub experimental_windivert: bool,
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
    #[serde(default)]
    pub critical_apps: Vec<String>,
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
    pub fn critical_apps_for_profile(&self, profile_name: &str) -> Vec<String> {
        let mut apps = self.critical_apps.clone();
        if let Some(profile) = self.profiles.get(profile_name) {
            for app in &profile.critical_apps {
                if !apps.contains(app) {
                    apps.push(app.clone());
                }
            }
        }
        apps
    }
}

/// Returns whether a process executable name matches a configured app name.
///
/// The comparison is case-insensitive and ignores a trailing `.exe` on either side,
/// so `Discord.exe` matches `discord` and `discord.exe`.
#[must_use]
pub fn app_name_matches(exe_name: &str, configured: &str) -> bool {
    fn strip(name: &str) -> &str {
        let name = name.trim();
        name.len()
            .checked_sub(4)
            .filter(|&at| name.is_char_boundary(at) && name[at..].eq_ignore_ascii_case(".exe"))
            .map_or(name, |at| &name[..at])
    }
    strip(exe_name).eq_ignore_ascii_case(strip(configured))
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            general: GeneralConfig::default(),
            critical_apps: vec![
                "discord".to_string(),
                "zoom".to_string(),
                "code".to_string(),
            ],
            default_throttle_limit_kbps: default_throttle_limit(),
            profiles: HashMap::new(),
            shaping: ShapingConfig::default(),
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
    fn shipped_config_toml_is_parsed_fully() {
        let config: AppConfig = toml::from_str(include_str!("../../../config.toml")).unwrap();
        assert!(config.critical_apps.iter().any(|a| a == "discord"));
        assert_eq!(config.default_throttle_limit_kbps, 1000);
        assert_eq!(config.general.poll_interval_ms, 500);
        assert_eq!(config.throttle_limit_for_profile("gaming").as_kbps(), 500);
    }

    #[test]
    fn shaping_is_off_by_default_and_parses() {
        assert!(!AppConfig::default().shaping.experimental_windivert);
        let config: AppConfig = toml::from_str("critical_apps = []").unwrap();
        assert!(!config.shaping.experimental_windivert);
        let config: AppConfig = toml::from_str("[shaping]\nexperimental_windivert = true").unwrap();
        assert!(config.shaping.experimental_windivert);
        let shipped: AppConfig = toml::from_str(include_str!("../../../config.toml")).unwrap();
        assert!(!shipped.shaping.experimental_windivert);
    }

    #[test]
    fn app_name_matching_ignores_case_and_exe_suffix() {
        assert!(app_name_matches("Discord.exe", "discord"));
        assert!(app_name_matches("discord", "DISCORD.EXE"));
        assert!(app_name_matches("zoom.exe", "zoom.exe"));
        assert!(!app_name_matches("discord.exe", "zoom"));
        assert!(!app_name_matches("exe", "e"));
    }

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
                critical_apps: vec!["steam.exe".to_string()],
                throttle_limit_kbps: Some(500),
            },
        );

        let limit = config.throttle_limit_for_profile("gaming");
        assert_eq!(limit.as_kbps(), 500);

        let apps = config.critical_apps_for_profile("gaming");
        assert!(apps.iter().any(|a| a == "steam.exe"));
        assert!(apps.iter().any(|a| a == "discord"));
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
        assert_eq!(
            parsed.general.poll_interval_ms,
            config.general.poll_interval_ms
        );
        assert_eq!(
            parsed.default_throttle_limit_kbps,
            config.default_throttle_limit_kbps
        );
    }
}
