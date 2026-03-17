#[cfg(target_os = "windows")]
use std::ffi::OsString;
#[cfg(target_os = "windows")]
use std::sync::mpsc::channel;
#[cfg(target_os = "windows")]
use std::time::Duration;
#[cfg(target_os = "windows")]
use windows_service::{
    define_windows_service,
    service::{
        ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
        ServiceType,
    },
    service_control_handler::{self, ServiceControlHandlerResult},
    service_dispatcher,
};

#[cfg(target_os = "windows")]
const SERVICE_NAME: &str = "bwp-engine-service";

#[cfg(target_os = "windows")]
pub fn run_as_service() -> anyhow::Result<()> {
    // This is the entry point called by the Service Control Manager
    service_dispatcher::start(SERVICE_NAME, ffi_service_main)?;
    Ok(())
}

#[cfg(target_os = "windows")]
define_windows_service!(ffi_service_main, my_service_main);

#[cfg(target_os = "windows")]
fn my_service_main(_arguments: Vec<OsString>) {
    if let Err(_e) = run_service() {
        // Log error (we can't print to console in a service)
    }
}

#[cfg(target_os = "windows")]
fn run_service() -> windows_service::Result<()> {
    // Create a channel to communicate the STOP signal
    let (stop_tx, stop_rx) = channel();

    let event_handler = move |control_event| -> ServiceControlHandlerResult {
        match control_event {
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            ServiceControl::Stop => {
                let _ = stop_tx.send(());
                ServiceControlHandlerResult::NoError
            }
            _ => ServiceControlHandlerResult::NotImplemented,
        }
    };

    let status_handle = service_control_handler::register(SERVICE_NAME, event_handler)?;

    // Tell SCM that we are starting
    status_handle.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::StartPending,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::from_secs(2),
        process_id: None,
    })?;

    // Tell SCM that we are running
    status_handle.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::Running,
        controls_accepted: ServiceControlAccept::STOP,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    })?;

    // Launch the main engine in a new thread
    let engine_thread = std::thread::spawn(|| {
        if let Err(e) = crate::run_engine_core() {
            tracing::error!("Service engine core terminated with error: {}", e);
        }
    });

    // Wait for the SCM stop signal
    let _ = stop_rx.recv();

    // The STOP signal is received. We should gracefully shut down the engine.
    // The easiest way is to push a Stop command to the IPC server, or directly stop the token.
    // But since the engine runs in a tokio runtime, we can trigger shutdown via IPC client,
    // or just let the thread drop (which kills the WinDivert handle) and exit.
    // Cleanest way is to connect via IPC to itself and send Stop.
    let _ = std::process::Command::new(std::env::current_exe().unwrap_or_else(|_| "bwp-engine.exe".into()))
        .arg("--internal-stop")
        .output(); // Send the stop command (Wait for thread to finish)
    
    let _ = engine_thread.join();

    // Tell SCM that we stopped
    status_handle.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::Stopped,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    })?;

    Ok(())
}
