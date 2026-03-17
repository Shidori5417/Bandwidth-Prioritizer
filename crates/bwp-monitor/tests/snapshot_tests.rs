use bwp_monitor::windows::WindowsMonitor;
use bwp_monitor::ProcessMonitor;
use tokio::runtime::Runtime;

#[test]
#[cfg(target_os = "windows")]
fn test_snapshot_connections_runs_without_panic() {
    let rt = Runtime::new().unwrap();
    let monitor = WindowsMonitor::new();

    let result = rt.block_on(async {
        monitor.list_connections().await
    });

    assert!(result.is_ok(), "Snapshot should not panic and return Ok");
}
