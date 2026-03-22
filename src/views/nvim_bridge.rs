//! Neovim RPC bridge.
//!
//! Spawns `nvim --embed` on a dedicated tokio thread and exposes a
//! synchronous API for the main Ratatui event loop.  The two sides
//! communicate through:
//!
//! - `std::sync::mpsc::channel` — main thread → nvim (commands)
//! - `Arc<Mutex<ScreenState>>` — nvim handler → main thread (screen)

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::{Color, Modifier, Style};
use rmpv::Value;

// ── Public types ─────────────────────────────────────────────────

/// Commands sent from the main thread to the nvim runtime thread.
pub enum NvimCommand {
    OpenFile(PathBuf),
    SendKey(String),
    Resize(u16, u16),
    Quit,
}

/// A single cell in the nvim virtual grid.
#[derive(Clone, Debug)]
pub struct NvimCell {
    /// UTF-8 grapheme cluster (usually one char, may be multi-byte).
    pub text: String,
    /// Highlight group ID — look up in `ScreenState::highlights`.
    pub hl_id: u64,
}

impl Default for NvimCell {
    fn default() -> Self {
        Self { text: " ".to_string(), hl_id: 0 }
    }
}

/// Shared screen state updated by the nvim redraw handler.
pub struct ScreenState {
    pub grid: Vec<Vec<NvimCell>>,
    /// (row, col) in grid coordinates.
    pub cursor: (u16, u16),
    pub highlights: HashMap<u64, Style>,
    /// Nvim mode string (e.g. "normal", "insert", "visual").
    pub mode: String,
    /// Set by the handler after `flush`; cleared by the caller.
    pub needs_redraw: bool,
    /// Set when a `BufWritePost` notification arrives; cleared by caller.
    pub buf_written: bool,
    /// Set when the nvim process exits.
    pub exited: bool,
    pub width: u16,
    pub height: u16,
}

impl ScreenState {
    fn new(width: u16, height: u16) -> Self {
        let w = width.max(1) as usize;
        let h = height.max(1) as usize;
        Self {
            grid: vec![vec![NvimCell::default(); w]; h],
            cursor: (0, 0),
            highlights: HashMap::new(),
            mode: "normal".to_string(),
            needs_redraw: false,
            buf_written: false,
            exited: false,
            width,
            height,
        }
    }

    fn resize_grid(&mut self, width: u16, height: u16) {
        let w = width.max(1) as usize;
        let h = height.max(1) as usize;
        self.width = width;
        self.height = height;
        self.grid = vec![vec![NvimCell::default(); w]; h];
    }

    fn clear_grid(&mut self) {
        for row in &mut self.grid {
            for cell in row.iter_mut() {
                *cell = NvimCell::default();
            }
        }
    }

    pub fn apply_grid_line(&mut self, row: usize, col_start: usize, cells: &[Value]) {
        if row >= self.grid.len() {
            return;
        }
        let grid_width = self.grid[row].len();
        let mut col = col_start;
        let mut last_hl_id: u64 = 0;

        for cell_val in cells {
            let Value::Array(cell_arr) = cell_val else { continue };

            let text = match cell_arr.first() {
                Some(Value::String(s)) => s.as_str().unwrap_or(" ").to_string(),
                _ => " ".to_string(),
            };
            let hl_id = cell_arr.get(1).and_then(as_u64).unwrap_or(last_hl_id);
            let repeat = cell_arr.get(2).and_then(as_u64).unwrap_or(1) as usize;
            last_hl_id = hl_id;

            for _ in 0..repeat {
                if col >= grid_width {
                    break;
                }
                self.grid[row][col] = NvimCell { text: text.clone(), hl_id };
                col += 1;
            }
        }
    }

    fn scroll_grid(&mut self, top: usize, bot: usize, left: usize, right: usize, rows: i64) {
        let height = self.grid.len();
        let width = self.grid.first().map(|r| r.len()).unwrap_or(0);
        let bot = bot.min(height);
        let right = right.min(width);

        if rows > 0 {
            // Scroll up: copy row src → dst where dst < src
            let rows = rows as usize;
            for dst in top..bot {
                let src = dst + rows;
                if src < bot {
                    for col in left..right {
                        self.grid[dst][col] = self.grid[src][col].clone();
                    }
                } else {
                    for col in left..right {
                        self.grid[dst][col] = NvimCell::default();
                    }
                }
            }
        } else if rows < 0 {
            // Scroll down: copy row src → dst where dst > src
            let rows = (-rows) as usize;
            for dst in (top..bot).rev() {
                if let Some(src) = dst.checked_sub(rows) {
                    if src >= top {
                        for col in left..right {
                            self.grid[dst][col] = self.grid[src][col].clone();
                        }
                        continue;
                    }
                }
                for col in left..right {
                    self.grid[dst][col] = NvimCell::default();
                }
            }
        }
    }
}

