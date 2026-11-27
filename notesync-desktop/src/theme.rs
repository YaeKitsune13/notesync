use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::{rgb, App, Hsla};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU8, Ordering};
use std::sync::RwLock;

// Прозрачность фоновых цветов (1.0 — непрозрачный). Только для фонов и рамок.
#[derive(Clone, Copy)]
struct Alpha {
    crust: f32,
    mantle: f32,
    base: f32,
    surface0: f32,
    surface1: f32,
}

const OPAQUE: Alpha = Alpha {
    crust: 1.0,
    mantle: 1.0,
    base: 1.0,
    surface0: 1.0,
    surface1: 1.0,
};

// Палитры Catppuccin: Mocha, Macchiato, Frappé (тёмные) и Latte (светлая).
#[derive(Clone, Copy)]
pub struct Palette {
    crust: u32,
    mantle: u32,
    base: u32,
    surface0: u32,
    surface1: u32,
    overlay0: u32,
    subtext0: u32,
    text: u32,
    blue: u32,
    green: u32,
    peach: u32,
    red: u32,
    lavender: u32,
    sapphire: u32,
    maroon: u32,
    teal: u32,
    yellow: u32,
    light: bool,
    alpha: Alpha,
}

const MOCHA: Palette = Palette {
    crust: 0x11111b,
    mantle: 0x181825,
    base: 0x1e1e2e,
    surface0: 0x313244,
    surface1: 0x45475a,
    overlay0: 0x6c7086,
    subtext0: 0xa6adc8,
    text: 0xcdd6f4,
    blue: 0x89b4fa,
    green: 0xa6e3a1,
    peach: 0xfab387,
    red: 0xf38ba8,
    lavender: 0xb4befe,
    sapphire: 0x74c7ec,
    maroon: 0xeba0ac,
    teal: 0x94e2d5,
    yellow: 0xf9e2af,
    light: false,
    alpha: OPAQUE,
};

const MACCHIATO: Palette = Palette {
    crust: 0x181926,
    mantle: 0x1e2030,
    base: 0x24273a,
    surface0: 0x363a4f,
    surface1: 0x494d64,
    overlay0: 0x6e738d,
    subtext0: 0xa5adcb,
    text: 0xcad3f5,
    blue: 0x8aadf4,
    green: 0xa6da95,
    peach: 0xf5a97f,
    red: 0xed8796,
    lavender: 0xb7bdf8,
    sapphire: 0x7dc4e4,
    maroon: 0xee99a0,
    teal: 0x8bd5ca,
    yellow: 0xeed49f,
    light: false,
    alpha: OPAQUE,
};

const FRAPPE: Palette = Palette {
    crust: 0x232634,
    mantle: 0x292c3c,
    base: 0x303446,
    surface0: 0x414559,
    surface1: 0x51576d,
    overlay0: 0x737994,
    subtext0: 0xa5adce,
    text: 0xc6d0f5,
    blue: 0x8caaee,
    green: 0xa6d189,
    peach: 0xef9f76,
    red: 0xe78284,
    lavender: 0xbabbf1,
    sapphire: 0x85c1dc,
    maroon: 0xea999c,
    teal: 0x81c8be,
    yellow: 0xe5c890,
    light: false,
    alpha: OPAQUE,
};

const LATTE: Palette = Palette {
    crust: 0xdce0e8,
    mantle: 0xe6e9ef,
    base: 0xeff1f5,
    surface0: 0xccd0da,
    surface1: 0xbcc0cc,
    overlay0: 0x9ca0b0,
    subtext0: 0x6c6f85,
    text: 0x4c4f69,
    blue: 0x1e66f5,
    green: 0x40a02b,
    peach: 0xfe640b,
    red: 0xd20f39,
    lavender: 0x7287fd,
    sapphire: 0x209fb5,
    maroon: 0xe64553,
    teal: 0x179299,
    yellow: 0xdf8e1d,
    light: true,
    alpha: OPAQUE,
};

