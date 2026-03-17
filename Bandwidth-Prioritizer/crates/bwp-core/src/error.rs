//! Unified error types for Bandwidth-Prioritizer.
//!
//! Uses `thiserror` for ergonomic, zero-cost error definitions.
//! All crate-level errors funnel into [`BwpError`] so callers get
//! a single error type to match against.

use thiserror::Error;

/// Convenience alias used throughout the workspace.
pub type BwpResult<T> = Result<T, BwpError>;

/// Top-level error enum for Bandwidth-Prioritizer.
///
/// Each variant maps to a distinct failure domain, making it easy
/// to handle errors at appropriate granularity.
#[derive(Debug, Error)]
pub enum BwpError {
    /// Configuration file could not be read or parsed.
    #[error("Configuration error: {0}")]
    Config(String),

    /// Process monitoring subsystem failure.
    #[error("Monitor error: {0}")]
    Monitor(String),

    /// Traffic control / throttle operation failure.
    #[error("Controller error: {0}")]
    Controller(String),

    /// An I/O error occurred at the OS level.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// A platform-specific API call failed.
    #[error("Platform API error: {0}")]
    Platform(String),

    /// Catch-all for unexpected errors.
    #[error("Internal error: {0}")]
    Internal(String),
}