// ── NvimBridge (public handle) ────────────────────────────────────

/// Synchronous handle held by `NvimOverlay` on the main thread.
pub struct NvimBridge {
    cmd_tx: std::sync::mpsc::Sender<NvimCommand>,
    pub screen: Arc<Mutex<ScreenState>>,
    /// Kept alive to prevent the tokio thread from being killed prematurely.
    _thread: std::thread::JoinHandle<()>,
}

impl NvimBridge {
    /// Spawn a background tokio thread running `nvim --embed` and return the
    /// synchronous handle.  Fails if `nvim` is not found in `$PATH`.
    pub fn spawn(width: u16, height: u16) -> anyhow::Result<Self> {
        let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<NvimCommand>();
        let screen = Arc::new(Mutex::new(ScreenState::new(width, height)));
        let screen_clone = screen.clone();

        let thread = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("failed to build tokio runtime for nvim");
            rt.block_on(async move {
                if let Err(e) = run_nvim(width, height, cmd_rx, screen_clone).await {
                    eprintln!("nvim bridge error: {e}");
                }
            });
        });

        Ok(Self { cmd_tx, screen, _thread: thread })
    }

    pub fn open_file(&self, path: PathBuf) {
        let _ = self.cmd_tx.send(NvimCommand::OpenFile(path));
    }

    pub fn send_key(&self, key: &str) {
        let _ = self.cmd_tx.send(NvimCommand::SendKey(key.to_string()));
    }

    pub fn resize(&self, width: u16, height: u16) {
        let _ = self.cmd_tx.send(NvimCommand::Resize(width, height));
    }

    pub fn screen(&self) -> std::sync::MutexGuard<'_, ScreenState> {
        self.screen.lock().expect("nvim screen mutex poisoned")
    }
}

impl Drop for NvimBridge {
    fn drop(&mut self) {
        // Best-effort: tell the nvim thread to quit cleanly.
        let _ = self.cmd_tx.send(NvimCommand::Quit);
    }
}

// ── Neovim handler ───────────────────────────────────────────────

#[derive(Clone)]
struct NeovimHandler {
    screen: Arc<Mutex<ScreenState>>,
}

#[async_trait::async_trait]
impl nvim_rs::Handler for NeovimHandler {
    type Writer = nvim_rs::compat::tokio::Compat<tokio::process::ChildStdin>;

    async fn handle_notify(
        &self,
        name: String,
        args: Vec<Value>,
        _neovim: nvim_rs::Neovim<Self::Writer>,
    ) {
        match name.as_str() {
            "redraw" => self.process_redraw(args),
            "crumb_buf_written" => {
                if let Ok(mut s) = self.screen.lock() {
                    s.buf_written = true;
                }
            }
            _ => {}
        }
    }

    async fn handle_request(
        &self,
        _name: String,
        _args: Vec<Value>,
        _neovim: nvim_rs::Neovim<Self::Writer>,
    ) -> Result<Value, Value> {
        Err(Value::from("not supported"))
    }
}

