//! # bwp-engine
//!
//! The central orchestration and state machine layer for Bandwidth-Prioritizer.
//!
//! Connects the `bwp-monitor` network event stream with the `bwp-controller`
//! traffic shaping engine. Makes high-speed routing decisions by resolving PIDs
//! via a lock-free `DashMap` cache and checking against the zero-cost config
//! manager (`ArcSwap`).
//!
//! **Status:** the state machine, IPC server and service wiring work. Real packet
//! shaping (controller) and connection monitoring (ETW) do not; see the README.

/// IPC server for CLI connections.
pub mod ipc_server;
/// Engine state machine.
pub mod state_machine;

pub use state_machine::{Engine, EngineEvent};
