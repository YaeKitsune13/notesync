use iced::widget::{
    button, column, container, horizontal_space, row, scrollable, text, text_editor, text_input,
    vertical_rule,
};
use iced::{
    Alignment, Background, Border, Color, Element, Font, Length, Subscription, Task, Theme,
};
use lucide_icons::{Icon, LUCIDE_FONT_BYTES};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

// ---------- палитра Catppuccin Mocha ----------

mod palette {
    use iced::Color;

    pub const CRUST: Color = Color::from_rgb(0.067, 0.067, 0.106); // #11111b
    pub const MANTLE: Color = Color::from_rgb(0.094, 0.094, 0.145); // #181825
    pub const BASE: Color = Color::from_rgb(0.118, 0.118, 0.180); // #1e1e2e
    pub const SURFACE0: Color = Color::from_rgb(0.192, 0.196, 0.267); // #313244
    pub const SURFACE1: Color = Color::from_rgb(0.271, 0.278, 0.353); // #45475a
    pub const OVERLAY0: Color = Color::from_rgb(0.424, 0.439, 0.525); // #6c7086
    pub const SUBTEXT0: Color = Color::from_rgb(0.651, 0.678, 0.784); // #a6adc8
    pub const TEXT: Color = Color::from_rgb(0.804, 0.839, 0.957); // #cdd6f4
    pub const BLUE: Color = Color::from_rgb(0.537, 0.706, 0.980); // #89b4fa
    pub const LAVENDER: Color = Color::from_rgb(0.706, 0.745, 0.996); // #b4befe
    pub const GREEN: Color = Color::from_rgb(0.651, 0.890, 0.631); // #a6e3a1
    pub const PEACH: Color = Color::from_rgb(0.980, 0.702, 0.529); // #fab387
    pub const RED: Color = Color::from_rgb(0.953, 0.545, 0.659); // #f38ba8
}

// ---------- модели API ----------

#[derive(Clone, Debug, Deserialize)]
struct HealthDto {
    name: String,
    version: String,
}

#[derive(Clone, Debug, Deserialize)]
struct ServerNote {
    id: String,
    body: String,
    rev: i64,
    deleted: bool,
    conflicts: i64,
}

#[derive(Clone, Debug, Deserialize)]
struct ListResponse {
    seq: i64,
    notes: Vec<ServerNote>,
}

#[derive(Serialize)]
struct PutRequest {
    body: String,
    base_rev: i64,
    device_id: String,
    edit_id: String,
}

#[derive(Clone, Debug, Deserialize)]
struct MutateResponse {
    status: String,
    rev: i64,
    conflict_version_id: Option<i64>,
}

#[derive(Clone, Debug, Deserialize)]
struct NoteVersionDto {
    id: i64,
    rev: i64,
    body: String,
    device_id: String,
    created_at: String,
    is_conflict: bool,
    resolved: bool,
}

#[derive(Clone, Debug, Deserialize)]
struct VersionsResponse {
    versions: Vec<NoteVersionDto>,
}

#[derive(Serialize)]
struct ResolveRequest {
    action: String,
    device_id: String,
}

#[derive(Clone, Debug, Deserialize)]
struct LogEntryDto {
    id: i64,
    ts: String,
    device_id: String,
    note_id: String,
    event: String,
    details: String,
}

#[derive(Clone, Debug, Deserialize)]
struct LogResponse {
    log: Vec<LogEntryDto>,
}

#[derive(Clone, Debug)]
struct NoteItem {
    id: String,
    body: String,
    base_rev: i64,
    dirty: bool,
    has_conflict: bool,
    edit_id: String,
}

impl NoteItem {
    fn title(&self) -> String {
        self.body
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or("Без названия")
            .to_string()
    }

    fn preview(&self) -> String {
        self.body
            .lines()
            .skip(1)
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .take(2)
            .collect::<Vec<_>>()
            .join(" ")
    }
}

// ---------- конфигурация ----------

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ClientConfig {
    server_url: String,
    token: String,
    device_id: String,
}

fn config_path() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".config/notesync-desktop.json")
    } else {
        PathBuf::from("notesync-desktop.json")
    }
}

fn load_config() -> ClientConfig {
    if let Ok(raw) = std::fs::read_to_string(config_path()) {
        if let Ok(cfg) = serde_json::from_str::<ClientConfig>(&raw) {
            return cfg;
        }
    }
    ClientConfig {
        server_url: std::env::var("NOTESYNC_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:8787".to_string()),
        token: std::env::var("NOTESYNC_TOKEN").unwrap_or_default(),
        device_id: format!("pc-{}", &uuid::Uuid::new_v4().to_string()[..8]),
    }
}

fn save_config(cfg: &ClientConfig) {
    let path = config_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(cfg) {
        let _ = std::fs::write(path, json);
    }
}

