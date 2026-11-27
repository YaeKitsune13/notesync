mod app;
mod models;
mod search;
mod theme;

use app::*;
use gpui_kit::assets::Assets;
use gpui_kit::*;
use models::{load_config, normalize_config};

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
