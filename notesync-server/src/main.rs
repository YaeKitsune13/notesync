// Сервер блокнота: хранит заметки, ревизии, конфликтные версии и лог.
// Всё в одном файле, без лишних слоёв.
//
// Главное правило: сервер НИКОГДА не выбрасывает присланный текст.
// Если клиент правил не последнюю ревизию — его текст сохраняется
// как отдельная конфликтная версия, а основная заметка не меняется.

use axum::{
    extract::{Path, Query, Request, State},
    http::{header, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post, put},
    Json, Router,
};
use mdns_sd;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

// ---------- общее состояние и ошибки ----------

struct App {
    db: Mutex<Connection>,
    token: String,
}
type Shared = Arc<App>;

// Ошибка API: код ответа + текст. rusqlite::Error превращается в неё через `?`.
struct ApiErr(StatusCode, String);

impl From<rusqlite::Error> for ApiErr {
    fn from(e: rusqlite::Error) -> Self {
        ApiErr(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    }
}
impl IntoResponse for ApiErr {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}
type Res = Result<Json<Value>, ApiErr>;

fn not_found(what: &str) -> ApiErr {
    ApiErr(StatusCode::NOT_FOUND, format!("{what} не найдена"))
}
fn bad_request(msg: &str) -> ApiErr {
    ApiErr(StatusCode::BAD_REQUEST, msg.to_string())
}

// ---------- база данных ----------

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS notes (
  id TEXT PRIMARY KEY,
  body TEXT NOT NULL,
  rev INTEGER NOT NULL,
  deleted INTEGER NOT NULL DEFAULT 0,
  updated_at TEXT NOT NULL,
  seq INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS note_versions (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  note_id TEXT NOT NULL,
  rev INTEGER NOT NULL,
  body TEXT NOT NULL,
  device_id TEXT NOT NULL,
  created_at TEXT NOT NULL,
  is_conflict INTEGER NOT NULL DEFAULT 0,
  resolved INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS edits (
  edit_id TEXT PRIMARY KEY,
  response TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS log (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  ts TEXT NOT NULL,
  device_id TEXT NOT NULL,
  note_id TEXT NOT NULL,
  event TEXT NOT NULL,
  details TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_notes_seq ON notes(seq);
CREATE INDEX IF NOT EXISTS idx_versions_note ON note_versions(note_id);
";

fn now(c: &Connection) -> rusqlite::Result<String> {
    c.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%SZ','now')", [], |r| {
        r.get(0)
    })
}

// Следующий глобальный номер изменения. Работает, потому что вся работа
// с базой идёт под одним замком (Mutex) и внутри транзакции.
fn next_seq(c: &Connection) -> rusqlite::Result<i64> {
    c.query_row("SELECT COALESCE(MAX(seq),0)+1 FROM notes", [], |r| r.get(0))
}

fn add_log(
    c: &Connection,
    device: &str,
    note_id: &str,
    event: &str,
    details: &str,
) -> rusqlite::Result<()> {
    let ts = now(c)?;
    c.execute(
        "INSERT INTO log (ts, device_id, note_id, event, details) VALUES (?1,?2,?3,?4,?5)",
        params![ts, device, note_id, event, details],
    )?;
    Ok(())
}

// Записать версию текста. Возвращает id версии.
fn add_version(
    c: &Connection,
    note_id: &str,
    rev: i64,
    body: &str,
    device: &str,
    is_conflict: bool,
) -> rusqlite::Result<i64> {
    let ts = now(c)?;
    c.execute(
        "INSERT INTO note_versions (note_id, rev, body, device_id, created_at, is_conflict) VALUES (?1,?2,?3,?4,?5,?6)",
        params![note_id, rev, body, device, ts, is_conflict as i64],
    )?;
    Ok(c.last_insert_rowid())
}

// Какое устройство записало последнюю обычную (не конфликтную) версию заметки.
fn head_device(c: &Connection, note_id: &str) -> rusqlite::Result<Option<String>> {
    c.query_row(
        "SELECT device_id FROM note_versions WHERE note_id = ?1 AND is_conflict = 0 ORDER BY id DESC LIMIT 1",
        params![note_id],
        |r| r.get(0),
    )
    .optional()
}

// Строка заметки -> JSON. Последнее поле — число неразрешённых конфликтов.
const NOTE_SELECT: &str = "SELECT id, body, rev, deleted, updated_at, seq,
    (SELECT COUNT(*) FROM note_versions v WHERE v.note_id = notes.id AND v.is_conflict = 1 AND v.resolved = 0)
    FROM notes";

fn row_to_note(r: &rusqlite::Row) -> rusqlite::Result<Value> {
    Ok(json!({
        "id": r.get::<_, String>(0)?,
        "body": r.get::<_, String>(1)?,
        "rev": r.get::<_, i64>(2)?,
        "deleted": r.get::<_, i64>(3)? != 0,
        "updated_at": r.get::<_, String>(4)?,
        "seq": r.get::<_, i64>(5)?,
        "conflicts": r.get::<_, i64>(6)?,
    }))
}

// ---------- проверка токена ----------

async fn auth(State(app): State<Shared>, req: Request, next: Next) -> Response {
    if req.uri().path() == "/health" {
        return next.run(req).await;
    }
    let ok = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(|v| v == format!("Bearer {}", app.token))
        .unwrap_or(false);
    if ok {
        next.run(req).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "нужен токен" })),
        )
            .into_response()
    }
}

