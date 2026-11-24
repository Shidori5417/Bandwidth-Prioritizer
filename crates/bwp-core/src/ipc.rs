//! Inter-Process Communication (IPC) Abstractions
//!
//! Provides a secure, platform-native transport layer wrapped in length-prefixed JSON framing.
//! Uses Named Pipes on Windows (ACLed) and Unix Sockets on Linux (chmod 600).

use crate::types::ThrottleState;
use serde::{Deserialize, Serialize};
use std::io::{Error, ErrorKind, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

// ─── IPC Protocol Types ─────────────────────────────────────────────

/// Commands sent from the CLI `bwp-cli` to the daemon `bwp-engine`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IpcCommand {
    /// Request current engine status.
    Status,
    /// Apply a specific throttle profile manually.
    ApplyProfile(String),
    /// Add an application to the priority whitelist.
    AddApp(String),
    /// Remove an application from the priority whitelist.
    RemoveApp(String),
    /// Shut down the bandwidth prioritizer engine.
    Stop,
}

/// Responses sent from `bwp-engine` back to `bwp-cli`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IpcResponse {
    /// Operation successfully applied.
    Ok,
    /// An error occurred during processing.
    Error(String),
    /// Current engine state payload.
    StatusPayload {
        /// Current state mode (Normal/PriorityMode).
        state: ThrottleState,
        /// Name of the active profile (if any).
        active_profile: Option<String>,
        /// Number of throttled limits currently enforced.
        throttled_pids_count: usize,
        /// Number of detected priority (whitelisted) pids running.
        priority_pids_count: usize,
        /// Enforced limit across throttled processes in Kbps.
        current_limit_kbps: u64,
        /// Throttled applications list: (App Name, PID, Limit in Kbps)
        throttled_apps: Vec<(std::sync::Arc<str>, u32, u64)>,
        /// Recent engine events for the TUI log
        recent_events: Vec<String>,
    },
}

// ─── Framing Protocol ───────────────────────────────────────────────

/// A length-prefixed JSON frame connection wrapping any AsyncRead/Write stream.
pub struct IpcConnection<S> {
    stream: S,
}

impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin> IpcConnection<S> {
    /// Wraps a connected stream (named pipe or Unix socket) in a framed IPC connection.
    pub fn new(stream: S) -> Self {
        Self { stream }
    }

    /// Generic un-framed read logic.
    async fn recv_payload<T: serde::de::DeserializeOwned>(&mut self) -> Result<Option<T>> {
        let len = match self.stream.read_u32_le().await {
            Ok(len) => len,
            Err(e) if e.kind() == ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e),
        };

        if len > 10 * 1024 * 1024 {
            // Guard against massive payloads
            return Err(Error::new(ErrorKind::InvalidData, "IPC payload too large"));
        }

        let mut buf = vec![0u8; len as usize];
        let _ = self.stream.read_exact(&mut buf).await?;

        let msg =
            serde_json::from_slice(&buf).map_err(|e| Error::new(ErrorKind::InvalidData, e))?;
        Ok(Some(msg))
    }

    /// Generic length-prefixed write logic.
    async fn send_payload<T: Serialize>(&mut self, msg: &T) -> Result<()> {
        let payload = serde_json::to_vec(msg).map_err(|e| Error::new(ErrorKind::InvalidData, e))?;
        self.stream.write_u32_le(payload.len() as u32).await?;
        self.stream.write_all(&payload).await?;
        self.stream.flush().await?;
        Ok(())
    }

    /// For the ENGINE: Receive a command.
    pub async fn recv_command(&mut self) -> Result<Option<IpcCommand>> {
        self.recv_payload().await
    }

    /// For the ENGINE: Send a response.
    pub async fn send_response(&mut self, resp: &IpcResponse) -> Result<()> {
        self.send_payload(resp).await
    }

    /// For the CLI: Send a command.
    pub async fn send_command(&mut self, cmd: &IpcCommand) -> Result<()> {
        self.send_payload(cmd).await
    }

    /// For the CLI: Wait for a response.
    pub async fn recv_response(&mut self) -> Result<Option<IpcResponse>> {
        self.recv_payload().await
    }
}

// ─── Platform-Specific Transport Interfaces ─────────────────────────

#[cfg(target_os = "windows")]
/// Named pipe transport (Windows).
pub mod windows {
    use super::*;
    use tokio::net::windows::named_pipe::{
        ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
    };

