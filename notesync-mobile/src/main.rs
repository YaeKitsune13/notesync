use dioxus::prelude::*;
use mdns_sd::{ServiceDaemon, ServiceEvent};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

// ---------- модели данных ----------

#[derive(Clone, Debug, PartialEq)]
struct LocalNote {
    id: String,
    body: String,
    base_rev: i64,
    dirty: bool,
    has_conflict: bool,
    deleted: bool,
    edit_id: String,
}

#[derive(Clone, Debug, PartialEq)]
struct Settings {
    manual_url: String,
    discovered_url: String,
    token: String,
    device_id: String,
    last_seq: i64,
}

impl Settings {
    fn active_url(&self) -> String {
        let m = normalize_url(&self.manual_url);
        if !m.is_empty() {
            return m;
        }
        normalize_url(&self.discovered_url)
    }
}

fn normalize_url(raw: &str) -> String {
    let s = raw.trim().trim_end_matches('/');
    if s.is_empty() {
        return String::new();
    }
    if s.starts_with("http://") || s.starts_with("https://") {
        s.to_string()
    } else {
        format!("http://{s}")
    }
}

#[derive(Clone, Debug, PartialEq)]
enum SyncStatus {
    Idle,
    Syncing,
    Ok,
    Offline(String),
    ConflictNotice(String),
}

#[derive(Deserialize)]
struct HealthDto {
    name: String,
    version: String,
}

#[derive(Deserialize)]
struct ServerNote {
    id: String,
    body: String,
    rev: i64,
    deleted: bool,
    conflicts: i64,
}

#[derive(Deserialize)]
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