// ---------- UI хелперы для иконок и кнопок ----------

fn icon<'a>(ic: Icon, size: f32) -> text::Text<'a> {
    text(ic.unicode().to_string())
        .font(Font::with_name("lucide"))
        .size(size)
}

fn ghost_btn<'a>(
    content: impl Into<Element<'a, Message>>,
    msg: Message,
) -> button::Button<'a, Message> {
    button(content)
        .padding([6, 10])
        .style(|_theme, status| {
            let bg = match status {
                button::Status::Hovered => Some(Background::Color(palette::SURFACE0)),
                button::Status::Pressed => Some(Background::Color(palette::SURFACE1)),
                _ => None,
            };
            button::Style {
                background: bg,
                text_color: palette::TEXT,
                border: Border {
                    radius: 8.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            }
        })
        .on_press(msg)
}

fn primary_btn<'a>(
    content: impl Into<Element<'a, Message>>,
    msg: Message,
) -> button::Button<'a, Message> {
    button(content)
        .padding([7, 14])
        .style(|_theme, status| {
            let bg = match status {
                button::Status::Hovered => palette::LAVENDER,
                _ => palette::BLUE,
            };
            button::Style {
                background: Some(Background::Color(bg)),
                text_color: palette::CRUST,
                border: Border {
                    radius: 8.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            }
        })
        .on_press(msg)
}

fn danger_btn<'a>(
    content: impl Into<Element<'a, Message>>,
    msg: Message,
) -> button::Button<'a, Message> {
    button(content)
        .padding([6, 10])
        .style(|_theme, status| {
            let bg = match status {
                button::Status::Hovered => Some(Background::Color(Color::from_rgba(
                    0.953, 0.545, 0.659, 0.18,
                ))),
                _ => Some(Background::Color(Color::from_rgba(
                    0.953, 0.545, 0.659, 0.10,
                ))),
            };
            button::Style {
                background: bg,
                text_color: palette::RED,
                border: Border {
                    radius: 8.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            }
        })
        .on_press(msg)
}

// ---------- приложение ----------

#[derive(Clone, Debug, PartialEq)]
enum RightView {
    Editor,
    Settings,
    Conflicts(String),
    Log,
}

struct DesktopApp {
    cfg: ClientConfig,
    http: reqwest::Client,

    notes: HashMap<String, NoteItem>,
    order: Vec<String>,
    selected_id: Option<String>,
    search_query: String,
    editor: text_editor::Content,

    right_view: RightView,
    url_input: String,
    token_input: String,
    health_status: String,

    versions: Vec<NoteVersionDto>,
    versions_loading: bool,
    log_entries: Vec<LogEntryDto>,
    log_loading: bool,

    last_seq: i64,
    status: String,
    last_edit_at: Option<Instant>,
}

#[derive(Debug, Clone)]
enum Message {
    SelectNote(String),
    NewNote,
    DeleteSelected,
    SearchChanged(String),
    EditorAction(text_editor::Action),
    SyncNow,
    Tick,

    OpenSettings,
    CloseRightView,
    UrlChanged(String),
    TokenChanged(String),
    CheckHealth,
    HealthChecked(Result<HealthDto, String>),
    SaveSettings,

    OpenConflicts(String),
    VersionsLoaded(Result<VersionsResponse, String>),
    ResolveConflict {
        note_id: String,
        version_id: i64,
        action: String,
    },
    ConflictResolved(Result<MutateResponse, String>),
    OpenLog,
    LogLoaded(Result<LogResponse, String>),

    Pulled(Result<ListResponse, String>),
    Pushed {
        id: String,
        edit_id: String,
        res: Result<MutateResponse, String>,
    },
    Deleted {
        res: Result<MutateResponse, String>,
    },
}

impl DesktopApp {
    fn new() -> (Self, Task<Message>) {
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

        let app = Self {
            url_input: cfg.server_url.clone(),
            token_input: cfg.token.clone(),
            cfg,
            http,
            notes: HashMap::new(),
            order: Vec::new(),
            selected_id: None,
            search_query: String::new(),
            editor: text_editor::Content::new(),
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
        };

        let cmd = app.pull_task();
        (app, cmd)
    }

    fn base_url(&self) -> String {
        let raw = self.cfg.server_url.trim().trim_end_matches('/');
        if raw.starts_with("http://") || raw.starts_with("https://") {
            raw.to_string()
        } else {
            format!("http://{raw}")
        }
    }

    fn pull_task(&self) -> Task<Message> {
        if self.cfg.token.trim().is_empty() || self.cfg.server_url.trim().is_empty() {
            return Task::none();
        }
        let url = format!("{}/notes?since={}", self.base_url(), self.last_seq);
        let auth = format!("Bearer {}", self.cfg.token.trim());
        let client = self.http.clone();

        Task::perform(
            async move {
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
            },
            Message::Pulled,
        )
    }

