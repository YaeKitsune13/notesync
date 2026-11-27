use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Clone, Debug, Deserialize)]
pub struct HealthDto {
    pub name: String,
    pub version: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ServerNote {
    pub id: String,
    pub body: String,
    pub rev: i64,
    pub deleted: bool,
    pub conflicts: i64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ListResponse {
    pub seq: i64,
    pub notes: Vec<ServerNote>,
}

#[derive(Serialize)]
pub struct PutRequest {
    pub body: String,
    pub base_rev: i64,
    pub device_id: String,
    pub edit_id: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct MutateResponse {
    pub status: String,
    pub rev: i64,
    pub conflict_version_id: Option<i64>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct NoteVersionDto {
    pub id: i64,
    #[allow(dead_code)]
    pub rev: i64,
    pub body: String,
    pub device_id: String,
    #[allow(dead_code)]
    pub created_at: String,
    pub is_conflict: bool,
    pub resolved: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct VersionsResponse {
    pub versions: Vec<NoteVersionDto>,
}

#[derive(Serialize)]
pub struct ResolveRequest {
    pub action: String,
    pub device_id: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct LogEntryDto {
    #[allow(dead_code)]
    pub id: i64,
    pub ts: String,
    pub device_id: String,
    pub note_id: String,
    pub event: String,
    pub details: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct LogResponse {
    pub log: Vec<LogEntryDto>,
}

#[derive(Clone, Debug)]
pub struct NoteItem {
    pub id: String,
    pub body: String,
    pub base_rev: i64,
    pub dirty: bool,
    pub has_conflict: bool,
    pub edit_id: String,
}

impl NoteItem {
    pub fn title(&self) -> String {
        self.body
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or("Без названия")
            .to_string()
    }

    pub fn preview(&self) -> String {
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

// Файл настроек ~/.config/notesync-desktop.json. Все ключи необязательны:
// чего нет в файле, берётся значение по умолчанию (файл при запуске дописывается).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClientConfig {
    // подключение
    pub server_url: String,
    pub token: String,
    pub device_id: String,
    // внешний вид
    #[serde(default = "default_theme")]
    pub theme: String, // mocha | macchiato | frappe | latte | имя файла из ~/.config/notesync/themes
    #[serde(default = "default_font_size")]
    pub font_size: f32, // размер текста заметки, px
    #[serde(default = "default_ui_scale")]
    pub ui_scale: f32, // масштаб интерфейса (Ctrl + / Ctrl -), 0.6..2.5
    #[serde(default = "default_opacity")]
    pub window_opacity: f32, // прозрачность фонов, 0.2..1.0
    #[serde(default)]
    pub window_blur: bool, // размытие под окном (если поддерживает композитор)
    #[serde(default = "default_sidebar_width")]
    pub sidebar_width: f32,
    #[serde(default)]
    pub font_family: String, // шрифт интерфейса и превью ("" — по умолчанию)
    #[serde(default)]
    pub editor_font_family: String, // шрифт редактора ("" — по умолчанию)
    #[serde(default = "default_true")]
    pub show_note_preview: bool, // вторая строка с началом текста в списке
    // поведение
    #[serde(default = "default_start_mode")]
    pub start_mode: String, // preview | split | edit
    #[serde(default = "default_true")]
    pub confirm_delete: bool,
    #[serde(default = "default_save_delay")]
    pub save_delay_ms: u64, // пауза в наборе перед отправкой на сервер
    #[serde(default = "default_sync_interval")]
    pub sync_interval_ms: u64, // как часто спрашивать сервер о новых заметках
    // Поверх выбранной темы: {"blue": "#f5c2e7", "base": "#1e1e2ecc"} (8 цифр — с прозрачностью)
    #[serde(default)]
    pub colors: HashMap<String, String>,
}

fn default_theme() -> String {
    "mocha".to_string()
}

fn default_font_size() -> f32 {
    15.0
}

fn default_ui_scale() -> f32 {
    1.0
}

fn default_opacity() -> f32 {
    1.0
}

fn default_sidebar_width() -> f32 {
    260.0
}

fn default_true() -> bool {
    true
}

fn default_start_mode() -> String {
    "preview".to_string()
}

fn default_save_delay() -> u64 {
    300
}

fn default_sync_interval() -> u64 {
    1000
}

impl ClientConfig {
    fn new(server_url: String, token: String, device_id: String) -> Self {
        ClientConfig {
            server_url,
            token,
            device_id,
            theme: default_theme(),
            font_size: default_font_size(),
            ui_scale: default_ui_scale(),
            window_opacity: default_opacity(),
            window_blur: false,
            sidebar_width: default_sidebar_width(),
            font_family: String::new(),
            editor_font_family: String::new(),
            show_note_preview: true,
            start_mode: default_start_mode(),
            confirm_delete: true,
            save_delay_ms: default_save_delay(),
            sync_interval_ms: default_sync_interval(),
            colors: HashMap::new(),
        }
    }
}

pub fn config_path() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".config/notesync-desktop.json")
    } else {
        PathBuf::from("notesync-desktop.json")
    }
}

// Читает настройки; при ошибке в JSON возвращает текст ошибки (файл не трогаем).
pub fn try_load_config() -> Result<ClientConfig, String> {
    let raw = std::fs::read_to_string(config_path())
        .map_err(|e| format!("Не удалось прочитать {}: {e}", config_path().display()))?;
    serde_json::from_str::<ClientConfig>(&raw)
        .map_err(|e| format!("Ошибка в {}: {e}", config_path().display()))
}

pub fn load_config() -> ClientConfig {
    try_load_config().unwrap_or_else(|_| {
        ClientConfig::new(
            std::env::var("NOTESYNC_URL").unwrap_or_else(|_| "http://127.0.0.1:8787".to_string()),
            std::env::var("NOTESYNC_TOKEN").unwrap_or_default(),
            format!("pc-{}", &uuid::Uuid::new_v4().to_string()[..8]),
        )
    })
}

// Дописывает в файл настроек недостающие ключи (только если файл читается без ошибок).
pub fn normalize_config() {
    if let Ok(cfg) = try_load_config() {
        save_config(&cfg);
    }
}

pub fn save_config(cfg: &ClientConfig) {
    let path = config_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(cfg) {
        let _ = std::fs::write(path, json);
    }
}