#[derive(Deserialize)]
struct MutateResponse {
    status: String,
    rev: i64,
    conflict_version_id: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
struct NoteVersionDto {
    id: i64,
    rev: i64,
    body: String,
    device_id: String,
    created_at: String,
    is_conflict: bool,
    resolved: bool,
}

#[derive(Deserialize)]
struct VersionsResponse {
    versions: Vec<NoteVersionDto>,
}

#[derive(Serialize)]
struct ResolveRequest {
    action: String,
    device_id: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
struct LogEntryDto {
    id: i64,
    ts: String,
    device_id: String,
    note_id: String,
    event: String,
    details: String,
}

#[derive(Deserialize)]
struct LogResponse {
    log: Vec<LogEntryDto>,
}

// ---------- локальная SQLite ----------

fn db_path() -> PathBuf {
    if let Ok(dir) = std::env::var("HOME") {
        let p = PathBuf::from(dir);
        if p.exists() {
            return p.join("notesync_mobile.db");
        }
    }
    PathBuf::from("/data/data/com.notesync.mobile/files/notesync_mobile.db")
}

fn open_db() -> Connection {
    let path = db_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let conn = Connection::open(&path).expect("не удалось открыть локальную БД");
    conn.execute_batch(
        "
        PRAGMA journal_mode=WAL;
        CREATE TABLE IF NOT EXISTS notes (
            id TEXT PRIMARY KEY,
            body TEXT NOT NULL,
            base_rev INTEGER NOT NULL DEFAULT 0,
            dirty INTEGER NOT NULL DEFAULT 0,
            has_conflict INTEGER NOT NULL DEFAULT 0,
            deleted INTEGER NOT NULL DEFAULT 0,
            pending_edit_id TEXT NOT NULL DEFAULT ''
        );
        CREATE TABLE IF NOT EXISTS meta (
            k TEXT PRIMARY KEY,
            v TEXT NOT NULL
        );
        ",
    )
    .expect("не удалось создать таблицы");

    let has_dev: Option<String> = conn
        .query_row("SELECT v FROM meta WHERE k = 'device_id'", [], |r| r.get(0))
        .optional()
        .unwrap_or(None);
    if has_dev.is_none() {
        let dev = format!("android-{}", &uuid::Uuid::new_v4().to_string()[..8]);
        let _ = conn.execute(
            "INSERT INTO meta (k, v) VALUES ('device_id', ?1)",
            params![dev],
        );
    }
    conn
}

fn get_meta(conn: &Connection, key: &str, default: &str) -> String {
    conn.query_row("SELECT v FROM meta WHERE k = ?1", params![key], |r| {
        r.get(0)
    })
    .optional()
    .unwrap_or(None)
    .unwrap_or_else(|| default.to_string())
}

fn set_meta(conn: &Connection, key: &str, val: &str) {
    let _ = conn.execute(
        "INSERT INTO meta (k, v) VALUES (?1, ?2) ON CONFLICT(k) DO UPDATE SET v = excluded.v",
        params![key, val],
    );
}

fn load_settings(conn: &Connection) -> Settings {
    let old_url = get_meta(conn, "server_url", "");
    let manual = get_meta(conn, "manual_url", &old_url);
    Settings {
        manual_url: manual,
        discovered_url: get_meta(conn, "discovered_url", ""),
        token: get_meta(conn, "token", ""),
        device_id: get_meta(conn, "device_id", "android-device"),
        last_seq: get_meta(conn, "last_seq", "0").parse().unwrap_or(0),
    }
}

fn load_notes(conn: &Connection) -> Vec<LocalNote> {
    let mut stmt = conn
        .prepare("SELECT id, body, base_rev, dirty, has_conflict, deleted, pending_edit_id FROM notes WHERE deleted = 0 ORDER BY rowid DESC")
        .unwrap();
    stmt.query_map([], |r| {
        Ok(LocalNote {
            id: r.get(0)?,
            body: r.get(1)?,
            base_rev: r.get(2)?,
            dirty: r.get::<_, i64>(3)? != 0,
            has_conflict: r.get::<_, i64>(4)? != 0,
            deleted: r.get::<_, i64>(5)? != 0,
            edit_id: r.get(6)?,
        })
    })
    .unwrap()
    .filter_map(Result::ok)
    .collect()
}

fn save_local_edit(conn: &Connection, id: &str, body: &str) {
    let edit_id = uuid::Uuid::new_v4().to_string();
    let _ = conn.execute(
        "INSERT INTO notes (id, body, base_rev, dirty, has_conflict, deleted, pending_edit_id)
         VALUES (?1, ?2, 0, 1, 0, 0, ?3)
         ON CONFLICT(id) DO UPDATE SET body = excluded.body, dirty = 1, deleted = 0, pending_edit_id = excluded.pending_edit_id",
        params![id, body, edit_id],
    );
}

fn delete_local_note(conn: &Connection, id: &str, base_rev: i64) -> bool {
    if base_rev == 0 {
        let _ = conn.execute("DELETE FROM notes WHERE id = ?1", params![id]);
        false
    } else {
        let edit_id = uuid::Uuid::new_v4().to_string();
        let _ = conn.execute(
            "UPDATE notes SET deleted = 1, dirty = 1, pending_edit_id = ?2 WHERE id = ?1",
            params![id, edit_id],
        );
        true
    }
}

// ---------- mDNS-поиск сервера ----------

static MDNS_GEN: AtomicU64 = AtomicU64::new(0);

fn start_mdns_browser(
    db: Arc<Mutex<Connection>>,
    mut settings_sig: Signal<Settings>,
    status: Signal<SyncStatus>,
    notes: Signal<Vec<LocalNote>>,
) {
    let gen = MDNS_GEN.fetch_add(1, Ordering::SeqCst) + 1;

    spawn(async move {
        let Ok(mdns) = ServiceDaemon::new() else {
            return;
        };
        let Ok(receiver) = mdns.browse("_notesync._tcp.local.") else {
            let _ = mdns.shutdown();
            return;
        };

        while MDNS_GEN.load(Ordering::SeqCst) == gen {
            let rx = receiver.clone();
            let ev =
                tokio::task::spawn_blocking(move || rx.recv_timeout(Duration::from_millis(500)))
                    .await;

            match ev {
                Ok(Ok(ServiceEvent::ServiceResolved(info))) => {
                    let port = info.get_port();
                    if port == 0 {
                        continue;
                    }
                    let addrs = info.get_addresses();
                    let best = addrs
                        .iter()
                        .find(|a| match a {
                            IpAddr::V4(v4) => !v4.is_loopback() && !v4.is_link_local(),
                            IpAddr::V6(_) => false,
                        })
                        .or_else(|| {
                            addrs.iter().find(|a| {
                                !a.is_loopback()
                                    && match a {
                                        IpAddr::V4(v4) => !v4.is_link_local(),
                                        IpAddr::V6(v6) => !v6.is_unicast_link_local(),
                                    }
                            })
                        });

                    if let Some(ip) = best {
                        let found_url = match ip {
                            IpAddr::V4(v4) => format!("http://{v4}:{port}"),
                            IpAddr::V6(v6) => format!("http://[{v6}]:{port}"),
                        };

                        let should_sync = {
                            let conn = db.lock().unwrap();
                            let prev = load_settings(&conn);
                            let prev_active = prev.active_url();
                            set_meta(&conn, "discovered_url", &found_url);
                            let updated = load_settings(&conn);
                            let new_active = updated.active_url();
                            settings_sig.set(updated);
                            prev.manual_url.trim().is_empty() && prev_active != new_active
                        };
                        if should_sync {
                            trigger_sync(&db, status, notes, settings_sig);
                        }
                    }
                }
                Ok(Ok(_)) => {}
                Ok(Err(e))
                    if e.to_string().contains("timeout") || e.to_string().contains("timed out") => {
                }
                Ok(Err(_)) | Err(_) => break,
            }
        }
        let _ = mdns.shutdown();
    });
}

// ---------- API-запросы (синхронизация, версии, resolve, лог) ----------

async fn check_health(base_url: &str) -> Result<String, String> {
    let url = normalize_url(base_url);
    if url.is_empty() {
        return Err("Адрес пуст".into());
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(4))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(format!("{url}/health"))
        .send()
        .await
        .map_err(|e| format!("Ошибка ({url}): {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("Ошибка HTTP {}", resp.status()));
    }
    let h: HealthDto = resp.json().await.map_err(|e| e.to_string())?;
    Ok(format!("OK: {} v{} ({url})", h.name, h.version))
}

async fn fetch_versions(settings: &Settings, note_id: &str) -> Result<Vec<NoteVersionDto>, String> {
    let url = settings.active_url();
    if url.is_empty() || settings.token.is_empty() {
        return Err("Сервер или токен не заданы".into());
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(format!("{url}/notes/{note_id}/versions"))
        .header("Authorization", format!("Bearer {}", settings.token))
        .send()
        .await
        .map_err(|e| format!("Нет связи: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("Ошибка HTTP {}", resp.status()));
    }
    let data: VersionsResponse = resp.json().await.map_err(|e| e.to_string())?;
    Ok(data.versions)
}

async fn resolve_conflict_api(
    settings: &Settings,
    note_id: &str,
    version_id: i64,
    action: &str,
) -> Result<(), String> {
    let url = settings.active_url();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;
    let payload = ResolveRequest {
        action: action.to_string(),
        device_id: settings.device_id.clone(),
    };
    let resp = client
        .post(format!(
            "{url}/notes/{note_id}/conflicts/{version_id}/resolve"
        ))
        .header("Authorization", format!("Bearer {}", settings.token))
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("Нет связи: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("Ошибка HTTP {}", resp.status()));
    }
    Ok(())
}

async fn fetch_log(settings: &Settings) -> Result<Vec<LogEntryDto>, String> {
    let url = settings.active_url();
    if url.is_empty() || settings.token.is_empty() {
        return Err("Сервер или токен не заданы".into());
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(format!("{url}/log?limit=100"))
        .header("Authorization", format!("Bearer {}", settings.token))
        .send()
        .await
        .map_err(|e| format!("Нет связи: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("Ошибка HTTP {}", resp.status()));
    }
    let data: LogResponse = resp.json().await.map_err(|e| e.to_string())?;
    Ok(data.log)
}

async fn sync_with_server(db: Arc<Mutex<Connection>>) -> SyncStatus {
    let (settings, dirty_items) = {
        let conn = db.lock().unwrap();
        let s = load_settings(&conn);
        let mut stmt = conn
            .prepare(
                "SELECT id, body, base_rev, deleted, pending_edit_id FROM notes WHERE dirty = 1",
            )
            .unwrap();
        let items: Vec<(String, String, i64, bool, String)> = stmt
            .query_map([], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get::<_, i64>(3)? != 0,
                    r.get(4)?,
                ))
            })
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        (s, items)
    };

    let url = settings.active_url();
    if url.is_empty() {
        return SyncStatus::Offline("Адрес сервера не задан".into());
    }
    if settings.token.is_empty() {
        return SyncStatus::Offline("Токен не задан".into());
    }

    let client = match reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(4))
        .timeout(Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(e) => return SyncStatus::Offline(format!("Ошибка клиента: {e}")),
    };
    let auth = format!("Bearer {}", settings.token);
    let mut conflict_msg: Option<String> = None;

    // Шаг 1. Pull
    let pull_resp = match client
        .get(format!("{url}/notes?since={}", settings.last_seq))
        .header("Authorization", &auth)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => return SyncStatus::Offline(format!("Нет связи: {e}")),
    };

    if pull_resp.status().as_u16() == 401 {
        return SyncStatus::Offline("Неверный токен (401)".into());
    }
    if !pull_resp.status().is_success() {
        return SyncStatus::Offline(format!("Ошибка сервера: {}", pull_resp.status()));
    }

    let pulled: ListResponse = match pull_resp.json().await {
        Ok(d) => d,
        Err(e) => return SyncStatus::Offline(format!("Ошибка ответа: {e}")),
    };

    {
        let conn = db.lock().unwrap();
        for sn in pulled.notes {
            let local: Option<(i64, bool)> = conn
                .query_row(
                    "SELECT base_rev, dirty FROM notes WHERE id = ?1",
                    params![sn.id],
                    |r| Ok((r.get(0)?, r.get::<_, i64>(1)? != 0)),
                )
                .optional()
                .unwrap_or(None);

            let has_conf = if sn.conflicts > 0 { 1 } else { 0 };
            match local {
                None => {
                    let _ = conn.execute(
                        "INSERT INTO notes (id, body, base_rev, dirty, has_conflict, deleted, pending_edit_id)
                         VALUES (?1, ?2, ?3, 0, ?4, ?5, '')",
                        params![sn.id, sn.body, sn.rev, has_conf, sn.deleted as i64],
                    );
                }
                Some((_, false)) => {
                    let _ = conn.execute(
                        "UPDATE notes SET body = ?2, base_rev = ?3, deleted = ?4, has_conflict = ?5 WHERE id = ?1",
                        params![sn.id, sn.body, sn.rev, sn.deleted as i64, has_conf],
                    );
                }
                Some((local_rev, true)) if local_rev != sn.rev && !sn.deleted => {
                    let _ = conn.execute(
                        "UPDATE notes SET has_conflict = 1 WHERE id = ?1",
                        params![sn.id],
                    );
                }
                _ => {}
            }
        }
        if pulled.seq > settings.last_seq {
            set_meta(&conn, "last_seq", &pulled.seq.to_string());
        }
    }

    // Шаг 2. Push
    let mut pushed_any = false;
    for (id, body, base_rev, deleted, edit_id) in dirty_items {
        pushed_any = true;
        if deleted {
            if base_rev == 0 {
                let conn = db.lock().unwrap();
                let _ = conn.execute("DELETE FROM notes WHERE id = ?1", params![id]);
                continue;
            }
            let del_url = format!(
                "{url}/notes/{id}?base_rev={base_rev}&device_id={}",
                settings.device_id
            );
            let resp = match client
                .delete(&del_url)
                .header("Authorization", &auth)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => return SyncStatus::Offline(format!("Нет связи: {e}")),
            };
            if !resp.status().is_success() {
                return SyncStatus::Offline(format!("Ошибка сервера: {}", resp.status()));
            }
            let res: MutateResponse = match resp.json().await {
                Ok(d) => d,
                Err(e) => return SyncStatus::Offline(format!("Ошибка ответа: {e}")),
            };

            if res.status == "conflict" {
                if let Ok(srv_resp) = client
                    .get(format!("{url}/notes/{id}"))
                    .header("Authorization", &auth)
                    .send()
                    .await
                {
                    if let Ok(srv) = srv_resp.json::<ServerNote>().await {
                        let conn = db.lock().unwrap();
                        let cur_edit: Option<String> = conn
                            .query_row(
                                "SELECT pending_edit_id FROM notes WHERE id = ?1",
                                params![id],
                                |r| r.get(0),
                            )
                            .optional()
                            .unwrap_or(None);
                        if cur_edit.as_deref() == Some(edit_id.as_str()) {
                            let _ = conn.execute(
                                "UPDATE notes SET body = ?2, base_rev = ?3, deleted = ?4, dirty = 0, has_conflict = 1 WHERE id = ?1",
                                params![id, srv.body, srv.rev, srv.deleted as i64],
                            );
                            conflict_msg = Some(
                                "Конфликт удаления: заметка изменена на другом устройстве".into(),
                            );
                        }
                    }
                }
            } else {
                let conn = db.lock().unwrap();
                let cur_edit: Option<String> = conn
                    .query_row(
                        "SELECT pending_edit_id FROM notes WHERE id = ?1",
                        params![id],
                        |r| r.get(0),
                    )
                    .optional()
                    .unwrap_or(None);
                if cur_edit.as_deref() == Some(edit_id.as_str()) {
                    let _ = conn.execute(
                        "UPDATE notes SET base_rev = ?2, dirty = 0 WHERE id = ?1",
                        params![id, res.rev],
                    );
                }
            }
        } else {
            let eff_edit = if edit_id.is_empty() {
                uuid::Uuid::new_v4().to_string()
            } else {
                edit_id.clone()
            };
            let payload = PutRequest {
                body,
                base_rev,
                device_id: settings.device_id.clone(),
                edit_id: eff_edit.clone(),
            };
            let resp = match client
                .put(format!("{url}/notes/{id}"))
                .header("Authorization", &auth)
                .json(&payload)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => return SyncStatus::Offline(format!("Нет связи: {e}")),
            };
            if !resp.status().is_success() {
                return SyncStatus::Offline(format!("Ошибка сервера: {}", resp.status()));
            }
            let res: MutateResponse = match resp.json().await {
                Ok(d) => d,
                Err(e) => return SyncStatus::Offline(format!("Ошибка ответа: {e}")),
            };

            if res.status == "conflict" {
                if let Ok(srv_resp) = client
                    .get(format!("{url}/notes/{id}"))
                    .header("Authorization", &auth)
                    .send()
                    .await
                {
                    if let Ok(srv) = srv_resp.json::<ServerNote>().await {
                        let conn = db.lock().unwrap();
                        let cur_edit: Option<String> = conn
                            .query_row(
                                "SELECT pending_edit_id FROM notes WHERE id = ?1",
                                params![id],
                                |r| r.get(0),
                            )
                            .optional()
                            .unwrap_or(None);
                        if cur_edit.as_deref() == Some(edit_id.as_str()) {
                            let _ = conn.execute(
                                "UPDATE notes SET body = ?2, base_rev = ?3, deleted = ?4, dirty = 0, has_conflict = 1 WHERE id = ?1",
                                params![id, srv.body, srv.rev, srv.deleted as i64],
                            );
                            let vid = res
                                .conflict_version_id
                                .map(|v| v.to_string())
                                .unwrap_or_else(|| "?".into());
                            conflict_msg = Some(format!(
                                "Конфликт: ваша версия сохранена на сервере (версия #{vid})"
                            ));
                        }
                    }
                }
            } else {
                let conn = db.lock().unwrap();
                let cur_edit: Option<String> = conn
                    .query_row(
                        "SELECT pending_edit_id FROM notes WHERE id = ?1",
                        params![id],
                        |r| r.get(0),
                    )
                    .optional()
                    .unwrap_or(None);
                if cur_edit.as_deref() == Some(edit_id.as_str()) {
                    let _ = conn.execute(
                        "UPDATE notes SET base_rev = ?2, dirty = 0 WHERE id = ?1",
                        params![id, res.rev],
                    );
                }
            }
        }
    }