// ---------- обработчики ----------

async fn health() -> Json<Value> {
    Json(json!({ "name": "notesync", "version": env!("CARGO_PKG_VERSION") }))
}

#[derive(Deserialize)]
struct SinceQuery {
    since: Option<i64>,
}

// GET /notes?since=N — всё, что изменилось после номера N (включая удалённые).
async fn list_notes(State(app): State<Shared>, Query(q): Query<SinceQuery>) -> Res {
    let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
    let since = q.since.unwrap_or(0);
    let mut stmt = db.prepare(&format!("{NOTE_SELECT} WHERE seq > ?1 ORDER BY seq"))?;
    let notes: Vec<Value> = stmt
        .query_map(params![since], row_to_note)?
        .collect::<rusqlite::Result<_>>()?;
    let max_seq: i64 = db.query_row("SELECT COALESCE(MAX(seq),0) FROM notes", [], |r| r.get(0))?;
    Ok(Json(json!({ "seq": max_seq, "notes": notes })))
}

// GET /notes/:id
async fn get_note(State(app): State<Shared>, Path(id): Path<String>) -> Res {
    let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
    let note = db
        .query_row(
            &format!("{NOTE_SELECT} WHERE id = ?1"),
            params![id],
            row_to_note,
        )
        .optional()?
        .ok_or_else(|| not_found("заметка"))?;
    Ok(Json(note))
}

#[derive(Deserialize)]
struct PutBody {
    body: String,
    base_rev: i64, // какую ревизию правил клиент (0 — новая заметка)
    device_id: String,
    edit_id: String, // уникальный id правки: повторная отправка не создаёт дублей
}