impl NeovimHandler {
    fn process_redraw(&self, args: Vec<Value>) {
        let Ok(mut screen) = self.screen.lock() else { return };
        let mut flush_seen = false;

        for event in &args {
            let Value::Array(event_arr) = event else { continue };
            let name = match event_arr.first() {
                Some(Value::String(s)) => s.as_str().unwrap_or("").to_string(),
                _ => continue,
            };

            // Each remaining element is one instance of this event type.
            for instance in event_arr.iter().skip(1) {
                let Value::Array(inst) = instance else { continue };
                match name.as_str() {
                    "grid_resize" => {
                        // [grid_id, width, height]
                        if inst.len() >= 3 {
                            let w = as_u64(&inst[1]).unwrap_or(0) as u16;
                            let h = as_u64(&inst[2]).unwrap_or(0) as u16;
                            if w > 0 && h > 0 {
                                screen.resize_grid(w, h);
                            }
                        }
                    }
                    "grid_line" => {
                        // [grid_id, row, col_start, cells]
                        if inst.len() >= 4 {
                            let row = as_u64(&inst[1]).unwrap_or(0) as usize;
                            let col = as_u64(&inst[2]).unwrap_or(0) as usize;
                            if let Value::Array(cells) = &inst[3] {
                                screen.apply_grid_line(row, col, cells);
                            }
                        }
                    }
                    "grid_cursor_goto" => {
                        // [grid_id, row, col]
                        if inst.len() >= 3 {
                            let row = as_u64(&inst[1]).unwrap_or(0) as u16;
                            let col = as_u64(&inst[2]).unwrap_or(0) as u16;
                            screen.cursor = (row, col);
                        }
                    }
                    "grid_clear" => {
                        screen.clear_grid();
                    }
                    "grid_scroll" => {
                        // [grid_id, top, bot, left, right, rows, cols]
                        if inst.len() >= 7 {
                            let top = as_u64(&inst[1]).unwrap_or(0) as usize;
                            let bot = as_u64(&inst[2]).unwrap_or(0) as usize;
                            let left = as_u64(&inst[3]).unwrap_or(0) as usize;
                            let right = as_u64(&inst[4]).unwrap_or(0) as usize;
                            let rows = as_i64(&inst[5]).unwrap_or(0);
                            screen.scroll_grid(top, bot, left, right, rows);
                        }
                    }
                    "hl_attr_define" => {
                        // [id, rgb_attrs, cterm_attrs, info]
                        if inst.len() >= 2 {
                            let id = as_u64(&inst[0]).unwrap_or(0);
                            if let Value::Map(attrs) = &inst[1] {
                                let style = parse_hl_attrs(attrs);
                                screen.highlights.insert(id, style);
                            }
                        }
                    }
                    "default_colors_set" => {
                        // [fg, bg, sp, ...] — set default highlight (id=0)
                        if !inst.is_empty() {
                            let mut style = Style::default();
                            if let Some(c) = inst.first().and_then(parse_color) {
                                style = style.fg(c);
                            }
                            if let Some(c) = inst.get(1).and_then(parse_color) {
                                style = style.bg(c);
                            }
                            screen.highlights.insert(0, style);
                        }
                    }
                    "mode_change" => {
                        // [mode_name, mode_idx]
                        if let Some(Value::String(mode)) = inst.first() {
                            screen.mode = mode.as_str().unwrap_or("normal").to_string();
                        }
                    }
                    "flush" => {
                        flush_seen = true;
                    }
                    _ => {}
                }
            }
        }

        if flush_seen {
            screen.needs_redraw = true;
        }
    }
}

// ── Nvim runtime (runs on the tokio thread) ──────────────────────

