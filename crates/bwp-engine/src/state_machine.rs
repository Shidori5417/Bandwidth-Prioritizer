//! The engine state machine: Normal <-> PriorityMode transitions driven by
//! connection events and IPC commands.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use bwp_controller::{ThrottleCommand, TrafficController};
use bwp_core::config::app_name_matches;
use bwp_core::ipc::{IpcCommand, IpcResponse};
use bwp_core::manager::ConfigManager;
use bwp_core::{AppConfig, BandwidthLimit, BwpResult, MonitorEvent, ThrottleState};
use dashmap::DashMap;
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, info, warn};

/// A unified event wrapping raw network detections alongside IPC administrative queries.
pub enum EngineEvent {
    /// Kernel/ETW detected a network socket change.
    Monitor(MonitorEvent),
    /// A local administrator sent an IPC instruction.
    Ipc(IpcCommand, oneshot::Sender<IpcResponse>),
}

/// The Core Brain of the Bandwidth Prioritizer.
pub struct Engine<C: TrafficController> {
    /// Lock-free cache: PID -> Executable Name
    pid_cache: DashMap<u32, Arc<str>>,
    /// Current zero-cost config reference
    config_manager: Arc<ConfigManager>,
    /// Traffic interceptor interface
    controller: Arc<C>,

    // Local State
    /// Number of currently open connections per PID. A PID is only forgotten
    /// when its last connection closes.
    connection_counts: HashMap<u32, usize>,
    /// PIDs currently identified as priority apps
    priority_pids: HashSet<u32>,
    /// PIDs currently identified as normal apps (throttable)
    normal_pids: HashSet<u32>,
    /// Current engine state
    pub state: ThrottleState,
    /// Active profile name (if any)
    pub active_profile: Option<String>,
    /// Ring buffer of recent events for UI presentation
    pub recent_events: VecDeque<String>,
}