    fn push_note_task(&self, item: &NoteItem) -> Task<Message> {
        if self.cfg.token.trim().is_empty() || self.cfg.server_url.trim().is_empty() {
            return Task::none();
        }
        let url = format!("{}/notes/{}", self.base_url(), item.id);
        let auth = format!("Bearer {}", self.cfg.token.trim());
        let client = self.http.clone();
        let id = item.id.clone();
        let edit_id = item.edit_id.clone();
        let payload = PutRequest {
            body: item.body.clone(),
            base_rev: item.base_rev,
            device_id: self.cfg.device_id.clone(),
            edit_id: edit_id.clone(),
        };

        Task::perform(
            async move {
                let res = async {
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
                }
                .await;
                Message::Pushed { id, edit_id, res }
            },
            |m| m,
        )
    }

    fn load_versions_task(&self, note_id: &str) -> Task<Message> {
        let url = format!("{}/notes/{note_id}/versions", self.base_url());
        let auth = format!("Bearer {}", self.cfg.token.trim());
        let client = self.http.clone();

        Task::perform(
            async move {
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
            },
            Message::VersionsLoaded,
        )
    }

    fn load_log_task(&self) -> Task<Message> {
        let url = format!("{}/log?limit=100", self.base_url());
        let auth = format!("Bearer {}", self.cfg.token.trim());
        let client = self.http.clone();

        Task::perform(
            async move {
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
            },
            Message::LogLoaded,
        )
    }

