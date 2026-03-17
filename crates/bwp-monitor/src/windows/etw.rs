//! Event Tracing for Windows (ETW) Integration.
//!
//! Streams real-time network connection events using the `Microsoft-Windows-TCPIP`
//! provider. This entirely bypasses the O(N) overhead and latency of polling.

use bwp_core::{BwpError, MonitorEvent, BwpResult, ConnectionInfo, Protocol, ConnectionState};
use ferrisetw::parser::Parser;
use ferrisetw::provider::Provider;
use ferrisetw::trace::{TraceTrait, UserTrace};
use tokio::sync::mpsc;
use tracing::{debug, error, info, trace};
use std::sync::Arc;
use std::time::Duration;

/// The GUID for `Microsoft-Windows-TCPIP` ETW Provider.
/// {2F07E2EE-15DB-40F1-90EF-9D7BA282188A}
const TCPIP_PROVIDER_GUID: &str = "2F07E2EE-15DB-40F1-90EF-9D7BA282188A";

/// Attempts to spawn the ETW trace session in a background blocking thread.
///
/// Returns an error immediately if the session fails to start (e.g. access denied).
pub fn spawn_etw_listener(tx: mpsc::Sender<MonitorEvent>) -> BwpResult<()> {
    // We clone the sender to pass it to the callback closure.
    let callback_tx = tx.clone();
    
    // We create the ETW provider targeting network events.
    let provider = Provider::by_guid(TCPIP_PROVIDER_GUID)
        .add_callback(move |record, schema_locator| {
            // Callback execution context: synchronous background kernel ETW worker thread.
            // Parse the event into a strongly-typed schema if possible.
            match schema_locator.event_schema(record) {
                Ok(schema) => {
                    let task_name = schema.task_name();
                    let opcode_name = schema.opcode_name();
                    
                    // Simple logging for real-time trace events.
                    // We parse `TcpConnect` and `TcpDisconnect` opcodes.
                    // (Event 10: TcpIp_TypeGroup1 usually contains Connect)
                    if task_name == "TcpCreateEndpoint" || task_name == "TcpEndpointClosure" || opcode_name == "Connect" {
                        let mut parser = Parser::create(record, &schema);
                        
                        // Parse network payload. Field names depend on the exact ETW schema version.
                        // For a robust implementation, we wrap in try blocks.
                        let pid: u32 = record.process_id();
                        
                        // Example: Try to read IP arrays, ports if known fields match.
                        // Currently, this is a skeleton showing the interception point.
                        if let Ok(_) = parser.try_parse::<u32>("LocalPort") {
                            // Convert ETW types to our ConnectionInfo
                            let info = ConnectionInfo {
                                pid,
                                protocol: Protocol::Tcp,
                                local_addr: "0.0.0.0:0".to_string(), // Replace with actual parsing
                                remote_addr: None,
                                state: if task_name.contains("Closure") { ConnectionState::Other } else { ConnectionState::Established },
                            };
                            
                            let event = if task_name.contains("Closure") {
                                MonitorEvent::ConnectionClosed(info)
                            } else {
                                MonitorEvent::ConnectionOpened(info)
                            };

                            let _ = callback_tx.try_send(event);
                        }
                    }
                }
                Err(e) => {
                    trace!("Could not schema for ETW record: {:?}", e);
                }
            }
        })
        .build();

    let trace_name = "BwpNetworkTraceSession";
    
    // Create the User Trace session. Requires Administrative privileges.
    let trace_session = match UserTrace::new().named(trace_name.to_string()).enable(provider).start() {
        Ok(t) => t,
        Err(e) => return Err(BwpError::Platform(format!("ETW Start Failed: {e:?}"))),
    };

    // We must process events synchronously. Move this to a dedicated OS thread.
    std::thread::spawn(move || {
        info!("ETW Trace Session '{}' active. Processing events...", trace_name);
        
        let mut session = trace_session;
        if let Err(e) = session.0.process() {
            error!("ETW event processing error: {:?}", e);
        }
        
        // When dropped or process stops, the ETW session is automatically stopped 
        // by the Drop impl in ferrisetw.
    });

    Ok(())
}
