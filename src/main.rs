mod app;
mod cli;
mod config;
mod domain;
mod parser;
mod store;
mod util;
mod views;

use std::io;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use clap::Parser;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

use app::App;
use config::Config;
use store::SqliteStore;

fn main() -> Result<()> {
    let cli = cli::Cli::parse();

    let cfg = config::load()?;
    let db_name = cli.vault.as_deref()
        .map(|v| format!("{}.db", v))
        .unwrap_or_else(|| "crumbs.db".to_string());
    let db_path = cfg.data_dir.join(&db_name);
    // Ensure data dir exists
    std::fs::create_dir_all(&cfg.data_dir)?;
    let store: Arc<dyn store::Store + Send + Sync> = Arc::new(SqliteStore::new(&db_path)?);

    match cli.command {
        None => run_tui(cfg, store, None),
        Some(cli::Command::Tui { tag }) => run_tui(cfg, store, tag),
        Some(cmd) => {
            if let Err(e) = cli::execute(cmd, store) {
                cli::output::print_error(&e.to_string());
                std::process::exit(1);
            }
            Ok(())
        }
    }
}

fn run_tui(
    cfg: Config,
    store: Arc<dyn store::Store + Send + Sync>,
    tag_filter: Option<String>,
) -> Result<()> {
    // Terminal setup
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(cfg, store, tag_filter);

    // Initial data load for all views.
    app.broadcast_reload();

    loop {
        terminal.draw(|frame| app.draw(frame))?;

        if event::poll(Duration::from_millis(50))? {
            let ev = event::read()?;
            if let Some(app::message::AppMessage::Quit) = app.handle_event(&ev, &mut terminal) {
                break;
            }
        }

        app.process_pending_messages();
    }

    // Teardown
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    Ok(())
}