// Выбранная тема: 0 Mocha, 1 Macchiato, 2 Frappé, 3 Latte, 4 своя из файла.
static FLAVOR: AtomicU8 = AtomicU8::new(0);
static CUSTOM: RwLock<Option<Palette>> = RwLock::new(None);
static CUSTOM_NAME: RwLock<String> = RwLock::new(String::new());
// Итоговая палитра: тема + переопределения цветов из настроек.
static CURRENT: RwLock<Palette> = RwLock::new(MOCHA);
// Прозрачность фонов из настроек (f32 в битах), 1.0 по умолчанию.
static OPACITY: AtomicU32 = AtomicU32::new(0x3f80_0000);
static OVERRIDES: RwLock<Vec<(String, String)>> = RwLock::new(Vec::new());

const FIELDS: [&str; 17] = [
    "crust", "mantle", "base", "surface0", "surface1", "overlay0", "subtext0", "text", "blue",
    "green", "peach", "red", "lavender", "sapphire", "maroon", "teal", "yellow",
];

/// "#rrggbb" или "#rrggbbaa" -> (цвет, прозрачность 0..1)
fn parse_color(s: &str) -> Option<(u32, f32)> {
    let s = s.trim().trim_start_matches('#');
    match s.len() {
        6 => u32::from_str_radix(s, 16).ok().map(|c| (c, 1.0)),
        8 => {
            let v = u32::from_str_radix(s, 16).ok()?;
            Some((v >> 8, (v & 0xff) as f32 / 255.0))
        }
        _ => None,
    }
}

fn set_field(p: &mut Palette, name: &str, c: u32, a: f32) {
    match name {
        "crust" => {
            p.crust = c;
            p.alpha.crust = a;
        }
        "mantle" => {
            p.mantle = c;
            p.alpha.mantle = a;
        }
        "base" => {
            p.base = c;
            p.alpha.base = a;
        }
        "surface0" => {
            p.surface0 = c;
            p.alpha.surface0 = a;
        }
        "surface1" => {
            p.surface1 = c;
            p.alpha.surface1 = a;
        }
        "overlay0" => p.overlay0 = c,
        "subtext0" => p.subtext0 = c,
        "text" => p.text = c,
        "blue" => p.blue = c,
        "green" => p.green = c,
        "peach" => p.peach = c,
        "red" => p.red = c,
        "lavender" => p.lavender = c,
        "sapphire" => p.sapphire = c,
        "maroon" => p.maroon = c,
        "teal" => p.teal = c,
        "yellow" => p.yellow = c,
        _ => {}
    }
}

fn get_field(p: &Palette, name: &str) -> (u32, f32) {
    match name {
        "crust" => (p.crust, p.alpha.crust),
        "mantle" => (p.mantle, p.alpha.mantle),
        "base" => (p.base, p.alpha.base),
        "surface0" => (p.surface0, p.alpha.surface0),
        "surface1" => (p.surface1, p.alpha.surface1),
        "overlay0" => (p.overlay0, 1.0),
        "subtext0" => (p.subtext0, 1.0),
        "text" => (p.text, 1.0),
        "blue" => (p.blue, 1.0),
        "green" => (p.green, 1.0),
        "peach" => (p.peach, 1.0),
        "red" => (p.red, 1.0),
        "lavender" => (p.lavender, 1.0),
        "sapphire" => (p.sapphire, 1.0),
        "maroon" => (p.maroon, 1.0),
        "teal" => (p.teal, 1.0),
        _ => (p.yellow, 1.0),
    }
}

fn palette_from_json(v: &serde_json::Value) -> Palette {
    let light = v.get("light").and_then(|x| x.as_bool()).unwrap_or(false);
    // Чего нет в файле — берём из Mocha (тёмная) или Latte (светлая).
    let mut p = if light { LATTE } else { MOCHA };
    p.light = light;
    for name in FIELDS {
        if let Some((c, a)) = v.get(name).and_then(|x| x.as_str()).and_then(parse_color) {
            set_field(&mut p, name, c, a);
        }
    }
    p
}

