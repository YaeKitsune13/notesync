mod palette;
mod sync;
mod tabs;
mod views;

use crate::models::*;
use crate::search::find_all;
use crate::theme;
use gpui_kit::component::{
    h_flex,
    input::{InputEvent, InputState, TextareaState},
    text::{RangeHighlight, RenderedText, TextViewState, TextViewStyle},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::time::{Duration, Instant};

pub(super) const START_TAB: &str = "::start";

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum ViewMode {
    Preview,
    Split,
    Edit,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum RightView {
    Editor,
    Start,
    Settings,
    Conflicts(String),
    Log,
}

actions!(
    notesync,
    [
        FindInNote,
        TogglePreview,
        Dismiss,
        NewNote,
        NewTab,
        CloseTab,
        QuickOpen,
        ToggleSidebar,
        QoNext,
        QoPrev,
        Tab1,
        Tab2,
        Tab3,
        Tab4,
        Tab5,
        Tab6,
        Tab7,
        Tab8,
        Tab9,
        Tab10,
        CommandPalette,
        ZoomIn,
        ZoomOut,
        ZoomReset
    ]
);

pub fn window_bg(cfg: &ClientConfig) -> WindowBackgroundAppearance {
    if !theme::has_transparency() {
        WindowBackgroundAppearance::Opaque
    } else if cfg.window_blur {
        WindowBackgroundAppearance::Blurred
    } else {
        WindowBackgroundAppearance::Transparent
    }
}

pub(super) fn mode_from(name: &str) -> ViewMode {
    match name {
        "split" => ViewMode::Split,
        "edit" => ViewMode::Edit,
        _ => ViewMode::Preview,
    }
}

pub(super) fn md_style(font_size: f32) -> TextViewStyle {
    let mut style = TextViewStyle::default()
        .paragraph_gap(rems(0.9))
        .heading_font_size(|level, base| {
            base * match level {
                1 => 1.8,
                2 => 1.5,
                3 => 1.25,
                4 => 1.1,
                _ => 1.0,
            }
        })
        .code_block(
            StyleRefinement::default()
                .p_3()
                .rounded_md()
                .bg(theme::crust()),
        )
        .inline_code(HighlightStyle {
            background_color: Some(theme::surface0()),
            ..Default::default()
        });
    style.heading_base_font_size = px(font_size);
    style
}

pub struct NoteSyncApp {
    pub(super) cfg: ClientConfig,
    pub(super) http: reqwest::Client,
    pub(super) window_handle: AnyWindowHandle,

    pub(super) notes: HashMap<String, NoteItem>,
    pub(super) order: Vec<String>,
    pub(super) selected_id: Option<String>,
    pub(super) pending_delete: Option<String>,
    pub(super) pushing: HashSet<String>,
    pub(super) mode: ViewMode,
    pub(super) last_search_query: String,
    pub(super) open_tabs: Vec<String>,
    pub(super) sidebar_visible: bool,

    pub(super) qo_open: bool,
    pub(super) qo_input: Entity<InputState>,
    pub(super) qo_ix: usize,
    pub(super) qo_commands: bool,

    pub(super) startup_done: bool,
    pub(super) custom_themes: Vec<String>,

    pub(super) preview_state: Entity<TextViewState>,
    pub(super) preview_body: String,
    pub(super) psearch_open: bool,
    pub(super) psearch_input: Entity<InputState>,
    pub(super) psearch_matches: Vec<Range<usize>>,
    pub(super) psearch_ix: usize,
    pub(super) psearch_text: Option<RenderedText>,
    pub(super) focus: FocusHandle,

    pub(super) search_input: Entity<InputState>,
    pub(super) editor_input: Entity<TextareaState>,
    pub(super) url_input: Entity<InputState>,
    pub(super) token_input: Entity<InputState>,

    pub(super) ignore_editor_change: bool,
    pub(super) right_view: RightView,
    pub(super) health_status: String,

    pub(super) versions: Vec<NoteVersionDto>,
    pub(super) versions_loading: bool,
    pub(super) log_entries: Vec<LogEntryDto>,
    pub(super) log_loading: bool,

    pub(super) last_seq: i64,
    pub(super) status: String,
    pub(super) last_edit_at: Option<Instant>,

    _subscriptions: Vec<Subscription>,
}

impl NoteSyncApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let cfg = load_config();
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .expect("http client");

        let need_setup = cfg.token.trim().is_empty() || cfg.server_url.trim().is_empty();
        let status = if need_setup {
            "Укажи адрес сервера и токен в Настройках".to_string()
        } else {
            "Подключение по API...".to_string()
        };

        let search_input = cx.new(|cx| InputState::new(window, cx).placeholder("Поиск заметок..."));
        let focus = cx.focus_handle();
        window.set_rem_size(px(16.0 * cfg.ui_scale));
        let start_mode = cfg.start_mode.clone();

        let preview_state = cx.new(|cx| {
            TextViewState::markdown("", cx)
                .selectable(true)
                .scrollable(true)
        });
        let qo_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Введите для поиска..."));
        let psearch_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Найти в тексте..."));
        let editor_input = cx.new(|cx| {
            TextareaState::new(window, cx).searchable(true).placeholder(
                "Первая строка — заголовок\nТекст в Markdown, UTF-8 (汉(hàn)字(zì) / pinyin)...",
            )
        });
        let url_input = cx.new(|cx| {
            let mut st = InputState::new(window, cx).placeholder("http://127.0.0.1:8787");
            st.set_value(&cfg.server_url, window, cx);
            st
        });
        let token_input = cx.new(|cx| {
            let mut st =
                InputState::new(window, cx).placeholder("Вставь токен из notes.db.token...");
            st.set_value(&cfg.token, window, cx);
            st
        });

        let subscriptions = vec![
            cx.subscribe_in(
                &qo_input,
                window,
                |this, _, ev: &InputEvent, window, cx| match ev {
                    InputEvent::Change => {
                        this.qo_ix = 0;
                        cx.notify();
                    }
                    InputEvent::PressEnter { .. } => this.qo_accept(window, cx),
                    _ => {}
                },
            ),
            cx.subscribe_in(
                &psearch_input,
                window,
                |this, _, ev: &InputEvent, _, cx| match ev {
                    InputEvent::Change => {
                        this.psearch_ix = 0;
                        this.run_psearch(true, cx);
                    }
                    InputEvent::PressEnter { .. } => this.psearch_step(1, cx),
                    _ => {}
                },
            ),
            cx.observe(&preview_state, |this, _, cx| {
                if this.psearch_open {
                    this.run_psearch(false, cx);
                }
            }),
            cx.observe(&editor_input, |this, editor, cx| {
                let (open, query, found) = {
                    let s = editor.read(cx).search_session();
                    (s.open, s.query.clone(), !s.matcher.is_empty())
                };
                if !open {
                    this.last_search_query.clear();
                    return;
                }
                if query != this.last_search_query {
                    this.last_search_query = query;
                    if found {
                        editor.update(cx, |state, cx| {
                            let _ = state.previous_search_match(cx);
                            let _ = state.next_search_match(cx);
                        });
                    }
                }
            }),
            cx.subscribe_in(&search_input, window, |_, _, ev: &InputEvent, _, cx| {
                if matches!(ev, InputEvent::Change) {
                    cx.notify();
                }
            }),
            cx.subscribe_in(
                &editor_input,
                window,
                |this, editor_input, ev: &InputEvent, _, cx| {
                    if matches!(ev, InputEvent::Change) {
                        if this.ignore_editor_change {
                            return;
                        }
                        let new_body = editor_input.read(cx).value().to_string();
                        this.sync_preview(&new_body, cx);
                        if let Some(id) = &this.selected_id {
                            if let Some(note) = this.notes.get_mut(id) {
                                if note.body != new_body {
                                    note.body = new_body;
                                    note.dirty = true;
                                    note.edit_id = uuid::Uuid::new_v4().to_string();
                                    this.last_edit_at = Some(Instant::now());
                                    this.status = "Изменено...".into();
                                    cx.notify();
                                }
                            }
                        }
                    }
                },
            ),
        ];

        let app = Self {
            cfg,
            http,
            window_handle: window.window_handle(),
            notes: HashMap::new(),
            order: Vec::new(),
            selected_id: None,
            search_input,
            editor_input,
            url_input,
            token_input,
            ignore_editor_change: false,
            pending_delete: None,
            pushing: HashSet::new(),
            mode: mode_from(&start_mode),
            last_search_query: String::new(),
            open_tabs: Vec::new(),
            sidebar_visible: true,
            qo_open: false,
            qo_input,
            qo_ix: 0,
            qo_commands: false,
            startup_done: false,
            custom_themes: theme::list_custom(),
            preview_state,
            preview_body: String::new(),
            psearch_open: false,
            psearch_input,
            psearch_matches: Vec::new(),
            psearch_ix: 0,
            psearch_text: None,
            focus,
            right_view: if need_setup {
                RightView::Settings
            } else {
                RightView::Editor
            },
            health_status: String::new(),
            versions: Vec::new(),
            versions_loading: false,
            log_entries: Vec::new(),
            log_loading: false,
            last_seq: 0,
            status,
            last_edit_at: None,
            _subscriptions: subscriptions,
        };

        window.focus(&app.focus, cx);
        app.spawn_pull(cx);
        app.start_ticker(cx);
        app
    }

    pub(super) fn sync_preview(&mut self, body: &str, cx: &mut Context<Self>) {
        if self.mode != ViewMode::Edit && self.preview_body != body {
            self.preview_body = body.to_string();
            self.preview_state
                .update(cx, |state, cx| state.set_text(body, cx));
        }
    }

    pub(super) fn set_editor_text(
        &mut self,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ignore_editor_change = true;
        self.editor_input.update(cx, |state, cx| {
            state.set_value(text, window, cx);
        });
        self.ignore_editor_change = false;
        self.sync_preview(text, cx);
    }

    pub(super) fn run_psearch(&mut self, force: bool, cx: &mut Context<Self>) {
        let text = self.preview_state.read(cx).rendered_text();
        if !force && self.psearch_text.as_ref() == Some(&text) {
            return;
        }
        let query = self.psearch_input.read(cx).value().to_string();
        self.psearch_matches = find_all(text.as_str(), &query);
        if self.psearch_ix >= self.psearch_matches.len() {
            self.psearch_ix = 0;
        }
        self.psearch_text = Some(text);
        self.apply_psearch(true, cx);
    }

    pub(super) fn apply_psearch(&mut self, reveal: bool, cx: &mut Context<Self>) {
        let current_ix = self.psearch_ix;
        let highlights: Vec<RangeHighlight> = self
            .psearch_matches
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let color = if i == current_ix {
                    theme::peach().opacity(0.65)
                } else {
                    theme::yellow().opacity(0.35)
                };
                RangeHighlight::new(r.clone(), color)
            })
            .collect();
        let current = self.psearch_matches.get(current_ix).cloned();
        self.preview_state.update(cx, |state, cx| {
            let _ = state.set_range_highlights(highlights, cx);
            if reveal {
                if let Some(range) = current {
                    let _ = state.reveal_range(range, cx);
                }
            }
        });
        cx.notify();
    }

    pub(super) fn psearch_step(&mut self, dir: i32, cx: &mut Context<Self>) {
        let n = self.psearch_matches.len();
        if n == 0 {
            return;
        }
        self.psearch_ix = if dir >= 0 {
            (self.psearch_ix + 1) % n
        } else {
            (self.psearch_ix + n - 1) % n
        };
        self.apply_psearch(true, cx);
    }

    pub(super) fn close_psearch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.psearch_open = false;
        self.psearch_matches.clear();
        self.psearch_text = None;
        self.preview_state
            .update(cx, |state, cx| state.clear_range_highlights(cx));
        window.focus(&self.focus, cx);
        cx.notify();
    }
}