impl<C: TrafficController + 'static> Engine<C> {
    /// Creates an engine in `Normal` state with no tracked processes.
    pub fn new(config_manager: Arc<ConfigManager>, controller: Arc<C>) -> Self {
        Self {
            pid_cache: DashMap::new(),
            config_manager,
            controller,
            connection_counts: HashMap::new(),
            priority_pids: HashSet::new(),
            normal_pids: HashSet::new(),
            state: ThrottleState::Normal,
            active_profile: None,
            recent_events: VecDeque::with_capacity(10),
        }
    }

    fn push_event(&mut self, msg: String) {
        if self.recent_events.len() == 10 {
            let _ = self.recent_events.pop_front();
        }
        self.recent_events.push_back(msg);
    }

    /// The throttle limit currently applicable to normal apps.
    fn effective_limit(&self, config: &AppConfig) -> BandwidthLimit {
        match self.active_profile {
            Some(ref profile) => config.throttle_limit_for_profile(profile),
            None => config.effective_throttle_limit(),
        }
    }

    /// The critical app names currently applicable.
    fn critical_apps(&self, config: &AppConfig) -> Vec<String> {
        match self.active_profile {
            Some(ref profile) => config.critical_apps_for_profile(profile),
            None => config.critical_apps.clone(),
        }
    }

    /// Primary event loop processing both IPC requests and raw network events dynamically over a single MPSC stream.
    ///
    /// # Errors
    /// Currently never fails; the `Result` is kept for future fallible shutdown steps.
    pub async fn run_event_loop(
        &mut self,
        mut event_rx: mpsc::Receiver<EngineEvent>,
    ) -> BwpResult<()> {
        info!("Bandwidth Engine state machine running...");

        while let Some(event) = event_rx.recv().await {
            match event {
                EngineEvent::Monitor(MonitorEvent::ConnectionOpened(info)) => {
                    self.handle_connection_opened(info.pid).await;
                }
                EngineEvent::Monitor(MonitorEvent::ConnectionClosed(info)) => {
                    self.handle_connection_closed(info.pid).await;
                }
                EngineEvent::Ipc(command, response_tx) => {
                    let is_stop = matches!(command, IpcCommand::Stop);
                    let resp = self.handle_ipc_command(command).await;
                    let _ = response_tx.send(resp);
                    if is_stop {
                        info!("Graceful shutdown initiated by IPC Stop command.");
                        break;
                    }
                }
            }
        }

        warn!("Engine event stream closed. Engine shutting down.");
        let _ = self
            .controller
            .send_command(ThrottleCommand::ClearAll)
            .await;
        Ok(())
    }

    /// Dynamically evaluates CLI/UI administration triggers
    async fn handle_ipc_command(&mut self, command: IpcCommand) -> IpcResponse {
        match command {
            IpcCommand::Status => {
                let config = self.config_manager.get_shared_config().load();
                let current_limit_kbps = self.effective_limit(&config).as_kbps();

                // Build throttled apps list
                let mut throttled_apps = Vec::new();
                if self.state == ThrottleState::PriorityMode {
                    let limit_kbps = current_limit_kbps;
                    for &pid in &self.normal_pids {
                        let name = self.resolve_exe_name(pid);
                        throttled_apps.push((name, pid, limit_kbps));
                    }
                }

                IpcResponse::StatusPayload {
                    state: self.state,
                    active_profile: self.active_profile.clone(),
                    throttled_pids_count: self.normal_pids.len(),
                    priority_pids_count: self.priority_pids.len(),
                    current_limit_kbps,
                    throttled_apps,
                    recent_events: self.recent_events.iter().cloned().collect(),
                }
            }
            IpcCommand::ApplyProfile(profile_name) => {
                let config = self.config_manager.get_shared_config().load();
                if config.profiles.contains_key(&profile_name) {
                    self.active_profile = Some(profile_name.clone());
                    let msg = format!("Profile '{}' applied.", profile_name);
                    info!("{}", msg);
                    self.push_event(msg);
                    self.re_evaluate_connections().await;
                    IpcResponse::Ok
                } else if profile_name.is_empty()
                    || profile_name.eq_ignore_ascii_case("default")
                    || profile_name.eq_ignore_ascii_case("none")
                {
                    self.active_profile = None;
                    let msg = "Profile cleared. Reverting to default configuration.".to_string();
                    info!("{}", msg);
                    self.push_event(msg);
                    self.re_evaluate_connections().await;
                    IpcResponse::Ok
                } else {
                    IpcResponse::Error(format!("Profile '{}' not found", profile_name))
                }
            }
            IpcCommand::AddApp(app) => {
                let shared_config = self.config_manager.get_shared_config();
                let config = shared_config.load();
                let mut new_config = (**config).clone();
                if !new_config
                    .critical_apps
                    .iter()
                    .any(|c| c.eq_ignore_ascii_case(&app))
                {
                    new_config.critical_apps.push(app.clone());
                    shared_config.store(Arc::new(new_config));
                    let msg = format!("App '{}' added to critical apps dynamically.", app);
                    info!("{}", msg);
                    self.push_event(msg);
                    self.re_evaluate_connections().await;
                }
                IpcResponse::Ok
            }
            IpcCommand::RemoveApp(app) => {
                let shared_config = self.config_manager.get_shared_config();
                let config = shared_config.load();
                let mut new_config = (**config).clone();
                if let Some(pos) = new_config
                    .critical_apps
                    .iter()
                    .position(|x| x.eq_ignore_ascii_case(&app))
                {
                    let _ = new_config.critical_apps.remove(pos);
                    shared_config.store(Arc::new(new_config));
                    let msg = format!("App '{}' removed from critical apps dynamically.", app);
                    info!("{}", msg);
                    self.push_event(msg);
                    self.re_evaluate_connections().await;
                }
                IpcResponse::Ok
            }
            IpcCommand::Stop => {
                warn!("Received Stop command. Shutting down throttle engine.");
                let _ = self
                    .controller
                    .send_command(ThrottleCommand::ClearAll)
                    .await;
                IpcResponse::Ok
            }
        }
    }

    /// Forgets the cached executable name of a PID (to be wired to process-exit events).
    pub fn drop_process_from_cache(&self, pid: u32) {
        let _ = self.pid_cache.remove(&pid);
    }

    async fn handle_connection_opened(&mut self, pid: u32) {
        *self.connection_counts.entry(pid).or_insert(0) += 1;

        // 1. Resolve Name O(1) mostly
        let exe_name = self.resolve_exe_name(pid);

        // 2. Load latest config O(1) lock-free
        let config = self.config_manager.get_shared_config().load();
        let critical_apps = self.critical_apps(&config);
        let is_priority = critical_apps.iter().any(|c| app_name_matches(&exe_name, c));

        if is_priority {
            if self.priority_pids.insert(pid) {
                let msg = format!("Priority App Detected: {exe_name} (PID: {pid})");
                info!("{}", msg);
                self.push_event(msg);
                if self.state != ThrottleState::PriorityMode {
                    self.enter_priority_mode().await;
                }
            }
        } else if self.normal_pids.insert(pid) {
            debug!("Normal App Registered: {exe_name} (PID: {pid})");
            if self.state == ThrottleState::PriorityMode {
                // Apply throttle instantly to this new connection
                let limit = self.effective_limit(&config);
                let _ = self
                    .controller
                    .send_command(ThrottleCommand::ApplyLimit { pid, limit })
                    .await;
            }
        }
    }

    async fn handle_connection_closed(&mut self, pid: u32) {
        // A process usually owns several connections: only forget it once the last one is gone.
        if let Some(count) = self.connection_counts.get_mut(&pid) {
            *count = count.saturating_sub(1);
            if *count > 0 {
                return;
            }
            let _ = self.connection_counts.remove(&pid);
        }

        if self.normal_pids.remove(&pid) && self.state == ThrottleState::PriorityMode {
            let _ = self
                .controller
                .send_command(ThrottleCommand::RemoveLimit { pid })
                .await;
        }

        if self.priority_pids.remove(&pid) {
            let msg = format!("Priority App Disconnected (PID: {pid})");
            info!("{}", msg);
            self.push_event(msg);
            if self.priority_pids.is_empty() && self.state == ThrottleState::PriorityMode {
                self.exit_priority_mode().await;
            }
        }
    }

    async fn enter_priority_mode(&mut self) {
        let msg = format!(
            "Engaging Priority Mode! Throttling {} normal background processes.",
            self.normal_pids.len()
        );
        info!("{}", msg);
        self.push_event(msg);
        self.state = ThrottleState::PriorityMode;

        let config = self.config_manager.get_shared_config().load();
        let limit = self.effective_limit(&config);

        for &pid in &self.normal_pids {
            let _ = self
                .controller
                .send_command(ThrottleCommand::ApplyLimit { pid, limit })
                .await;
        }
    }

    async fn re_evaluate_connections(&mut self) {
        let config = self.config_manager.get_shared_config().load();

        let critical_apps = self.critical_apps(&config);

        let mut to_priority = Vec::new();
        let mut to_normal = Vec::new();

        for &pid in &self.normal_pids {
            let exe_name = self.resolve_exe_name(pid);
            if critical_apps.iter().any(|c| app_name_matches(&exe_name, c)) {
                to_priority.push(pid);
            }
        }

        for &pid in &self.priority_pids {
            let exe_name = self.resolve_exe_name(pid);
            if !critical_apps.iter().any(|c| app_name_matches(&exe_name, c)) {
                to_normal.push(pid);
            }
        }

        for pid in to_priority {
            let _ = self.normal_pids.remove(&pid);
            let _ = self.priority_pids.insert(pid);
            if self.state == ThrottleState::PriorityMode {
                let _ = self
                    .controller
                    .send_command(ThrottleCommand::RemoveLimit { pid })
                    .await;
            }
        }

        for pid in to_normal {
            let _ = self.priority_pids.remove(&pid);
            let _ = self.normal_pids.insert(pid);
            // Limit applied below if we stay in or enter PriorityMode
        }

        if self.priority_pids.is_empty() && self.state == ThrottleState::PriorityMode {
            // No priority apps remain, exit PriorityMode
            self.exit_priority_mode().await;
        } else if !self.priority_pids.is_empty() && self.state != ThrottleState::PriorityMode {
            // Priority apps present, enter PriorityMode
            self.enter_priority_mode().await;
        } else if self.state == ThrottleState::PriorityMode {
            // Update limits for all normal pids based on the (possibly new) profile/config
            let limit = self.effective_limit(&config);
            for &pid in &self.normal_pids {
                let _ = self
                    .controller
                    .send_command(ThrottleCommand::ApplyLimit { pid, limit })
                    .await;
            }
        }
    }

    async fn exit_priority_mode(&mut self) {
        let msg = "Returning to Normal Mode! Unthrottling all processes.".to_string();
        info!("{}", msg);
        self.push_event(msg);
        self.state = ThrottleState::Normal;
        let _ = self
            .controller
            .send_command(ThrottleCommand::ClearAll)
            .await;
    }

    /// Resolves the executable name of a PID, caching successful lookups.
    ///
    /// Falls back to `unknown_{pid}.exe` (not cached) when the process cannot be found,
    /// for example because it already exited.
    fn resolve_exe_name(&self, pid: u32) -> Arc<str> {
        if let Some(name) = self.pid_cache.get(&pid) {
            return name.clone();
        }

        // Cache miss: ask the OS (via `sysinfo`, cross-platform).
        if let Some(name) = lookup_process_name(pid) {
            let name: Arc<str> = Arc::from(name);
            let _ = self.pid_cache.insert(pid, name.clone());
            return name;
        }
        Arc::from(format!("unknown_{pid}.exe"))
    }
}

