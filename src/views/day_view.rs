use std::sync::Arc;

use crossterm::event::Event;
use ratatui::layout::Rect;
use ratatui::Frame;
use ratatui::widgets::Paragraph;
use ratatui::layout::Alignment;

use crate::app::message::AppMessage;
use crate::app::theme::Theme;
use crate::store::Store;
use super::View;

#[allow(dead_code)]
pub struct DayView {
    store: Arc<dyn Store>,
}

#[allow(dead_code)]
impl DayView {
    pub fn new(store: Arc<dyn Store>) -> Self {
        Self { store }
    }
}

impl View for DayView {
    fn handle_event(&mut self, _event: &Event) -> Option<AppMessage> {
        None
    }

    fn handle_message(&mut self, _msg: &AppMessage) {}

    fn draw(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let placeholder = Paragraph::new("Day View")
            .alignment(Alignment::Center)
            .style(theme.dim);
        frame.render_widget(placeholder, area);
    }
}