    fn update(&mut self, msg: Message) -> Task<Message> {
        match msg {
            Message::SearchChanged(q) => {
                self.search_query = q;
                Task::none()
            }

            Message::OpenSettings => {
                self.right_view = RightView::Settings;
                self.url_input = self.cfg.server_url.clone();
                self.token_input = self.cfg.token.clone();
                self.health_status.clear();
                Task::none()
            }

            Message::CloseRightView => {
                self.right_view = RightView::Editor;
                Task::none()
            }

            Message::UrlChanged(v) => {
                self.url_input = v;
                Task::none()
            }

            Message::TokenChanged(v) => {
                self.token_input = v;
                Task::none()
            }

            Message::CheckHealth => {
                let raw = self.url_input.trim().trim_end_matches('/');
                if raw.is_empty() {
                    self.health_status = "Адрес пуст".into();
                    return Task::none();
                }
                let url = if raw.starts_with("http://") || raw.starts_with("https://") {
                    format!("{raw}/health")
                } else {
                    format!("http://{raw}/health")
                };
                self.health_status = "Проверка...".into();
                let client = self.http.clone();

                Task::perform(
                    async move {
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
                    },
                    Message::HealthChecked,
                )
            }

            Message::HealthChecked(res) => {
                self.health_status = match res {
                    Ok(h) => format!("OK: {} v{}", h.name, h.version),
                    Err(e) => e,
                };
                Task::none()
            }

            Message::SaveSettings => {
                self.cfg.server_url = self.url_input.trim().to_string();
                self.cfg.token = self.token_input.trim().to_string();
                save_config(&self.cfg);
                self.right_view = RightView::Editor;
                self.last_seq = 0;
                self.notes.clear();
                self.order.clear();
                self.selected_id = None;
                self.editor = text_editor::Content::new();
                self.status = "Подключение...".into();
                self.pull_task()
            }

            Message::OpenConflicts(note_id) => {
                self.right_view = RightView::Conflicts(note_id.clone());
                self.versions.clear();
                self.versions_loading = true;
                self.load_versions_task(&note_id)
            }

            Message::VersionsLoaded(res) => {
                self.versions_loading = false;
                match res {
                    Ok(data) => self.versions = data.versions,
                    Err(err) => self.status = err,
                }
                Task::none()
            }

            Message::ResolveConflict {
                note_id,
                version_id,
                action,
            } => {
                let url = format!(
                    "{}/notes/{note_id}/conflicts/{version_id}/resolve",
                    self.base_url()
                );
                let auth = format!("Bearer {}", self.cfg.token.trim());
                let client = self.http.clone();
                let payload = ResolveRequest {
                    action,
                    device_id: self.cfg.device_id.clone(),
                };

                Task::perform(
                    async move {
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
                    },
                    Message::ConflictResolved,
                )
            }

            Message::ConflictResolved(res) => match res {
                Ok(_) => {
                    self.status = "Конфликт разрешён".into();
                    let mut tasks = vec![self.pull_task()];
                    if let RightView::Conflicts(ref note_id) = self.right_view {
                        self.versions_loading = true;
                        tasks.push(self.load_versions_task(note_id));
                    }
                    Task::batch(tasks)
                }
                Err(err) => {
                    self.status = err;
                    Task::none()
                }
            },

            Message::OpenLog => {
                self.right_view = RightView::Log;
                self.log_loading = true;
                self.load_log_task()
            }

            Message::LogLoaded(res) => {
                self.log_loading = false;
                match res {
                    Ok(data) => self.log_entries = data.log,
                    Err(err) => self.status = err,
                }
                Task::none()
            }

            Message::SelectNote(id) => {
                self.right_view = RightView::Editor;
                let mut tasks = Vec::new();
                if let Some(prev_id) = &self.selected_id {
                    if let Some(prev) = self.notes.get(prev_id) {
                        if prev.dirty {
                            tasks.push(self.push_note_task(prev));
                        }
                    }
                }
                self.last_edit_at = None;
                if let Some(note) = self.notes.get(&id) {
                    self.editor = text_editor::Content::with_text(&note.body);
                    self.selected_id = Some(id);
                }
                Task::batch(tasks)
            }

            Message::NewNote => {
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
                self.editor = text_editor::Content::new();
                self.last_edit_at = Some(Instant::now());
                self.status = "Новая заметка...".into();
                Task::none()
            }

            Message::DeleteSelected => {
                let Some(id) = self.selected_id.take() else {
                    return Task::none();
                };
                let Some(note) = self.notes.remove(&id) else {
                    return Task::none();
                };
                self.order.retain(|x| x != &id);
                self.editor = text_editor::Content::new();

                if note.base_rev == 0 {
                    return Task::none();
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

                Task::perform(
                    async move {
                        let res = async {
                            let resp = client
                                .delete(&url)
                                .header("Authorization", auth)
                                .send()
                                .await
                                .map_err(|e| format!("Нет связи: {e}"))?;
                            resp.json::<MutateResponse>()
                                .await
                                .map_err(|e| e.to_string())
                        }
                        .await;
                        Message::Deleted { res }
                    },
                    |m| m,
                )
            }

            Message::EditorAction(action) => {
                let is_edit = action.is_edit();
                self.editor.perform(action);

                if is_edit {
                    if let Some(id) = &self.selected_id {
                        if let Some(note) = self.notes.get_mut(id) {
                            note.body = self.editor.text();
                            note.dirty = true;
                            note.edit_id = uuid::Uuid::new_v4().to_string();
                            self.last_edit_at = Some(Instant::now());
                            self.status = "Изменено...".into();
                        }
                    }
                }
                Task::none()
            }

            Message::SyncNow => {
                let mut tasks = vec![self.pull_task()];
                for item in self.notes.values() {
                    if item.dirty {
                        tasks.push(self.push_note_task(item));
                    }
                }
                Task::batch(tasks)
            }

            Message::Tick => {
                let mut tasks = Vec::new();
                if let Some(t) = self.last_edit_at {
                    if t.elapsed() >= Duration::from_secs(1) {
                        self.last_edit_at = None;
                        if let Some(id) = &self.selected_id {
                            if let Some(note) = self.notes.get(id) {
                                if note.dirty {
                                    self.status = "Сохранение...".into();
                                    tasks.push(self.push_note_task(note));
                                }
                            }
                        }
                    }
                }
                tasks.push(self.pull_task());
                Task::batch(tasks)
            }

            Message::Pulled(Ok(data)) => {
                if data.seq > self.last_seq {
                    self.last_seq = data.seq;
                }
                let mut to_push: Vec<NoteItem> = Vec::new();

                for sn in data.notes {
                    if sn.deleted {
                        let is_dirty = self.notes.get(&sn.id).map(|n| n.dirty).unwrap_or(false);
                        if !is_dirty {
                            self.notes.remove(&sn.id);
                            self.order.retain(|x| x != &sn.id);
                            if self.selected_id.as_deref() == Some(sn.id.as_str()) {
                                self.selected_id = None;
                                self.editor = text_editor::Content::new();
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
                                    self.editor = text_editor::Content::with_text(&local.body);
                                }
                            } else if local.base_rev != sn.rev {
                                local.has_conflict = true;
                                to_push.push(local.clone());
                            }
                        }
                    }
                }

                let push_tasks: Vec<Task<Message>> = to_push
                    .iter()
                    .map(|item| self.push_note_task(item))
                    .collect();

                if self.selected_id.is_none() {
                    if let Some(first_id) = self.order.first().cloned() {
                        if let Some(n) = self.notes.get(&first_id) {
                            self.editor = text_editor::Content::with_text(&n.body);
                            self.selected_id = Some(first_id);
                        }
                    }
                }

                let has_conflicts = self.notes.values().any(|n| n.has_conflict);
                let has_dirty = self.notes.values().any(|n| n.dirty);
                if has_conflicts {
                    self.status = "Есть конфликты!".into();
                } else if !has_dirty {
                    self.status = "Синхронизировано".into();
                }

                Task::batch(push_tasks)
            }

            Message::Pulled(Err(err)) => {
                self.status = err;
                Task::none()
            }

            Message::Pushed { id, edit_id, res } => match res {
                Ok(mut_res) => {
                    if let Some(note) = self.notes.get_mut(&id) {
                        if note.edit_id == edit_id {
                            note.dirty = false;
                            if mut_res.status == "conflict" {
                                note.has_conflict = true;
                                let vid = mut_res.conflict_version_id.unwrap_or(0);
                                self.status = format!("Конфликт! Сохранено как версия #{vid}");
                            } else {
                                note.base_rev = mut_res.rev;
                                self.status = "Синхронизировано".into();
                            }
                        }
                    }
                    self.pull_task()
                }
                Err(err) => {
                    self.status = err;
                    Task::none()
                }
            },

            Message::Deleted { res } => match res {
                Ok(mut_res) => {
                    if mut_res.status == "conflict" {
                        self.status = "Конфликт удаления: заметка изменена на сервере".into();
                    } else {
                        self.status = "Заметка удалена".into();
                    }
                    self.last_seq = 0;
                    self.pull_task()
                }
                Err(err) => {
                    self.status = err;
                    Task::none()
                }
            },
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        iced::time::every(Duration::from_secs(1)).map(|_| Message::Tick)
    }

    fn view(&self) -> Element<'_, Message> {
        // ---------- ЛЕВЫЙ САЙДБАР ----------
        let brand = row![
            icon(Icon::NotebookPen, 18.0).color(palette::BLUE),
            text("NoteSync").size(16).color(palette::TEXT),
            horizontal_space(),
            ghost_btn(icon(Icon::RefreshCw, 15.0), Message::SyncNow),
            ghost_btn(icon(Icon::History, 15.0), Message::OpenLog),
            ghost_btn(icon(Icon::Settings, 15.0), Message::OpenSettings),
        ]
        .spacing(4)
        .align_y(Alignment::Center);

        let new_button = primary_btn(
            row![icon(Icon::Plus, 16.0), text("Новая заметка").size(14)]
                .spacing(6)
                .align_y(Alignment::Center),
            Message::NewNote,
        )
        .width(Length::Fill);

        let search_box = text_input("Поиск заметок...", &self.search_query)
            .on_input(Message::SearchChanged)
            .padding([7, 10])
            .size(13)
            .style(|_theme, _status| text_input::Style {
                background: Background::Color(palette::BASE),
                border: Border {
                    radius: 8.0.into(),
                    width: 1.0,
                    color: palette::SURFACE0,
                },
                icon: palette::OVERLAY0,
                placeholder: palette::OVERLAY0,
                value: palette::TEXT,
                selection: palette::SURFACE1,
            });

        let query_lower = self.search_query.trim().to_lowercase();
        let mut cards_col = column![].spacing(6);

        for id in &self.order {
            if let Some(note) = self.notes.get(id) {
                if !query_lower.is_empty() && !note.body.to_lowercase().contains(&query_lower) {
                    continue;
                }
                let is_selected = matches!(self.right_view, RightView::Editor)
                    && self.selected_id.as_deref() == Some(id.as_str());

                let mut title_row = row![text(note.title()).size(14).color(palette::TEXT)]
                    .spacing(6)
                    .align_y(Alignment::Center);

                if note.has_conflict {
                    title_row = title_row.push(horizontal_space());
                    title_row = title_row.push(icon(Icon::AlertTriangle, 14.0).color(palette::RED));
                } else if note.dirty {
                    title_row = title_row.push(horizontal_space());
                    title_row = title_row.push(text("●").size(10).color(palette::BLUE));
                }

                let preview = note.preview();
                let card_content: Element<'_, Message> = if preview.is_empty() {
                    column![title_row].into()
                } else {
                    column![title_row, text(preview).size(12).color(palette::SUBTEXT0)]
                        .spacing(4)
                        .into()
                };

                let card = button(card_content)
                    .width(Length::Fill)
                    .padding([10, 12])
                    .style(move |_theme, status| {
                        let bg = if is_selected {
                            palette::SURFACE0
                        } else if matches!(status, button::Status::Hovered) {
                            Color::from_rgba(0.192, 0.196, 0.267, 0.55)
                        } else {
                            Color::TRANSPARENT
                        };
                        button::Style {
                            background: Some(Background::Color(bg)),
                            text_color: palette::TEXT,
                            border: Border {
                                radius: 10.0.into(),
                                width: if is_selected { 1.0 } else { 0.0 },
                                color: if note.has_conflict {
                                    palette::RED
                                } else {
                                    palette::BLUE
                                },
                            },
                            ..Default::default()
                        }
                    })
                    .on_press(Message::SelectNote(id.clone()));

                cards_col = cards_col.push(card);
            }
        }

