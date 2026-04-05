use std::sync::Arc;

use crossterm::event::Event;
use ratatui::layout::Rect;
use ratatui::Frame;

use crate::app::message::AppMessage;
use crate::app::theme::Theme;
use crate::store::Store;
use super::View;
use super::task_list::TaskListView;

/// Thin wrapper around the task list view.
pub struct TasksTab {
    pub list: TaskListView,
}

impl TasksTab {
    pub fn new(store: Arc<dyn Store>) -> Self {
        Self {
            list: TaskListView::new(store),
        }
    }
}

impl TasksTab {
    /// Return the ID of the currently focused task, if any.
    pub fn focused_entity_id(&self) -> Option<String> {
        self.list.focused_entity_id()
    }
}

impl View for TasksTab {
    fn handle_event(&mut self, event: &Event) -> Option<AppMessage> {
        self.list.handle_event(event)
    }

    fn handle_message(&mut self, msg: &AppMessage) {
        self.list.handle_message(msg);
    }

    fn draw(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        self.list.draw(frame, area, theme);
    }

    fn captures_input(&self) -> bool {
        self.list.captures_input()
    }
}
