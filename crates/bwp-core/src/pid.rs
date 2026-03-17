//! PID File Management Utility
//!
//! Provides functions to create, check, and remove PID files, 
//! ensuring only one instance of the engine runs at a time.

use std::fs;
use std::io::Result;
use std::path::PathBuf;

#[cfg(target_os = "windows")]
pub fn get_pid_file_path() -> PathBuf {
    let mut path = PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_else(|_| "C:\\ProgramData".to_string()));
    path.push("bwp");
    // Ensure directory exists
    let _ = fs::create_dir_all(&path);
    path.push("bwp.pid");
    path
}

#[cfg(target_os = "linux")]
pub fn get_pid_file_path() -> PathBuf {
    PathBuf::from("/var/run/bwp.pid")
}

/// Reads the PID from the PID file if it exists.
pub fn read_pid() -> Result<Option<u32>> {
    let path = get_pid_file_path();
    if !path.exists() {
        return Ok(None);
    }
    
    let content = fs::read_to_string(&path)?;
    match content.trim().parse::<u32>() {
        Ok(pid) => Ok(Some(pid)),
        Err(_) => Ok(None),
    }
}

/// Writes the current process ID to the PID file.
pub fn write_pid() -> Result<()> {
    let path = get_pid_file_path();
    let pid = std::process::id();
    fs::write(path, pid.to_string())
}

/// Removes the PID file.
pub fn remove_pid() -> Result<()> {
    let path = get_pid_file_path();
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

/// Checks if a process with the given PID is currently running.
#[cfg(target_os = "windows")]
pub fn is_process_running(pid: u32) -> bool {
    use windows::core::HRESULT;
    use windows::Win32::Foundation::{CloseHandle, ERROR_ACCESS_DENIED, STILL_ACTIVE};
    use windows::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    // SAFETY: We are calling Windows API functions to query process information.
    // The handle is properly closed if it's successfully opened.
    #[allow(unsafe_code)]
    let handle_result = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) };

    match handle_result {
        Ok(handle) => {
            let mut exit_code: u32 = 0;
            #[allow(unsafe_code)]
            let result = unsafe { GetExitCodeProcess(handle, &mut exit_code) };
            #[allow(unsafe_code)]
            unsafe {
                let _ = CloseHandle(handle);
            }

            if result.is_ok() {
                exit_code == STILL_ACTIVE.0 as u32
            } else {
                true // If we can open it but not get exit code, assume it's running
            }
        }
        Err(err) => {
            // ERROR_ACCESS_DENIED means the process is running but we don't have permission to query it
            // ERROR_INVALID_PARAMETER usually means the process doesn't exist
            let hresult: HRESULT = ERROR_ACCESS_DENIED.into();
            err.code() == hresult
        }
    }
}

#[cfg(target_os = "linux")]
pub fn is_process_running(pid: u32) -> bool {
    // Check if the process directory exists in procfs
    PathBuf::from(format!("/proc/{}", pid)).exists()
}
