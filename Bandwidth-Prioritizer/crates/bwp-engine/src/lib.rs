//! # bwp-engine
//!
//! The central orchestration and state machine layer for Bandwidth-Prioritizer.
//!
//! Connects the `bwp-monitor` network event stream with the `bwp-controller`
//! traffic shaping engine. Makes high-speed routing decisions by resolving PIDs
//! via a lock-free `DashMap` cache and checking against the zero-cost config
//! manager (`ArcSwap`).

pub mod state_machine;
pub mod ipc_server;

pub use state_machine::{Engine, EngineEvent};