        let sidebar = container(
            column![
                brand,
                new_button,
                search_box,
                scrollable(cards_col).height(Length::Fill)
            ]
            .spacing(10),
        )
        .width(290)
        .height(Length::Fill)
        .padding(14)
        .style(|_| container::Style {
            background: Some(Background::Color(palette::MANTLE)),
            ..Default::default()
        });

        // ---------- ПРАВАЯ РАБОЧАЯ ОБЛАСТЬ ----------
        let right_pane: Element<'_, Message> = match &self.right_view {
            RightView::Settings => {
                let header = row![
                    icon(Icon::Settings, 20.0).color(palette::BLUE),
                    text("Настройки подключения").size(19).color(palette::TEXT),
                    horizontal_space(),
                    ghost_btn(
                        row![icon(Icon::X, 15.0), text("Закрыть").size(13)]
                            .spacing(5)
                            .align_y(Alignment::Center),
                        Message::CloseRightView
                    ),
                ]
                .spacing(8)
                .align_y(Alignment::Center);

                let mut form = column![
                    text("Адрес сервера (HTTP API)")
                        .size(13)
                        .color(palette::SUBTEXT0),
                    text_input("http://127.0.0.1:8787", &self.url_input)
                        .on_input(Message::UrlChanged)
                        .padding(10),
                    text("Токен доступа").size(13).color(palette::SUBTEXT0),
                    text_input("Вставь токен из notes.db.token...", &self.token_input)
                        .on_input(Message::TokenChanged)
                        .padding(10),
                    row![
                        primary_btn(
                            row![icon(Icon::Check, 15.0), text("Сохранить и синхр.").size(13)]
                                .spacing(6)
                                .align_y(Alignment::Center),
                            Message::SaveSettings
                        ),
                        ghost_btn(
                            row![
                                icon(Icon::Activity, 15.0),
                                text("Проверить /health").size(13)
                            ]
                            .spacing(6)
                            .align_y(Alignment::Center),
                            Message::CheckHealth
                        ),
                    ]
                    .spacing(10),
                ]
                .spacing(12)
                .max_width(480);

                if !self.health_status.is_empty() {
                    let color = if self.health_status.starts_with("OK") {
                        palette::GREEN
                    } else {
                        palette::RED
                    };
                    form = form.push(text(&self.health_status).size(13).color(color));
                }

                form = form.push(
                    text(format!("ID устройства: {}", self.cfg.device_id))
                        .size(12)
                        .color(palette::OVERLAY0),
                );

                let card = container(form).padding(20).style(|_| container::Style {
                    background: Some(Background::Color(palette::MANTLE)),
                    border: Border {
                        radius: 12.0.into(),
                        width: 1.0,
                        color: palette::SURFACE0,
                    },
                    ..Default::default()
                });

                container(column![header, card].spacing(18))
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .padding(24)
                    .into()
            }