    // Шаг 3. Обновляем seq
    if pushed_any {
        let cur_seq = {
            let conn = db.lock().unwrap();
            get_meta(&conn, "last_seq", "0").parse::<i64>().unwrap_or(0)
        };
        if let Ok(after_resp) = client
            .get(format!("{url}/notes?since={cur_seq}"))
            .header("Authorization", &auth)
            .send()
            .await
        {
            if let Ok(after) = after_resp.json::<ListResponse>().await {
                if after.seq > cur_seq {
                    let conn = db.lock().unwrap();
                    set_meta(&conn, "last_seq", &after.seq.to_string());
                }
            }
        }
    }

    if let Some(msg) = conflict_msg {
        SyncStatus::ConflictNotice(msg)
    } else {
        SyncStatus::Ok
    }
}

// ---------- UI на Dioxus ----------

#[derive(Clone, PartialEq)]
enum Screen {
    List,
    Edit(String),
    Conflicts(String),
    Log,
    Settings,
}

fn note_title(body: &str) -> String {
    body.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("Без названия")
        .to_string()
}

fn note_preview(body: &str) -> String {
    body.lines()
        .skip(1)
        .take(2)
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn trigger_sync(
    db: &Arc<Mutex<Connection>>,
    mut status: Signal<SyncStatus>,
    mut notes: Signal<Vec<LocalNote>>,
    mut settings_sig: Signal<Settings>,
) {
    let db_clone = db.clone();
    spawn(async move {
        status.set(SyncStatus::Syncing);
        let res = sync_with_server(db_clone.clone()).await;
        status.set(res);
        let conn = db_clone.lock().unwrap();
        notes.set(load_notes(&conn));
        settings_sig.set(load_settings(&conn));
    });
}

fn main() {
    dioxus::launch(App);
}

#[component]
fn App() -> Element {
    let db = use_hook(|| Arc::new(Mutex::new(open_db())));
    let mut screen = use_signal(|| Screen::List);
    let mut notes = use_signal(|| load_notes(&db.lock().unwrap()));
    let mut settings_sig = use_signal(|| load_settings(&db.lock().unwrap()));
    let mut status = use_signal(|| SyncStatus::Idle);
    let mut edit_tick = use_signal(|| 0u64);

    let mut manual_url_input = use_signal(|| settings_sig().manual_url.clone());
    let mut token_input = use_signal(|| settings_sig().token.clone());
    let mut check_result = use_signal(String::new);

    // Состояния экранов конфликтов и лога
    let mut versions_list = use_signal(Vec::<NoteVersionDto>::new);
    let mut versions_error = use_signal(String::new);
    let mut log_list = use_signal(Vec::<LogEntryDto>::new);
    let mut log_error = use_signal(String::new);

    let db_init = db.clone();
    use_effect(move || {
        start_mdns_browser(db_init.clone(), settings_sig, status, notes);
        trigger_sync(&db_init, status, notes, settings_sig);
    });

    let db_debounce = db.clone();
    use_effect(move || {
        let tick = edit_tick();
        if tick == 0 {
            return;
        }
        let db_clone = db_debounce.clone();
        spawn(async move {
            tokio::time::sleep(Duration::from_secs(1)).await;
            if edit_tick() == tick {
                status.set(SyncStatus::Syncing);
                let res = sync_with_server(db_clone.clone()).await;
                status.set(res);
                let conn = db_clone.lock().unwrap();
                notes.set(load_notes(&conn));
                settings_sig.set(load_settings(&conn));
            }
        });
    });

    let conflict_count = notes()
        .iter()
        .filter(|n| n.has_conflict && !n.deleted)
        .count();
    let dirty_count = notes().iter().filter(|n| n.dirty).count();

    let (status_text, status_bg, status_fg) = match status() {
        SyncStatus::Syncing => ("Синхронизация...".to_string(), "#2b2930", "#e6e0e9"),
        SyncStatus::ConflictNotice(ref msg) => (msg.clone(), "#8c1d18", "#f9dedc"),
        _ if conflict_count > 0 => (
            format!("Есть конфликты: {conflict_count}"),
            "#8c1d18",
            "#f9dedc",
        ),
        SyncStatus::Offline(ref reason) => (reason.clone(), "#4a4458", "#e8def8"),
        _ if dirty_count > 0 => (
            format!("Ожидает отправки: {dirty_count}"),
            "#2b2930",
            "#e6e0e9",
        ),
        SyncStatus::Ok => ("Синхронизировано".to_string(), "#2b2930", "#e6e0e9"),
        SyncStatus::Idle => ("Готово".to_string(), "#2b2930", "#e6e0e9"),
    };

    rsx! {
        style { "html, body {{ margin: 0; padding: 0; background: #141218; width: 100%; height: 100%; overflow-x: hidden; }}" }
        div {
            style: "margin: 0; padding: 0; font-family: system-ui, -apple-system, Roboto, sans-serif; background: #141218; color: #e6e0e9; min-height: 100vh; display: flex; flex-direction: column; user-select: none;",

            // ---------- ЭКРАН 1: СПИСОК ЗАМЕТОК ----------
            if let Screen::List = screen() {
                div {
                    style: "height: 56px; padding: 0 8px 0 16px; display: flex; align-items: center; justify-content: space-between; background: #141218;",
                    span { style: "font-size: 22px; font-weight: 400;", "Заметки" }
                    div { style: "display: flex; align-items: center; gap: 2px;",
                        button {
                            style: "background: transparent; color: #d0bcff; border: none; padding: 8px 10px; font-size: 14px; font-weight: 500;",
                            onclick: {
                                let db_btn = db.clone();
                                move |_| trigger_sync(&db_btn, status, notes, settings_sig)
                            },
                            "Синхр."
                        }
                        button {
                            style: "background: transparent; color: #d0bcff; border: none; padding: 8px 10px; font-size: 14px; font-weight: 500;",
                            onclick: move |_| {
                                let s = settings_sig();
                                log_error.set(String::new());
                                screen.set(Screen::Log);
                                spawn(async move {
                                    match fetch_log(&s).await {
                                        Ok(items) => log_list.set(items),
                                        Err(e) => log_error.set(e),
                                    }
                                });
                            },
                            "Лог"
                        }
                        button {
                            style: "background: transparent; color: #d0bcff; border: none; padding: 8px 10px; font-size: 14px; font-weight: 500;",
                            onclick: move |_| {
                                check_result.set(String::new());
                                screen.set(Screen::Settings);
                            },
                            "Настройки"
                        }
                    }
                }

                div {
                    style: "background: {status_bg}; color: {status_fg}; padding: 6px 16px; font-size: 12px; line-height: 16px;",
                    "{status_text}"
                }

                if notes().is_empty() {
                    div {
                        style: "flex: 1; display: flex; flex-direction: column; align-items: center; justify-content: center; padding: 24px; text-align: center;",
                        div { style: "font-size: 16px; margin-bottom: 8px;", "Заметок пока нет" }
                        div { style: "font-size: 12px; color: #cac4d0; max-width: 280px;",
                            "Нажмите + чтобы создать заметку, или укажите адрес и токен в Настройках"
                        }
                    }
                } else {
                    div {
                        style: "flex: 1; padding: 8px 12px 88px 12px; display: flex; flex-direction: column; gap: 10px; overflow-y: auto;",
                        for note in notes() {
                            {
                                let id = note.id.clone();
                                let title = note_title(&note.body);
                                let preview = note_preview(&note.body);
                                let mut badge = String::new();
                                if note.has_conflict {
                                    badge.push_str("[Конфликт] ");
                                }
                                if note.dirty {
                                    badge.push_str("[Не отпр.]");
                                }
                                let badge_color = if note.has_conflict { "#f2b8b5" } else { "#d0bcff" };

                                rsx! {
                                    div {
                                        key: "{note.id}",
                                        style: "background: #211f26; border-radius: 12px; padding: 14px 16px; display: flex; flex-direction: column; gap: 4px; box-shadow: 0 1px 3px rgba(0,0,0,0.3);",
                                        onclick: move |_| screen.set(Screen::Edit(id.clone())),
                                        div {
                                            style: "display: flex; justify-content: space-between; align-items: center; gap: 8px;",
                                            span {
                                                style: "font-size: 16px; font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; flex: 1;",
                                                "{title}"
                                            }
                                            if !badge.is_empty() {
                                                span {
                                                    style: "font-size: 11px; font-weight: 500; color: {badge_color}; white-space: nowrap;",
                                                    "{badge.trim()}"
                                                }
                                            }
                                        }
                                        if !preview.is_empty() {
                                            div {
                                                style: "font-size: 13px; color: #cac4d0; line-height: 18px; display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden;",
                                                "{preview}"
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                button {
                    style: "position: fixed; right: 16px; bottom: 20px; width: 56px; height: 56px; border-radius: 16px; background: #4f378b; color: #eaddff; border: none; font-size: 28px; display: flex; align-items: center; justify-content: center; box-shadow: 0 4px 12px rgba(0,0,0,0.45);",
                    onclick: {
                        let db_new = db.clone();
                        move |_| {
                            let new_id = uuid::Uuid::new_v4().to_string();
                            {
                                let conn = db_new.lock().unwrap();
                                save_local_edit(&conn, &new_id, "");
                                notes.set(load_notes(&conn));
                            }
                            screen.set(Screen::Edit(new_id));
                        }
                    },
                    "+"
                }
            }

            // ---------- ЭКРАН 2: РЕДАКТОР ЗАМЕТКИ ----------
            if let Screen::Edit(note_id) = screen() {
                {
                    let current = notes().into_iter().find(|n| n.id == note_id);
                    let current_body = current.as_ref().map(|n| n.body.clone()).unwrap_or_default();
                    let current_title = note_title(&current_body);
                    let has_conflict = current.as_ref().map(|n| n.has_conflict).unwrap_or(false);
                    let base_rev = current.as_ref().map(|n| n.base_rev).unwrap_or(0);

                    let id_input = note_id.clone();
                    let id_del = note_id.clone();
                    let id_conf = note_id.clone();
                    let db_edit = db.clone();
                    let db_del = db.clone();
                    let db_back = db.clone();

                    rsx! {
                        div {
                            style: "height: 56px; padding: 0 8px; display: flex; align-items: center; justify-content: space-between; background: #141218; gap: 4px;",
                            button {
                                style: "background: transparent; color: #d0bcff; border: none; padding: 8px 10px; font-size: 14px; font-weight: 500;",
                                onclick: move |_| {
                                    trigger_sync(&db_back, status, notes, settings_sig);
                                    screen.set(Screen::List);
                                },
                                "Назад"
                            }
                            span {
                                style: "font-size: 17px; font-weight: 400; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; flex: 1;",
                                "{current_title}"
                            }
                            button {
                                style: if has_conflict {
                                    "background: #8c1d18; color: #f9dedc; border: none; padding: 6px 10px; border-radius: 14px; font-size: 13px; font-weight: 500;"
                                } else {
                                    "background: transparent; color: #d0bcff; border: none; padding: 8px 10px; font-size: 13px; font-weight: 500;"
                                },
                                onclick: move |_| {
                                    let s = settings_sig();
                                    let nid = id_conf.clone();
                                    versions_error.set(String::new());
                                    screen.set(Screen::Conflicts(nid.clone()));
                                    spawn(async move {
                                        match fetch_versions(&s, &nid).await {
                                            Ok(v) => versions_list.set(v),
                                            Err(e) => versions_error.set(e),
                                        }
                                    });
                                },
                                "Конфликты"
                            }
                            button {
                                style: "background: transparent; color: #f2b8b5; border: none; padding: 8px 10px; font-size: 14px; font-weight: 500;",
                                onclick: move |_| {
                                    let need_sync = {
                                        let conn = db_del.lock().unwrap();
                                        let s = delete_local_note(&conn, &id_del, base_rev);
                                        notes.set(load_notes(&conn));
                                        s
                                    };
                                    if need_sync {
                                        trigger_sync(&db_del, status, notes, settings_sig);
                                    }
                                    screen.set(Screen::List);
                                },
                                "Удалить"
                            }
                        }

                        if has_conflict {
                            div {
                                style: "background: #8c1d18; color: #f9dedc; padding: 10px 16px; font-size: 12px;",
                                "У этой заметки есть конфликт — нажмите «Конфликты» сверху для выбора версии"
                            }
                        } else if let SyncStatus::Offline(ref reason) = status() {
                            div {
                                style: "background: #4a4458; color: #e8def8; padding: 10px 16px; font-size: 12px;",
                                "{reason} — правки сохраняются локально"
                            }
                        }

                        div { style: "flex: 1; padding: 12px; display: flex;",
                            textarea {
                                style: "flex: 1; width: 100%; background: #1d1b20; color: #e6e0e9; border: 1px solid #49454f; border-radius: 12px; padding: 14px; font-size: 16px; line-height: 24px; font-family: inherit; box-sizing: border-box; resize: none; outline: none; user-select: text;",
                                value: "{current_body}",
                                oninput: move |evt| {
                                    let val = evt.value();
                                    {
                                        let conn = db_edit.lock().unwrap();
                                        save_local_edit(&conn, &id_input, &val);
                                        notes.set(load_notes(&conn));
                                    }
                                    edit_tick.set(edit_tick() + 1);
                                }
                            }
                        }
                    }
                }
            }

            // ---------- ЭКРАН 3: КОНФЛИКТЫ ----------
            if let Screen::Conflicts(note_id) = screen() {
                {
                    let server_body = notes()
                        .into_iter()
                        .find(|n| n.id == note_id)
                        .map(|n| n.body)
                        .unwrap_or_default();
                    let unresolved: Vec<NoteVersionDto> = versions_list()
                        .into_iter()
                        .filter(|v| v.is_conflict && !v.resolved)
                        .collect();
                    let back_id = note_id.clone();

                    rsx! {
                        div {
                            style: "height: 56px; padding: 0 8px; display: flex; align-items: center; gap: 8px; background: #141218;",
                            button {
                                style: "background: transparent; color: #d0bcff; border: none; padding: 10px 12px; font-size: 14px; font-weight: 500;",
                                onclick: move |_| screen.set(Screen::Edit(back_id.clone())),
                                "Назад"
                            }
                            span { style: "font-size: 18px; font-weight: 400;", "Конфликты" }
                        }

                        div { style: "padding: 12px; display: flex; flex-direction: column; gap: 14px; overflow-y: auto;",
                            if !versions_error().is_empty() {
                                div { style: "color: #f2b8b5; font-size: 13px;", "{versions_error}" }
                            } else if unresolved.is_empty() {
                                div { style: "color: #cac4d0; font-size: 14px;", "Неразрешённых конфликтов для этой заметки нет." }
                            } else {
                                for ver in unresolved {
                                    {
                                        let vid = ver.id;
                                        let nid_keep = note_id.clone();
                                        let nid_use = note_id.clone();
                                        let nid_both = note_id.clone();
                                        let db_keep = db.clone();
                                        let db_use = db.clone();
                                        let db_both = db.clone();

                                        rsx! {
                                            div {
                                                key: "{ver.id}",
                                                style: "background: #211f26; border-radius: 12px; padding: 12px; display: flex; flex-direction: column; gap: 10px; border: 1px solid #49454f;",

                                                div { style: "font-size: 12px; color: #d0bcff; font-weight: 600;", "Текущая версия на сервере:" }
                                                pre {
                                                    style: "margin: 0; white-space: pre-wrap; word-break: break-word; background: #141218; padding: 10px; border-radius: 8px; font-size: 14px; font-family: inherit;",
                                                    "{server_body}"
                                                }

                                                div { style: "font-size: 12px; color: #f2b8b5; font-weight: 600;",
                                                    "Конфликтная версия #{ver.id} ({ver.device_id}):"
                                                }
                                                pre {
                                                    style: "margin: 0; white-space: pre-wrap; word-break: break-word; background: #141218; padding: 10px; border-radius: 8px; font-size: 14px; font-family: inherit; border: 1px solid #8c1d18;",
                                                    "{ver.body}"
                                                }

                                                div { style: "display: flex; gap: 8px; flex-wrap: wrap;",
                                                    button {
                                                        style: "background: #313244; color: #e6e0e9; border: none; padding: 8px 12px; border-radius: 16px; font-size: 13px;",
                                                        onclick: move |_| {
                                                            let s = settings_sig();
                                                            let nid = nid_keep.clone();
                                                            let dbc = db_keep.clone();
                                                            spawn(async move {
                                                                if let Err(e) = resolve_conflict_api(&s, &nid, vid, "keep_server").await {
                                                                    versions_error.set(e);
                                                                    return;
                                                                }
                                                                if let Ok(v) = fetch_versions(&s, &nid).await {
                                                                    versions_list.set(v);
                                                                }
                                                                trigger_sync(&dbc, status, notes, settings_sig);
                                                            });
                                                        },
                                                        "Оставить серверную"
                                                    }
                                                    button {
                                                        style: "background: #d0bcff; color: #381e72; border: none; padding: 8px 12px; border-radius: 16px; font-size: 13px; font-weight: 500;",
                                                        onclick: move |_| {
                                                            let s = settings_sig();
                                                            let nid = nid_use.clone();
                                                            let dbc = db_use.clone();
                                                            spawn(async move {
                                                                if let Err(e) = resolve_conflict_api(&s, &nid, vid, "use_this").await {
                                                                    versions_error.set(e);
                                                                    return;
                                                                }
                                                                if let Ok(v) = fetch_versions(&s, &nid).await {
                                                                    versions_list.set(v);
                                                                }
                                                                trigger_sync(&dbc, status, notes, settings_sig);
                                                            });
                                                        },
                                                        "Взять эту"
                                                    }
                                                    button {
                                                        style: "background: transparent; color: #d0bcff; border: 1px solid #938f99; padding: 8px 12px; border-radius: 16px; font-size: 13px;",
                                                        onclick: move |_| {
                                                            let s = settings_sig();
                                                            let nid = nid_both.clone();
                                                            let dbc = db_both.clone();
                                                            spawn(async move {
                                                                if let Err(e) = resolve_conflict_api(&s, &nid, vid, "keep_both").await {
                                                                    versions_error.set(e);
                                                                    return;
                                                                }
                                                                if let Ok(v) = fetch_versions(&s, &nid).await {
                                                                    versions_list.set(v);
                                                                }
                                                                trigger_sync(&dbc, status, notes, settings_sig);
                                                            });
                                                        },
                                                        "Сохранить обе"
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // ---------- ЭКРАН 4: ЖУРНАЛ СОБЫТИЙ ----------
            if let Screen::Log = screen() {
                div {
                    style: "height: 56px; padding: 0 8px; display: flex; align-items: center; justify-content: space-between; background: #141218;",
                    button {
                        style: "background: transparent; color: #d0bcff; border: none; padding: 10px 12px; font-size: 14px; font-weight: 500;",
                        onclick: move |_| screen.set(Screen::List),
                        "Назад"
                    }
                    span { style: "font-size: 18px; font-weight: 400;", "Журнал событий" }
                    button {
                        style: "background: transparent; color: #d0bcff; border: none; padding: 10px 12px; font-size: 14px; font-weight: 500;",
                        onclick: move |_| {
                            let s = settings_sig();
                            log_error.set(String::new());
                            spawn(async move {
                                match fetch_log(&s).await {
                                    Ok(items) => log_list.set(items),
                                    Err(e) => log_error.set(e),
                                }
                            });
                        },
                        "↻"
                    }
                }

                div { style: "padding: 12px; display: flex; flex-direction: column; gap: 8px; overflow-y: auto;",
                    if !log_error().is_empty() {
                        div { style: "color: #f2b8b5; font-size: 13px;", "{log_error}" }
                    } else if log_list().is_empty() {
                        div { style: "color: #cac4d0; font-size: 14px;", "Записей пока нет." }
                    } else {
                        for entry in log_list() {
                            div {
                                key: "{entry.id}",
                                style: "background: #211f26; border-radius: 10px; padding: 10px 12px; display: flex; flex-direction: column; gap: 4px;",
                                div { style: "display: flex; justify-content: space-between; font-size: 12px;",
                                    span {
                                        style: if entry.event == "conflict" { "color: #f2b8b5; font-weight: 600;" } else { "color: #d0bcff; font-weight: 600;" },
                                        "{entry.event.to_uppercase()}"
                                    }
                                    span { style: "color: #cac4d0;", "{entry.ts}" }
                                }
                                div { style: "font-size: 12px; color: #cac4d0;",
                                    "Устройство: {entry.device_id}"
                                }
                                if !entry.details.is_empty() {
                                    div { style: "font-size: 13px; color: #e6e0e9;", "{entry.details}" }
                                }
                            }
                        }
                    }
                }
            }

            // ---------- ЭКРАН 5: НАСТРОЙКИ ----------
            if let Screen::Settings = screen() {
                {
                    let cur_settings = settings_sig();
                    let db_save = db.clone();
                    let db_reset = db.clone();
                    let db_mdns = db.clone();

                    rsx! {
                        div {
                            style: "height: 56px; padding: 0 8px; display: flex; align-items: center; gap: 8px; background: #141218;",
                            button {
                                style: "background: transparent; color: #d0bcff; border: none; padding: 10px 12px; font-size: 14px; font-weight: 500;",
                                onclick: move |_| screen.set(Screen::List),
                                "Назад"
                            }
                            span { style: "font-size: 20px; font-weight: 400;", "Настройки" }
                        }

                        div {
                            style: "padding: 16px; display: flex; flex-direction: column; gap: 14px;",

                            div { style: "display: flex; flex-direction: column; gap: 6px;",
                                label { style: "font-size: 12px; color: #cac4d0;",
                                    "Адрес сервера вручную (приоритетнее автопоиска)"
                                }
                                input {
                                    style: "background: #1d1b20; color: #e6e0e9; border: 1px solid #938f99; border-radius: 8px; padding: 14px 12px; font-size: 15px; outline: none; user-select: text;",
                                    placeholder: "http://192.168.1.50:8787",
                                    value: "{manual_url_input}",
                                    oninput: move |e| manual_url_input.set(e.value()),
                                }
                            }

                            div { style: "font-size: 12px; color: #cac4d0;",
                                if cur_settings.discovered_url.is_empty() {
                                    "Автопоиск (mDNS): сервер пока не найден"
                                } else {
                                    "Автопоиск (mDNS): найден {cur_settings.discovered_url}"
                                }
                            }

                            div { style: "display: flex; gap: 8px; flex-wrap: wrap;",
                                button {
                                    style: "background: transparent; color: #d0bcff; border: 1px solid #938f99; padding: 8px 16px; border-radius: 20px; font-size: 13px;",
                                    onclick: move |_| {
                                        start_mdns_browser(db_mdns.clone(), settings_sig, status, notes);
                                        check_result.set("Идёт поиск сервера в локальной сети...".into());
                                    },
                                    "Искать заново"
                                }
                                if !manual_url_input().trim().is_empty() && !cur_settings.discovered_url.is_empty() {
                                    button {
                                        style: "background: transparent; color: #d0bcff; border: 1px solid #938f99; padding: 8px 16px; border-radius: 20px; font-size: 13px;",
                                        onclick: move |_| manual_url_input.set(String::new()),
                                        "Сбросить ручной"
                                    }
                                }
                            }

                            div { style: "display: flex; flex-direction: column; gap: 6px;",
                                label { style: "font-size: 12px; color: #cac4d0;",
                                    "Токен (из файла *.token на сервере)"
                                }
                                input {
                                    style: "background: #1d1b20; color: #e6e0e9; border: 1px solid #938f99; border-radius: 8px; padding: 14px 12px; font-size: 15px; outline: none; user-select: text;",
                                    value: "{token_input}",
                                    oninput: move |e| token_input.set(e.value()),
                                }
                            }

                            div { style: "display: flex; gap: 8px; flex-wrap: wrap;",
                                button {
                                    style: "background: #d0bcff; color: #381e72; border: none; padding: 10px 20px; border-radius: 20px; font-size: 14px; font-weight: 500;",
                                    onclick: move |_| {
                                        {
                                            let conn = db_save.lock().unwrap();
                                            set_meta(&conn, "manual_url", manual_url_input().trim());
                                            set_meta(&conn, "token", token_input().trim());
                                            settings_sig.set(load_settings(&conn));
                                        }
                                        trigger_sync(&db_save, status, notes, settings_sig);
                                        screen.set(Screen::List);
                                    },
                                    "Сохранить и синхр."
                                }

                                button {
                                    style: "background: transparent; color: #d0bcff; border: 1px solid #938f99; padding: 10px 18px; border-radius: 20px; font-size: 14px; font-weight: 500;",
                                    onclick: move |_| {
                                        let url_to_check = if !manual_url_input().trim().is_empty() {
                                            manual_url_input()
                                        } else {
                                            cur_settings.discovered_url.clone()
                                        };
                                        spawn(async move {
                                            check_result.set("Проверка...".into());
                                            match check_health(&url_to_check).await {
                                                Ok(ok) => check_result.set(ok),
                                                Err(err) => check_result.set(err),
                                            }
                                        });
                                    },
                                    "Проверить /health"
                                }
                            }

                            if !check_result().is_empty() {
                                div { style: "font-size: 12px; color: #cac4d0;", "{check_result}" }
                            }

                            hr { style: "border: none; border-top: 1px solid #49454f; width: 100%; margin: 4px 0;" }

                            div { style: "font-size: 12px; color: #cac4d0;", "ID устройства: {cur_settings.device_id}" }
                            div { style: "font-size: 12px; color: #cac4d0;", "Последний seq: {cur_settings.last_seq}" }

                            button {
                                style: "align-self: flex-start; background: transparent; color: #d0bcff; border: 1px solid #938f99; padding: 10px 18px; border-radius: 20px; font-size: 14px; font-weight: 500;",
                                onclick: move |_| {
                                    {
                                        let conn = db_reset.lock().unwrap();
                                        set_meta(&conn, "last_seq", "0");
                                        let _ = conn.execute("DELETE FROM notes", []);
                                        notes.set(Vec::new());
                                        settings_sig.set(load_settings(&conn));
                                    }
                                    trigger_sync(&db_reset, status, notes, settings_sig);
                                    check_result.set("Локальный кэш сброшен, идёт полная загрузка...".into());
                                },
                                "Сбросить кэш (seq=0)"
                            }
                        }
                    }
                }
            }
        }
    }
}

