use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

use super::theme::Theme;

/// Full-screen help overlay showing keybindings as a concise reference.
pub struct HelpOverlay;

impl HelpOverlay {
    pub fn new() -> Self {
        Self
    }

    pub fn draw(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        frame.render_widget(Clear, area);

        let block = Block::default()
            .title(" Help  ?:close ")
            .title_style(theme.title)
            .borders(Borders::ALL)
            .border_style(theme.border);

        let inner = block.inner(area);
        frame.render_widget(block, area);

        let mut lines: Vec<Line<'_>> = Vec::new();

        add_section(&mut lines, "Navigation", &NAVIGATION, theme);
        lines.push(Line::from(""));
        add_section(&mut lines, "Actions", &ACTIONS, theme);
        lines.push(Line::from(""));
        add_section(&mut lines, "Overlays & Filters", &OVERLAYS, theme);
        lines.push(Line::from(""));
        add_section(&mut lines, "Sink (rapid capture)", &SINK, theme);

        let para = Paragraph::new(lines).wrap(Wrap { trim: false });
        frame.render_widget(para, inner);
    }
}

// ── Keybinding definitions ────────────────────────────────────────

type Binding = (&'static str, &'static str);

static NAVIGATION: &[Binding] = &[
    ("j / k",       "Move down / up"),
    ("h / l",       "Move left / right (columns, panes)"),
    ("g / G",       "Jump to first / last item"),
    ("Tab",         "Cycle focus between panes"),
    ("D T C N P",   "Switch tab"),
    ("[ / ]",       "Prev / next month (calendar)"),
    ("t",           "Jump to today (calendar)"),
];

static ACTIONS: &[Binding] = &[
    ("n",           "Create new item"),
    ("e",           "Edit focused field inline"),
    ("Enter",       "Open in editor / reveal private"),
    ("Space",       "Cycle status or priority"),
    ("a",           "Toggle archive"),
    ("d",           "Delete (y/n confirmation)"),
    ("p",           "Toggle pin"),
    ("S",           "Sort by focused column"),
    ("A",           "Toggle archived visibility"),
    ("R",           "Rebuild index"),
    ("q",           "Quit"),
];

static OVERLAYS: &[Binding] = &[
    ("s",           "Open sink (rapid capture)"),
    ("/",           "Search"),
    ("f",           "Filter (tasks)"),
    ("Ctrl+T",      "Set / clear tag filter"),
    ("Ctrl+K",      "Command palette"),
("Esc",         "Close overlay / cancel"),
];

static SINK: &[Binding] = &[
    ("todo: ...",    "Auto-create a task"),
    ("@... / #...",  "Mention person / tag topic"),
    ("[p]",          "Mark entry as private"),
];

// ── Helpers ───────────────────────────────────────────────────────

fn add_section<'a>(
    lines: &mut Vec<Line<'a>>,
    title: &'a str,
    bindings: &'a [Binding],
    theme: &Theme,
) {
    lines.push(Line::from(Span::styled(
        format!(" {} ", title),
        theme.title,
    )));
    lines.push(Line::from(Span::styled(
        " ────────────────────────────",
        theme.border,
    )));

    let key_width = bindings
        .iter()
        .map(|(k, _)| k.len())
        .max()
        .unwrap_or(10);

    for (key, desc) in bindings {
        let padded_key = format!(" {:width$}  ", key, width = key_width);
        lines.push(Line::from(vec![
            Span::styled(padded_key, theme.topic),
            Span::styled(*desc, theme.title.remove_modifier(ratatui::style::Modifier::BOLD)),
        ]));
    }
}