            RightView::Conflicts(note_id) => {
                let server_body: &str = self
                    .notes
                    .get(note_id)
                    .map(|n| n.body.as_str())
                    .unwrap_or("");

                let header = row![
                    icon(Icon::GitMerge, 20.0).color(palette::RED),
                    text("Разрешение конфликтов").size(19).color(palette::TEXT),
                    horizontal_space(),
                    ghost_btn(
                        row![icon(Icon::ArrowLeft, 15.0), text("К редактору").size(13)]
                            .spacing(5)
                            .align_y(Alignment::Center),
                        Message::CloseRightView
                    ),
                ]
                .spacing(8)
                .align_y(Alignment::Center);

                let unresolved: Vec<&NoteVersionDto> = self
                    .versions
                    .iter()
                    .filter(|v| v.is_conflict && !v.resolved)
                    .collect();

                let mut body_col = column![header].spacing(16);

                if self.versions_loading {
                    body_col =
                        body_col.push(text("Загрузка версий...").size(14).color(palette::SUBTEXT0));
                } else if unresolved.is_empty() {
                    body_col = body_col.push(
                        row![
                            icon(Icon::CheckCircle2, 16.0).color(palette::GREEN),
                            text("Активных конфликтов для этой заметки нет.")
                                .size(14)
                                .color(palette::GREEN),
                        ]
                        .spacing(8)
                        .align_y(Alignment::Center),
                    );
                } else {
                    for ver in unresolved {
                        let nid = note_id.clone();
                        let vid = ver.id;

                        let left_card = container(
                            column![
                                row![
                                    icon(Icon::Server, 14.0).color(palette::BLUE),
                                    text("Текущая на сервере").size(13).color(palette::BLUE),
                                ]
                                .spacing(6)
                                .align_y(Alignment::Center),
                                scrollable(text(server_body).size(14).color(palette::TEXT))
                                    .height(200),
                            ]
                            .spacing(8),
                        )
                        .width(Length::FillPortion(1))
                        .padding(14)
                        .style(|_| container::Style {
                            background: Some(Background::Color(palette::MANTLE)),
                            border: Border {
                                radius: 10.0.into(),
                                width: 1.0,
                                color: palette::SURFACE0,
                            },
                            ..Default::default()
                        });

                        let right_card = container(
                            column![
                                row![
                                    icon(Icon::AlertTriangle, 14.0).color(palette::RED),
                                    text(format!("Конфликт #{} ({})", ver.id, ver.device_id))
                                        .size(13)
                                        .color(palette::RED),
                                ]
                                .spacing(6)
                                .align_y(Alignment::Center),
                                scrollable(text(&ver.body).size(14).color(palette::TEXT))
                                    .height(200),
                            ]
                            .spacing(8),
                        )
                        .width(Length::FillPortion(1))
                        .padding(14)
                        .style(|_| container::Style {
                            background: Some(Background::Color(palette::MANTLE)),
                            border: Border {
                                radius: 10.0.into(),
                                width: 1.0,
                                color: palette::RED,
                            },
                            ..Default::default()
                        });

                        let actions = row![
                            ghost_btn(
                                text("Оставить серверную").size(13),
                                Message::ResolveConflict {
                                    note_id: nid.clone(),
                                    version_id: vid,
                                    action: "keep_server".into(),
                                }
                            ),
                            primary_btn(
                                text("Взять эту").size(13),
                                Message::ResolveConflict {
                                    note_id: nid.clone(),
                                    version_id: vid,
                                    action: "use_this".into(),
                                }
                            ),
                            ghost_btn(
                                text("Сохранить обе").size(13),
                                Message::ResolveConflict {
                                    note_id: nid,
                                    version_id: vid,
                                    action: "keep_both".into(),
                                }
                            ),
                        ]
                        .spacing(8);

                        body_col = body_col.push(
                            column![row![left_card, right_card].spacing(12), actions].spacing(10),
                        );
                    }
                }

                container(scrollable(body_col))
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .padding(24)
                    .into()
            }

