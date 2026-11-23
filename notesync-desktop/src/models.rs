use serde::{Deserialize, Serialize};
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClientConfig {
    pub server_url: String,
    pub token: String,
    pub device_id: String,
}

pub fn config_path() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".config/notesync-desktop.json")
    } else {
        PathBuf::from("notesync-desktop.json")
    }
}

pub fn load_config() -> ClientConfig {
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

pub fn save_config(cfg: &ClientConfig) {
    let path = config_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(cfg) {
        let _ = std::fs::write(path, json);
    }
}
