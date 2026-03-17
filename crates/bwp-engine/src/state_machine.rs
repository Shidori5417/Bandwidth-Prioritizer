use std::collections::{HashSet, VecDeque};
use std::sync::Arc;
use bwp_core::{BwpResult, MonitorEvent, ThrottleState, BandwidthLimit};
use bwp_core::manager::ConfigManager;
use bwp_controller::{TrafficController, ThrottleCommand};
use dashmap::DashMap;
use tokio::sync::{mpsc, oneshot};
use tracing::{info, debug, warn};
use bwp_core::ipc::{IpcCommand, IpcResponse};

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
    pub fn new(config_manager: Arc<ConfigManager>, controller: Arc<C>) -> Self {
        Self {
            pid_cache: DashMap::new(),
            config_manager,
            controller,
            priority_pids: HashSet::new(),
            normal_pids: HashSet::new(),
            state: ThrottleState::Normal,
            active_profile: None,
            recent_events: VecDeque::with_capacity(10),
        }
    }

    fn push_event(&mut self, msg: String) {
        if self.recent_events.len() == 10 {
            self.recent_events.pop_front();
        }
        self.recent_events.push_back(msg);
    }

    /// Primary event loop processing both IPC requests and raw network events dynamically over a single MPSC stream.
    pub async fn run_event_loop(&mut self, mut event_rx: mpsc::Receiver<EngineEvent>) -> BwpResult<()> {
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
        let _ = self.controller.send_command(ThrottleCommand::ClearAll).await;
        Ok(())
    }

    /// Dynamically evaluates CLI/UI administration triggers
    async fn handle_ipc_command(&mut self, command: IpcCommand) -> IpcResponse {
        match command {
            IpcCommand::Status => {
                let config = self.config_manager.get_shared_config().load();
                
                // Build throttled apps list
                let mut throttled_apps = Vec::new();
                if self.state == ThrottleState::PriorityMode {
                    let limit_kbps = config.default_throttle_limit_kbps;
                    for &pid in &self.normal_pids {
                        let name = self.resolve_exe_name(pid).to_string();
                        throttled_apps.push((name, pid, limit_kbps));
                    }
                }

                let current_limit_kbps = if let Some(ref profile) = self.active_profile {
                    config.throttle_limit_for_profile(profile).as_kbps()
                } else {
                    config.default_throttle_limit_kbps
                };

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
                } else if profile_name.is_empty() || profile_name.eq_ignore_ascii_case("default") || profile_name.eq_ignore_ascii_case("none") {
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
                if !new_config.critical_apps.iter().any(|c| c.eq_ignore_ascii_case(&app)) {
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
                if let Some(pos) = new_config.critical_apps.iter().position(|x| x.eq_ignore_ascii_case(&app)) {
                    new_config.critical_apps.remove(pos);
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
                let _ = self.controller.send_command(ThrottleCommand::ClearAll).await;
                IpcResponse::Ok
            }
        }
    }

    /// Invoked theoretically when ETW ProcessStop triggers 
    /// (to be wired securely alongside network events)
    pub fn drop_process_from_cache(&self, pid: u32) {
        self.pid_cache.remove(&pid);
    }

    async fn handle_connection_opened(&mut self, pid: u32) {
        // 1. Resolve Name O(1) mostly
        let exe_name = self.resolve_exe_name(pid);
        
        // 2. Load latest config O(1) lock-free
        let config = self.config_manager.get_shared_config().load();
        
        let critical_apps = if let Some(ref profile) = self.active_profile {
            config.critical_apps_for_profile(profile)
        } else {
            config.critical_apps.clone()
        };

        let is_priority = critical_apps.iter().any(|c| exe_name.eq_ignore_ascii_case(c));

        if is_priority {
            if self.priority_pids.insert(pid) {
                let msg = format!("Priority App Detected: {} (PID: {})", exe_name, pid);
                info!("{}", msg);
                self.push_event(msg);
                if self.state != ThrottleState::PriorityMode {
                    self.enter_priority_mode().await;
                }
            }
        } else {
            if self.normal_pids.insert(pid) {
                debug!("Normal App Registered: {} (PID: {})", exe_name, pid);
                if self.state == ThrottleState::PriorityMode {
                    // Apply throttle instantly to this new connection
                    let limit = if let Some(ref profile) = self.active_profile {
                        config.throttle_limit_for_profile(profile)
                    } else {
                        config.effective_throttle_limit()
                    };
                    let _ = self.controller.send_command(ThrottleCommand::ApplyLimit { pid, limit }).await;
                }
            }
        }
    }

    async fn handle_connection_closed(&mut self, pid: u32) {
        if self.normal_pids.remove(&pid) {
            if self.state == ThrottleState::PriorityMode {
                let _ = self.controller.send_command(ThrottleCommand::RemoveLimit { pid }).await;
            }
        }

        if self.priority_pids.remove(&pid) {
            let msg = format!("Priority App Disconnected (PID: {})", pid);
            info!("{}", msg);
            self.push_event(msg);
            if self.priority_pids.is_empty() && self.state == ThrottleState::PriorityMode {
                self.exit_priority_mode().await;
            }
        }
    }

    async fn enter_priority_mode(&mut self) {
        let msg = format!("Engaging Priority Mode! Throttling {} normal background processes.", self.normal_pids.len());
        info!("{}", msg);
        self.push_event(msg);
        self.state = ThrottleState::PriorityMode;
        
        let config = self.config_manager.get_shared_config().load();
        let limit = if let Some(ref profile) = self.active_profile {
            config.throttle_limit_for_profile(profile)
        } else {
            config.effective_throttle_limit()
        };
        
        for &pid in &self.normal_pids {
            let _ = self.controller.send_command(ThrottleCommand::ApplyLimit { pid, limit }).await;
        }
    }

    async fn re_evaluate_connections(&mut self) {
        let config = self.config_manager.get_shared_config().load();

        let critical_apps = if let Some(ref profile) = self.active_profile {
            config.critical_apps_for_profile(profile)
        } else {
            config.critical_apps.clone()
        };

        let mut to_priority = Vec::new();
        let mut to_normal = Vec::new();

        for &pid in &self.normal_pids {
            let exe_name = self.resolve_exe_name(pid);
            if critical_apps.iter().any(|c| exe_name.eq_ignore_ascii_case(c)) {
                to_priority.push(pid);
            }
        }

        for &pid in &self.priority_pids {
            let exe_name = self.resolve_exe_name(pid);
            if !critical_apps.iter().any(|c| exe_name.eq_ignore_ascii_case(c)) {
                to_normal.push(pid);
            }
        }

        for pid in to_priority {
            self.normal_pids.remove(&pid);
            self.priority_pids.insert(pid);
            if self.state == ThrottleState::PriorityMode {
                let _ = self.controller.send_command(ThrottleCommand::RemoveLimit { pid }).await;
            }
        }

        for pid in to_normal {
            self.priority_pids.remove(&pid);
            self.normal_pids.insert(pid);
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
            let limit = if let Some(ref profile) = self.active_profile {
                config.throttle_limit_for_profile(profile)
            } else {
                config.effective_throttle_limit()
            };
            for &pid in &self.normal_pids {
                let _ = self.controller.send_command(ThrottleCommand::ApplyLimit { pid, limit }).await;
            }
        }
    }

    async fn exit_priority_mode(&mut self) {
        let msg = "Returning to Normal Mode! Unthrottling all processes.".to_string();
        info!("{}", msg);
        self.push_event(msg);
        self.state = ThrottleState::Normal;
        let _ = self.controller.send_command(ThrottleCommand::ClearAll).await;
    }

    // Lightweight mock resolving logic
    fn resolve_exe_name(&self, pid: u32) -> Arc<str> {
        if let Some(name) = self.pid_cache.get(&pid) {
            return name.clone();
        }

        // Cache miss: Expensive OpenProcess / QueryFullProcessImageNameW
        let name: Arc<str> = Arc::from(format!("unknown_{pid}.exe"));
        self.pid_cache.insert(pid, name.clone());
        name
    }
}
