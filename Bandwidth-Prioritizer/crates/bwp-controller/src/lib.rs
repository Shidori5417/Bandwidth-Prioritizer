//! # bwp-controller
//!
//! Traffic control and bandwidth throttling engine for **Bandwidth-Prioritizer**.
//!
//! Provides:
//! - Platform-agnostic [`TrafficController`] trait
//! - Token Bucket rate limiter (shared across platforms)

pub mod traits;

pub use traits::TrafficController;