// PUT /notes/:id — сохранить правку.
async fn put_note(
    State(app): State<Shared>,
    Path(id): Path<String>,
    Json(req): Json<PutBody>,
) -> Res {
    let mut db = app.db.lock().unwrap_or_else(|e| e.into_inner());
    let tx = db.transaction()?;

    // Эту правку уже принимали (клиент не получил ответ и отправил снова)?
    let saved: Option<String> = tx
        .query_row(
            "SELECT response FROM edits WHERE edit_id = ?1",
            params![req.edit_id],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(saved) = saved {
        return Ok(Json(
            serde_json::from_str(&saved).unwrap_or(json!({ "status": "ok" })),
        ));
    }

    let current: Option<(String, i64, bool, i64)> = tx
        .query_row(
            "SELECT body, rev, deleted, seq FROM notes WHERE id = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, i64>(2)? != 0, r.get(3)?)),
        )
        .optional()?;
    let ts = now(&tx)?;

    let result = match current {
        // Новой заметки ещё нет — создаём.
        None => {
            let seq = next_seq(&tx)?;
            tx.execute(
                "INSERT INTO notes (id, body, rev, deleted, updated_at, seq) VALUES (?1,?2,1,0,?3,?4)",
                params![id, req.body, ts, seq],
            )?;
            add_version(&tx, &id, 1, &req.body, &req.device_id, false)?;
            add_log(&tx, &req.device_id, &id, "create", "")?;
            json!({ "status": "ok", "rev": 1, "seq": seq })
        }
        Some((body, rev, deleted, note_seq)) => {
            // Принимаем, если клиент правил последнюю ревизию, либо если последнюю версию
            // записало это же устройство (устаревший base_rev из-за двух правок подряд):
            // чужие правки при этом не теряются.
            let accept = req.base_rev == rev
                || (!deleted && head_device(&tx, &id)?.as_deref() == Some(req.device_id.as_str()));
            if accept {
                let new_rev = rev + 1;
                let seq = next_seq(&tx)?;
                tx.execute(
                    "UPDATE notes SET body=?1, rev=?2, deleted=0, updated_at=?3, seq=?4 WHERE id=?5",
                    params![req.body, new_rev, ts, seq, id],
                )?;
                add_version(&tx, &id, new_rev, &req.body, &req.device_id, false)?;
                add_log(&tx, &req.device_id, &id, "update", "")?;
                json!({ "status": "ok", "rev": new_rev, "seq": seq })
            } else if !deleted && body == req.body {
                // Ревизии разные, но текст тот же — настоящего конфликта нет.
                json!({ "status": "ok", "rev": rev, "seq": note_seq })
            } else {
                // Конфликт: основную заметку не трогаем, текст клиента — в отдельную версию.
                let vid = add_version(&tx, &id, rev, &req.body, &req.device_id, true)?;
                let seq = next_seq(&tx)?;
                // seq увеличиваем, чтобы другие клиенты увидели новый конфликт при синхронизации
                tx.execute("UPDATE notes SET seq = ?1 WHERE id = ?2", params![seq, id])?;
                add_log(
                    &tx,
                    &req.device_id,
                    &id,
                    "conflict",
                    &format!(
                        "правка основана на rev {}, на сервере rev {}; конфликтная версия {}",
                        req.base_rev, rev, vid
                    ),
                )?;
                json!({ "status": "conflict", "rev": rev, "seq": seq, "conflict_version_id": vid })
            }
        }
    };

    tx.execute(
        "INSERT INTO edits (edit_id, response) VALUES (?1, ?2)",
        params![req.edit_id, result.to_string()],
    )?;
    tx.commit()?;
    Ok(Json(result))
}

// PUT /notes/:id/stream — потоковая запись.
// Без проверки base_rev и без конфликтных версий: последний присланный текст становится
// основным. Чтобы не терять историю совсем, раз в минуту в неё попадает снимок текста.
async fn put_stream(
    State(app): State<Shared>,
    Path(id): Path<String>,
    Json(req): Json<PutBody>,
) -> Res {
    let mut db = app.db.lock().unwrap_or_else(|e| e.into_inner());
    let tx = db.transaction()?;

    let current: Option<(String, i64, bool)> = tx
        .query_row(
            "SELECT body, rev, deleted FROM notes WHERE id = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, i64>(2)? != 0)),
        )
        .optional()?;
    let ts = now(&tx)?;

    let result = match current {
        None => {
            let seq = next_seq(&tx)?;
            tx.execute(
                "INSERT INTO notes (id, body, rev, deleted, updated_at, seq) VALUES (?1,?2,1,0,?3,?4)",
                params![id, req.body, ts, seq],
            )?;
            add_version(&tx, &id, 1, &req.body, &req.device_id, false)?;
            add_log(&tx, &req.device_id, &id, "create", "поток")?;
            json!({ "status": "ok", "rev": 1, "seq": seq })
        }
        Some((body, rev, deleted)) => {
            if !deleted && body == req.body {
                json!({ "status": "ok", "rev": rev })
            } else {
                let new_rev = rev + 1;
                let seq = next_seq(&tx)?;
                tx.execute(
                    "UPDATE notes SET body=?1, rev=?2, deleted=0, updated_at=?3, seq=?4 WHERE id=?5",
                    params![req.body, new_rev, ts, seq, id],
                )?;
                let recent: i64 = tx.query_row(
                    "SELECT COUNT(*) FROM note_versions WHERE note_id = ?1 AND is_conflict = 0
                     AND created_at > strftime('%Y-%m-%dT%H:%M:%SZ','now','-60 seconds')",
                    params![id],
                    |r| r.get(0),
                )?;
                if recent == 0 {
                    add_version(&tx, &id, new_rev, &req.body, &req.device_id, false)?;
                    add_log(&tx, &req.device_id, &id, "update", "поток")?;
                }
                json!({ "status": "ok", "rev": new_rev, "seq": seq })
            }
        }
    };
    tx.commit()?;
    Ok(Json(result))
}

#[derive(Deserialize)]
struct DeleteQuery {
    base_rev: i64,
    device_id: String,
}

// DELETE /notes/:id?base_rev=N&device_id=...  — мягкое удаление.
async fn delete_note(
    State(app): State<Shared>,
    Path(id): Path<String>,
    Query(q): Query<DeleteQuery>,
) -> Res {
    let mut db = app.db.lock().unwrap_or_else(|e| e.into_inner());
    let tx = db.transaction()?;

    let current: Option<(i64, bool)> = tx
        .query_row(
            "SELECT rev, deleted FROM notes WHERE id = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get::<_, i64>(1)? != 0)),
        )
        .optional()?;
    let (rev, deleted) = current.ok_or_else(|| not_found("заметка"))?;

    let result = if deleted {
        // Уже удалена (например, повторный запрос) — считаем успехом.
        json!({ "status": "ok", "rev": rev })
    } else if q.base_rev == rev {
        let new_rev = rev + 1;
        let seq = next_seq(&tx)?;
        let ts = now(&tx)?;
        tx.execute(
            "UPDATE notes SET rev=?1, deleted=1, updated_at=?2, seq=?3 WHERE id=?4",
            params![new_rev, ts, seq, id],
        )?;
        add_log(&tx, &q.device_id, &id, "delete", "")?;
        json!({ "status": "ok", "rev": new_rev, "seq": seq })
    } else {
        // Заметку правили после того, как клиент её видел — удаление отклоняем.
        add_log(
            &tx,
            &q.device_id,
            &id,
            "conflict",
            &format!(
                "удаление отклонено: клиент видел rev {}, на сервере rev {}",
                q.base_rev, rev
            ),
        )?;
        json!({ "status": "conflict", "rev": rev })
    };
    tx.commit()?;
    Ok(Json(result))
}

// GET /notes/:id/versions — история и конфликтные версии (новые сверху).
async fn versions(State(app): State<Shared>, Path(id): Path<String>) -> Res {
    let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
    let exists: Option<i64> = db
        .query_row("SELECT 1 FROM notes WHERE id = ?1", params![id], |r| {
            r.get(0)
        })
        .optional()?;
    if exists.is_none() {
        return Err(not_found("заметка"));
    }
    let mut stmt = db.prepare(
        "SELECT id, rev, body, device_id, created_at, is_conflict, resolved
         FROM note_versions WHERE note_id = ?1 ORDER BY id DESC",
    )?;
    let list: Vec<Value> = stmt
        .query_map(params![id], |r| {
            Ok(json!({
                "id": r.get::<_, i64>(0)?,
                "rev": r.get::<_, i64>(1)?,
                "body": r.get::<_, String>(2)?,
                "device_id": r.get::<_, String>(3)?,
                "created_at": r.get::<_, String>(4)?,
                "is_conflict": r.get::<_, i64>(5)? != 0,
                "resolved": r.get::<_, i64>(6)? != 0,
            }))
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(Json(json!({ "versions": list })))
}

#[derive(Deserialize)]
struct ResolveBody {
    action: String, // keep_server | use_this | keep_both
    device_id: String,
}

// POST /notes/:id/conflicts/:vid/resolve — решить конфликт.
async fn resolve(
    State(app): State<Shared>,
    Path((id, vid)): Path<(String, i64)>,
    Json(req): Json<ResolveBody>,
) -> Res {
    let mut db = app.db.lock().unwrap_or_else(|e| e.into_inner());
    let tx = db.transaction()?;

    let ver: Option<(String, bool, bool)> = tx
        .query_row(
            "SELECT body, is_conflict, resolved FROM note_versions WHERE id = ?1 AND note_id = ?2",
            params![vid, id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get::<_, i64>(1)? != 0,
                    r.get::<_, i64>(2)? != 0,
                ))
            },
        )
        .optional()?;
    let (ver_body, is_conflict, resolved) = ver.ok_or_else(|| not_found("версия"))?;
    if !is_conflict {
        return Err(bad_request("это не конфликтная версия"));
    }
    if resolved {
        return Err(bad_request("конфликт уже решён"));
    }
    let rev: i64 = tx
        .query_row("SELECT rev FROM notes WHERE id = ?1", params![id], |r| {
            r.get(0)
        })
        .optional()?
        .ok_or_else(|| not_found("заметка"))?;

    let mut result = json!({ "status": "ok", "rev": rev });
    match req.action.as_str() {
        // Оставить текст на сервере, конфликтную версию отбросить (она остаётся в истории).
        "keep_server" => {}
        // Текст конфликтной версии становится основным.
        "use_this" => {
            let new_rev = rev + 1;
            let seq = next_seq(&tx)?;
            let ts = now(&tx)?;
            tx.execute(
                "UPDATE notes SET body=?1, rev=?2, deleted=0, updated_at=?3, seq=?4 WHERE id=?5",
                params![ver_body, new_rev, ts, seq, id],
            )?;
            add_version(&tx, &id, new_rev, &ver_body, &req.device_id, false)?;
            result["rev"] = json!(new_rev);
        }
        // Текст конфликтной версии становится новой отдельной заметкой.
        "keep_both" => {
            let new_id = uuid::Uuid::new_v4().to_string();
            let seq = next_seq(&tx)?;
            let ts = now(&tx)?;
            tx.execute(
                "INSERT INTO notes (id, body, rev, deleted, updated_at, seq) VALUES (?1,?2,1,0,?3,?4)",
                params![new_id, ver_body, ts, seq],
            )?;
            add_version(&tx, &new_id, 1, &ver_body, &req.device_id, false)?;
            add_log(
                &tx,
                &req.device_id,
                &new_id,
                "create",
                &format!("из конфликта заметки {id}"),
            )?;
            result["new_note_id"] = json!(new_id);
        }
        _ => return Err(bad_request("action: keep_server, use_this или keep_both")),
    }

    tx.execute(
        "UPDATE note_versions SET resolved = 1 WHERE id = ?1",
        params![vid],
    )?;
    // Число конфликтов у заметки изменилось — увеличиваем seq, чтобы клиенты это увидели.
    let seq = next_seq(&tx)?;
    tx.execute("UPDATE notes SET seq = ?1 WHERE id = ?2", params![seq, id])?;
    add_log(
        &tx,
        &req.device_id,
        &id,
        "resolve",
        &format!("{} (версия {})", req.action, vid),
    )?;
    tx.commit()?;
    Ok(Json(result))
}

#[derive(Deserialize)]
struct LogQuery {
    limit: Option<i64>,
}

// GET /log?limit=N — журнал событий (новые сверху).
async fn get_log(State(app): State<Shared>, Query(q): Query<LogQuery>) -> Res {
    let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
    let limit = q.limit.unwrap_or(100).clamp(1, 1000);
    let mut stmt = db.prepare(
        "SELECT id, ts, device_id, note_id, event, details FROM log ORDER BY id DESC LIMIT ?1",
    )?;
    let list: Vec<Value> = stmt
        .query_map(params![limit], |r| {
            Ok(json!({
                "id": r.get::<_, i64>(0)?,
                "ts": r.get::<_, String>(1)?,
                "device_id": r.get::<_, String>(2)?,
                "note_id": r.get::<_, String>(3)?,
                "event": r.get::<_, String>(4)?,
                "details": r.get::<_, String>(5)?,
            }))
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(Json(json!({ "log": list })))
}

// ---------- запуск ----------

// Токен берём из NOTESYNC_TOKEN, иначе из файла <база>.token (создаём при первом запуске).
fn load_token(db_path: &str) -> String {
    if let Ok(t) = std::env::var("NOTESYNC_TOKEN") {
        if !t.trim().is_empty() {
            return t.trim().to_string();
        }
    }
    let path = format!("{db_path}.token");
    if let Ok(t) = std::fs::read_to_string(&path) {
        if !t.trim().is_empty() {
            return t.trim().to_string();
        }
    }
    let t = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    std::fs::write(&path, &t).expect("не удалось записать файл токена");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    println!("Создан новый токен, он лежит в файле {path}");
    t
}

// Объявляем сервер в локальной сети через mDNS (_notesync._tcp.local.).
// Если mDNS не запустился (например, заблокирован фаерволом), сервер всё равно работает.
fn start_mdns(port: u16) -> Option<mdns_sd::ServiceDaemon> {
    let host = std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "notesync-pc".to_string());
    let host_fqdn = format!("{host}.local.");

    let mdns = match mdns_sd::ServiceDaemon::new() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("mDNS не запущен: {e}");
            return None;
        }
    };
    let info = match mdns_sd::ServiceInfo::new(
        "_notesync._tcp.local.",
        "notesync",
        &host_fqdn,
        "",
        port,
        None::<std::collections::HashMap<String, String>>,
    ) {
        Ok(i) => i.enable_addr_auto(),
        Err(e) => {
            eprintln!("mDNS ошибка ServiceInfo: {e}");
            return None;
        }
    };
    if let Err(e) = mdns.register(info) {
        eprintln!("mDNS не удалось зарегистрировать сервис: {e}");
        return None;
    }
    println!("mDNS: объявлен сервис _notesync._tcp.local. ({host_fqdn}:{port})");
    Some(mdns)
}