/// Looks up the process name for `pid` using the operating system's process table.
fn lookup_process_name(pid: u32) -> Option<String> {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

    let pid = Pid::from_u32(pid);
    let mut system = System::new();
    let _ = system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::nothing(),
    );
    let name = system.process(pid)?.name().to_string_lossy().into_owned();
    (!name.is_empty()).then_some(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bwp_core::BandwidthLimit;

    /// Records every command it receives.
    #[derive(Default)]
    struct MockController {
        commands: std::sync::Mutex<Vec<ThrottleCommand>>,
    }

    impl TrafficController for MockController {
        async fn send_command(&self, command: ThrottleCommand) -> BwpResult<()> {
            self.commands.lock().unwrap().push(command);
            Ok(())
        }

        async fn active_rules(&self) -> BwpResult<Vec<(u32, BandwidthLimit)>> {
            Ok(vec![])
        }
    }

    fn engine_with(config_manager: ConfigManager) -> (Engine<MockController>, Arc<MockController>) {
        let controller = Arc::new(MockController::default());
        let engine = Engine::new(Arc::new(config_manager), controller.clone());
        (engine, controller)
    }

    fn default_engine() -> (Engine<MockController>, Arc<MockController>) {
        engine_with(ConfigManager::new_or_default("dummy.toml"))
    }

    fn seed_name(engine: &Engine<MockController>, pid: u32, name: &str) {
        let _ = engine.pid_cache.insert(pid, Arc::from(name));
    }

    #[test]
    fn test_resolve_exe_name_caching() {
        let (engine, _) = default_engine();
        let pid = std::process::id();

        let name1 = engine.resolve_exe_name(pid);
        assert!(engine.pid_cache.contains_key(&pid));

        // Overwrite the cache to simulate a different resolved name.
        seed_name(&engine, pid, "mocked.exe");
        let name2 = engine.resolve_exe_name(pid);
        assert_eq!(name2.as_ref(), "mocked.exe");
        assert_ne!(name1.as_ref(), "mocked.exe");
    }

    #[test]
    fn test_resolve_exe_name_for_current_process() {
        let (engine, _) = default_engine();
        let name = engine.resolve_exe_name(std::process::id());
        assert!(!name.starts_with("unknown_"), "got {name}");

        // Linux reports the (15 char truncated) `comm`, Windows the full file name.
        let exe = std::env::current_exe().unwrap();
        let file_name = exe.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            file_name.starts_with(name.as_ref()),
            "{file_name} vs {name}"
        );
    }

    #[test]
    fn test_resolve_exe_name_unknown_pid_is_not_cached() {
        let (engine, _) = default_engine();
        let pid = u32::MAX - 1;
        assert_eq!(
            engine.resolve_exe_name(pid).as_ref(),
            format!("unknown_{pid}.exe")
        );
        assert!(!engine.pid_cache.contains_key(&pid));
    }

    #[tokio::test]
    async fn closing_one_of_many_connections_keeps_priority_pid() {
        let (mut engine, _) = default_engine();
        seed_name(&engine, 10, "Discord.exe");

        engine.handle_connection_opened(10).await;
        engine.handle_connection_opened(10).await;
        assert_eq!(engine.state, ThrottleState::PriorityMode);

        engine.handle_connection_closed(10).await;
        assert!(engine.priority_pids.contains(&10));
        assert_eq!(engine.state, ThrottleState::PriorityMode);

        engine.handle_connection_closed(10).await;
        assert!(engine.priority_pids.is_empty());
        assert_eq!(engine.state, ThrottleState::Normal);
    }

    #[tokio::test]
    async fn closing_one_of_many_connections_keeps_normal_pid_throttled() {
        let (mut engine, controller) = default_engine();
        seed_name(&engine, 10, "discord.exe");
        seed_name(&engine, 20, "chrome.exe");

        engine.handle_connection_opened(10).await;
        engine.handle_connection_opened(20).await;
        engine.handle_connection_opened(20).await;

        engine.handle_connection_closed(20).await;
        assert!(engine.normal_pids.contains(&20));
        assert!(
            !controller
                .commands
                .lock()
                .unwrap()
                .contains(&ThrottleCommand::RemoveLimit { pid: 20 })
        );

        engine.handle_connection_closed(20).await;
        assert!(!engine.normal_pids.contains(&20));
        assert!(
            controller
                .commands
                .lock()
                .unwrap()
                .contains(&ThrottleCommand::RemoveLimit { pid: 20 })
        );
    }

    #[tokio::test]
    async fn closing_unknown_pid_is_a_noop() {
        let (mut engine, _) = default_engine();
        engine.handle_connection_closed(999).await;
        assert_eq!(engine.state, ThrottleState::Normal);
    }

    #[tokio::test]
    async fn status_reports_profile_limit() {
        let path =
            std::env::temp_dir().join(format!("bwp_engine_status_{}.toml", std::process::id()));
        std::fs::write(
            &path,
            "critical_apps = [\"discord\"]\ndefault_throttle_limit_kbps = 1000\n\n\
             [profiles.gaming]\ncritical_apps = [\"steam\"]\nthrottle_limit_kbps = 500\n",
        )
        .unwrap();
        let manager = ConfigManager::new(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        let (mut engine, _) = engine_with(manager);
        seed_name(&engine, 10, "discord.exe");
        seed_name(&engine, 20, "chrome.exe");
        engine.handle_connection_opened(10).await;
        engine.handle_connection_opened(20).await;

        let resp = engine
            .handle_ipc_command(IpcCommand::ApplyProfile("gaming".into()))
            .await;
        assert!(matches!(resp, IpcResponse::Ok));

        // The profile limit (500) must be reported, not the global default (1000).
        match engine.handle_ipc_command(IpcCommand::Status).await {
            IpcResponse::StatusPayload {
                current_limit_kbps,
                throttled_apps,
                ..
            } => {
                assert_eq!(current_limit_kbps, 500);
                assert!(!throttled_apps.is_empty());
                assert!(throttled_apps.iter().all(|(_, _, kbps)| *kbps == 500));
            }
            other => panic!("unexpected response: {other:?}"),
        }
    }
}