fn palette_to_json(p: &Palette) -> String {
    let mut m = serde_json::Map::new();
    m.insert("light".to_string(), serde_json::Value::Bool(p.light));
    for name in FIELDS {
        let (c, a) = get_field(p, name);
        let hex = if a < 0.999 {
            format!("#{:06x}{:02x}", c, (a * 255.0).round() as u8)
        } else {
            format!("#{:06x}", c)
        };
        m.insert(name.to_string(), serde_json::Value::String(hex));
    }
    serde_json::to_string_pretty(&serde_json::Value::Object(m)).unwrap_or_default()
}

// ---------- темы Zed ----------

// Первый найденный и не полностью прозрачный цвет из списка ключей стиля Zed.
fn zed_color(style: &serde_json::Value, keys: &[&str]) -> Option<(u32, f32)> {
    for k in keys {
        if let Some((c, a)) = style.get(*k).and_then(|x| x.as_str()).and_then(parse_color) {
            if a > 0.05 {
                return Some((c, a));
            }
        }
    }
    None
}

fn palette_from_zed(style: &serde_json::Value, light: bool) -> Palette {
    let mut p = if light { LATTE } else { MOCHA };
    p.light = light;
    let map: [(&str, &[&str]); 17] = [
        ("base", &["editor.background", "background"]),
        (
            "mantle",
            &["surface.background", "panel.background", "title_bar.background"],
        ),
        (
            "crust",
            &["status_bar.background", "title_bar.background", "tab_bar.background"],
        ),
        ("surface0", &["element.background", "ghost_element.hover"]),
        ("surface1", &["element.hover", "border.variant"]),
        ("overlay0", &["text.disabled", "border"]),
        ("subtext0", &["text.muted"]),
        ("text", &["text", "editor.foreground"]),
        ("blue", &["text.accent", "terminal.ansi.blue"]),
        ("green", &["terminal.ansi.green", "success"]),
        ("peach", &["warning", "terminal.ansi.bright_yellow"]),
        ("red", &["error", "terminal.ansi.red"]),
        ("lavender", &["terminal.ansi.bright_blue", "text.accent"]),
        ("sapphire", &["terminal.ansi.cyan", "info"]),
        ("maroon", &["terminal.ansi.bright_red", "error"]),
        ("teal", &["terminal.ansi.bright_cyan", "terminal.ansi.cyan"]),
        ("yellow", &["terminal.ansi.yellow", "warning"]),
    ];
    for (name, keys) in map {
        if let Some((c, a)) = zed_color(style, keys) {
            set_field(&mut p, name, c, a);
        }
    }
    p
}

fn slug(name: &str) -> String {
    let s: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    s.split('-')
        .filter(|x| !x.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

fn json_files(dir: &std::path::Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("json"))
                .collect()
        })
        .unwrap_or_default()
}

/// Конвертирует темы Zed (~/.config/zed/themes и установленные расширения) в свои темы.
/// Возвращает (сколько тем импортировано, сколько файлов не удалось прочитать).
pub fn import_zed() -> (usize, usize) {
    let Ok(home) = std::env::var("HOME") else {
        return (0, 0);
    };
    let home = PathBuf::from(home);
    let mut files = json_files(&home.join(".config/zed/themes"));
    if let Ok(exts) = std::fs::read_dir(home.join(".local/share/zed/extensions/installed")) {
        for ext in exts.filter_map(|e| e.ok()) {
            files.extend(json_files(&ext.path().join("themes")));
        }
    }
    let out = themes_dir();
    let _ = std::fs::create_dir_all(&out);
    let (mut imported, mut failed) = (0, 0);
    for file in files {
        let Ok(raw) = std::fs::read_to_string(&file) else {
            failed += 1;
            continue;
        };
        // Файлы Zed допускают комментарии на отдельных строках.
        let raw: String = raw
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        let Ok(doc) = serde_json::from_str::<serde_json::Value>(&raw) else {
            failed += 1;
            continue;
        };
        let Some(themes) = doc.get("themes").and_then(|t| t.as_array()) else {
            continue;
        };
        for t in themes {
            let name = t.get("name").and_then(|n| n.as_str()).unwrap_or("theme");
            let light = t.get("appearance").and_then(|a| a.as_str()) == Some("light");
            let Some(style) = t.get("style") else {
                continue;
            };
            let pal = palette_from_zed(style, light);
            let path = out.join(format!("zed-{}.json", slug(name)));
            if std::fs::write(&path, palette_to_json(&pal)).is_ok() {
                imported += 1;
            }
        }
    }
    (imported, failed)
}

