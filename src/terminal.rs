//! Terminal lifecycle helpers — setup, teardown, and startup context picker.

use std::{io, time::Duration};

use anyhow::{Context, Result};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};

use kubectui::{
    app::{AppAction, AppState},
    k8s::client::K8sClient,
    state::ClusterSnapshot,
    ui,
};

/// Configures terminal in alternate screen + raw mode for TUI rendering.
pub fn setup_terminal() -> Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode().context("failed enabling raw mode")?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)
        .context("failed entering alternate screen")?;

    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend).context("failed creating terminal backend")?;
    Ok(terminal)
}

/// Restores terminal state back to canonical mode.
pub fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
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

/// Shows a context picker at startup and returns a connected `K8sClient`.
/// If only one context exists or the user presses Esc, connects to the default context.
pub async fn pick_context_at_startup(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut AppState,
) -> Result<K8sClient> {
    let contexts = K8sClient::list_contexts();
    let current = kube::config::Kubeconfig::read()
        .ok()
        .and_then(|cfg| cfg.current_context);

    if contexts.len() <= 1 {
        return K8sClient::connect()
            .await
            .context("unable to initialize Kubernetes client");
    }

    app.open_context_picker(contexts, current);

    loop {
        let snapshot = ClusterSnapshot::default();
        terminal
            .draw(|frame| ui::render(frame, app, &snapshot))
            .context("failed to render startup context picker")?;

        if event::poll(Duration::from_millis(16)).context("failed to poll events")?
            && let Event::Key(key) = event::read().context("failed to read event")?
        {
            match app.handle_key_event(key) {
                AppAction::SelectContext(ctx) => {
                    app.close_context_picker();
                    return K8sClient::connect_with_context(&ctx)
                        .await
                        .with_context(|| format!("failed to connect to context '{ctx}'"));
                }
                AppAction::CloseContextPicker | AppAction::Quit => {
                    app.close_context_picker();
                    return K8sClient::connect()
                        .await
                        .context("unable to initialize Kubernetes client");
                }
                _ => {}
            }
        }
    }
}
