use super::{NoteSyncApp, RightView};
use crate::models::*;
use gpui_kit::*;
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

impl NoteSyncApp {
    pub(super) fn base_url(&self) -> String {
        let raw = self.cfg.server_url.trim().trim_end_matches('/');
        if raw.starts_with("http://") || raw.starts_with("https://") {
            raw.to_string()
        } else {
            format!("http://{raw}")
        }
    }

    pub(super) fn start_ticker(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let mut n: u64 = 0;
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(250))
                    .await;
                n += 1;
                let ok = this.update(cx, |app, cx| {
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

    pub(super) fn sync_now(&mut self, cx: &mut Context<Self>) {
        self.spawn_pull(cx);
        for item in self.notes.values().cloned().collect::<Vec<_>>() {
            if item.dirty {
                self.spawn_push(item, cx);
            }
        }
    }

    pub(super) fn spawn_pull(&self, cx: &mut Context<Self>) {
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
                                to_push.push(local.clone());
                            }
                        }
                    }
                }

                for item in to_push {
                    self.spawn_push(item, cx);
                }

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
                    self.sync_preview(&body, cx);
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

    pub(super) fn spawn_push(&mut self, item: NoteItem, cx: &mut Context<Self>) {
        if self.cfg.token.trim().is_empty() || self.cfg.server_url.trim().is_empty() {
            return;
        }
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
                        app.last_edit_at = Some(Instant::now());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn spawn_delete_remote(&self, id: String, base_rev: i64, cx: &mut Context<Self>) {
        if base_rev == 0 {
            return;
        }
        let url = format!(
            "{}/notes/{}?base_rev={}&device_id={}",
            self.base_url(),
            id,
            base_rev,
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

    pub(super) fn check_health(&mut self, cx: &mut Context<Self>) {
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

    pub(super) fn open_conflicts(&mut self, note_id: String, cx: &mut Context<Self>) {
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

    pub(super) fn resolve_conflict(
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

    pub(super) fn open_log(&mut self, cx: &mut Context<Self>) {
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