impl Render for NoteSyncApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let right_pane = match &self.right_view {
            RightView::Settings => self.render_settings(cx),
            RightView::Conflicts(note_id) => self.render_conflicts(note_id.clone(), cx),
            RightView::Log => self.render_log(cx),
            RightView::Start => self.render_start(cx),
            RightView::Editor => self.render_editor(cx),
        };

        v_flex()
            .relative()
            .size_full()
            .key_context("NoteSync")
            .when(!self.cfg.font_family.is_empty(), |el| {
                el.font_family(self.cfg.font_family.clone())
            })
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &TogglePreview, window, cx| {
                let next = if this.mode == ViewMode::Edit {
                    ViewMode::Split
                } else {
                    ViewMode::Edit
                };
                this.set_mode(next, window, cx);
            }))
            .on_action(cx.listener(|this, _: &FindInNote, window, cx| {
                this.find_in_note(window, cx);
            }))
            .on_action(cx.listener(|this, _: &Dismiss, window, cx| {
                if this.qo_open {
                    this.close_quick(window, cx);
                } else if this.psearch_open {
                    this.close_psearch(window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &NewNote, window, cx| this.new_note(window, cx)))
            .on_action(cx.listener(|this, _: &NewTab, window, cx| this.new_start_tab(window, cx)))
            .on_action(
                cx.listener(|this, _: &CloseTab, window, cx| this.close_active_tab(window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &QuickOpen, window, cx| this.open_quick(false, window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &CommandPalette, window, cx| {
                    this.open_quick(true, window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &ZoomIn, _, cx| {
                this.set_font_size(this.cfg.font_size + 2.0, cx);
            }))
            .on_action(cx.listener(|this, _: &ZoomOut, _, cx| {
                this.set_font_size(this.cfg.font_size - 2.0, cx);
            }))
            .on_action(cx.listener(|this, _: &ZoomReset, _, cx| {
                this.set_font_size(15.0, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| {
                this.sidebar_visible = !this.sidebar_visible;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &QoNext, _, cx| this.qo_step(1, cx)))
            .on_action(cx.listener(|this, _: &QoPrev, _, cx| this.qo_step(-1, cx)))
            .on_action(cx.listener(|this, _: &Tab1, window, cx| this.goto_tab(0, window, cx)))
            .on_action(cx.listener(|this, _: &Tab2, window, cx| this.goto_tab(1, window, cx)))
            .on_action(cx.listener(|this, _: &Tab3, window, cx| this.goto_tab(2, window, cx)))
            .on_action(cx.listener(|this, _: &Tab4, window, cx| this.goto_tab(3, window, cx)))
            .on_action(cx.listener(|this, _: &Tab5, window, cx| this.goto_tab(4, window, cx)))
            .on_action(cx.listener(|this, _: &Tab6, window, cx| this.goto_tab(5, window, cx)))
            .on_action(cx.listener(|this, _: &Tab7, window, cx| this.goto_tab(6, window, cx)))
            .on_action(cx.listener(|this, _: &Tab8, window, cx| this.goto_tab(7, window, cx)))
            .on_action(cx.listener(|this, _: &Tab9, window, cx| this.goto_tab(8, window, cx)))
            .on_action(cx.listener(|this, _: &Tab10, window, cx| this.goto_tab(9, window, cx)))
            .bg(theme::base())
            .text_color(theme::text())
            .child(
                h_flex()
                    .flex_1()
                    .w_full()
                    .overflow_hidden()
                    .when(self.sidebar_visible, |el| el.child(self.render_sidebar(cx)))
                    .child(right_pane),
            )
            .child(self.render_status_bar())
            .children(self.render_confirm_overlay(cx))
            .children(self.render_quick_overlay(cx))
    }
}