#[tokio::main]
async fn main() {
    let port: u16 = std::env::var("NOTESYNC_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8787);
    let db_path = std::env::var("NOTESYNC_DB").unwrap_or_else(|_| "notes.db".to_string());
    let token = load_token(&db_path);

    let conn = Connection::open(&db_path).expect("не удалось открыть базу");
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")
        .expect("pragma");
    conn.execute_batch(SCHEMA)
        .expect("не удалось создать таблицы");

    let app: Shared = Arc::new(App {
        db: Mutex::new(conn),
        token,
    });

    let router = Router::new()
        .route("/health", get(health))
        .route("/notes", get(list_notes))
        .route(
            "/notes/:id",
            get(get_note).put(put_note).delete(delete_note),
        )
        .route("/notes/:id/stream", put(put_stream))
        .route("/notes/:id/versions", get(versions))
        .route("/notes/:id/conflicts/:vid/resolve", post(resolve))
        .route("/log", get(get_log))
        .layer(middleware::from_fn_with_state(app.clone(), auth))
        .with_state(app);

    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port))
        .await
        .expect("не удалось занять порт");
    println!("notesync слушает порт {port}, база: {db_path}");
    let mdns = start_mdns(port);
    axum::serve(listener, router).await.expect("ошибка сервера");
    if let Some(m) = mdns {
        let _ = m.shutdown();
    }
}
