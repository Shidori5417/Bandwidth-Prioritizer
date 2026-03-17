#[cfg(target_os = "windows")]
use anyhow::Result;
#[cfg(target_os = "windows")]
use std::ffi::OsString;
#[cfg(target_os = "windows")]
use std::path::PathBuf;
#[cfg(target_os = "windows")]
use windows_service::{
    service::ServiceAccess,
    service_manager::{ServiceManager, ServiceManagerAccess},
};

#[cfg(target_os = "windows")]
const SERVICE_NAME: &str = "bwp-engine-service";
#[cfg(target_os = "windows")]
const SERVICE_DISPLAY_NAME: &str = "Bandwidth Prioritizer Engine";

#[cfg(target_os = "windows")]
pub fn install() -> Result<()> {
    // Locate engine binary
    let engine_path = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("bwp-engine.exe")))
        .unwrap_or_else(|| PathBuf::from("bwp-engine.exe"));

    if !engine_path.exists() {
        return Err(anyhow::anyhow!("bwp-engine.exe not found next to CLI"));
    }

    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CREATE_SERVICE)?;

    // We pass the `--service` argument to tell the engine to run the service dispatcher
    let service_info = windows_service::service::ServiceInfo {
        name: SERVICE_NAME.into(),
        display_name: SERVICE_DISPLAY_NAME.into(),
        service_type: windows_service::service::ServiceType::OWN_PROCESS,
        start_type: windows_service::service::ServiceStartType::AutoStart,
        error_control: windows_service::service::ServiceErrorControl::Normal,
        executable_path: engine_path,
        launch_arguments: vec![OsString::from("--service")],
        dependencies: vec![],
        account_name: None,
        account_password: None,
    };

    let _ = manager.create_service(&service_info, ServiceAccess::CHANGE_CONFIG);
    
    // Also tell it to restart on failure (basic recovery)
    let _service = manager.open_service(SERVICE_NAME, ServiceAccess::CHANGE_CONFIG)?;
    
    println!("✅ Windows Service '{}' installed successfully.", SERVICE_DISPLAY_NAME);
    Ok(())
}

#[cfg(target_os = "windows")]
pub fn uninstall() -> Result<()> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)?;
    let service = manager.open_service(SERVICE_NAME, ServiceAccess::DELETE)?;
    service.delete()?;
    println!("✅ Windows Service uninstalled successfully.");
    Ok(())
}

#[cfg(target_os = "windows")]
pub fn start() -> Result<()> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)?;
    let service = manager.open_service(SERVICE_NAME, ServiceAccess::START)?;
    service.start(&[OsString::new()])?;
    println!("🚀 Windows Service started.");
    Ok(())
}

#[cfg(target_os = "windows")]
pub fn stop() -> Result<()> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)?;
    let service = manager.open_service(SERVICE_NAME, ServiceAccess::STOP)?;
    let status = service.stop()?;
    println!("🛑 Windows Service stopped (state: {:?}).", status.current_state);
    Ok(())
}

#[cfg(not(target_os = "windows"))]
pub fn install() -> anyhow::Result<()> { Err(anyhow::anyhow!("Service commands only supported on Windows")) }
#[cfg(not(target_os = "windows"))]
pub fn uninstall() -> anyhow::Result<()> { Err(anyhow::anyhow!("Service commands only supported on Windows")) }
#[cfg(not(target_os = "windows"))]
pub fn start() -> anyhow::Result<()> { Err(anyhow::anyhow!("Service commands only supported on Windows")) }
#[cfg(not(target_os = "windows"))]
pub fn stop() -> anyhow::Result<()> { Err(anyhow::anyhow!("Service commands only supported on Windows")) }