    // Windows constants for named pipe
    /// Name of the engine named pipe.
    pub const PIPE_NAME: &str = r"\\.\pipe\BandwidthPrioritizerIpc";

    /// CLI connects to the running engine using Named Pipes.
    pub async fn connect_client() -> Result<IpcConnection<NamedPipeClient>> {
        let client = ClientOptions::new().open(PIPE_NAME)?;
        Ok(IpcConnection::new(client))
    }

    /// Engine binds a Named Pipe Server for incoming IPC connections.
    /// Security is naturally handled if run as Administrator (only admins can bind a global pipe).
    pub fn bind_server() -> Result<NamedPipeServer> {
        let server = ServerOptions::new()
            .first_pipe_instance(true)
            .create(PIPE_NAME)?;
        Ok(server)
    }

    /// Binds the next sequentially accepted named pipe connection instance.
    pub fn loop_server() -> Result<NamedPipeServer> {
        ServerOptions::new().create(PIPE_NAME)
    }
}

#[cfg(target_os = "linux")]
/// Unix domain socket transport (Linux).
pub mod linux {
    use super::*;
    use tokio::net::{UnixListener, UnixStream};

    // Unix Socket path
    /// Path of the engine socket.
    pub const SOCKET_PATH: &str = "/var/run/bwp.sock";

    /// CLI connects to the running engine through the Unix socket.
    pub async fn connect_client() -> Result<IpcConnection<UnixStream>> {
        let stream = UnixStream::connect(SOCKET_PATH).await?;
        Ok(IpcConnection::new(stream))
    }

    /// Engine binds the Unix socket for incoming IPC connections.
    pub fn bind_server() -> Result<UnixListener> {
        let _ = std::fs::remove_file(SOCKET_PATH);

        // Bind the socket inside a private temporary directory, then rename it into place.
        // `umask` affects the whole process (thread-unsafe), so a private directory is preferred.
        // The PID makes the path unpredictable, and `create_private_dir` refuses to reuse
        // anything already at that path (including a planted symlink).
        let temp_dir_path = format!("/var/run/bwp_sock_{}", std::process::id());
        let _ = std::fs::remove_dir_all(&temp_dir_path);
        create_private_dir(&temp_dir_path)?;

        use std::os::unix::fs::PermissionsExt;

        let temp_socket_path = format!("{}/bwp.sock", temp_dir_path);

        // Bind the socket inside the secure directory
        let listener = UnixListener::bind(&temp_socket_path)?;

        // Explicitly set socket permissions to 600
        let mut sock_perms = std::fs::metadata(&temp_socket_path)?.permissions();
        sock_perms.set_mode(0o600);
        std::fs::set_permissions(&temp_socket_path, sock_perms)?;

        // Atomically move the socket to its final destination
        std::fs::rename(&temp_socket_path, SOCKET_PATH)?;

        // Cleanup the temporary directory
        let _ = std::fs::remove_dir(&temp_dir_path);

        Ok(listener)
    }

    /// Creates `path` as a new directory with mode 0700 in a single step.
    ///
    /// Unlike `create_dir_all` followed by `set_permissions`, this never leaves the
    /// directory world-accessible, and it fails with `AlreadyExists` if anything,
    /// such as a symlink planted by another user, already occupies the path.
    pub(crate) fn create_private_dir(path: &str) -> Result<()> {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new().mode(0o700).create(path)
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::linux::create_private_dir;
    use std::fs;
    use std::os::unix::fs::{PermissionsExt, symlink};

    #[test]
    fn private_dir_is_created_with_mode_0700() {
        let path = format!("/tmp/bwp_test_sock_{}", std::process::id());
        let _ = fs::remove_dir_all(&path);

        create_private_dir(&path).expect("directory should be created");

        let metadata = fs::metadata(&path).unwrap();
        assert!(metadata.is_dir());
        assert_eq!(metadata.permissions().mode() & 0o777, 0o700);

        fs::remove_dir_all(&path).unwrap();
    }

    #[test]
    fn private_dir_refuses_existing_symlink() {
        let target = format!("/tmp/bwp_test_target_{}", std::process::id());
        let link = format!("/tmp/bwp_test_link_{}", std::process::id());
        let _ = fs::remove_dir_all(&target);
        let _ = fs::remove_file(&link);

        fs::create_dir_all(&target).unwrap();
        symlink(&target, &link).unwrap();

        let err = create_private_dir(&link).expect_err("symlink must not be reused");
        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);

        let _ = fs::remove_dir_all(&target);
        let _ = fs::remove_file(&link);
    }
}
