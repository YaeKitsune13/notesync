mod models;
mod theme;

use gpui_kit::assets::Assets;
use gpui_kit::component::{
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
    menu::{ContextMenuExt as _, PopupMenuItem},
    text::TextView,
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

#[derive(Clone, Debug, PartialEq)]
enum RightView {
    Editor,
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
    reading: bool,

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
        let editor_input = cx.new(|cx| {
            TextareaState::new(window, cx).placeholder(
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
            reading: false,
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
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            let ok = this.update(cx, |app, cx| {
                if let Some(t) = app.last_edit_at {
                    if t.elapsed() >= Duration::from_secs(1) {
                        app.last_edit_at = None;
                        if let Some(id) = &app.selected_id {
                            if let Some(note) = app.notes.get(id).cloned() {
                                if note.dirty {
                                    app.status = "Сохранение...".into();
                                    app.spawn_push(note, cx);
                                }
                            }
                        }
                    }
                }
                app.spawn_pull(cx);
            });
            if ok.is_err() {
                break;
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
                            } else if local.base_rev != sn.rev {
                                local.has_conflict = true;
                                to_push.push(local.clone());
                            }
                        }
                    }
                }

                for item in to_push {
                    self.spawn_push(item, cx);
                }

                if self.selected_id.is_none() {
                    if let Some(first_id) = self.order.first().cloned() {
                        self.selected_id = Some(first_id);
                        need_sync_editor = true;
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

    fn spawn_push(&self, item: NoteItem, cx: &mut Context<Self>) {
        if self.cfg.token.trim().is_empty() || self.cfg.server_url.trim().is_empty() {
            return;
        }
        let url = format!("{}/notes/{}", self.base_url(), item.id);
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
                match res {
                    Ok(mut_res) => {
                        if let Some(note) = app.notes.get_mut(&id) {
                            if note.edit_id == edit_id {
                                note.dirty = false;
                                if mut_res.status == "conflict" {
                                    note.has_conflict = true;
                                    let vid = mut_res.conflict_version_id.unwrap_or(0);
                                    app.status = format!("Конфликт! Сохранено как версия #{vid}");
                                } else {
                                    note.base_rev = mut_res.rev;
                                    app.status = "Синхронизировано".into();
                                }
                            }
                        }
                        app.spawn_pull(cx);
                    }
                    Err(err) => app.status = err,
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn select_note(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.right_view = RightView::Editor;
        if let Some(prev_id) = &self.selected_id {
            if let Some(prev) = self.notes.get(prev_id).cloned() {
                if prev.dirty {
                    self.spawn_push(prev, cx);
                }
            }
        }
        self.last_edit_at = None;
        if let Some(body) = self.notes.get(&id).map(|n| n.body.clone()) {
            self.reading = !body.trim().is_empty();
            self.selected_id = Some(id);
            self.set_editor_text(&body, window, cx);
        }
        cx.notify();
    }

    fn new_note(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
        self.selected_id = Some(id);
        self.reading = false;
        self.set_editor_text("", window, cx);
        self.last_edit_at = Some(Instant::now());
        self.status = "Новая заметка...".into();
        cx.notify();
    }

    // Удаление любой заметки по id (меню по ПКМ в списке и кнопка «Удалить»).
    fn delete_note(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(note) = self.notes.remove(&id) else {
            return;
        };
        self.order.retain(|x| x != &id);
        if self.selected_id.as_deref() == Some(id.as_str()) {
            self.selected_id = None;
            self.set_editor_text("", window, cx);
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
        let mut cards = v_flex().gap_1p5().w_full();
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
                    .px_3()
                    .py_2p5()
                    .gap_1()
                    .rounded_lg()
                    .cursor_pointer()
                    .when(is_selected, |el| {
                        el.bg(theme::surface0())
                            .border_1()
                            .border_color(if note.has_conflict {
                                theme::red()
                            } else {
                                theme::blue()
                            })
                    })
                    .when(!is_selected, |el| {
                        el.hover(|s| s.bg(theme::surface0().opacity(0.55)))
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.select_note(note_id.clone(), window, cx);
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
                    .when(!preview.is_empty(), |el| {
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
                            .on_click(move |_, _window, cx| {
                                let id = id.clone();
                                view.update(cx, |this, cx| {
                                    this.pending_delete = Some(id);
                                    cx.notify();
                                });
                            }),
                    )
                });

                cards = cards.child(card);
            }
        }

        let sidebar = v_flex()
            .w(px(290.))
            .h_full()
            .p_3p5()
            .gap_2p5()
            .bg(theme::mantle())
            .border_r_1()
            .border_color(theme::surface0())
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .items_center()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(Icon::new(IconName::BookOpen).text_color(theme::blue()))
                            .child(
                                div()
                                    .text_base()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme::text())
                                    .child("NoteSync"),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                Button::new("sync-btn")
                                    .ghost()
                                    .small()
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
                                    .small()
                                    .icon(IconName::Info)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.open_log(cx);
                                    })),
                            )
                            .child(
                                Button::new("settings-btn")
                                    .ghost()
                                    .small()
                                    .icon(IconName::Settings)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.open_settings(window, cx);
                                    })),
                            ),
                    ),
            )
            .child(
                Button::new("new-note-btn")
                    .primary()
                    .w_full()
                    .icon(IconName::Plus)
                    .label("Новая заметка")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.new_note(window, cx);
                    })),
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

        // ---------- ПРАВАЯ ОБЛАСТЬ ----------
        let right_pane: AnyElement = match &self.right_view {
            RightView::Settings => v_flex()
                .flex_1()
                .h_full()
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

            RightView::Editor => {
                if let Some(sel_id) = self.selected_id.clone() {
                    let note_info = self.notes.get(&sel_id);
                    let rev_text = note_info
                        .map(|n| format!("rev {}", n.base_rev))
                        .unwrap_or_default();
                    let has_conf = note_info.map(|n| n.has_conflict).unwrap_or(false);

                    let reading = self.reading;
                    let read_btn = Button::new("toggle-read")
                        .ghost()
                        .small()
                        .icon(Icon::new(if reading {
                            gpui_kit::assets::IconName::Pencil
                        } else {
                            gpui_kit::assets::IconName::BookOpen
                        }))
                        .label(if reading {
                            "Править"
                        } else {
                            "Читать"
                        })
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.reading = !this.reading;
                            cx.notify();
                        }));

                    let conf_btn = if has_conf {
                        Button::new("open-conflicts")
                            .danger()
                            .small()
                            .icon(IconName::TriangleAlert)
                            .label("Конфликты")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.open_conflicts(sel_id.clone(), cx);
                            }))
                    } else {
                        Button::new("open-versions")
                            .ghost()
                            .small()
                            .icon(IconName::Replace)
                            .label("Версии")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.open_conflicts(sel_id.clone(), cx);
                            }))
                    };

                    v_flex()
                        .flex_1()
                        .h_full()
                        .p_5()
                        .gap_3()
                        .child(
                            h_flex()
                                .w_full()
                                .justify_between()
                                .items_center()
                                .child(
                                    div()
                                        .px_2p5()
                                        .py_1()
                                        .rounded_md()
                                        .bg(theme::surface0())
                                        .text_xs()
                                        .text_color(theme::subtext0())
                                        .child(rev_text),
                                )
                                .child(h_flex().gap_2().child(read_btn).child(conf_btn)),
                        )
                        .child(
                            v_flex()
                                .flex_1()
                                .min_h_0()
                                .w_full()
                                .p_4()
                                .rounded_xl()
                                .bg(theme::mantle())
                                .border_1()
                                .border_color(theme::surface0())
                                .child(div().flex_1().min_h_0().w_full().child(if self.reading {
                                    let note_id = self.selected_id.clone().unwrap_or_default();
                                    let body = self.editor_input.read(cx).value().to_string();
                                    TextView::markdown(
                                        SharedString::from(format!("reader-{note_id}")),
                                        body,
                                    )
                                    .selectable(true)
                                    .scrollable(true)
                                    .size_full()
                                    .into_any_element()
                                } else {
                                    Textarea::new(&self.editor_input)
                                        .appearance(false)
                                        .bordered(false)
                                        .h_full()
                                        .into_any_element()
                                })),
                        )
                        .into_any_element()
                } else {
                    v_flex()
                        .flex_1()
                        .h_full()
                        .items_center()
                        .justify_center()
                        .gap_3()
                        .child(Icon::new(IconName::File).text_color(theme::overlay0()))
                        .child(
                            div()
                                .text_sm()
                                .text_color(theme::subtext0())
                                .child("Выбери заметку слева или создай новую"),
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

        let status_bar = h_flex()
            .w_full()
            .px_4()
            .py_1p5()
            .justify_between()
            .items_center()
            .bg(theme::crust())
            .border_t_1()
            .border_color(theme::surface0())
            .child(
                div()
                    .text_xs()
                    .text_color(status_color)
                    .child(self.status.clone()),
            )
            .child(div().text_xs().text_color(theme::overlay0()).child(format!(
                "{}  •  seq {}",
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

        v_flex()
            .relative()
            .size_full()
            .bg(theme::base())
            .text_color(theme::text())
            .child(
                h_flex()
                    .flex_1()
                    .w_full()
                    .overflow_hidden()
                    .child(sidebar)
                    .child(right_pane),
            )
            .child(status_bar)
            .children(confirm_overlay)
    }
}

fn main() {
    let app = gpui_kit::application().with_assets(Assets);

    app.run(move |cx| {
        gpui_kit::init(cx);
        gpui_kit::component::Theme::change(gpui_kit::component::ThemeMode::Dark, None, cx);
        theme::apply(cx);

        let window_options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(1080.), px(700.)), cx)),
            titlebar: Some(TitlebarOptions {
                title: Some("NoteSync".into()),
                ..Default::default()
            }),
            ..Default::default()
        };

        gpui_kit::open_window(window_options, cx, |window, cx| {
            cx.new(|cx| NoteSyncApp::new(window, cx))
        })
        .expect("Failed to open window");
    });
}