async fn run_nvim(
    width: u16,
    height: u16,
    cmd_rx: std::sync::mpsc::Receiver<NvimCommand>,
    screen: Arc<Mutex<ScreenState>>,
) -> anyhow::Result<()> {
    use nvim_rs::{create::tokio as create, UiAttachOptions};
    use tokio::process::Command;

    let handler = NeovimHandler { screen: screen.clone() };

    let mut cmd = Command::new("nvim");
    cmd.arg("--embed");

    let (nvim, io_handler, mut child) = create::new_child_cmd(&mut cmd, handler).await?;

    // Drive the msgpack-RPC event loop in a background task.
    // Set exited flag when the loop ends (nvim process exited).
    let screen_exit = screen.clone();
    tokio::spawn(async move {
        let _ = io_handler.await;
        if let Ok(mut s) = screen_exit.lock() {
            s.exited = true;
        }
    });

    // Attach as a remote UI with linegrid and RGB colour support.
    let mut opts = UiAttachOptions::new();
    opts.set_rgb(true);
    opts.set_linegrid_external(true);
    nvim.ui_attach(width as i64, height as i64, &opts).await?;

    // Command dispatch loop — polls the sync channel every 5 ms.
    loop {
        match cmd_rx.try_recv() {
            Ok(cmd) => match cmd {
                NvimCommand::OpenFile(path) => {
                    let path_str = path.to_string_lossy();
                    // Use Lua so we can properly escape the path.
                    let _ = nvim
                        .exec_lua(
                            "vim.cmd('edit ' .. vim.fn.fnameescape(...))",
                            vec![Value::from(path_str.as_ref())],
                        )
                        .await;
                    // Register a BufWritePost autocommand so we know when
                    // the user saves.
                    let _ = nvim
                        .exec_lua(
                            r#"
vim.api.nvim_create_autocmd("BufWritePost", {
  buffer = 0,
  callback = function()
    vim.rpcnotify(0, "crumb_buf_written")
  end,
})
"#,
                            vec![],
                        )
                        .await;
                }
                NvimCommand::SendKey(key) => {
                    let _ = nvim.input(&key).await;
                }
                NvimCommand::Resize(w, h) => {
                    let _ = nvim.ui_try_resize(w as i64, h as i64).await;
                }
                NvimCommand::Quit => {
                    // Force-quit nvim; ignore errors (process may already be gone).
                    let _ = nvim.command("qa!").await;
                    break;
                }
            },
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }

    // Kill the child process if it's still running.
    let _ = child.kill().await;
    Ok(())
}

// ── Key conversion ───────────────────────────────────────────────

/// Convert a crossterm `KeyEvent` into a nvim input string.
pub fn key_to_nvim_input(key: &KeyEvent) -> String {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);

    match key.code {
        KeyCode::Char(c) => {
            if ctrl {
                format!("<C-{}>", c)
            } else if alt {
                format!("<M-{}>", c)
            } else if c == '<' {
                "<lt>".to_string()
            } else if c == '\\' {
                "<Bslash>".to_string()
            } else {
                c.to_string()
            }
        }
        KeyCode::Enter => "<CR>".to_string(),
        KeyCode::Esc => "<Esc>".to_string(),
        KeyCode::Backspace => "<BS>".to_string(),
        KeyCode::Tab => "<Tab>".to_string(),
        KeyCode::BackTab => "<S-Tab>".to_string(),
        KeyCode::Left => {
            if ctrl { "<C-Left>".to_string() } else { "<Left>".to_string() }
        }
        KeyCode::Right => {
            if ctrl { "<C-Right>".to_string() } else { "<Right>".to_string() }
        }
        KeyCode::Up => "<Up>".to_string(),
        KeyCode::Down => "<Down>".to_string(),
        KeyCode::Home => "<Home>".to_string(),
        KeyCode::End => "<End>".to_string(),
        KeyCode::PageUp => "<PageUp>".to_string(),
        KeyCode::PageDown => "<PageDown>".to_string(),
        KeyCode::Delete => "<Del>".to_string(),
        KeyCode::Insert => "<Insert>".to_string(),
        KeyCode::F(n) => format!("<F{}>", n),
        _ => String::new(),
    }
}

// ── Helpers ──────────────────────────────────────────────────────

fn as_u64(val: &Value) -> Option<u64> {
    match val {
        Value::Integer(i) => i.as_u64(),
        _ => None,
    }
}

fn as_i64(val: &Value) -> Option<i64> {
    match val {
        Value::Integer(i) => i.as_i64(),
        _ => None,
    }
}

fn parse_color(val: &Value) -> Option<Color> {
    let n = match val {
        Value::Integer(i) => {
            let v = i.as_i64()?;
            if v < 0 {
                return None; // -1 = default / inherit
            }
            v as u64
        }
        _ => return None,
    };
    let r = ((n >> 16) & 0xff) as u8;
    let g = ((n >> 8) & 0xff) as u8;
    let b = (n & 0xff) as u8;
    Some(Color::Rgb(r, g, b))
}

fn parse_hl_attrs(attrs: &[(Value, Value)]) -> Style {
    let mut style = Style::default();
    for (key, val) in attrs {
        let key_str = match key {
            Value::String(s) => s.as_str().unwrap_or("").to_string(),
            _ => continue,
        };
        match key_str.as_str() {
            "foreground" => {
                if let Some(c) = parse_color(val) {
                    style = style.fg(c);
                }
            }
            "background" => {
                if let Some(c) = parse_color(val) {
                    style = style.bg(c);
                }
            }
            "bold" => {
                if *val == Value::Boolean(true) {
                    style = style.add_modifier(Modifier::BOLD);
                }
            }
            "italic" => {
                if *val == Value::Boolean(true) {
                    style = style.add_modifier(Modifier::ITALIC);
                }
            }
            "underline" | "underlineline" => {
                if *val == Value::Boolean(true) {
                    style = style.add_modifier(Modifier::UNDERLINED);
                }
            }
            "reverse" => {
                if *val == Value::Boolean(true) {
                    style = style.add_modifier(Modifier::REVERSED);
                }
            }
            "strikethrough" => {
                if *val == Value::Boolean(true) {
                    style = style.add_modifier(Modifier::CROSSED_OUT);
                }
            }
            _ => {}
        }
    }
    style
}
