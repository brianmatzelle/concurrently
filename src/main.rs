mod agent;
mod api;
mod app;
mod orchestrator;
mod ui;

use std::io;
use std::time::{Duration, Instant};

use app::{App, AppMode};
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;

    // Get API key
    let api_key = std::env::var("ANTHROPIC_API_KEY").unwrap_or_else(|_| {
        eprintln!("Error: ANTHROPIC_API_KEY environment variable not set");
        eprintln!("  export ANTHROPIC_API_KEY=sk-ant-...");
        std::process::exit(1);
    });

    // Terminal setup
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_app(&mut terminal, api_key).await;

    // Restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    if let Err(e) = result {
        eprintln!("Error: {e}");
    }

    Ok(())
}

async fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    api_key: String,
) -> color_eyre::Result<()> {
    let mut app = App::new(api_key);
    let start = Instant::now();
    let tick_rate = Duration::from_millis(33); // ~30fps

    loop {
        app.elapsed_ms = start.elapsed().as_millis() as u64;

        // Process any pending agent events
        app.process_events();

        // Draw
        terminal.draw(|f| ui::draw(f, &app))?;

        // Handle input with timeout
        if event::poll(tick_rate)? {
            if let Event::Key(key) = event::read()? {
                // Global: Ctrl+C to quit
                if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c')
                {
                    return Ok(());
                }

                match app.mode {
                    AppMode::Input => match key.code {
                        KeyCode::Enter => {
                            if !app.input.is_empty() {
                                app.submit_task();
                            }
                        }
                        KeyCode::Char(c) => {
                            app.input.push(c);
                        }
                        KeyCode::Backspace => {
                            app.input.pop();
                        }
                        _ => {}
                    },
                    AppMode::Running => match key.code {
                        KeyCode::Up => app.select_prev(),
                        KeyCode::Down => app.select_next(),
                        KeyCode::Char('j') => app.scroll_down(),
                        KeyCode::Char('k') => app.scroll_up(),
                        _ => {}
                    },
                    AppMode::Done => match key.code {
                        KeyCode::Char('q') => return Ok(()),
                        KeyCode::Char('n') => {
                            // Fold results before starting new round, preserve kernel
                            app.fold_results_into_kernel();
                            let kernel = app.kernel.clone();
                            let api_key = app.api_key.clone();
                            app = App::new(api_key);
                            app.kernel = kernel;
                        }
                        KeyCode::Char('s') => {
                            app.mode = AppMode::Running;
                            app.synthesize_results();
                        }
                        KeyCode::Up => app.select_prev(),
                        KeyCode::Down => app.select_next(),
                        KeyCode::Char('j') => app.scroll_down(),
                        KeyCode::Char('k') => app.scroll_up(),
                        _ => {}
                    },
                }
            }
        }
    }
}
