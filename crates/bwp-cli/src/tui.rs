use anyhow::{Context, Result};
use bwp_core::ipc::{IpcCommand, IpcResponse};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph, Row, Table},
    Terminal,
};
use std::{io, time::Duration};
use tokio::time::Instant;

#[cfg(target_os = "windows")]
use bwp_core::ipc::windows::connect_client;

#[cfg(target_os = "linux")]
use bwp_core::ipc::linux::connect_client;

pub async fn run_tui() -> Result<()> {
    // Install panic hook to restore terminal on crash
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
        original_hook(info);
    }));

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let res = run_app(&mut terminal).await;

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        println!("{:?}", err);
    }

    Ok(())
}

async fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    let tick_rate = Duration::from_millis(500);
    let mut last_tick = Instant::now();

    let mut current_status: Option<IpcResponse> = None;
    let mut error_msg: Option<String> = None;

    loop {
        // Only fetch status if enough time has passed
        if last_tick.elapsed() >= tick_rate {
            match connect_client().await {
                Ok(mut conn) => {
                    if let Err(e) = conn.send_command(&IpcCommand::Status).await {
                        error_msg = Some(format!("Failed to send status command: {}", e));
                    } else {
                        match conn.recv_response().await {
                            Ok(Some(resp)) => {
                                current_status = Some(resp);
                                error_msg = None;
                            }
                            Ok(None) => error_msg = Some("Connection closed by engine.".to_string()),
                            Err(e) => error_msg = Some(format!("Error receiving response: {}", e)),
                        }
                    }
                }
                Err(e) => {
                    error_msg = Some(format!("Could not connect to engine: {}", e));
                    current_status = None;
                }
            }
            last_tick = Instant::now();
        }

        let _ = terminal.draw(|f| draw_ui(f, &current_status, &error_msg))?;

        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or_else(|| Duration::from_secs(0));

        // Note: crossterm::event::poll blocks the thread, but with a short timeout it's acceptable in this simple loop
        if crossterm::event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Char('s') => {
                        // Send stop command
                        if let Ok(mut conn) = connect_client().await {
                            let _ = conn.send_command(&IpcCommand::Stop).await;
                        }
                        return Ok(());
                    }
                    _ => {}
                }
            }
        }
    }
}

fn draw_ui(f: &mut ratatui::Frame, status: &Option<IpcResponse>, error: &Option<String>) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Length(3), // Header
                Constraint::Min(5),    // Table
                Constraint::Length(3), // Footer
            ]
            .as_ref(),
        )
        .split(f.area());

    // --- Header ---
    if let Some(err) = error {
        let header = Paragraph::new(format!(" Error: {} ", err))
            .style(Style::default().fg(Color::Red))
            .block(Block::default().borders(Borders::ALL).title(" Bandwidth-Prioritizer "));
        f.render_widget(header, chunks[0]);
    } else if let Some(IpcResponse::StatusPayload {
        state,
        active_profile,
        ..
    }) = status
    {
        let prof_str = active_profile.as_deref().unwrap_or("None");
        let state_str = format!("{:?}", state);
        let status_color = if state_str == "PriorityMode" { Color::Red } else { Color::Green };
        
        // Use single string for paragraph since Spans from text module has changed across ratatui versions
        // and using simple string is safer here without checking the exact ratatui version.
        let header_text = format!("Profil: {:<15} Durum: {}", prof_str, state_str);
        
        let header = Paragraph::new(header_text)
            .style(Style::default().fg(status_color))
            .block(Block::default().borders(Borders::ALL).title(" Bandwidth-Prioritizer "));
        f.render_widget(header, chunks[0]);
    } else {
        let header = Paragraph::new(" Loading... ")
            .block(Block::default().borders(Borders::ALL).title(" Bandwidth-Prioritizer "));
        f.render_widget(header, chunks[0]);
    }

    // --- Middle Table (Throttled Apps) ---
    let mut table_rows = vec![];
    let mut footer_stats = "Priority App'ler: 0    Kısıtlanan: 0".to_string();

    if let Some(IpcResponse::StatusPayload {
        throttled_apps,
        priority_pids_count,
        throttled_pids_count,
        ..
    }) = status
    {
        footer_stats = format!(
            "Priority App'ler: {}    Kısıtlanan: {}",
            priority_pids_count, throttled_pids_count
        );

        for (name, pid, limit) in throttled_apps {
            table_rows.push(Row::new(vec![
                name.to_string(),
                pid.to_string(),
                format!("{} kbps", limit),
            ]));
        }
    }

    let table = Table::new(table_rows, [
            Constraint::Percentage(40),
            Constraint::Percentage(20),
            Constraint::Percentage(40),
        ])
        .header(Row::new(vec!["Kısıtlanan Uygulama", "PID", "Limit"]).style(Style::default().fg(Color::Yellow)))
        .block(Block::default().borders(Borders::ALL));
        
    f.render_widget(table, chunks[1]);

    // --- Footer ---
    let footer_text = format!("{}\n[q] Çık  [s] Stop", footer_stats);
    let footer = Paragraph::new(footer_text)
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(footer, chunks[2]);
}
