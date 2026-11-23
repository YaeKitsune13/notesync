use gpui_kit::component::Theme;
use gpui_kit::{rgb, App, Hsla};

pub fn crust() -> Hsla {
    rgb(0x11111b).into()
}

pub fn mantle() -> Hsla {
    rgb(0x181825).into()
}

pub fn base() -> Hsla {
    rgb(0x1e1e2e).into()
}

pub fn surface0() -> Hsla {
    rgb(0x313244).into()
}

pub fn surface1() -> Hsla {
    rgb(0x45475a).into()
}

pub fn overlay0() -> Hsla {
    rgb(0x6c7086).into()
}

pub fn subtext0() -> Hsla {
    rgb(0xa6adc8).into()
}

pub fn text() -> Hsla {
    rgb(0xcdd6f4).into()
}

pub fn blue() -> Hsla {
    rgb(0x89b4fa).into()
}

pub fn green() -> Hsla {
    rgb(0xa6e3a1).into()
}

pub fn peach() -> Hsla {
    rgb(0xfab387).into()
}

pub fn red() -> Hsla {
    rgb(0xf38ba8).into()
}

/// Красит глобальную тему gpui-kit в Catppuccin Mocha.
///
/// Вызывать ПОСЛЕ `Theme::change(ThemeMode::Dark, ..)`: `change` подгружает
/// встроенную тёмную тему и затирает прежние цвета. `Theme::update` сам
/// синхронизирует `colors` и `tokens`, пересобирает проекцию для скроллбаров
/// и перерисовывает окна. `global_mut` этого не делает, а кнопки читают
/// `tokens.button_*`, так что правка через него осталась бы незамеченной.
pub fn apply(cx: &mut App) {
    let (base, mantle, crust) = (base(), mantle(), crust());
    let (surface0, surface1, overlay0) = (surface0(), surface1(), overlay0());
    let (text, subtext0) = (text(), subtext0());
    let (blue, green, peach, red) = (blue(), green(), peach(), red());
    let blue_hover: Hsla = rgb(0xb4befe).into(); // lavender
    let blue_active: Hsla = rgb(0x74c7ec).into(); // sapphire
    let red_hover: Hsla = rgb(0xeba0ac).into(); // maroon
    let red_active: Hsla = rgb(0xe64553).into();
    let green_hover: Hsla = rgb(0x94e2d5).into(); // teal
    let peach_hover: Hsla = rgb(0xf9e2af).into(); // yellow

    Theme::update(cx, |theme| {
        let c = &mut theme.colors;

        // Поверхности
        c.background = base;
        c.foreground = text;
        c.border = surface0;
        c.input = surface0;
        c.muted = surface0;
        c.muted_foreground = subtext0;
        c.popover = mantle;
        c.popover_foreground = text;
        c.group_box = mantle;
        c.group_box_foreground = text;
        c.overlay = crust.opacity(0.6);
        c.window_border = surface0;
        c.title_bar = mantle;
        c.title_bar_border = surface0;
        c.status_bar = crust;
        c.status_bar_border = surface0;
        c.skeleton = surface0;

        // Акцент, фокус, выделение, каретка
        c.accent = surface0;
        c.accent_foreground = text;
        c.ring = blue;
        c.caret = text;
        c.selection = blue.opacity(0.30);
        c.link = blue;
        c.link_hover = blue_hover;
        c.link_active = blue_active;

        // primary: это видно у Button::primary() (читает button_primary*)
        c.primary = blue;
        c.primary_hover = blue_hover;
        c.primary_active = blue_active;
        c.primary_foreground = crust;
        c.button_primary = blue;
        c.button_primary_hover = blue_hover;
        c.button_primary_active = blue_active;
        c.button_primary_foreground = crust;

        // Обычные и secondary-кнопки
        c.button = surface0;
        c.button_hover = surface1;
        c.button_active = surface1;
        c.button_foreground = text;
        c.secondary = surface0;
        c.secondary_hover = surface1;
        c.secondary_active = surface1;
        c.secondary_foreground = text;
        c.button_secondary = surface0;
        c.button_secondary_hover = surface1;
        c.button_secondary_active = surface1;
        c.button_secondary_foreground = text;

        // danger
        c.danger = red;
        c.danger_hover = red_hover;
        c.danger_active = red_active;
        c.danger_foreground = crust;
        c.button_danger = red;
        c.button_danger_hover = red_hover;
        c.button_danger_active = red_active;
        c.button_danger_foreground = crust;

        // success, warning, info
        c.success = green;
        c.success_hover = green_hover;
        c.success_active = green_hover;
        c.success_foreground = crust;
        c.button_success = green;
        c.button_success_hover = green_hover;
        c.button_success_active = green_hover;
        c.button_success_foreground = crust;
        c.warning = peach;
        c.warning_hover = peach_hover;
        c.warning_active = peach_hover;
        c.warning_foreground = crust;
        c.button_warning = peach;
        c.button_warning_hover = peach_hover;
        c.button_warning_active = peach_hover;
        c.button_warning_foreground = crust;
        c.info = blue;
        c.info_hover = blue_hover;
        c.info_active = blue_active;
        c.info_foreground = crust;
        c.button_info = blue;
        c.button_info_hover = blue_hover;
        c.button_info_active = blue_active;
        c.button_info_foreground = crust;

        // Списки и сайдбар
        c.list = mantle;
        c.list_hover = surface0.opacity(0.55);
        c.list_active = surface0;
        c.list_active_border = blue;
        c.list_even = mantle;
        c.list_head = mantle;
        c.sidebar = mantle;
        c.sidebar_foreground = text;
        c.sidebar_border = surface0;
        c.sidebar_accent = surface0;
        c.sidebar_accent_foreground = text;
        c.sidebar_primary = blue;
        c.sidebar_primary_foreground = crust;

        // Скроллбары и drag
        c.scrollbar = base.opacity(0.0);
        c.scrollbar_thumb = surface1;
        c.scrollbar_thumb_hover = overlay0;
        c.drag_border = blue;
        c.drop_target = blue.opacity(0.15);
    });
}
