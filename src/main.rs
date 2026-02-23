//! KubecTUI entry point.
//!
//! This module wires terminal lifecycle management, the application state machine,
//! the Kubernetes client, and the ratatui rendering pipeline.

mod app;
mod k8s;
mod state;
mod ui;

use std::{io, time::Duration};

use anyhow::{Context, Result};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};

use crate::{
    app::{AppAction, AppState},
    k8s::client::K8sClient,
    state::GlobalState,
};

/// Main asynchronous runtime entrypoint.
#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    let mut terminal = setup_terminal().context("failed to initialize terminal")?;
    let run_result = run_app(&mut terminal).await;
    let restore_result = restore_terminal(&mut terminal);

    if let Err(err) = restore_result {
        eprintln!("failed to restore terminal state: {err:#}");
    }

    run_result
}

/// Runs KubecTUI's event loop.
async fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    let client = K8sClient::connect()
        .await
        .context("unable to initialize Kubernetes client")?;

    let mut app = AppState::default();
    let mut global_state = GlobalState::default();

    if let Err(err) = global_state.refresh(&client).await {
        app.set_error(format!("Initial data refresh failed: {err:#}"));
    }

    let mut tick = tokio::time::interval(Duration::from_millis(200));

    loop {
        let snapshot = global_state.snapshot();
        terminal
            .draw(|frame| ui::render(frame, &app, &snapshot))
            .context("failed to render frame")?;

        if app.should_quit() {
            break;
        }

        if event::poll(Duration::from_millis(1)).context("failed to poll terminal events")? {
            if let Event::Key(key) = event::read().context("failed to read terminal event")? {
                match app.handle_key_event(key) {
                    AppAction::None => {}
                    AppAction::Quit => break,
                    AppAction::RefreshData => {
                        if let Err(err) = global_state.refresh(&client).await {
                            app.set_error(format!("Refresh failed: {err:#}"));
                        } else {
                            app.clear_error();
                        }
                    }
                }
            }
        }

        tick.tick().await;
    }

    Ok(())
}

/// Configures terminal in alternate screen + raw mode for TUI rendering.
fn setup_terminal() -> Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode().context("failed enabling raw mode")?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)
        .context("failed entering alternate screen")?;

    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend).context("failed creating terminal backend")?;
    Ok(terminal)
}

/// Restores terminal state back to canonical mode.
fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    disable_raw_mode().context("failed disabling raw mode")?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )
    .context("failed leaving alternate screen")?;
    terminal.show_cursor().context("failed to show cursor")?;
    Ok(())
}