            RightView::Log => {
                let header = row![
                    icon(Icon::History, 20.0).color(palette::BLUE),
                    text("Журнал синхронизации").size(19).color(palette::TEXT),
                    horizontal_space(),
                    ghost_btn(
                        row![icon(Icon::RefreshCw, 14.0), text("Обновить").size(13)]
                            .spacing(5)
                            .align_y(Alignment::Center),
                        Message::OpenLog
                    ),
                    ghost_btn(
                        row![icon(Icon::X, 14.0), text("Закрыть").size(13)]
                            .spacing(5)
                            .align_y(Alignment::Center),
                        Message::CloseRightView
                    ),
                ]
                .spacing(8)
                .align_y(Alignment::Center);

                let mut log_col = column![header].spacing(12);

                if self.log_loading {
                    log_col = log_col.push(
                        text("Загрузка журнала...")
                            .size(14)
                            .color(palette::SUBTEXT0),
                    );
                } else if self.log_entries.is_empty() {
                    log_col =
                        log_col.push(text("Записей пока нет.").size(14).color(palette::SUBTEXT0));
                } else {
                    for entry in &self.log_entries {
                        let (ev_icon, ev_color) = match entry.event.as_str() {
                            "conflict" => (Icon::AlertTriangle, palette::RED),
                            "resolve" => (Icon::CheckCircle2, palette::GREEN),
                            "delete" => (Icon::Trash2, palette::PEACH),
                            _ => (Icon::FileEdit, palette::BLUE),
                        };

                        let short_id = if entry.note_id.len() > 8 {
                            &entry.note_id[..8]
                        } else {
                            &entry.note_id
                        };

                        let mut card_col = column![row![
                            icon(ev_icon, 14.0).color(ev_color),
                            text(entry.event.to_uppercase()).size(12).color(ev_color),
                            text(format!("• {} • {}", entry.device_id, entry.ts))
                                .size(12)
                                .color(palette::SUBTEXT0),
                            horizontal_space(),
                            text(format!("#{short_id}"))
                                .size(11)
                                .color(palette::OVERLAY0),
                        ]
                        .spacing(6)
                        .align_y(Alignment::Center)]
                        .spacing(4);

                        if !entry.details.is_empty() {
                            card_col =
                                card_col.push(text(&entry.details).size(13).color(palette::TEXT));
                        }

                        let card = container(card_col)
                            .width(Length::Fill)
                            .padding([10, 14])
                            .style(|_| container::Style {
                                background: Some(Background::Color(palette::MANTLE)),
                                border: Border {
                                    radius: 8.0.into(),
                                    width: 1.0,
                                    color: palette::SURFACE0,
                                },
                                ..Default::default()
                            });

                        log_col = log_col.push(card);
                    }
                }

                container(scrollable(log_col))
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .padding(24)
                    .into()
            }