// ---------- свои темы ----------

/// Папка со своими темами: каждый файл `<имя>.json` — одна тема.
pub fn themes_dir() -> PathBuf {
    match std::env::var("HOME") {
        Ok(home) => PathBuf::from(home).join(".config/notesync/themes"),
        Err(_) => PathBuf::from("notesync-themes"),
    }
}

/// Имена своих тем (без `.json`), по алфавиту.
pub fn list_custom() -> Vec<String> {
    let mut names: Vec<String> = json_files(&themes_dir())
        .iter()
        .filter_map(|p| p.file_stem().and_then(|s| s.to_str()).map(String::from))
        .collect();
    names.sort();
    names
}

fn load_custom(name: &str) -> Option<Palette> {
    let raw = std::fs::read_to_string(themes_dir().join(format!("{name}.json"))).ok()?;
    let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
    Some(palette_from_json(&value))
}

/// Создаёт файл новой темы (копия текущей палитры) и возвращает её имя.
pub fn create_custom() -> Result<String, String> {
    let dir = themes_dir();
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Не удалось создать {}: {e}", dir.display()))?;
    let mut name = "my-theme".to_string();
    let mut n = 2;
    while dir.join(format!("{name}.json")).exists() {
        name = format!("my-theme-{n}");
        n += 1;
    }
    let path = dir.join(format!("{name}.json"));
    std::fs::write(&path, palette_to_json(&pal()))
        .map_err(|e| format!("Не удалось записать {}: {e}", path.display()))?;
    Ok(name)
}

/// Выбирает тему по имени: встроенная или своя из файла (нет файла — Mocha).
pub fn set_flavor(name: &str) {
    let builtin = match name {
        "mocha" => Some(0),
        "macchiato" => Some(1),
        "frappe" => Some(2),
        "latte" => Some(3),
        _ => None,
    };
    if let Some(v) = builtin {
        FLAVOR.store(v, Ordering::Relaxed);
        return;
    }
    match load_custom(name) {
        Some(p) => {
            if let Ok(mut c) = CUSTOM.write() {
                *c = Some(p);
            }
            if let Ok(mut n) = CUSTOM_NAME.write() {
                *n = name.to_string();
            }
            FLAVOR.store(4, Ordering::Relaxed);
        }
        None => FLAVOR.store(0, Ordering::Relaxed),
    }
}

pub fn flavor_name() -> String {
    match FLAVOR.load(Ordering::Relaxed) {
        1 => "macchiato".to_string(),
        2 => "frappe".to_string(),
        3 => "latte".to_string(),
        4 => CUSTOM_NAME.read().map(|n| n.clone()).unwrap_or_default(),
        _ => "mocha".to_string(),
    }
}

/// Прозрачность фонов и переопределения цветов из настроек (применяются в `change`).
pub fn configure(opacity: f32, overrides: &HashMap<String, String>) {
    OPACITY.store(opacity.clamp(0.2, 1.0).to_bits(), Ordering::Relaxed);
    if let Ok(mut o) = OVERRIDES.write() {
        *o = overrides
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
    }
}

fn opacity() -> f32 {
    f32::from_bits(OPACITY.load(Ordering::Relaxed))
}

fn base_palette() -> Palette {
    match FLAVOR.load(Ordering::Relaxed) {
        1 => MACCHIATO,
        2 => FRAPPE,
        3 => LATTE,
        4 => CUSTOM.read().ok().and_then(|c| *c).unwrap_or(MOCHA),
        _ => MOCHA,
    }
}

