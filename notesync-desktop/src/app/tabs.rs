use super::{mode_from, NoteSyncApp, RightView, ViewMode, START_TAB};
use crate::models::NoteItem;
use gpui_kit::*;
use std::time::Instant;

impl NoteSyncApp {
    pub(super) fn select_note(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.right_view = RightView::Editor;
        let prev = self
            .selected_id
            .clone()
            .and_then(|prev_id| self.notes.get(&prev_id).cloned());
        if let Some(prev) = prev {
            if prev.dirty {
                self.spawn_push(prev, cx);
            }
        }
        self.last_edit_at = None;
        if let Some(body) = self.notes.get(&id).map(|n| n.body.clone()) {
            if self.mode != ViewMode::Split {
                self.mode = if body.trim().is_empty() {
                    ViewMode::Edit
                } else {
                    mode_from(&self.cfg.start_mode)
                };
            }
            if !self.open_tabs.contains(&id) {
                self.open_tabs.push(id.clone());
            }
            self.selected_id = Some(id);
            self.set_editor_text(&body, window, cx);
        }
        cx.notify();
    }

    pub(super) fn new_note(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let prev = self
            .selected_id
            .clone()
            .and_then(|id| self.notes.get(&id).cloned());
        if let Some(prev) = prev {
            if prev.dirty {
                self.spawn_push(prev, cx);
            }
        }
        self.right_view = RightView::Editor;
        let id = uuid::Uuid::new_v4().to_string();
        let item = NoteItem {
            id: id.clone(),
            body: String::new(),
            base_rev: 0,
            dirty: true,
            has_conflict: false,
            edit_id: uuid::Uuid::new_v4().to_string(),
        };
        self.notes.insert(id.clone(), item);
        self.order.insert(0, id.clone());
        self.open_tabs.push(id.clone());
        self.selected_id = Some(id);
        if self.mode != ViewMode::Split {
            self.mode = ViewMode::Edit;
        }
        self.set_editor_text("", window, cx);
        self.last_edit_at = Some(Instant::now());
        self.status = "Новая заметка...".into();
        cx.notify();
    }

    pub(super) fn delete_note(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(note) = self.notes.remove(&id) else {
            return;
        };
        self.order.retain(|x| x != &id);
        self.open_tabs.retain(|x| x != &id);
        if self.selected_id.as_deref() == Some(id.as_str()) {
            self.selected_id = None;
            self.set_editor_text("", window, cx);
            if let Some(next) = self.open_tabs.last().cloned() {
                self.goto_tab_id(next, window, cx);
            }
        }
        cx.notify();
        self.spawn_delete_remote(id, note.base_rev, cx);
    }

    pub(super) fn close_tab(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pos) = self.open_tabs.iter().position(|t| t == &id) else {
            return;
        };
        self.open_tabs.remove(pos);
        let is_start = id == START_TAB;
        let was_active = if is_start {
            matches!(self.right_view, RightView::Start)
        } else {
            self.selected_id.as_deref() == Some(id.as_str())
                && matches!(self.right_view, RightView::Editor)
        };
        if !is_start {
            if let Some(note) = self.notes.get(&id).cloned() {
                if note.dirty {
                    self.spawn_push(note, cx);
                }
            }
            if self.selected_id.as_deref() == Some(id.as_str()) {
                self.selected_id = None;
            }
        }
        if was_active {
            let next = self
                .open_tabs
                .get(pos)
                .or_else(|| self.open_tabs.last())
                .cloned();
            match next {
                Some(next) => self.goto_tab_id(next, window, cx),
                None => {
                    self.right_view = RightView::Editor;
                    self.selected_id = None;
                    self.set_editor_text("", window, cx);
                }
            }
        }
        cx.notify();
    }

    pub(super) fn goto_tab_id(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        if id == START_TAB {
            self.new_start_tab(window, cx);
        } else {
            self.select_note(id, window, cx);
        }
    }

    pub(super) fn goto_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.open_tabs.get(index).cloned() {
            self.goto_tab_id(id, window, cx);
        }
    }

    pub(super) fn new_start_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let cur = self
            .selected_id
            .clone()
            .and_then(|id| self.notes.get(&id).cloned());
        if let Some(cur) = cur {
            if cur.dirty {
                self.spawn_push(cur, cx);
            }
        }
        if !self.open_tabs.iter().any(|t| t == START_TAB) {
            self.open_tabs.push(START_TAB.to_string());
        }
        self.right_view = RightView::Start;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    pub(super) fn open_note(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.right_view, RightView::Start) {
            if let Some(pos) = self.open_tabs.iter().position(|t| t == START_TAB) {
                self.open_tabs.remove(pos);
                if !self.open_tabs.contains(&id) {
                    self.open_tabs.insert(pos, id.clone());
                }
            }
        }
        self.select_note(id, window, cx);
    }

    pub(super) fn close_active_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let active = match self.right_view {
            RightView::Start => Some(START_TAB.to_string()),
            RightView::Editor => self.selected_id.clone(),
            _ => None,
        };
        if let Some(id) = active {
            self.close_tab(id, window, cx);
        }
    }

    pub(super) fn set_mode(&mut self, mode: ViewMode, window: &mut Window, cx: &mut Context<Self>) {
        self.mode = mode;
        if mode == ViewMode::Preview {
            window.focus(&self.focus, cx);
        }
        if mode != ViewMode::Edit {
            let body = self.editor_input.read(cx).value().to_string();
            self.sync_preview(&body, cx);
        }
        cx.notify();
    }

    pub(super) fn find_in_note(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode == ViewMode::Preview {
            self.psearch_open = true;
            self.psearch_input.update(cx, |s, cx| s.focus(window, cx));
            self.run_psearch(true, cx);
        } else {
            self.editor_input
                .update(cx, |s, cx| s.open_search(false, cx));
        }
        cx.notify();
    }
}
