use super::{md_style, NoteSyncApp, RightView, ViewMode, START_TAB};
use crate::app::palette::QoItem;
use crate::models::{config_path, NoteVersionDto};
use crate::theme;
use gpui_kit::component::{
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, Textarea},
    menu::{ContextMenuExt as _, PopupMenuItem},
    text::TextView,
    v_flex, Icon, IconName, Sizable,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

impl NoteSyncApp {
    pub(super) fn render_sidebar(&self, cx: &mut Context<Self>) -> Div {
        let query_lower = self.search_input.read(cx).value().trim().to_lowercase();
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

        v_flex()
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
                                        this.sync_now(cx);
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
            )
    }

    pub(super) fn tab_bar(&self, right: impl IntoElement, cx: &mut Context<Self>) -> Div {
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

    pub(super) fn render_settings(&self, cx: &mut Context<Self>) -> AnyElement {
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
            .child(div().text_xs().text_color(theme::subtext0()).child("Тема"))
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
                    .child("Масштаб интерфейса"),
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
                    .child("Размер текста / Лупа (Ctrl+= / Ctrl+-, Ctrl+0 — сброс)"),
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
                                this.set_font_size(this.cfg.font_size - 2.0, cx);
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
                                this.set_font_size(this.cfg.font_size + 2.0, cx);
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
                    (
                        "Ctrl+= / Ctrl+- / Ctrl+0",
                        "лупа текста (увеличить / уменьшить / сброс)",
                    ),
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

        v_flex()
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
            .into_any_element()
    }

    pub(super) fn render_conflicts(&self, note_id: String, cx: &mut Context<Self>) -> AnyElement {
        let server_body = self
            .notes
            .get(&note_id)
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
                let nid1 = note_id.clone();
                let nid2 = note_id.clone();
                let nid3 = note_id.clone();
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
                                        .child(div().text_xs().text_color(theme::red()).child(
                                            format!("Конфликт #{} ({})", ver.id, ver.device_id),
                                        ))
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

    pub(super) fn render_log(&self, cx: &mut Context<Self>) -> AnyElement {
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
                                            div().text_xs().text_color(theme::subtext0()).child(
                                                format!("• {} • {}", entry.device_id, entry.ts),
                                            ),
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

    pub(super) fn render_start(&self, cx: &mut Context<Self>) -> AnyElement {
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

    pub(super) fn render_editor(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(sel_id) = self.selected_id.clone() else {
            return v_flex()
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
                .into_any_element();
        };

        let note_info = self.notes.get(&sel_id);
        let rev_text = note_info
            .map(|n| format!("rev {}", n.base_rev))
            .unwrap_or_default();
        let has_conf = note_info.map(|n| n.has_conflict).unwrap_or(false);

        let mode = self.mode;
        let mode_btn =
            |id: &'static str, label: &'static str, target: ViewMode, cx: &mut Context<Self>| {
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

        let fs = self.cfg.font_size * self.cfg.ui_scale;
        let lh = px(fs * 1.45);
        let editor_font = self.cfg.editor_font_family.clone();
        let preview = TextView::new(&self.preview_state)
            .style(md_style(fs))
            .text_size(px(fs))
            .line_height(lh)
            .size_full();

        let editor = Textarea::new(&self.editor_input)
            .appearance(false)
            .bordered(false)
            .text_size(px(fs))
            .line_height(lh)
            .when(!editor_font.is_empty(), |t| t.font_family(editor_font))
            .h_full();
        let counter = if self.psearch_matches.is_empty() {
            "0/0".to_string()
        } else {
            format!("{}/{}", self.psearch_ix + 1, self.psearch_matches.len())
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
                .child(div().text_xs().text_color(theme::subtext0()).child(counter))
                .child(
                    Button::new("ps-prev")
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(gpui_kit::assets::IconName::ChevronUp))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.psearch_step(-1, cx);
                        })),
                )
                .child(
                    Button::new("ps-next")
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(gpui_kit::assets::IconName::ChevronDown))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.psearch_step(1, cx);
                        })),
                )
                .child(
                    Button::new("ps-close")
                        .ghost()
                        .xsmall()
                        .icon(IconName::Close)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.close_psearch(window, cx);
                        })),
                )
        });

        let workspace = match self.mode {
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
        };

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
                    .child(div().flex_1().min_h_0().w_full().child(workspace)),
            )
            .into_any_element()
    }

    pub(super) fn render_status_bar(&self) -> Div {
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

        h_flex()
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
            )))
    }

    pub(super) fn render_confirm_overlay(&self, cx: &mut Context<Self>) -> Option<Div> {
        let id = self.pending_delete.clone()?;
        let title = self.notes.get(&id).map(|n| n.title()).unwrap_or_default();
        Some(
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
                ),
        )
    }

    pub(super) fn render_quick_overlay(&self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        if !self.qo_open {
            return None;
        }
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
        Some(
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
                ),
        )
    }
}
