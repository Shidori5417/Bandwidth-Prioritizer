use std::collections::HashSet;
use std::sync::Arc;
use bwp_core::{BwpResult, MonitorEvent, ThrottleState, BandwidthLimit};
use bwp_core::manager::ConfigManager;
use bwp_controller::{TrafficController, ThrottleCommand};
use dashmap::DashMap;
use tokio::sync::mpsc;
use tracing::{info, debug, warn};

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
    state: ThrottleState,
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
        }
    }

    /// Primary event loop processing raw network events from the Monitor driver
    pub async fn run_event_loop(&mut self, mut monitor_rx: mpsc::Receiver<MonitorEvent>) -> BwpResult<()> {
        info!("Bandwidth Engine state machine running...");

        while let Some(event) = monitor_rx.recv().await {
            match event {
                MonitorEvent::ConnectionOpened(info) => {
                    self.handle_connection_opened(info.pid).await;
                }
                MonitorEvent::ConnectionClosed(info) => {
                    self.handle_connection_closed(info.pid).await;
                }
            }
        }

        warn!("Monitor event stream closed. Engine shutting down.");
        let _ = self.controller.send_command(ThrottleCommand::ClearAll).await;
        Ok(())
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
        
        let is_priority = config.critical_apps.iter().any(|c| exe_name.eq_ignore_ascii_case(c));

        if is_priority {
            if self.priority_pids.insert(pid) {
                info!("Priority App Detected: {} (PID: {})", exe_name, pid);
                if self.state != ThrottleState::PriorityMode {
                    self.enter_priority_mode().await;
                }
            }
        } else {
            if self.normal_pids.insert(pid) {
                debug!("Normal App Registered: {} (PID: {})", exe_name, pid);
                if self.state == ThrottleState::PriorityMode {
                    // Apply throttle instantly to this new connection
                    let limit = config.effective_throttle_limit();
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
            info!("Priority App Disconnected (PID: {})", pid);
            if self.priority_pids.is_empty() && self.state == ThrottleState::PriorityMode {
                self.exit_priority_mode().await;
            }
        }
    }

    async fn enter_priority_mode(&mut self) {
        info!("Engaging Priority Mode! Throttling {} normal background processes.", self.normal_pids.len());
        self.state = ThrottleState::PriorityMode;
        
        let config = self.config_manager.get_shared_config().load();
        let limit = config.effective_throttle_limit();
        
        for &pid in &self.normal_pids {
            let _ = self.controller.send_command(ThrottleCommand::ApplyLimit { pid, limit }).await;
        }
    }

    async fn exit_priority_mode(&mut self) {
        info!("Returning to Normal Mode! Unthrottling all processes.");
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
