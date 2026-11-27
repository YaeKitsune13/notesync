mod models;
mod theme;

use gpui_kit::assets::Assets;
use gpui_kit::component::{
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
    menu::{ContextMenuExt as _, PopupMenuItem},
    text::{RangeHighlight, RenderedText, TextView, TextViewState, TextViewStyle},
    v_flex, Icon, IconName, Sizable,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use models::*;
use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

fn tokio_rt() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime")
    })
}

// Все вхождения query в text без учёта регистра. Возвращает байтовые диапазоны text.
fn find_all(text: &str, query: &str) -> Vec<std::ops::Range<usize>> {
    let needle: Vec<char> = query
        .chars()
        .map(|c| c.to_lowercase().next().unwrap_or(c))
        .collect();
    if needle.is_empty() {
        return Vec::new();
    }
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i + needle.len() <= chars.len() {
        let hit = (0..needle.len()).all(|k| {
            chars[i + k]
                .1
                .to_lowercase()
                .next()
                .unwrap_or(chars[i + k].1)
                == needle[k]
        });
        if hit {
            let start = chars[i].0;
            let end = chars
                .get(i + needle.len())
                .map(|c| c.0)
                .unwrap_or(text.len());
            out.push(start..end);
            i += needle.len();
        } else {
            i += 1;
        }
    }
    out
}

fn mode_from(name: &str) -> ViewMode {
    match name {
        "split" => ViewMode::Split,
        "edit" => ViewMode::Edit,
        _ => ViewMode::Preview,
    }
}

