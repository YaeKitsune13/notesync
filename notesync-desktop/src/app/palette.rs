use super::{window_bg, NoteSyncApp, RightView, ViewMode};
use crate::models::{save_config, try_load_config};
use crate::theme;
use gpui_kit::*;

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Cmd {
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

#[derive(Clone)]
pub(super) enum QoItem {
    Note(String),
    Cmd(Cmd),
}

impl NoteSyncApp {
    pub(super) fn cmd_info(&self, cmd: &Cmd) -> (String, &'static str) {
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
            Cmd::ZoomIn => ("Лупа (увеличить текст)".to_string(), "Ctrl+="),
            Cmd::ZoomOut => ("Лупа (уменьшить текст)".to_string(), "Ctrl+-"),
            Cmd::ZoomReset => ("Лупа (сбросить размер текста)".to_string(), "Ctrl+0"),
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

    pub(super) fn qo_items(&self, cx: &App) -> Vec<QoItem> {
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

    pub(super) fn open_quick(
        &mut self,
        commands: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.qo_open = true;
        self.qo_commands = commands;
        self.qo_ix = 0;
        self.qo_input.update(cx, |s, cx| {
            s.set_value("", window, cx);
            s.focus(window, cx);
        });
        cx.notify();
    }

    pub(super) fn close_quick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.qo_open = false;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    pub(super) fn qo_step(&mut self, dir: i32, cx: &mut Context<Self>) {
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

    pub(super) fn qo_run(&mut self, item: QoItem, window: &mut Window, cx: &mut Context<Self>) {
        self.close_quick(window, cx);
        match item {
            QoItem::Note(id) => self.open_note(id, window, cx),
            QoItem::Cmd(cmd) => self.run_cmd(cmd, window, cx),
        }
    }

    pub(super) fn qo_accept(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
            Cmd::ZoomIn => self.set_font_size(self.cfg.font_size + 2.0, cx),
            Cmd::ZoomOut => self.set_font_size(self.cfg.font_size - 2.0, cx),
            Cmd::ZoomReset => self.set_font_size(15.0, cx),
            Cmd::ReloadConfig => self.reload_config(window, cx),
            Cmd::ImportZed => self.import_zed_themes(window, cx),
            Cmd::NewTheme => self.add_custom_theme(window, cx),
            Cmd::ToggleBlur => self.toggle_blur(window, cx),
            Cmd::Theme(name) => self.set_flavor(&name, window, cx),
        }
    }

    pub(super) fn refresh_look(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        theme::configure(self.cfg.window_opacity, &self.cfg.colors);
        theme::set_flavor(&self.cfg.theme);
        theme::change(cx);
        window.set_background_appearance(window_bg(&self.cfg));
        window.set_rem_size(px(16.0 * self.cfg.ui_scale));
        cx.notify();
    }

    pub(super) fn set_opacity(&mut self, value: f32, window: &mut Window, cx: &mut Context<Self>) {
        self.cfg.window_opacity = ((value.clamp(0.2, 1.0)) * 20.0).round() / 20.0;
        save_config(&self.cfg);
        self.refresh_look(window, cx);
    }

    pub(super) fn toggle_blur(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cfg.window_blur = !self.cfg.window_blur;
        save_config(&self.cfg);
        self.refresh_look(window, cx);
    }

    pub(super) fn zoom(&mut self, delta: f32, window: &mut Window, cx: &mut Context<Self>) {
        self.set_scale(self.cfg.ui_scale + delta, window, cx);
    }

    pub(super) fn set_scale(&mut self, value: f32, window: &mut Window, cx: &mut Context<Self>) {
        self.cfg.ui_scale = ((value.clamp(0.6, 2.5)) * 20.0).round() / 20.0;
        save_config(&self.cfg);
        window.set_rem_size(px(16.0 * self.cfg.ui_scale));
        cx.notify();
    }

    pub(super) fn reload_config(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    pub(super) fn add_custom_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    pub(super) fn import_zed_themes(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let (imported, failed) = theme::import_zed();
        self.custom_themes = theme::list_custom();
        self.status = if imported == 0 && failed == 0 {
            "Темы Zed не найдены (~/.config/zed/themes и расширения)".to_string()
        } else {
            format!("Темы Zed: импортировано {imported}, не прочитано файлов {failed}. Выбор: Ctrl+Shift+P → «Тема»")
        };
        cx.notify();
    }

    pub(super) fn open_path(&mut self, path: &std::path::Path) {
        if std::process::Command::new("xdg-open")
            .arg(path)
            .spawn()
            .is_err()
        {
            self.status = format!("Не удалось открыть {}", path.display());
        }
    }

    pub(super) fn set_flavor(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.cfg.theme = key.to_string();
        save_config(&self.cfg);
        self.refresh_look(window, cx);
    }

    pub(super) fn set_font_size(&mut self, size: f32, cx: &mut Context<Self>) {
        self.cfg.font_size = size.clamp(10.0, 48.0);
        save_config(&self.cfg);
        cx.notify();
    }

    pub(super) fn set_start_mode(&mut self, key: &str, cx: &mut Context<Self>) {
        self.cfg.start_mode = key.to_string();
        save_config(&self.cfg);
        cx.notify();
    }

    pub(super) fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    pub(super) fn save_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
}