// Пересобирает итоговую палитру: тема + переопределения цветов.
fn rebuild() {
    let mut p = base_palette();
    if let Ok(o) = OVERRIDES.read() {
        for (name, value) in o.iter() {
            if let Some((c, a)) = parse_color(value) {
                set_field(&mut p, name, c, a);
            }
        }
    }
    if let Ok(mut cur) = CURRENT.write() {
        *cur = p;
    }
}

fn pal() -> Palette {
    CURRENT.read().map(|p| *p).unwrap_or(MOCHA)
}

/// Нужна ли окну прозрачность (настройка или цвета темы с прозрачностью).
pub fn has_transparency() -> bool {
    let a = pal().alpha;
    opacity() < 0.999
        || [a.crust, a.mantle, a.base, a.surface0, a.surface1]
            .iter()
            .any(|x| *x < 0.999)
}

// Фоновый цвет с прозрачностью темы и настройки.
fn bg(c: u32, alpha: f32) -> Hsla {
    let color: Hsla = rgb(c).into();
    color.opacity(alpha * opacity())
}

pub fn crust() -> Hsla {
    let p = pal();
    bg(p.crust, p.alpha.crust)
}

pub fn mantle() -> Hsla {
    let p = pal();
    bg(p.mantle, p.alpha.mantle)
}

pub fn base() -> Hsla {
    let p = pal();
    bg(p.base, p.alpha.base)
}

pub fn surface0() -> Hsla {
    let p = pal();
    bg(p.surface0, p.alpha.surface0)
}

pub fn surface1() -> Hsla {
    let p = pal();
    bg(p.surface1, p.alpha.surface1)
}

/// Фон всплывающих панелей: всегда непрозрачный, чтобы текст читался.
pub fn panel() -> Hsla {
    rgb(pal().mantle).into()
}

pub fn overlay0() -> Hsla {
    rgb(pal().overlay0).into()
}

pub fn subtext0() -> Hsla {
    rgb(pal().subtext0).into()
}

pub fn text() -> Hsla {
    rgb(pal().text).into()
}

pub fn blue() -> Hsla {
    rgb(pal().blue).into()
}

pub fn green() -> Hsla {
    rgb(pal().green).into()
}

pub fn peach() -> Hsla {
    rgb(pal().peach).into()
}

pub fn red() -> Hsla {
    rgb(pal().red).into()
}

pub fn yellow() -> Hsla {
    rgb(pal().yellow).into()
}

/// Включает базовую тему gpui-kit (светлую или тёмную) и красит её в текущую палитру.
/// Вызывать при старте и при любой смене темы, прозрачности и цветов.
pub fn change(cx: &mut App) {
    rebuild();
    let mode = if pal().light {
        ThemeMode::Light
    } else {
        ThemeMode::Dark
    };
    Theme::change(mode, None, cx);
    apply(cx);
}

/// Красит глобальную тему gpui-kit в текущую палитру Catppuccin.
///
/// Вызывать ПОСЛЕ `Theme::change`: `change` подгружает встроенную тему и затирает цвета.
/// `Theme::update` сам синхронизирует `colors` и `tokens` и перерисовывает окна,
/// а `global_mut` этого не делает: кнопки читают `tokens.button_*`.
fn apply(cx: &mut App) {
    let p = pal();
    let (base, mantle, crust) = (base(), mantle(), crust());
    let (surface0, surface1, overlay0) = (surface0(), surface1(), overlay0());
    let (text, subtext0) = (text(), subtext0());
    let (blue, green, peach, red) = (blue(), green(), peach(), red());
    let blue_hover: Hsla = rgb(p.lavender).into();
    let blue_active: Hsla = rgb(p.sapphire).into();
    let red_hover: Hsla = rgb(p.maroon).into();
    let red_active: Hsla = red;
    let green_hover: Hsla = rgb(p.teal).into();
    let peach_hover: Hsla = rgb(p.yellow).into();

    Theme::update(cx, |theme| {
        let c = &mut theme.colors;

        // Поверхности
        c.background = base;
        c.foreground = text;
        c.border = surface0;
        c.input = surface0;
        c.muted = surface0;
        c.muted_foreground = subtext0;
        c.popover = rgb(p.mantle).into(); // всплывающие окна всегда непрозрачные
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
