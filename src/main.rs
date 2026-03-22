mod app;
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
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

use app::App;
use store::{FlatFileStore, Store};

fn main() -> Result<()> {
    // Parse CLI args
    let mut tag_filter: Option<String> = None;
    for arg in std::env::args().skip(1) {
        if let Some(tag) = arg.strip_prefix("--tag=") {
            tag_filter = Some(tag.to_string());
        }
    }

    let cfg = config::load()?;
    let store = Arc::new(FlatFileStore::new(&cfg.data_dir)?);
    store.rebuild_index()?;

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