            RightView::Editor => {
                if let Some(sel_id) = &self.selected_id {
                    let note_info = self.notes.get(sel_id);
                    let rev_text = note_info
                        .map(|n| format!("rev {}", n.base_rev))
                        .unwrap_or_default();
                    let has_conf = note_info.map(|n| n.has_conflict).unwrap_or(false);

                    let conf_btn = if has_conf {
                        danger_btn(
                            row![icon(Icon::AlertTriangle, 14.0), text("Конфликты").size(13)]
                                .spacing(6)
                                .align_y(Alignment::Center),
                            Message::OpenConflicts(sel_id.clone()),
                        )
                    } else {
                        ghost_btn(
                            row![icon(Icon::GitMerge, 14.0), text("Версии").size(13)]
                                .spacing(6)
                                .align_y(Alignment::Center),
                            Message::OpenConflicts(sel_id.clone()),
                        )
                    };

                    let header = row![
                        container(text(rev_text).size(12).color(palette::SUBTEXT0))
                            .padding([4, 10])
                            .style(|_| container::Style {
                                background: Some(Background::Color(palette::SURFACE0)),
                                border: Border {
                                    radius: 6.0.into(),
                                    ..Default::default()
                                },
                                ..Default::default()
                            }),
                        horizontal_space(),
                        conf_btn,
                        danger_btn(
                            row![icon(Icon::Trash2, 14.0), text("Удалить").size(13)]
                                .spacing(5)
                                .align_y(Alignment::Center),
                            Message::DeleteSelected,
                        ),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center);

                    let editor_widget = text_editor(&self.editor)
                        .placeholder(
                            "Первая строка — заголовок\nТекст в Markdown, UTF-8 (汉(hàn)字(zì) / pinyin)...",
                        )
                        .on_action(Message::EditorAction)
                        .height(Length::Fill)
                        .padding(16)
                        .style(|_theme, _status| text_editor::Style {
                            background: Background::Color(palette::MANTLE),
                            border: Border {
                                radius: 12.0.into(),
                                width: 1.0,
                                color: palette::SURFACE0,
                            },
                            icon: palette::OVERLAY0,
                            placeholder: palette::OVERLAY0,
                            value: palette::TEXT,
                            selection: palette::SURFACE1,
                        });

                    container(column![header, editor_widget].spacing(12))
                        .width(Length::Fill)
                        .height(Length::Fill)
                        .padding(20)
                        .into()
                } else {
                    container(
                        column![
                            icon(Icon::FileText, 36.0).color(palette::OVERLAY0),
                            text("Выбери заметку слева или создай новую")
                                .size(15)
                                .color(palette::SUBTEXT0),
                        ]
                        .spacing(12)
                        .align_x(Alignment::Center),
                    )
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .center_x(Length::Fill)
                    .center_y(Length::Fill)
                    .into()
                }
            }
        };

        // ---------- НИЖНЯЯ СТРОКА СТАТУСА ----------
        let (st_icon, status_color) = if self.status.contains("Конфликт")
            || self.status.contains("конфликт")
        {
            (Icon::AlertTriangle, palette::RED)
        } else if self.status.contains("Нет связи") || self.status.contains("Ошибка")
        {
            (Icon::WifiOff, palette::PEACH)
        } else {
            (Icon::CheckCircle2, palette::GREEN)
        };

        let status_bar = container(
            row![
                icon(st_icon, 14.0).color(status_color),
                text(&self.status).size(12).color(status_color),
                horizontal_space(),
                icon(Icon::Server, 13.0).color(palette::OVERLAY0),
                text(format!("{}  •  seq {}", self.base_url(), self.last_seq))
                    .size(12)
                    .color(palette::OVERLAY0),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        )
        .width(Length::Fill)
        .padding([6, 16])
        .style(|_| container::Style {
            background: Some(Background::Color(palette::CRUST)),
            ..Default::default()
        });

        container(column![
            row![sidebar, vertical_rule(1), right_pane].height(Length::Fill),
            status_bar
        ])
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_| container::Style {
            background: Some(Background::Color(palette::BASE)),
            ..Default::default()
        })
        .into()
    }
}

fn main() -> iced::Result {
    iced::application("NoteSync", DesktopApp::update, DesktopApp::view)
        .theme(|_| Theme::CatppuccinMocha)
        .font(LUCIDE_FONT_BYTES)
        .default_font(Font::with_name("Noto Sans CJK SC"))
        .subscription(DesktopApp::subscription)
        .run_with(DesktopApp::new)
}
