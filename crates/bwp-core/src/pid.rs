//! PID File Management Utility
//!
//! Provides functions to create, check, and remove PID files, 
//! ensuring only one instance of the engine runs at a time.

use std::fs;
use std::io::Result;
use std::path::PathBuf;

#[cfg(test)]
thread_local! {
    static MOCK_PID_DIR: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

#[cfg(target_os = "windows")]
pub fn get_pid_file_path() -> PathBuf {
    #[cfg(test)]
    if let Some(mock_dir) = MOCK_PID_DIR.with(|m| m.borrow().clone()) {
        let mut path = mock_dir;
        path.push("bwp.pid");
        return path;
    }

    let base_dir = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| "C:\\ProgramData".to_string());
    let mut path = PathBuf::from(base_dir);
    path.push("bwp");
    // Ensure directory exists
    let _ = fs::create_dir_all(&path);
    path.push("bwp.pid");
    path
}

#[cfg(target_os = "linux")]
pub fn get_pid_file_path() -> PathBuf {
    #[cfg(test)]
    if let Some(mock_dir) = MOCK_PID_DIR.with(|m| m.borrow().clone()) {
        let mut path = mock_dir;
        path.push("bwp.pid");
        return path;
    }

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
pub fn is_process_running(_pid: u32) -> bool {
    // In a real implementation we would use `OpenProcess` from the Windows API.
    // For now, if the file exists and IPC fails, we'll assume it's dead, but this function
    // acts as a lightweight heuristic. Returning true relies on IPC failure to clean up.
    true
}

#[cfg(target_os = "linux")]
pub fn is_process_running(pid: u32) -> bool {
    // Check if the process directory exists in procfs
    PathBuf::from(format!("/proc/{}", pid)).exists()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::fs;
    use std::sync::Mutex;
    use once_cell::sync::Lazy;

    static ENV_MUTEX: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

    fn with_test_env<F>(test: F)
    where
        F: FnOnce(&PathBuf),
    {
        let _guard = ENV_MUTEX.lock().unwrap();

        let temp_dir = std::env::temp_dir().join(format!("bwp_test_{}", std::process::id()));
        let _ = fs::create_dir_all(&temp_dir);

        MOCK_PID_DIR.with(|m| {
            *m.borrow_mut() = Some(temp_dir.clone());
        });

        let pid_file_path = get_pid_file_path();

        // Run the test
        test(&pid_file_path);

        // Cleanup
        MOCK_PID_DIR.with(|m| {
            *m.borrow_mut() = None;
        });
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_read_pid_existing() {
        with_test_env(|pid_file_path| {
            // Write a valid PID
            if let Some(parent) = pid_file_path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            fs::write(pid_file_path, "12345\n").unwrap();

            let pid = read_pid().unwrap();
            assert_eq!(pid, Some(12345));
        });
    }

    #[test]
    fn test_read_pid_non_existing() {
        with_test_env(|pid_file_path| {
            // Ensure file doesn't exist
            if pid_file_path.exists() {
                fs::remove_file(pid_file_path).unwrap();
            }

            let pid = read_pid().unwrap();
            assert_eq!(pid, None);
        });
    }

    #[test]
    fn test_read_pid_invalid_content() {
        with_test_env(|pid_file_path| {
            // Write invalid content
            if let Some(parent) = pid_file_path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            fs::write(pid_file_path, "not_a_pid\n").unwrap();

            let pid = read_pid().unwrap();
            assert_eq!(pid, None);
        });
    }
}