// Оформление Markdown в режиме чтения.
fn md_style(font_size: f32) -> TextViewStyle {
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

// Команды окна Ctrl+Shift+P.
#[derive(Clone, Debug, PartialEq)]
enum Cmd {
    NewNote,
    NewTab,
    CloseTab,
    OpenNote,
    ToggleSidebar,
    Mode(ViewMode),
    Find,
    Sync,
    Settings,
    Log,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    ReloadConfig,
    ImportZed,
    NewTheme,
    ToggleBlur,
    Theme(String),
}

// Строка в окне быстрого выбора: заметка или команда.
#[derive(Clone)]
enum QoItem {
    Note(String),
    Cmd(Cmd),
}

// Фон окна: прозрачный, если нужна прозрачность; размытый, если включено размытие.
fn window_bg(cfg: &ClientConfig) -> WindowBackgroundAppearance {
    if !theme::has_transparency() {
        WindowBackgroundAppearance::Opaque
    } else if cfg.window_blur {
        WindowBackgroundAppearance::Blurred
    } else {
        WindowBackgroundAppearance::Transparent
    }
}

// Служебный id вкладки «Новая вкладка» (id заметок — uuid, с этим не пересекается).
const START_TAB: &str = "::start";

// Режим заметки: чтение, редактор и превью рядом (как в Zed), только редактор.
#[derive(Clone, Copy, Debug, PartialEq)]
enum ViewMode {
    Preview,
    Split,
    Edit,
}

// Горячие клавиши приложения: Ctrl+F — поиск по заметке, Ctrl+Shift+V — превью рядом.
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

#[derive(Clone, Debug, PartialEq)]
enum RightView {
    Editor,
    // стартовая вкладка (Ctrl+T): приветствие и выбор заметки
    Start,
    Settings,
    Conflicts(String),
    Log,
}

struct NoteSyncApp {
    cfg: ClientConfig,
    http: reqwest::Client,
    window_handle: AnyWindowHandle,

    notes: HashMap<String, NoteItem>,
    order: Vec<String>,
    selected_id: Option<String>,
    pending_delete: Option<String>,
    // заметки, которые прямо сейчас отправляются на сервер (по одной отправке на заметку)
    pushing: std::collections::HashSet<String>,
    mode: ViewMode,
    // последний запрос поиска, на который мы уже прокрутили редактор
    last_search_query: String,
    // открытые вкладки (id заметок по порядку открытия)
    open_tabs: Vec<String>,
    sidebar_visible: bool,
    // быстрый выбор заметки (Ctrl+P)
    qo_open: bool,
    qo_input: Entity<InputState>,
    qo_ix: usize,
    qo_commands: bool,
    // первая синхронизация уже прошла (первую заметку открываем только один раз при запуске)
    startup_done: bool,
    // свои темы (файлы в ~/.config/notesync/themes)
    custom_themes: Vec<String>,
    // превью Markdown и поиск по нему (Ctrl+F в режиме чтения)
    preview_state: Entity<TextViewState>,
    preview_body: String,
    psearch_open: bool,
    psearch_input: Entity<InputState>,
    psearch_matches: Vec<std::ops::Range<usize>>,
    psearch_ix: usize,
    psearch_text: Option<RenderedText>,
    focus: FocusHandle,

    search_input: Entity<InputState>,
    editor_input: Entity<TextareaState>,
    url_input: Entity<InputState>,
    token_input: Entity<InputState>,

    ignore_editor_change: bool,
    right_view: RightView,
    health_status: String,

    versions: Vec<NoteVersionDto>,
    versions_loading: bool,
    log_entries: Vec<LogEntryDto>,
    log_loading: bool,

    last_seq: i64,
    status: String,
    last_edit_at: Option<Instant>,

    _subscriptions: Vec<Subscription>,
}

impl NoteSyncApp {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
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
            // Ctrl+P: ввод фильтрует список, Enter открывает выбранную заметку.
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
            // Поиск в режиме чтения: запрос, Enter — следующее совпадение.
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
            // Текст превью перепарсился (другая заметка или правка): ищем заново.
            cx.observe(&preview_state, |this, _, cx| {
                if this.psearch_open {
                    this.run_psearch(false, cx);
                }
            }),
            // Ctrl+F: панель поиска только подсвечивает совпадения. Прокручиваем к первому
            // совпадению сразу при вводе запроса (previous + next оставляют текущее на месте).
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
            cx.subscribe_in(&editor_input, window, {
                move |this, editor_input, ev: &InputEvent, _, cx| {
                    if matches!(ev, InputEvent::Change) {
                        if this.ignore_editor_change {
                            return;
                        }
                        let new_body = editor_input.read(cx).value().to_string();
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
                }
            }),
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
            pushing: std::collections::HashSet::new(),
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

    fn base_url(&self) -> String {
        let raw = self.cfg.server_url.trim().trim_end_matches('/');
        if raw.starts_with("http://") || raw.starts_with("https://") {
            raw.to_string()
        } else {
            format!("http://{raw}")
        }
    }

    fn set_editor_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.ignore_editor_change = true;
        self.editor_input.update(cx, |state, cx| {
            state.set_value(text, window, cx);
        });
        self.ignore_editor_change = false;
    }

    fn start_ticker(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let mut n: u64 = 0;
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(250))
                    .await;
                n += 1;
                let ok = this.update(cx, |app, cx| {
                    // Запись потоковая: отправляем почти сразу после паузы в наборе.
                    let debounce = Duration::from_millis(app.cfg.save_delay_ms.max(100));
                    if let Some(t) = app.last_edit_at {
                        if t.elapsed() >= debounce {
                            app.last_edit_at = None;
                            let current = app
                                .selected_id
                                .clone()
                                .and_then(|id| app.notes.get(&id).cloned());
                            if let Some(note) = current {
                                if note.dirty {
                                    app.status = "Сохранение...".into();
                                    app.spawn_push(note, cx);
                                }
                            }
                        }
                    }
                    // Новые заметки с сервера забираем раз в секунду.
                    let every = (app.cfg.sync_interval_ms / 250).max(1);
                    if n % every == 0 {
                        app.spawn_pull(cx);
                    }
                });
                if ok.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn spawn_pull(&self, cx: &mut Context<Self>) {
        if self.cfg.token.trim().is_empty() || self.cfg.server_url.trim().is_empty() {
            return;
        }
        let url = format!("{}/notes?since={}", self.base_url(), self.last_seq);
        let auth = format!("Bearer {}", self.cfg.token.trim());
        let client = self.http.clone();

        cx.spawn(async move |this, cx| {
            let res: Result<ListResponse, String> = tokio_rt()
                .spawn(async move {
                    let resp = client
                        .get(&url)
                        .header("Authorization", auth)
                        .send()
                        .await
                        .map_err(|e| format!("Нет связи: {e}"))?;
                    if resp.status().as_u16() == 401 {
                        return Err("Неверный токен (401)".into());
                    }
                    if !resp.status().is_success() {
                        return Err(format!("Ошибка сервера: {}", resp.status()));
                    }
                    resp.json::<ListResponse>()
                        .await
                        .map_err(|e| format!("Ошибка JSON: {e}"))
                })
                .await
                .unwrap_or_else(|e| Err(format!("Task error: {e}")));

            let _ = this.update(cx, |app, cx| {
                app.on_pulled(res, cx);
            });
        })
        .detach();
    }

    fn on_pulled(&mut self, res: Result<ListResponse, String>, cx: &mut Context<Self>) {
        match res {
            Ok(data) => {
                if data.seq > self.last_seq {
                    self.last_seq = data.seq;
                }
                let mut to_push: Vec<NoteItem> = Vec::new();
                let mut need_sync_editor = false;

                for sn in data.notes {
                    if sn.deleted {
                        let is_dirty = self.notes.get(&sn.id).map(|n| n.dirty).unwrap_or(false);
                        if !is_dirty {
                            self.notes.remove(&sn.id);
                            self.order.retain(|x| x != &sn.id);
                            self.open_tabs.retain(|x| x != &sn.id);
                            if self.selected_id.as_deref() == Some(sn.id.as_str()) {
                                self.selected_id = None;
                                need_sync_editor = true;
                            }
                        }
                        continue;
                    }

                    match self.notes.get_mut(&sn.id) {
                        None => {
                            self.order.insert(0, sn.id.clone());
                            self.notes.insert(
                                sn.id.clone(),
                                NoteItem {
                                    id: sn.id,
                                    body: sn.body,
                                    base_rev: sn.rev,
                                    dirty: false,
                                    has_conflict: sn.conflicts > 0,
                                    edit_id: String::new(),
                                },
                            );
                        }
                        Some(local) => {
                            // Ответ pull устарел (мы уже отправили более новую ревизию) — пропускаем.
                            if sn.rev < local.base_rev {
                                continue;
                            }
                            if !local.dirty {
                                let text_changed = local.body != sn.body;
                                local.body = sn.body;
                                local.base_rev = sn.rev;
                                local.has_conflict = sn.conflicts > 0;
                                if text_changed
                                    && self.selected_id.as_deref() == Some(local.id.as_str())
                                {
                                    need_sync_editor = true;
                                }
                            } else if sn.rev > local.base_rev && !self.pushing.contains(&sn.id) {
                                // Ревизия на сервере новее нашей и это не наша отправка в пути:
                                // правили с другого устройства.
                                // Запись потоковая: наш текст просто становится основным.
                                to_push.push(local.clone());
                            }
                        }
                    }
                }

                for item in to_push {
                    self.spawn_push(item, cx);
                }

                // Первую заметку открываем один раз при запуске (и сразу во вкладке).
                // Если закрыть все вкладки, сама ничего не откроется.
                if !self.startup_done {
                    self.startup_done = true;
                    if self.selected_id.is_none() {
                        if let Some(first_id) = self.order.first().cloned() {
                            if !self.open_tabs.contains(&first_id) {
                                self.open_tabs.push(first_id.clone());
                            }
                            self.selected_id = Some(first_id);
                            need_sync_editor = true;
                        }
                    }
                }

                if need_sync_editor {
                    let body = self
                        .selected_id
                        .as_ref()
                        .and_then(|id| self.notes.get(id))
                        .map(|n| n.body.clone())
                        .unwrap_or_default();
                    let editor = self.editor_input.clone();
                    self.ignore_editor_change = true;
                    let _ = cx.update_window(self.window_handle, |_, window, cx| {
                        editor.update(cx, |st, cx| st.set_value(&body, window, cx));
                    });
                    self.ignore_editor_change = false;
                }

                let has_conflicts = self.notes.values().any(|n| n.has_conflict);
                let has_dirty = self.notes.values().any(|n| n.dirty);
                if has_conflicts {
                    self.status = "Есть конфликты!".into();
                } else if !has_dirty {
                    self.status = "Синхронизировано".into();
                }
            }
            Err(err) => {
                self.status = err;
            }
        }
        cx.notify();
    }

    fn spawn_push(&mut self, item: NoteItem, cx: &mut Context<Self>) {
        if self.cfg.token.trim().is_empty() || self.cfg.server_url.trim().is_empty() {
            return;
        }
        // Уже отправляется: после ответа заметка уйдёт снова, если останется dirty.
        if !self.pushing.insert(item.id.clone()) {
            return;
        }
        let url = format!("{}/notes/{}/stream", self.base_url(), item.id);
        let auth = format!("Bearer {}", self.cfg.token.trim());
        let client = self.http.clone();
        let id = item.id.clone();
        let edit_id = item.edit_id.clone();
        let payload = PutRequest {
            body: item.body,
            base_rev: item.base_rev,
            device_id: self.cfg.device_id.clone(),
            edit_id: edit_id.clone(),
        };

        cx.spawn(async move |this, cx| {
            let res: Result<MutateResponse, String> = tokio_rt()
                .spawn(async move {
                    let resp = client
                        .put(&url)
                        .header("Authorization", auth)
                        .json(&payload)
                        .send()
                        .await
                        .map_err(|e| format!("Нет связи: {e}"))?;
                    if !resp.status().is_success() {
                        return Err(format!("Ошибка сервера: {}", resp.status()));
                    }
                    resp.json::<MutateResponse>()
                        .await
                        .map_err(|e| e.to_string())
                })
                .await
                .unwrap_or_else(|e| Err(format!("Task error: {e}")));

            let _ = this.update(cx, |app, cx| {
                app.pushing.remove(&id);
                match res {
                    Ok(mut_res) => {
                        let mut resend: Option<NoteItem> = None;
                        if let Some(note) = app.notes.get_mut(&id) {
                            let same_edit = note.edit_id == edit_id;
                            if mut_res.status == "conflict" {
                                if same_edit {
                                    note.dirty = false;
                                    note.has_conflict = true;
                                    let vid = mut_res.conflict_version_id.unwrap_or(0);
                                    app.status = format!("Конфликт! Сохранено как версия #{vid}");
                                }
                            } else {
                                // Сервер принял правку: ревизия обновляется ВСЕГДА, даже если
                                // пока шла отправка, пользователь успел напечатать ещё.
                                note.base_rev = mut_res.rev;
                                if same_edit {
                                    note.dirty = false;
                                    app.status = "Синхронизировано".into();
                                } else {
                                    resend = Some(note.clone());
                                }
                            }
                        }
                        if let Some(item) = resend {
                            app.spawn_push(item, cx);
                        }
                        app.spawn_pull(cx);
                    }
                    Err(err) => {
                        app.status = err;
                        // Нет связи: повторим отправку после следующей паузы.
                        app.last_edit_at = Some(Instant::now());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn select_note(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
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

    fn new_note(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    // ---------- поиск по тексту превью ----------
    fn run_psearch(&mut self, force: bool, cx: &mut Context<Self>) {
        let text = self.preview_state.read(cx).rendered_text();
        // Тот же разбор текста: подсветка уже стоит (иначе подсветка запускала бы поиск снова).
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

    fn apply_psearch(&mut self, reveal: bool, cx: &mut Context<Self>) {
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

    fn psearch_step(&mut self, dir: i32, cx: &mut Context<Self>) {
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

    fn close_psearch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.psearch_open = false;
        self.psearch_matches.clear();
        self.psearch_text = None;
        self.preview_state
            .update(cx, |state, cx| state.clear_range_highlights(cx));
        window.focus(&self.focus, cx);
        cx.notify();
    }

    // ---------- вкладки ----------
    fn close_tab(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
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
            // Несохранённый текст закрываемой заметки уходит на сервер.
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

    // Переход на вкладку по id (заметка или стартовая).
    fn goto_tab_id(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        if id == START_TAB {
            self.new_start_tab(window, cx);
        } else {
            self.select_note(id, window, cx);
        }
    }

    // Alt+1..Alt+0: вкладка по номеру.
    fn goto_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.open_tabs.get(index).cloned() {
            self.goto_tab_id(id, window, cx);
        }
    }

    // Ctrl+T: вкладка «Привет» с выбором заметки (одна, повторное нажатие просто к ней возвращает).
    fn new_start_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    // Открыть заметку: если открыта стартовая вкладка, заметка занимает её место.
    fn open_note(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
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

    // ---------- Ctrl+P (заметки) и Ctrl+Shift+P (команды) ----------
    fn cmd_info(&self, cmd: &Cmd) -> (String, &'static str) {
        match cmd {
            Cmd::NewNote => ("Новая заметка".to_string(), "Ctrl+N"),
            Cmd::NewTab => ("Новая вкладка".to_string(), "Ctrl+T"),
            Cmd::CloseTab => ("Закрыть вкладку".to_string(), "Ctrl+W"),
            Cmd::OpenNote => ("Открыть заметку…".to_string(), "Ctrl+P"),
            Cmd::ToggleSidebar => ("Показать / скрыть список заметок".to_string(), "Ctrl+B"),
            Cmd::Mode(ViewMode::Preview) => ("Режим: читать".to_string(), ""),
            Cmd::Mode(ViewMode::Split) => {
                ("Режим: редактор и превью рядом".to_string(), "Ctrl+Shift+V")
            }
            Cmd::Mode(ViewMode::Edit) => ("Режим: редактор".to_string(), ""),
            Cmd::Find => ("Найти в заметке".to_string(), "Ctrl+F"),
            Cmd::Sync => ("Синхронизировать сейчас".to_string(), ""),
            Cmd::Settings => ("Открыть настройки".to_string(), ""),
            Cmd::Log => ("Открыть журнал".to_string(), ""),
            Cmd::ZoomIn => ("Масштаб: увеличить".to_string(), "Ctrl+="),
            Cmd::ZoomOut => ("Масштаб: уменьшить".to_string(), "Ctrl+-"),
            Cmd::ZoomReset => ("Масштаб: сбросить".to_string(), "Ctrl+0"),
            Cmd::ReloadConfig => ("Перечитать настройки (JSON)".to_string(), ""),
            Cmd::ImportZed => ("Импортировать темы из Zed".to_string(), ""),
            Cmd::NewTheme => ("Создать свою тему".to_string(), ""),
            Cmd::ToggleBlur => ("Размытие окна: вкл / выкл".to_string(), ""),
            Cmd::Theme(name) => (format!("Тема: {name}"), ""),
        }
    }

    fn all_cmds(&self) -> Vec<Cmd> {
        let mut cmds = vec![
            Cmd::NewNote,
            Cmd::NewTab,
            Cmd::CloseTab,
            Cmd::OpenNote,
            Cmd::ToggleSidebar,
            Cmd::Mode(ViewMode::Preview),
            Cmd::Mode(ViewMode::Split),
            Cmd::Mode(ViewMode::Edit),
            Cmd::Find,
            Cmd::Sync,
            Cmd::Settings,
            Cmd::Log,
            Cmd::ZoomIn,
            Cmd::ZoomOut,
            Cmd::ZoomReset,
            Cmd::ReloadConfig,
            Cmd::ToggleBlur,
            Cmd::ImportZed,
            Cmd::NewTheme,
        ];
        for name in ["mocha", "macchiato", "frappe", "latte"] {
            cmds.push(Cmd::Theme(name.to_string()));
        }
        for name in &self.custom_themes {
            cmds.push(Cmd::Theme(name.clone()));
        }
        cmds
    }

    fn qo_items(&self, cx: &App) -> Vec<QoItem> {
        let q = self.qo_input.read(cx).value().to_string().to_lowercase();
        if self.qo_commands {
            self.all_cmds()
                .into_iter()
                .filter(|c| q.is_empty() || self.cmd_info(c).0.to_lowercase().contains(&q))
                .take(12)
                .map(QoItem::Cmd)
                .collect()
        } else {
            self.order
                .iter()
                .filter(|id| {
                    q.is_empty()
                        || self
                            .notes
                            .get(*id)
                            .map(|n| n.body.to_lowercase().contains(&q))
                            .unwrap_or(false)
                })
                .take(8)
                .cloned()
                .map(QoItem::Note)
                .collect()
        }
    }

    fn open_quick(&mut self, commands: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.qo_open = true;
        self.qo_commands = commands;
        self.qo_ix = 0;
        self.qo_input.update(cx, |s, cx| {
            s.set_value("", window, cx);
            s.focus(window, cx);
        });
        cx.notify();
    }

    fn close_quick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.qo_open = false;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn qo_step(&mut self, dir: i32, cx: &mut Context<Self>) {
        let n = self.qo_items(cx).len();
        if n == 0 {
            return;
        }
        self.qo_ix = if dir >= 0 {
            (self.qo_ix + 1) % n
        } else {
            (self.qo_ix + n - 1) % n
        };
        cx.notify();
    }

    fn qo_run(&mut self, item: QoItem, window: &mut Window, cx: &mut Context<Self>) {
        self.close_quick(window, cx);
        match item {
            QoItem::Note(id) => self.open_note(id, window, cx),
            QoItem::Cmd(cmd) => self.run_cmd(cmd, window, cx),
        }
    }

    fn qo_accept(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let items = self.qo_items(cx);
        if let Some(item) = items.get(self.qo_ix).cloned() {
            self.qo_run(item, window, cx);
        }
    }

    fn run_cmd(&mut self, cmd: Cmd, window: &mut Window, cx: &mut Context<Self>) {
        match cmd {
            Cmd::NewNote => self.new_note(window, cx),
            Cmd::NewTab => self.new_start_tab(window, cx),
            Cmd::CloseTab => self.close_active_tab(window, cx),
            Cmd::OpenNote => self.open_quick(false, window, cx),
            Cmd::ToggleSidebar => {
                self.sidebar_visible = !self.sidebar_visible;
                cx.notify();
            }
            Cmd::Mode(mode) => self.set_mode(mode, window, cx),
            Cmd::Find => self.find_in_note(window, cx),
            Cmd::Sync => self.sync_now(cx),
            Cmd::Settings => self.open_settings(window, cx),
            Cmd::Log => self.open_log(cx),
            Cmd::ZoomIn => self.zoom(0.1, window, cx),
            Cmd::ZoomOut => self.zoom(-0.1, window, cx),
            Cmd::ZoomReset => self.set_scale(1.0, window, cx),
            Cmd::ReloadConfig => self.reload_config(window, cx),
            Cmd::ImportZed => self.import_zed_themes(window, cx),
            Cmd::NewTheme => self.add_custom_theme(window, cx),
            Cmd::ToggleBlur => self.toggle_blur(window, cx),
            Cmd::Theme(name) => self.set_flavor(&name, window, cx),
        }
    }

    fn sync_now(&mut self, cx: &mut Context<Self>) {
        self.spawn_pull(cx);
        for item in self.notes.values().cloned().collect::<Vec<_>>() {
            if item.dirty {
                self.spawn_push(item, cx);
            }
        }
    }

    fn close_active_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let active = match self.right_view {
            RightView::Start => Some(START_TAB.to_string()),
            RightView::Editor => self.selected_id.clone(),
            _ => None,
        };
        if let Some(id) = active {
            self.close_tab(id, window, cx);
        }
    }

    // Ctrl+F: в режиме чтения ищем по отрисованному тексту, иначе встроенный поиск редактора.
    fn find_in_note(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    // ---------- полоса вкладок ----------
    fn tab_bar(&self, right: impl IntoElement, cx: &mut Context<Self>) -> Div {
        let mut tabs_row = h_flex()
            .id("tabs-scroll")
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_x_scroll();
        for (i, tid) in self.open_tabs.iter().enumerate() {
            let is_start = tid == START_TAB;
            let (title, dirty, conflict) = if is_start {
                ("Новая вкладка".to_string(), false, false)
            } else {
                let Some(n) = self.notes.get(tid) else {
                    continue;
                };
                (n.title(), n.dirty, n.has_conflict)
            };
            let active = if is_start {
                matches!(self.right_view, RightView::Start)
            } else {
                self.selected_id.as_deref() == Some(tid.as_str())
                    && matches!(self.right_view, RightView::Editor)
            };
            let select_id = tid.clone();
            let close_id = tid.clone();
            let tab = h_flex()
                .id(SharedString::from(format!("tabbox-{tid}")))
                .h_full()
                .flex_shrink_0()
                .items_center()
                .gap_1()
                .pl_3()
                .pr_1()
                .border_r_1()
                .border_color(theme::surface0())
                .when(active, |el| el.bg(theme::base()))
                .when(!active, |el| {
                    el.hover(|s| s.bg(theme::surface0().opacity(0.4)))
                })
                .child(
                    h_flex()
                        .id(SharedString::from(format!("tab-{tid}")))
                        .h_full()
                        .items_center()
                        .gap_2()
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.goto_tab_id(select_id.clone(), window, cx);
                        }))
                        .when(i < 10, |el| {
                            el.child(
                                div()
                                    .text_xs()
                                    .text_color(theme::overlay0())
                                    .child(format!("{}", (i + 1) % 10)),
                            )
                        })
                        .child(
                            Icon::new(if is_start {
                                IconName::Plus
                            } else {
                                IconName::File
                            })
                            .small()
                            .text_color(theme::overlay0()),
                        )
                        .child(
                            div()
                                .max_w(px(180.))
                                .text_sm()
                                .text_color(if active {
                                    theme::text()
                                } else {
                                    theme::subtext0()
                                })
                                .truncate()
                                .child(title),
                        )
                        .when(conflict, |el| {
                            el.child(
                                Icon::new(IconName::TriangleAlert)
                                    .small()
                                    .text_color(theme::red()),
                            )
                        })
                        .when(!conflict && dirty, |el| {
                            el.child(div().text_xs().text_color(theme::blue()).child("●"))
                        }),
                )
                .child(
                    Button::new(SharedString::from(format!("tab-close-{tid}")))
                        .ghost()
                        .xsmall()
                        .icon(IconName::Close)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.close_tab(close_id.clone(), window, cx);
                        })),
                );
            tabs_row = tabs_row.child(tab);
        }

        h_flex()
            .w_full()
            .h(px(34.0 * self.cfg.ui_scale))
            .items_center()
            .justify_between()
            .bg(theme::mantle())
            .border_b_1()
            .border_color(theme::surface0())
            .child(
                div().px_1().child(
                    Button::new("toggle-sidebar")
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(gpui_kit::assets::IconName::PanelLeft))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.sidebar_visible = !this.sidebar_visible;
                            cx.notify();
                        })),
                ),
            )
            .child(tabs_row)
            .child(right)
    }

    // ---------- внешний вид из настроек ----------
    // Применяет тему, прозрачность, размытие и масштаб из self.cfg.
    fn refresh_look(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        theme::configure(self.cfg.window_opacity, &self.cfg.colors);
        theme::set_flavor(&self.cfg.theme);
        theme::change(cx);
        window.set_background_appearance(window_bg(&self.cfg));
        window.set_rem_size(px(16.0 * self.cfg.ui_scale));
        cx.notify();
    }

    fn set_opacity(&mut self, value: f32, window: &mut Window, cx: &mut Context<Self>) {
        self.cfg.window_opacity = ((value.clamp(0.2, 1.0)) * 20.0).round() / 20.0;
        save_config(&self.cfg);
        self.refresh_look(window, cx);
    }

    fn toggle_blur(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cfg.window_blur = !self.cfg.window_blur;
        save_config(&self.cfg);
        self.refresh_look(window, cx);
    }

    // Ctrl+= / Ctrl+-: масштаб интерфейса (шрифты, отступы, размеры панелей).
    fn zoom(&mut self, delta: f32, window: &mut Window, cx: &mut Context<Self>) {
        self.set_scale(self.cfg.ui_scale + delta, window, cx);
    }

    fn set_scale(&mut self, value: f32, window: &mut Window, cx: &mut Context<Self>) {
        self.cfg.ui_scale = ((value.clamp(0.6, 2.5)) * 20.0).round() / 20.0;
        save_config(&self.cfg);
        window.set_rem_size(px(16.0 * self.cfg.ui_scale));
        cx.notify();
    }

    // Перечитать настройки из JSON (после ручной правки файла).
    fn reload_config(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match try_load_config() {
            Ok(cfg) => {
                self.cfg = cfg;
                self.custom_themes = theme::list_custom();
                self.refresh_look(window, cx);
                self.status = "Настройки перечитаны".into();
            }
            Err(err) => self.status = err,
        }
        cx.notify();
    }

    fn add_custom_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match theme::create_custom() {
            Ok(name) => {
                self.cfg.theme = name.clone();
                save_config(&self.cfg);
                self.custom_themes = theme::list_custom();
                self.refresh_look(window, cx);
                self.status = format!(
                    "Создана тема «{name}»: отредактируйте файл в {} и нажмите «Перечитать»",
                    theme::themes_dir().display()
                );
            }
            Err(err) => self.status = err,
        }
        cx.notify();
    }

    fn import_zed_themes(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let (imported, failed) = theme::import_zed();
        self.custom_themes = theme::list_custom();
        self.status = if imported == 0 && failed == 0 {
            "Темы Zed не найдены (~/.config/zed/themes и расширения)".to_string()
        } else {
            format!("Темы Zed: импортировано {imported}, не прочитано файлов {failed}. Выбор: Ctrl+Shift+P → «Тема»")
        };
        cx.notify();
    }

    fn open_path(&mut self, path: &std::path::Path) {
        if std::process::Command::new("xdg-open")
            .arg(path)
            .spawn()
            .is_err()
        {
            self.status = format!("Не удалось открыть {}", path.display());
        }
    }

    // ---------- настройки внешнего вида ----------
    fn set_flavor(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.cfg.theme = key.to_string();
        save_config(&self.cfg);
        self.refresh_look(window, cx);
    }

    fn set_font_size(&mut self, size: f32, cx: &mut Context<Self>) {
        self.cfg.font_size = size.clamp(12.0, 24.0);
        save_config(&self.cfg);
        cx.notify();
    }

    fn set_start_mode(&mut self, key: &str, cx: &mut Context<Self>) {
        self.cfg.start_mode = key.to_string();
        save_config(&self.cfg);
        cx.notify();
    }

    fn set_mode(&mut self, mode: ViewMode, window: &mut Window, cx: &mut Context<Self>) {
        self.mode = mode;
        if mode == ViewMode::Preview {
            // В режиме чтения редактора нет: фокус на корень, чтобы работали Ctrl+F и Ctrl+Shift+V.
            window.focus(&self.focus, cx);
        }
        cx.notify();
    }

    // Удаление любой заметки по id (меню по ПКМ в списке и кнопка «Удалить»).
    fn delete_note(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(note) = self.notes.remove(&id) else {
            return;
        };
        self.order.retain(|x| x != &id);
        self.open_tabs.retain(|x| x != &id);
        if self.selected_id.as_deref() == Some(id.as_str()) {
            self.selected_id = None;
            self.set_editor_text("", window, cx);
            // Открываем соседнюю вкладку, если она есть.
            if let Some(next) = self.open_tabs.last().cloned() {
                self.select_note(next, window, cx);
            }
        }
        cx.notify();

        if note.base_rev == 0 {
            return;
        }

        let url = format!(
            "{}/notes/{}?base_rev={}&device_id={}",
            self.base_url(),
            id,
            note.base_rev,
            self.cfg.device_id
        );
        let auth = format!("Bearer {}", self.cfg.token.trim());
        let client = self.http.clone();

        cx.spawn(async move |this, cx| {
            let res: Result<MutateResponse, String> = tokio_rt()
                .spawn(async move {
                    let resp = client
                        .delete(&url)
                        .header("Authorization", auth)
                        .send()
                        .await
                        .map_err(|e| format!("Нет связи: {e}"))?;
                    resp.json::<MutateResponse>()
                        .await
                        .map_err(|e| e.to_string())
                })
                .await
                .unwrap_or_else(|e| Err(format!("Task error: {e}")));

            let _ = this.update(cx, |app, cx| {
                match res {
                    Ok(mut_res) => {
                        if mut_res.status == "conflict" {
                            app.status = "Конфликт удаления: заметка изменена на сервере".into();
                        } else {
                            app.status = "Заметка удалена".into();
                        }
                        app.last_seq = 0;
                        app.spawn_pull(cx);
                    }
                    Err(err) => app.status = err,
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.right_view = RightView::Settings;
        let url = self.cfg.server_url.clone();
        let token = self.cfg.token.clone();
        self.url_input
            .update(cx, |st, cx| st.set_value(&url, window, cx));
        self.token_input
            .update(cx, |st, cx| st.set_value(&token, window, cx));
        self.health_status.clear();
        cx.notify();
    }

    fn check_health(&mut self, cx: &mut Context<Self>) {
        let raw = self
            .url_input
            .read(cx)
            .value()
            .trim()
            .trim_end_matches('/')
            .to_string();
        if raw.is_empty() {
            self.health_status = "Адрес пуст".into();
            cx.notify();
            return;
        }
        let url = if raw.starts_with("http://") || raw.starts_with("https://") {
            format!("{raw}/health")
        } else {
            format!("http://{raw}/health")
        };
        self.health_status = "Проверка...".into();
        cx.notify();
        let client = self.http.clone();

        cx.spawn(async move |this, cx| {
            let res: Result<HealthDto, String> = tokio_rt()
                .spawn(async move {
                    let resp = client
                        .get(&url)
                        .send()
                        .await
                        .map_err(|e| format!("Ошибка: {e}"))?;
                    if !resp.status().is_success() {
                        return Err(format!("HTTP {}", resp.status()));
                    }
                    resp.json::<HealthDto>()
                        .await
                        .map_err(|e| format!("Ошибка JSON: {e}"))
                })
                .await
                .unwrap_or_else(|e| Err(format!("Task error: {e}")));

            let _ = this.update(cx, |app, cx| {
                app.health_status = match res {
                    Ok(h) => format!("OK: {} v{}", h.name, h.version),
                    Err(e) => e,
                };
                cx.notify();
            });
        })
        .detach();
    }

    fn save_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cfg.server_url = self.url_input.read(cx).value().trim().to_string();
        self.cfg.token = self.token_input.read(cx).value().trim().to_string();
        save_config(&self.cfg);
        self.right_view = RightView::Editor;
        self.last_seq = 0;
        self.notes.clear();
        self.order.clear();
        self.selected_id = None;
        self.set_editor_text("", window, cx);
        self.status = "Подключение...".into();
        self.spawn_pull(cx);
        cx.notify();
    }

    fn open_conflicts(&mut self, note_id: String, cx: &mut Context<Self>) {
        self.right_view = RightView::Conflicts(note_id.clone());
        self.versions.clear();
        self.versions_loading = true;
        cx.notify();

        let url = format!("{}/notes/{note_id}/versions", self.base_url());
        let auth = format!("Bearer {}", self.cfg.token.trim());
        let client = self.http.clone();

        cx.spawn(async move |this, cx| {
            let res: Result<VersionsResponse, String> = tokio_rt()
                .spawn(async move {
                    let resp = client
                        .get(&url)
                        .header("Authorization", auth)
                        .send()
                        .await
                        .map_err(|e| format!("Ошибка загрузки версий: {e}"))?;
                    if !resp.status().is_success() {
                        return Err(format!("HTTP {}", resp.status()));
                    }
                    resp.json::<VersionsResponse>()
                        .await
                        .map_err(|e| e.to_string())
                })
                .await
                .unwrap_or_else(|e| Err(format!("Task error: {e}")));

            let _ = this.update(cx, |app, cx| {
                app.versions_loading = false;
                match res {
                    Ok(data) => app.versions = data.versions,
                    Err(err) => app.status = err,
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn resolve_conflict(
        &mut self,
        note_id: String,
        version_id: i64,
        action: &'static str,
        cx: &mut Context<Self>,
    ) {
        let url = format!(
            "{}/notes/{note_id}/conflicts/{version_id}/resolve",
            self.base_url()
        );
        let auth = format!("Bearer {}", self.cfg.token.trim());
        let client = self.http.clone();
        let payload = ResolveRequest {
            action: action.to_string(),
            device_id: self.cfg.device_id.clone(),
        };

        cx.spawn(async move |this, cx| {
            let res: Result<MutateResponse, String> = tokio_rt()
                .spawn(async move {
                    let resp = client
                        .post(&url)
                        .header("Authorization", auth)
                        .json(&payload)
                        .send()
                        .await
                        .map_err(|e| format!("Ошибка resolve: {e}"))?;
                    if !resp.status().is_success() {
                        return Err(format!("HTTP {}", resp.status()));
                    }
                    resp.json::<MutateResponse>()
                        .await
                        .map_err(|e| e.to_string())
                })
                .await
                .unwrap_or_else(|e| Err(format!("Task error: {e}")));

            let _ = this.update(cx, |app, cx| {
                match res {
                    Ok(_) => {
                        app.status = "Конфликт разрешён".into();
                        app.spawn_pull(cx);
                        if let RightView::Conflicts(ref nid) = app.right_view {
                            let nid = nid.clone();
                            app.open_conflicts(nid, cx);
                        }
                    }
                    Err(err) => app.status = err,
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn open_log(&mut self, cx: &mut Context<Self>) {
        self.right_view = RightView::Log;
        self.log_loading = true;
        cx.notify();

        let url = format!("{}/log?limit=100", self.base_url());
        let auth = format!("Bearer {}", self.cfg.token.trim());
        let client = self.http.clone();

        cx.spawn(async move |this, cx| {
            let res: Result<LogResponse, String> = tokio_rt()
                .spawn(async move {
                    let resp = client
                        .get(&url)
                        .header("Authorization", auth)
                        .send()
                        .await
                        .map_err(|e| format!("Ошибка загрузки лога: {e}"))?;
                    if !resp.status().is_success() {
                        return Err(format!("HTTP {}", resp.status()));
                    }
                    resp.json::<LogResponse>().await.map_err(|e| e.to_string())
                })
                .await
                .unwrap_or_else(|e| Err(format!("Task error: {e}")));

            let _ = this.update(cx, |app, cx| {
                app.log_loading = false;
                match res {
                    Ok(data) => app.log_entries = data.log,
                    Err(err) => app.status = err,
                }
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for NoteSyncApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let query_lower = self.search_input.read(cx).value().trim().to_lowercase();

        // ---------- ЛЕВЫЙ САЙДБАР ----------
        let view = cx.entity();
        let mut cards = v_flex().gap_0p5().w_full();
        for id in &self.order {
            if let Some(note) = self.notes.get(id) {
                if !query_lower.is_empty() && !note.body.to_lowercase().contains(&query_lower) {
                    continue;
                }
                let is_selected = matches!(self.right_view, RightView::Editor)
                    && self.selected_id.as_deref() == Some(id.as_str());
                let note_id = id.clone();
                let preview = note.preview();

                let card = v_flex()
                    .id(SharedString::from(format!("note-{id}")))
                    .w_full()
                    .px_2()
                    .py_1p5()
                    .gap_0p5()
                    .rounded_md()
                    .cursor_pointer()
                    .when(is_selected, |el| el.bg(theme::surface0()))
                    .when(!is_selected, |el| {
                        el.hover(|s| s.bg(theme::surface0().opacity(0.55)))
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_note(note_id.clone(), window, cx);
                    }))
                    .child(
                        h_flex()
                            .w_full()
                            .justify_between()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(theme::text())
                                    .truncate()
                                    .child(note.title()),
                            )
                            .when(note.has_conflict, |el| {
                                el.child(
                                    Icon::new(IconName::TriangleAlert)
                                        .small()
                                        .text_color(theme::red()),
                                )
                            })
                            .when(!note.has_conflict && note.dirty, |el| {
                                el.child(div().text_xs().text_color(theme::blue()).child("●"))
                            }),
                    )
                    .when(!preview.is_empty() && self.cfg.show_note_preview, |el| {
                        el.child(
                            div()
                                .text_xs()
                                .text_color(theme::subtext0())
                                .truncate()
                                .child(preview),
                        )
                    });

                let menu_view = view.clone();
                let menu_id = id.clone();
                let card = card.context_menu(move |menu, _window, _cx| {
                    let view = menu_view.clone();
                    let id = menu_id.clone();
                    menu.item(
                        PopupMenuItem::new("Удалить")
                            .icon(gpui_kit::assets::IconName::Trash)
                            .on_click(move |_, window, cx| {
                                let id = id.clone();
                                view.update(cx, |this, cx| {
                                    if this.cfg.confirm_delete {
                                        this.pending_delete = Some(id);
                                        cx.notify();
                                    } else {
                                        this.delete_note(id, window, cx);
                                    }
                                });
                            }),
                    )
                });

                cards = cards.child(card);
            }
        }

        let sidebar = v_flex()
            .w(px(self.cfg.sidebar_width * self.cfg.ui_scale))
            .h_full()
            .p_2()
            .gap_2()
            .bg(theme::mantle())
            .border_r_1()
            .border_color(theme::surface0())
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .px_1()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme::subtext0())
                            .child("ЗАМЕТКИ"),
                    )
                    .child(
                        h_flex()
                            .gap_0p5()
                            .child(
                                Button::new("new-note-btn")
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::Plus)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.new_note(window, cx);
                                    })),
                            )
                            .child(
                                Button::new("sync-btn")
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::RefreshCw)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.spawn_pull(cx);
                                        for item in this.notes.values().cloned().collect::<Vec<_>>()
                                        {
                                            if item.dirty {
                                                this.spawn_push(item, cx);
                                            }
                                        }
                                    })),
                            )
                            .child(
                                Button::new("log-btn")
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::Info)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.open_log(cx);
                                    })),
                            )
                            .child(
                                Button::new("settings-btn")
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::Settings)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.open_settings(window, cx);
                                    })),
                            ),
                    ),
            )
            .child(Input::new(&self.search_input).small())
            .child(
                div()
                    .id("notes-scroll")
                    .flex_1()
                    .w_full()
                    .overflow_y_scroll()
                    .child(cards),
            );

        // ---------- НАСТРОЙКИ ВНЕШНЕГО ВИДА ----------
        let flavor = theme::flavor_name();
        let start = self.cfg.start_mode.clone();
        let font_size = self.cfg.font_size;
        let opacity_now = self.cfg.window_opacity;
        let blur_on = self.cfg.window_blur;
        let scale_now = self.cfg.ui_scale;
        let flavor_btn =
            |id: &'static str, label: &'static str, key: &'static str, cx: &mut Context<Self>| {
                let btn = Button::new(id).xsmall().label(label);
                let btn = if flavor == key {
                    btn.primary()
                } else {
                    btn.ghost()
                };
                btn.on_click(cx.listener(move |this, _, window, cx| {
                    this.set_flavor(key, window, cx);
                }))
            };
        let start_btn =
            |id: &'static str, label: &'static str, key: &'static str, cx: &mut Context<Self>| {
                let btn = Button::new(id).xsmall().label(label);
                let btn = if start == key {
                    btn.primary()
                } else {
                    btn.ghost()
                };
                btn.on_click(cx.listener(move |this, _, _, cx| {
                    this.set_start_mode(key, cx);
                }))
            };
        let mut custom_row = h_flex().gap_1().flex_wrap();
        for name in self.custom_themes.iter().take(12).cloned() {
            let btn = Button::new(SharedString::from(format!("th-custom-{name}")))
                .xsmall()
                .label(name.clone());
            let btn = if flavor == name {
                btn.primary()
            } else {
                btn.ghost()
            };
            let pick = name.clone();
            custom_row = custom_row.child(btn.on_click(cx.listener(move |this, _, window, cx| {
                this.set_flavor(&pick, window, cx);
            })));
        }
        let appearance = v_flex()
            .max_w(px(480.))
            .p_5()
            .gap_3()
            .rounded_xl()
            .bg(theme::mantle())
            .border_1()
            .border_color(theme::surface0())
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::text())
                    .child("Внешний вид"),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme::subtext0())
                    .child("Тема"),
            )
            .child(
                h_flex()
                    .gap_1()
                    .child(flavor_btn("th-mocha", "Mocha", "mocha", cx))
                    .child(flavor_btn("th-macchiato", "Macchiato", "macchiato", cx))
                    .child(flavor_btn("th-frappe", "Frappé", "frappe", cx))
                    .child(flavor_btn("th-latte", "Latte", "latte", cx)),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme::subtext0())
                    .child("Свои темы (JSON-файлы в ~/.config/notesync/themes)"),
            )
            .child(custom_row)
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        Button::new("th-add")
                            .xsmall()
                            .icon(IconName::Plus)
                            .label("Новая тема")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.add_custom_theme(window, cx);
                            })),
                    )
                    .child(
                        Button::new("th-zed")
                            .ghost()
                            .xsmall()
                            .label("Импорт из Zed")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.import_zed_themes(window, cx);
                            })),
                    )
                    .child(
                        Button::new("th-folder")
                            .ghost()
                            .xsmall()
                            .label("Папка тем")
                            .on_click(cx.listener(|this, _, _, cx| {
                                let dir = theme::themes_dir();
                                let _ = std::fs::create_dir_all(&dir);
                                this.open_path(&dir);
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme::subtext0())
                    .child("Прозрачность окна и размытие"),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Button::new("op-minus")
                            .ghost()
                            .xsmall()
                            .label("−")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.set_opacity(this.cfg.window_opacity - 0.05, window, cx);
                            })),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme::text())
                            .child(format!("{}%", (opacity_now * 100.0).round() as i32)),
                    )
                    .child(
                        Button::new("op-plus")
                            .ghost()
                            .xsmall()
                            .label("+")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.set_opacity(this.cfg.window_opacity + 0.05, window, cx);
                            })),
                    )
                    .child({
                        let blur_btn = Button::new("blur-toggle").xsmall().label("Размытие");
                        let blur_btn = if blur_on {
                            blur_btn.primary()
                        } else {
                            blur_btn.ghost()
                        };
                        blur_btn.on_click(cx.listener(|this, _, window, cx| {
                            this.toggle_blur(window, cx);
                        }))
                    }),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme::overlay0())
                    .child("Размытие работает, если композитор поддерживает его для окон. В niri: window-rule с background-effect { blur true } для app-id notesync."),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme::subtext0())
                    .child("Масштаб интерфейса (Ctrl + / Ctrl -, Ctrl 0 — сброс)"),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Button::new("zoom-minus")
                            .ghost()
                            .xsmall()
                            .label("−")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.zoom(-0.1, window, cx);
                            })),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme::text())
                            .child(format!("{}%", (scale_now * 100.0).round() as i32)),
                    )
                    .child(
                        Button::new("zoom-plus")
                            .ghost()
                            .xsmall()
                            .label("+")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.zoom(0.1, window, cx);
                            })),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme::subtext0())
                    .child("Файл настроек (JSON)"),
            )
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        Button::new("cfg-reload")
                            .xsmall()
                            .icon(IconName::RefreshCw)
                            .label("Перечитать")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.reload_config(window, cx);
                            })),
                    )
                    .child(
                        Button::new("cfg-open")
                            .ghost()
                            .xsmall()
                            .label("Открыть файл")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.open_path(&config_path());
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme::overlay0())
                    .child("~/.config/notesync-desktop.json: ключи theme, ui_scale, window_opacity, window_blur, sidebar_width, font_family, editor_font_family, font_size, start_mode, show_note_preview, confirm_delete, save_delay_ms, sync_interval_ms, colors."),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme::subtext0())
                    .child("Размер текста"),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Button::new("fs-minus")
                            .ghost()
                            .xsmall()
                            .label("−")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.set_font_size(this.cfg.font_size - 1.0, cx);
                            })),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme::text())
                            .child(format!("{} px", font_size as i32)),
                    )
                    .child(
                        Button::new("fs-plus")
                            .ghost()
                            .xsmall()
                            .label("+")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.set_font_size(this.cfg.font_size + 1.0, cx);
                            })),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme::subtext0())
                    .child("Как открывать заметку"),
            )
            .child(
                h_flex()
                    .gap_1()
                    .child(start_btn("st-preview", "Читать", "preview", cx))
                    .child(start_btn("st-split", "Рядом", "split", cx))
                    .child(start_btn("st-edit", "Править", "edit", cx)),
            );

        let shortcuts = v_flex()
            .max_w(px(480.))
            .p_5()
            .gap_1p5()
            .rounded_xl()
            .bg(theme::mantle())
            .border_1()
            .border_color(theme::surface0())
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::text())
                    .child("Горячие клавиши"),
            )
            .children(
                [
                    ("Ctrl+N", "новая заметка"),
                    ("Ctrl+T", "новая вкладка («Привет»)"),
                    ("Ctrl+W", "закрыть вкладку"),
                    ("Alt+1 … Alt+9, Alt+0", "вкладка по номеру (0 — десятая)"),
                    ("Ctrl+P", "открыть заметку по названию"),
                    ("Ctrl+B", "скрыть / показать список заметок"),
                    ("Ctrl+F", "поиск в заметке"),
                    ("Ctrl+Shift+V", "редактор / редактор с превью"),
                    ("Esc", "закрыть поиск или окно выбора"),
                ]
                .into_iter()
                .map(|(keys, what)| {
                    h_flex()
                        .gap_3()
                        .child(
                            div()
                                .w(px(170.))
                                .text_xs()
                                .text_color(theme::blue())
                                .child(keys),
                        )
                        .child(div().text_xs().text_color(theme::subtext0()).child(what))
                }),
            );

        // ---------- ПРАВАЯ ОБЛАСТЬ ----------
        let right_pane: AnyElement = match &self.right_view {
            RightView::Settings => v_flex()
                .id("settings-scroll")
                .flex_1()
                .h_full()
                .overflow_y_scroll()
                .p_6()
                .gap_4()
                .child(
                    h_flex()
                        .w_full()
                        .justify_between()
                        .items_center()
                        .child(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .child(Icon::new(IconName::Settings).text_color(theme::blue()))
                                .child(
                                    div()
                                        .text_lg()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(theme::text())
                                        .child("Настройки подключения"),
                                ),
                        )
                        .child(
                            Button::new("close-settings")
                                .ghost()
                                .small()
                                .icon(IconName::Close)
                                .label("Закрыть")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.right_view = RightView::Editor;
                                    cx.notify();
                                })),
                        ),
                )
                .child(
                    v_flex()
                        .max_w(px(480.))
                        .p_5()
                        .gap_3()
                        .rounded_xl()
                        .bg(theme::mantle())
                        .border_1()
                        .border_color(theme::surface0())
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme::subtext0())
                                .child("Адрес сервера (HTTP API)"),
                        )
                        .child(Input::new(&self.url_input))
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme::subtext0())
                                .child("Токен доступа"),
                        )
                        .child(Input::new(&self.token_input))
                        .child(
                            h_flex()
                                .gap_2p5()
                                .child(
                                    Button::new("save-settings")
                                        .primary()
                                        .icon(IconName::Check)
                                        .label("Сохранить и синхр.")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.save_settings(window, cx);
                                        })),
                                )
                                .child(
                                    Button::new("check-health")
                                        .ghost()
                                        .label("Проверить /health")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.check_health(cx);
                                        })),
                                ),
                        )
                        .when(!self.health_status.is_empty(), |el| {
                            let color = if self.health_status.starts_with("OK") {
                                theme::green()
                            } else {
                                theme::red()
                            };
                            el.child(
                                div()
                                    .text_xs()
                                    .text_color(color)
                                    .child(self.health_status.clone()),
                            )
                        })
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme::overlay0())
                                .child(format!("ID устройства: {}", self.cfg.device_id)),
                        ),
                )
                .child(appearance)
                .child(shortcuts)
                .into_any_element(),

            RightView::Conflicts(note_id) => {
                let note_id_owned = note_id.clone();
                let server_body = self
                    .notes
                    .get(&note_id_owned)
                    .map(|n| n.body.clone())
                    .unwrap_or_default();

                let unresolved: Vec<NoteVersionDto> = self
                    .versions
                    .iter()
                    .filter(|v| v.is_conflict && !v.resolved)
                    .cloned()
                    .collect();

                let mut list = v_flex().gap_4().w_full();
                if self.versions_loading {
                    list = list.child(
                        div()
                            .text_sm()
                            .text_color(theme::subtext0())
                            .child("Загрузка версий..."),
                    );
                } else if unresolved.is_empty() {
                    list = list.child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(Icon::new(IconName::CircleCheck).text_color(theme::green()))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(theme::green())
                                    .child("Активных конфликтов для этой заметки нет."),
                            ),
                    );
                } else {
                    for ver in unresolved {
                        let nid1 = note_id_owned.clone();
                        let nid2 = note_id_owned.clone();
                        let nid3 = note_id_owned.clone();
                        let vid = ver.id;

                        list = list.child(
                            v_flex()
                                .gap_2p5()
                                .w_full()
                                .child(
                                    h_flex()
                                        .gap_3()
                                        .w_full()
                                        .child(
                                            v_flex()
                                                .flex_1()
                                                .p_3p5()
                                                .gap_2()
                                                .rounded_lg()
                                                .bg(theme::mantle())
                                                .border_1()
                                                .border_color(theme::surface0())
                                                .child(
                                                    div()
                                                        .text_xs()
                                                        .text_color(theme::blue())
                                                        .child("Текущая на сервере"),
                                                )
                                                .child(
                                                    div()
                                                        .text_sm()
                                                        .text_color(theme::text())
                                                        .child(server_body.clone()),
                                                ),
                                        )
                                        .child(
                                            v_flex()
                                                .flex_1()
                                                .p_3p5()
                                                .gap_2()
                                                .rounded_lg()
                                                .bg(theme::mantle())
                                                .border_1()
                                                .border_color(theme::red())
                                                .child(
                                                    div().text_xs().text_color(theme::red()).child(
                                                        format!(
                                                            "Конфликт #{} ({})",
                                                            ver.id, ver.device_id
                                                        ),
                                                    ),
                                                )
                                                .child(
                                                    div()
                                                        .text_sm()
                                                        .text_color(theme::text())
                                                        .child(ver.body),
                                                ),
                                        ),
                                )
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .child(
                                            Button::new(("keep-srv", vid as usize))
                                                .ghost()
                                                .small()
                                                .label("Оставить серверную")
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    this.resolve_conflict(
                                                        nid1.clone(),
                                                        vid,
                                                        "keep_server",
                                                        cx,
                                                    );
                                                })),
                                        )
                                        .child(
                                            Button::new(("use-this", vid as usize))
                                                .primary()
                                                .small()
                                                .label("Взять эту")
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    this.resolve_conflict(
                                                        nid2.clone(),
                                                        vid,
                                                        "use_this",
                                                        cx,
                                                    );
                                                })),
                                        )
                                        .child(
                                            Button::new(("keep-both", vid as usize))
                                                .ghost()
                                                .small()
                                                .label("Сохранить обе")
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    this.resolve_conflict(
                                                        nid3.clone(),
                                                        vid,
                                                        "keep_both",
                                                        cx,
                                                    );
                                                })),
                                        ),
                                ),
                        );
                    }
                }

                v_flex()
                    .id("conflicts-scroll")
                    .flex_1()
                    .h_full()
                    .p_6()
                    .gap_4()
                    .overflow_y_scroll()
                    .child(
                        h_flex()
                            .w_full()
                            .justify_between()
                            .items_center()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .child(Icon::new(IconName::Replace).text_color(theme::red()))
                                    .child(
                                        div()
                                            .text_lg()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(theme::text())
                                            .child("Разрешение конфликтов"),
                                    ),
                            )
                            .child(
                                Button::new("close-conflicts")
                                    .ghost()
                                    .small()
                                    .icon(IconName::ArrowLeft)
                                    .label("К редактору")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.right_view = RightView::Editor;
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(list)
                    .into_any_element()
            }

            RightView::Log => {
                let mut rows = v_flex().gap_2p5().w_full();
                if self.log_loading {
                    rows = rows.child(
                        div()
                            .text_sm()
                            .text_color(theme::subtext0())
                            .child("Загрузка журнала..."),
                    );
                } else if self.log_entries.is_empty() {
                    rows = rows.child(
                        div()
                            .text_sm()
                            .text_color(theme::subtext0())
                            .child("Записей пока нет."),
                    );
                } else {
                    for entry in &self.log_entries {
                        let ev_color = match entry.event.as_str() {
                            "conflict" => theme::red(),
                            "resolve" => theme::green(),
                            "delete" => theme::peach(),
                            _ => theme::blue(),
                        };
                        let short_id = if entry.note_id.len() > 8 {
                            &entry.note_id[..8]
                        } else {
                            &entry.note_id
                        };

                        rows = rows.child(
                            v_flex()
                                .w_full()
                                .px_3p5()
                                .py_2p5()
                                .gap_1()
                                .rounded_lg()
                                .bg(theme::mantle())
                                .border_1()
                                .border_color(theme::surface0())
                                .child(
                                    h_flex()
                                        .w_full()
                                        .justify_between()
                                        .child(
                                            h_flex()
                                                .gap_2()
                                                .child(
                                                    div()
                                                        .text_xs()
                                                        .font_weight(FontWeight::SEMIBOLD)
                                                        .text_color(ev_color)
                                                        .child(entry.event.to_uppercase()),
                                                )
                                                .child(
                                                    div()
                                                        .text_xs()
                                                        .text_color(theme::subtext0())
                                                        .child(format!(
                                                            "• {} • {}",
                                                            entry.device_id, entry.ts
                                                        )),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(theme::overlay0())
                                                .child(format!("#{short_id}")),
                                        ),
                                )
                                .when(!entry.details.is_empty(), |el| {
                                    el.child(
                                        div()
                                            .text_sm()
                                            .text_color(theme::text())
                                            .child(entry.details.clone()),
                                    )
                                }),
                        );
                    }
                }

                v_flex()
                    .id("log-scroll")
                    .flex_1()
                    .h_full()
                    .p_6()
                    .gap_4()
                    .overflow_y_scroll()
                    .child(
                        h_flex()
                            .w_full()
                            .justify_between()
                            .items_center()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .child(Icon::new(IconName::Info).text_color(theme::blue()))
                                    .child(
                                        div()
                                            .text_lg()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(theme::text())
                                            .child("Журнал синхронизации"),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(
                                        Button::new("refresh-log")
                                            .ghost()
                                            .small()
                                            .icon(IconName::RefreshCw)
                                            .label("Обновить")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.open_log(cx);
                                            })),
                                    )
                                    .child(
                                        Button::new("close-log")
                                            .ghost()
                                            .small()
                                            .icon(IconName::Close)
                                            .label("Закрыть")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.right_view = RightView::Editor;
                                                cx.notify();
                                            })),
                                    ),
                            ),
                    )
                    .child(rows)
                    .into_any_element()
            }

            RightView::Start => {
                let mut list = v_flex().w_full().gap_0p5();
                for id in self.order.iter().take(40) {
                    let Some(n) = self.notes.get(id) else {
                        continue;
                    };
                    let open_id = id.clone();
                    list = list.child(
                        h_flex()
                            .id(SharedString::from(format!("start-{id}")))
                            .w_full()
                            .px_3()
                            .py_1p5()
                            .gap_2()
                            .items_center()
                            .rounded_md()
                            .cursor_pointer()
                            .hover(|s| s.bg(theme::surface0().opacity(0.55)))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_note(open_id.clone(), window, cx);
                            }))
                            .child(
                                Icon::new(IconName::File)
                                    .small()
                                    .text_color(theme::overlay0()),
                            )
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .max_w(px(260.))
                                    .text_sm()
                                    .text_color(theme::text())
                                    .truncate()
                                    .child(n.title()),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_xs()
                                    .text_color(theme::overlay0())
                                    .truncate()
                                    .child(n.preview()),
                            ),
                    );
                }
                v_flex()
                    .flex_1()
                    .h_full()
                    .min_w_0()
                    .child(self.tab_bar(div(), cx))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_h_0()
                            .items_center()
                            .pt_10()
                            .px_5()
                            .gap_3()
                            .bg(theme::base())
                            .child(
                                div()
                                    .text_2xl()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme::text())
                                    .child("Привет"),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(theme::subtext0())
                                    .child("Выбери заметку или нажми Ctrl+P"),
                            )
                            .child(
                                Button::new("start-new")
                                    .primary()
                                    .small()
                                    .icon(IconName::Plus)
                                    .label("Новая заметка")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.new_note(window, cx);
                                    })),
                            )
                            .child(
                                div()
                                    .id("start-list")
                                    .w_full()
                                    .max_w(px(620.))
                                    .flex_1()
                                    .min_h_0()
                                    .overflow_y_scroll()
                                    .child(list),
                            ),
                    )
                    .into_any_element()
            }

            RightView::Editor => {
                if let Some(sel_id) = self.selected_id.clone() {
                    let note_info = self.notes.get(&sel_id);
                    let rev_text = note_info
                        .map(|n| format!("rev {}", n.base_rev))
                        .unwrap_or_default();
                    let has_conf = note_info.map(|n| n.has_conflict).unwrap_or(false);

                    let mode = self.mode;
                    let mode_btn = |id: &'static str,
                                    label: &'static str,
                                    target: ViewMode,
                                    cx: &mut Context<Self>| {
                        let btn = Button::new(id).xsmall().label(label);
                        let btn = if mode == target {
                            btn.primary()
                        } else {
                            btn.ghost()
                        };
                        btn.on_click(cx.listener(move |this, _, window, cx| {
                            this.set_mode(target, window, cx);
                        }))
                    };
                    let read_btn = h_flex()
                        .gap_1()
                        .child(mode_btn("mode-preview", "Читать", ViewMode::Preview, cx))
                        .child(mode_btn("mode-split", "Рядом", ViewMode::Split, cx))
                        .child(mode_btn("mode-edit", "Править", ViewMode::Edit, cx));

                    let conf_btn = if has_conf {
                        Button::new("open-conflicts")
                            .danger()
                            .xsmall()
                            .icon(IconName::TriangleAlert)
                            .label("Конфликты")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.open_conflicts(sel_id.clone(), cx);
                            }))
                    } else {
                        Button::new("open-versions")
                            .ghost()
                            .xsmall()
                            .icon(IconName::Replace)
                            .label("Версии")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.open_conflicts(sel_id.clone(), cx);
                            }))
                    };

                    let right_group = h_flex()
                        .flex_shrink_0()
                        .items_center()
                        .gap_2()
                        .px_2()
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme::overlay0())
                                .child(rev_text),
                        )
                        .child(read_btn)
                        .child(conf_btn);

                    v_flex()
                        .flex_1()
                        .h_full()
                        .min_w_0()
                        .child(self.tab_bar(right_group, cx))
                        .child(
                            v_flex()
                                .flex_1()
                                .min_h_0()
                                .w_full()
                                .px_5()
                                .py_3()
                                .bg(theme::base())
                                .child(div().flex_1().min_h_0().w_full().child({
                                    let fs = self.cfg.font_size * self.cfg.ui_scale;
                                    // Текст превью обновляем только когда он виден.
                                    if self.mode != ViewMode::Edit {
                                        let body = self.editor_input.read(cx).value().to_string();
                                        if self.preview_body != body {
                                            self.preview_body = body.clone();
                                            self.preview_state
                                                .update(cx, |state, cx| state.set_text(&body, cx));
                                        }
                                    }
                                    let preview = TextView::new(&self.preview_state)
                                        .style(md_style(fs))
                                        .text_size(px(fs))
                                        .size_full();
                                    let editor_font = self.cfg.editor_font_family.clone();
                                    let editor = Textarea::new(&self.editor_input)
                                        .appearance(false)
                                        .bordered(false)
                                        .text_size(px(fs))
                                        .when(!editor_font.is_empty(), |t| {
                                            t.font_family(editor_font)
                                        })
                                        .h_full();
                                    let counter = if self.psearch_matches.is_empty() {
                                        "0/0".to_string()
                                    } else {
                                        format!(
                                            "{}/{}",
                                            self.psearch_ix + 1,
                                            self.psearch_matches.len()
                                        )
                                    };
                                    let search_bar = self.psearch_open.then(|| {
                                        h_flex()
                                            .w_full()
                                            .gap_2()
                                            .items_center()
                                            .px_2()
                                            .py_1()
                                            .rounded_md()
                                            .bg(theme::mantle())
                                            .border_1()
                                            .border_color(theme::surface0())
                                            .child(
                                                div()
                                                    .w(px(260.))
                                                    .child(Input::new(&self.psearch_input).small()),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(theme::subtext0())
                                                    .child(counter),
                                            )
                                            .child(
                                                Button::new("ps-prev")
                                                    .ghost()
                                                    .xsmall()
                                                    .icon(Icon::new(
                                                        gpui_kit::assets::IconName::ChevronUp,
                                                    ))
                                                    .on_click(cx.listener(|this, _, _, cx| {
                                                        this.psearch_step(-1, cx);
                                                    })),
                                            )
                                            .child(
                                                Button::new("ps-next")
                                                    .ghost()
                                                    .xsmall()
                                                    .icon(Icon::new(
                                                        gpui_kit::assets::IconName::ChevronDown,
                                                    ))
                                                    .on_click(cx.listener(|this, _, _, cx| {
                                                        this.psearch_step(1, cx);
                                                    })),
                                            )
                                            .child(
                                                Button::new("ps-close")
                                                    .ghost()
                                                    .xsmall()
                                                    .icon(IconName::Close)
                                                    .on_click(cx.listener(
                                                        |this, _, window, cx| {
                                                            this.close_psearch(window, cx);
                                                        },
                                                    )),
                                            )
                                    });
                                    match self.mode {
                                        ViewMode::Preview => v_flex()
                                            .size_full()
                                            .gap_2()
                                            .children(search_bar)
                                            .child(div().flex_1().min_h_0().w_full().child(preview))
                                            .into_any_element(),
                                        ViewMode::Edit => editor.into_any_element(),
                                        ViewMode::Split => h_flex()
                                            .size_full()
                                            .gap_3()
                                            .child(div().flex_1().min_w_0().h_full().child(editor))
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .h_full()
                                                    .pl_3()
                                                    .border_l_1()
                                                    .border_color(theme::surface0())
                                                    .child(preview),
                                            )
                                            .into_any_element(),
                                    }
                                })),
                        )
                        .into_any_element()
                } else {
                    v_flex()
                        .flex_1()
                        .h_full()
                        .min_w_0()
                        .child(self.tab_bar(div(), cx))
                        .child(
                            v_flex()
                                .flex_1()
                                .items_center()
                                .justify_center()
                                .gap_3()
                                .child(Icon::new(IconName::File).text_color(theme::overlay0()))
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(theme::subtext0())
                                        .child("Выбери заметку слева, Ctrl+P или Ctrl+N"),
                                ),
                        )
                        .into_any_element()
                }
            }
        };

        // ---------- НИЖНЯЯ СТРОКА СТАТУСА ----------
        let status_color = if self.status.contains("Конфликт") || self.status.contains("конфликт")
        {
            theme::red()
        } else if self.status.contains("Нет связи") || self.status.contains("Ошибка")
        {
            theme::peach()
        } else {
            theme::green()
        };

        let words = self
            .selected_id
            .as_ref()
            .and_then(|id| self.notes.get(id))
            .map(|n| format!("{} слов  •  ", n.body.split_whitespace().count()))
            .unwrap_or_default();

        let status_bar = h_flex()
            .w_full()
            .h(px(24.0 * self.cfg.ui_scale))
            .px_3()
            .justify_between()
            .items_center()
            .bg(theme::mantle())
            .border_t_1()
            .border_color(theme::surface0())
            .child(
                div()
                    .text_xs()
                    .text_color(status_color)
                    .child(self.status.clone()),
            )
            .child(div().text_xs().text_color(theme::overlay0()).child(format!(
                "{}{}  •  seq {}",
                words,
                self.base_url(),
                self.last_seq
            )));

        let confirm_overlay = self.pending_delete.clone().map(|id| {
            let title = self.notes.get(&id).map(|n| n.title()).unwrap_or_default();
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .occlude()
                .flex()
                .items_center()
                .justify_center()
                .bg(theme::crust().opacity(0.6))
                .child(
                    v_flex()
                        .w(px(360.))
                        .p_5()
                        .gap_3()
                        .rounded_xl()
                        .bg(theme::mantle())
                        .border_1()
                        .border_color(theme::surface1())
                        .child(
                            div()
                                .text_base()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme::text())
                                .child("Удалить заметку?"),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(theme::subtext0())
                                .truncate()
                                .child(title),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .justify_end()
                                .child(
                                    Button::new("delete-cancel")
                                        .small()
                                        .label("Отмена")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.pending_delete = None;
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    Button::new("delete-confirm")
                                        .danger()
                                        .small()
                                        .icon(Icon::new(gpui_kit::assets::IconName::Trash))
                                        .label("Удалить")
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            if let Some(id) = this.pending_delete.take() {
                                                this.delete_note(id, window, cx);
                                            }
                                        })),
                                ),
                        ),
                )
        });

        let quick_overlay = self.qo_open.then(|| {
            let items = self.qo_items(cx);
            let mut list = v_flex().w_full().gap_0p5();
            for (i, item) in items.iter().enumerate() {
                let (label, hint) = match item {
                    QoItem::Note(id) => (
                        self.notes.get(id).map(|n| n.title()).unwrap_or_default(),
                        "",
                    ),
                    QoItem::Cmd(cmd) => self.cmd_info(cmd),
                };
                let run_item = item.clone();
                list = list.child(
                    h_flex()
                        .id(SharedString::from(format!("qo-{i}")))
                        .w_full()
                        .px_3()
                        .py_1p5()
                        .gap_2()
                        .items_center()
                        .rounded_md()
                        .cursor_pointer()
                        .when(i == self.qo_ix, |el| el.bg(theme::surface0()))
                        .hover(|s| s.bg(theme::surface0().opacity(0.55)))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.qo_run(run_item.clone(), window, cx);
                        }))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_sm()
                                .text_color(theme::text())
                                .truncate()
                                .child(label),
                        )
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_xs()
                                .text_color(theme::overlay0())
                                .child(hint),
                        ),
                );
            }
            let title = if self.qo_commands {
                "Команды"
            } else {
                "Заметки"
            };
            div()
                .id("qo-overlay")
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .occlude()
                .flex()
                .justify_center()
                .items_start()
                .pt(px(90.))
                .bg(theme::crust().opacity(0.5))
                .on_click(cx.listener(|this, _, window, cx| this.close_quick(window, cx)))
                .child(
                    v_flex()
                        .key_context("QuickOpen")
                        // Стрелки вверх/вниз ловим раньше поля ввода: оно забирает их себе.
                        .capture_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
                            match ev.keystroke.key.as_str() {
                                "up" => {
                                    this.qo_step(-1, cx);
                                    cx.stop_propagation();
                                }
                                "down" => {
                                    this.qo_step(1, cx);
                                    cx.stop_propagation();
                                }
                                _ => {}
                            }
                        }))
                        .occlude()
                        .w(px(560.))
                        .gap_2()
                        .p_2()
                        .rounded_lg()
                        .bg(theme::panel())
                        .border_1()
                        .border_color(theme::surface1())
                        .child(
                            div()
                                .px_2()
                                .text_xs()
                                .text_color(theme::subtext0())
                                .child(title),
                        )
                        .child(Input::new(&self.qo_input).small())
                        .child(list)
                        .when(items.is_empty(), |el| {
                            el.child(
                                div()
                                    .px_3()
                                    .py_2()
                                    .text_sm()
                                    .text_color(theme::subtext0())
                                    .child("Ничего не найдено"),
                            )
                        }),
                )
        });

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
            .on_action(cx.listener(|this, _: &NewNote, window, cx| {
                this.new_note(window, cx);
            }))
            .on_action(cx.listener(|this, _: &NewTab, window, cx| {
                this.new_start_tab(window, cx);
            }))
            .on_action(cx.listener(|this, _: &CloseTab, window, cx| {
                this.close_active_tab(window, cx);
            }))
            .on_action(cx.listener(|this, _: &QuickOpen, window, cx| {
                this.open_quick(false, window, cx);
            }))
            .on_action(cx.listener(|this, _: &CommandPalette, window, cx| {
                this.open_quick(true, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ZoomIn, window, cx| this.zoom(0.1, window, cx)))
            .on_action(cx.listener(|this, _: &ZoomOut, window, cx| this.zoom(-0.1, window, cx)))
            .on_action(cx.listener(|this, _: &ZoomReset, window, cx| {
                this.set_scale(1.0, window, cx);
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
                    .when(self.sidebar_visible, |el| el.child(sidebar))
                    .child(right_pane),
            )
            .child(status_bar)
            .children(confirm_overlay)
            .children(quick_overlay)
    }
}

fn main() {
    let app = gpui_kit::application().with_assets(Assets);

    app.run(move |cx| {
        gpui_kit::init(cx);
        normalize_config();
        let cfg = load_config();
        theme::configure(cfg.window_opacity, &cfg.colors);
        theme::set_flavor(&cfg.theme);
        theme::change(cx);
        cx.bind_keys([
            KeyBinding::new("ctrl-f", FindInNote, Some("NoteSync")),
            KeyBinding::new("ctrl-shift-v", TogglePreview, Some("NoteSync")),
            KeyBinding::new("escape", Dismiss, Some("NoteSync")),
            KeyBinding::new("ctrl-n", NewNote, Some("NoteSync")),
            KeyBinding::new("ctrl-t", NewTab, Some("NoteSync")),
            KeyBinding::new("ctrl-w", CloseTab, Some("NoteSync")),
            KeyBinding::new("ctrl-p", QuickOpen, Some("NoteSync")),
            KeyBinding::new("ctrl-b", ToggleSidebar, Some("NoteSync")),
            KeyBinding::new("ctrl-shift-p", CommandPalette, Some("NoteSync")),
            KeyBinding::new("ctrl-=", ZoomIn, Some("NoteSync")),
            KeyBinding::new("ctrl-+", ZoomIn, Some("NoteSync")),
            KeyBinding::new("ctrl-shift-=", ZoomIn, Some("NoteSync")),
            KeyBinding::new("ctrl--", ZoomOut, Some("NoteSync")),
            KeyBinding::new("ctrl-_", ZoomOut, Some("NoteSync")),
            KeyBinding::new("ctrl-0", ZoomReset, Some("NoteSync")),
            KeyBinding::new("alt-1", Tab1, Some("NoteSync")),
            KeyBinding::new("alt-2", Tab2, Some("NoteSync")),
            KeyBinding::new("alt-3", Tab3, Some("NoteSync")),
            KeyBinding::new("alt-4", Tab4, Some("NoteSync")),
            KeyBinding::new("alt-5", Tab5, Some("NoteSync")),
            KeyBinding::new("alt-6", Tab6, Some("NoteSync")),
            KeyBinding::new("alt-7", Tab7, Some("NoteSync")),
            KeyBinding::new("alt-8", Tab8, Some("NoteSync")),
            KeyBinding::new("alt-9", Tab9, Some("NoteSync")),
            KeyBinding::new("alt-0", Tab10, Some("NoteSync")),
            // Внутри окна быстрого выбора (стрелки там забирает поле ввода)
            KeyBinding::new("ctrl-n", QoNext, Some("QuickOpen")),
            KeyBinding::new("ctrl-j", QoNext, Some("QuickOpen")),
            KeyBinding::new("ctrl-p", QoPrev, Some("QuickOpen")),
            KeyBinding::new("ctrl-k", QoPrev, Some("QuickOpen")),
        ]);

        let window_options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(1080.), px(700.)), cx)),
            titlebar: Some(TitlebarOptions {
                title: Some("NoteSync".into()),
                ..Default::default()
            }),
            window_background: window_bg(&cfg),
            // По app_id композитор находит окно (например, правило размытия в niri).
            app_id: Some("notesync".into()),
            ..Default::default()
        };

        gpui_kit::open_window(window_options, cx, |window, cx| {
            cx.new(|cx| NoteSyncApp::new(window, cx))
        })
        .expect("Failed to open window");
    });
}
