use gtk::prelude::*;
use gtk4 as gtk;
use serde::{Deserialize, Serialize};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;

const JNI_BACKENDS: &[(&str, &str)] = &[("cpp", "C++ libjnivm (default)"), ("rust", "Rust JNI VM")];
const LOG_CATEGORIES: &[(&str, &str, &str)] = &[
    ("base", "Base", "Launcher and Roblox lifecycle messages"),
    (
        "runtime",
        "Runtime",
        "Android compatibility and runtime details",
    ),
    (
        "jnivm",
        "JNI / JNI VM",
        "JNI calls, VM registration, and JNI bridge messages",
    ),
    ("linker", "Linker", "Dynamic linker and symbol resolution"),
    ("stubs", "Stubs", "Calls handled by compatibility stubs"),
    ("input", "Input", "Keyboard, mouse, and controller events"),
    ("webview", "WebView", "Roblox embedded browser bridge"),
    ("session", "Sessions", "Saved login and session activity"),
];
const RENDERERS: &[(&str, &str)] = &[
    ("auto", "Automatic"),
    ("vulkan", "Vulkan"),
    ("opengl", "OpenGL ES"),
];
const PRESENT_MODES: &[(&str, &str)] = &[
    ("fifo", "FIFO (steady VSync)"),
    ("fifo-relaxed", "FIFO relaxed"),
    ("mailbox", "Mailbox (low latency)"),
    ("immediate", "Immediate (tearing allowed)"),
    ("auto", "Automatic"),
];
const GL_SWAP_INTERVALS: &[(&str, &str)] = &[
    ("off", "Off (0)"),
    ("on", "On (1)"),
    ("adaptive", "Adaptive (-1, if supported)"),
];
const THEME_MODES: &[(&str, &str)] = &[("system", "System"), ("dark", "Dark"), ("light", "Light")];
const THEME_PRESETS: &[(&str, &str)] = &[
    ("default", "Default"),
    ("oled", "OLED"),
    ("moled", "MOLED"),
    ("glass", "Glass"),
    ("midnight", "Midnight"),
    ("nord", "Nord"),
    ("dracula", "Dracula"),
    ("rose-pine", "Rosé Pine"),
    ("solarized", "Solarized"),
    ("forest", "Forest"),
    ("sunset", "Sunset"),
];
const THEME_ACCENTS: &[(&str, &str)] = &[
    ("preset", "Match preset"),
    ("blue", "Blue"),
    ("violet", "Violet"),
    ("teal", "Teal"),
    ("green", "Green"),
    ("rose", "Rose"),
    ("amber", "Amber"),
    ("cyan", "Cyan"),
];
const GAME_BOOLEAN_OPTIONS: &[(&str, &str)] = &[
    ("AllTutorialsDisabled", "Disable all tutorials"),
    ("BadgeVisible", "Show badges"),
    ("CameraYInverted", "Invert camera Y axis"),
    ("ChatTranslationEnabled", "Enable chat translation"),
    (
        "ChatTranslationToggleEnabled",
        "Show chat translation toggle",
    ),
    ("ChatVisible", "Show chat"),
    ("Fullscreen", "Start fullscreen"),
    ("PerformanceStatsVisible", "Show performance stats"),
    ("PlayerListVisible", "Show player list"),
    ("PlayerNamesEnabled", "Show player names"),
    ("ReadAloud", "Enable read aloud"),
    ("ReducedMotion", "Reduce motion"),
    ("VignetteEnabled", "Enable vignette"),
];

const GAME_SLIDER_OPTIONS: &[(&str, &str, f64, f64, f64, f64)] = &[
    (
        "MouseSensitivity",
        "Mouse sensitivity",
        0.0,
        4.0,
        0.01,
        0.55,
    ),
    (
        "GamepadCameraSensitivity",
        "Gamepad camera sensitivity",
        0.0,
        4.0,
        0.01,
        0.55,
    ),
    (
        "FramerateCap",
        "Framerate cap (0 = unlimited)",
        0.0,
        360.0,
        1.0,
        120.0,
    ),
    (
        "GraphicsQualityLevel",
        "Graphics quality",
        1.0,
        10.0,
        1.0,
        1.0,
    ),
    ("MasterVolume", "Master volume", 0.0, 1.0, 0.01, 1.0),
    (
        "PartyVoiceVolume",
        "Party voice volume",
        0.0,
        1.0,
        0.01,
        1.0,
    ),
    ("VoiceChatVolume", "Voice chat volume", 0.0, 1.0, 0.01, 1.0),
    ("HapticStrength", "Haptic strength", 0.0, 1.0, 0.01, 1.0),
    (
        "PreferredTransparency",
        "Interface transparency",
        0.0,
        1.0,
        0.01,
        1.0,
    ),
    (
        "StartScreenWidth",
        "Start screen width",
        640.0,
        3840.0,
        10.0,
        800.0,
    ),
    (
        "StartScreenHeight",
        "Start screen height",
        480.0,
        2160.0,
        10.0,
        600.0,
    ),
];

fn choice_dropdown(choices: &[(&str, &str)], selected_id: &str) -> gtk::DropDown {
    let labels: Vec<_> = choices.iter().map(|(_, label)| *label).collect();
    let dropdown = gtk::DropDown::from_strings(&labels);
    let selected = choices
        .iter()
        .position(|(id, _)| *id == selected_id)
        .unwrap_or(0);
    dropdown.set_selected(selected as u32);
    dropdown
}

fn selected_choice_id<'a>(
    dropdown: &gtk::DropDown,
    choices: &'a [(&'a str, &str)],
) -> Option<&'a str> {
    choices.get(dropdown.selected() as usize).map(|(id, _)| *id)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Settings {
    pub gamemode: bool,
    pub discord_presence: bool,
    pub discord_application_id: String,
    pub renderer: String,
    pub vsync: bool,
    pub present_mode: String,
    pub gl_swap_interval: String,
    pub rust_jnivm: bool,
    pub jnivm_cpp_fallback: bool,
    pub host_libc: bool,
    pub fps_limit: Option<u32>,
    pub session: Option<String>,
    pub log_categories: Vec<String>,
    pub wasd_last_pressed: bool,
    pub disabled_fast_flags: Vec<String>,
    pub theme_mode: String,
    pub theme_preset: String,
    pub theme_accent: String,
    pub animations: bool,
    pub hardware_acceleration: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            gamemode: true,
            discord_presence: false,
            discord_application_id: String::new(),
            renderer: "auto".into(),
            vsync: true,
            present_mode: "fifo".into(),
            gl_swap_interval: "on".into(),
            rust_jnivm: false,
            jnivm_cpp_fallback: true,
            host_libc: false,
            fps_limit: None,
            session: None,
            log_categories: vec!["base".into(), "runtime".into()],
            wasd_last_pressed: false,
            disabled_fast_flags: Vec::new(),
            theme_mode: "dark".into(),
            theme_preset: "default".into(),
            theme_accent: "preset".into(),
            animations: true,
            hardware_acceleration: true,
        }
    }
}

pub(crate) fn path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("rusty-blox").join("settings.json")
}

pub(crate) fn fast_flags_path() -> PathBuf {
    path().with_file_name("fast-flags.json")
}

pub(crate) fn game_settings_path() -> PathBuf {
    crate::client::managed_install_dir()
        .and_then(|path| path.parent().map(|parent| parent.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("data/files/appData/GlobalBasicSettings_13.xml")
}

pub(crate) fn load() -> Settings {
    std::fs::read(path())
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

pub(crate) fn save(settings: &Settings) -> Result<(), String> {
    let path = path();
    let parent = path.parent().ok_or("settings path has no parent")?;
    std::fs::create_dir_all(parent).map_err(|e| format!("create settings directory: {e}"))?;
    let bytes = serde_json::to_vec_pretty(settings).map_err(|e| e.to_string())?;
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, bytes).map_err(|e| format!("write settings: {e}"))?;
    std::fs::rename(&temp, &path).map_err(|e| format!("save settings: {e}"))
}

fn accent_color(mode: &str, preset: &str, accent: &str) -> &'static str {
    let light = mode != "dark";
    if accent == "preset" {
        return match (light, preset) {
            (_, "dracula" | "rose-pine") => "#bd93f9",
            (_, "nord" | "solarized") => "#4d91b5",
            (_, "forest") => "#4b9b63",
            (_, "sunset") => "#d77a48",
            (false, "glass") => "#9a8cff",
            (false, "oled") => "#46a0ff",
            (false, "moled" | "midnight") => "#6ca8ff",
            (true, "glass") => "#6757d9",
            (_, _) => "#2878e5",
        };
    }
    match (light, accent) {
        (true, "violet") => "#7057d8",
        (true, "teal") => "#13877e",
        (true, "green") => "#32824f",
        (true, "rose") => "#cf5470",
        (true, "amber") => "#a86112",
        (true, "cyan") => "#087fa6",
        (false, "violet") => "#aa96ff",
        (false, "teal") => "#61cdbd",
        (false, "green") => "#78c995",
        (false, "rose") => "#ff829b",
        (false, "amber") => "#f5b864",
        (false, "cyan") => "#62c8ec",
        (true, _) => "#2878e5",
        (false, _) => "#69a5ff",
    }
}

fn theme_css(mode: &str, preset: &str, accent_id: &str) -> String {
    if mode == "system" {
        return "window { background: @theme_bg_color; color: @theme_fg_color; } .launcher-shell { padding: 10px; } .launcher-header { padding: 17px 21px; background: @theme_base_color; border: 1px solid alpha(@theme_fg_color, 0.12); border-radius: 14px; } .launcher-title { font-size: 23px; font-weight: 700; letter-spacing: -0.3px; } .launcher-subtitle { color: @insensitive_fg_color; } .launcher-sidebar { padding: 8px; background: @theme_base_color; border: 1px solid alpha(@theme_fg_color, 0.12); border-radius: 14px; } stacksidebar row { padding: 8px 10px; border-radius: 9px; } stacksidebar row:selected { background: @theme_selected_bg_color; color: @theme_selected_fg_color; } .launcher-content { padding: 5px 0 5px 3px; } .launcher-footer { padding: 12px 16px; background: @theme_base_color; border: 1px solid alpha(@theme_fg_color, 0.12); border-radius: 14px; } .launcher-page { padding: 8px 10px 16px; } .launcher-page-title { font-size: 21px; font-weight: 700; letter-spacing: -0.2px; } .launcher-page-copy { color: @insensitive_fg_color; } .launcher-hero { padding: 26px; background: @theme_base_color; border: 1px solid alpha(@theme_fg_color, 0.12); border-radius: 15px; } .launcher-status, .launcher-stat, .launcher-card { padding: 16px; background: @theme_base_color; border: 1px solid alpha(@theme_fg_color, 0.12); border-radius: 12px; } .launcher-card checkbutton, .launcher-page > checkbutton { padding: 5px 2px; } .launcher-flag-row { padding: 10px 12px; border-bottom: 1px solid alpha(@theme_fg_color, 0.09); } button { min-height: 34px; padding: 4px 12px; border-radius: 9px; } button.suggested-action { background: @theme_selected_bg_color; color: @theme_selected_fg_color; } entry, textview, spinbutton, dropdown, combobox { border-radius: 8px; } .launcher-eyebrow { color: $ACCENT_COLOR; font-size: 11px; font-weight: 700; letter-spacing: 1px; } .launcher-stat-title { color: @insensitive_fg_color; font-size: 11px; font-weight: 700; letter-spacing: 0.6px; } .launcher-stat-value { font-size: 16px; font-weight: 650; } .launcher-pill { padding: 5px 9px; background: alpha($ACCENT_COLOR, 0.14); border-radius: 999px; }".replace("$ACCENT_COLOR", accent_color(mode, preset, accent_id));
    }
    let accent = accent_color(mode, preset, accent_id);
    let light = mode == "light";
    let (background, surface, surface_alt, text, muted, border, _preset_accent) =
        match (light, preset) {
            (true, "oled") => (
                "#f8fafc", "#ffffff", "#eef2f7", "#172033", "#64748b", "#d5dce7", "#1677e8",
            ),
            (true, "glass") => (
                "#e8eef7",
                "rgba(255,255,255,0.82)",
                "rgba(237,243,251,0.78)",
                "#172033",
                "#64748b",
                "#c7d3e2",
                "#6757d9",
            ),
            (true, "moled") => (
                "#edf2fa", "#e1e9f5", "#d5e0f0", "#18243a", "#62718a", "#c2cee0", "#2775db",
            ),
            (true, "midnight") => (
                "#edf1f9", "#ffffff", "#e2e9f5", "#1c2940", "#64748b", "#cbd5e3", "#496eb1",
            ),
            (true, "nord") => (
                "#eceff4", "#ffffff", "#e5e9f0", "#2e3440", "#616e88", "#d4d9e2", "#5e81ac",
            ),
            (true, "dracula") => (
                "#f4f2f8", "#ffffff", "#ece8f4", "#282a36", "#696779", "#d9d4e3", "#8b63c7",
            ),
            (true, "rose-pine") => (
                "#f6f1f2", "#fffafa", "#eee5e8", "#433b4a", "#796f7f", "#dfd1d7", "#a55d79",
            ),
            (true, "solarized") => (
                "#fdf6e3", "#fffdf5", "#eee8d5", "#586e75", "#839496", "#ded7c6", "#268bd2",
            ),
            (true, "forest") => (
                "#eff4ef", "#ffffff", "#e4ede5", "#26382c", "#687a6c", "#d0ded2", "#3d8054",
            ),
            (true, "sunset") => (
                "#fbf1ec", "#fffaf6", "#f4e4dc", "#49353b", "#806b6e", "#e6d1c8", "#c56a42",
            ),
            (true, _) => (
                "#f2f4f8", "#ffffff", "#e9edf3", "#202938", "#697586", "#d6dce5", "#2878e5",
            ),
            (false, "oled") => (
                "#000000", "#080808", "#111111", "#f4f7fb", "#9aa6b6", "#252525", "#46a0ff",
            ),
            (false, "glass") => (
                "#111722",
                "rgba(30,39,54,0.82)",
                "rgba(39,50,68,0.78)",
                "#f1f5fc",
                "#a7b4c7",
                "#45536a",
                "#9a8cff",
            ),
            (false, "moled") => (
                "#101827", "#182337", "#202e45", "#edf3fc", "#a7b5ca", "#34445e", "#6ca8ff",
            ),
            (false, "midnight") => (
                "#0a0f1c", "#10192a", "#17243a", "#eef3fc", "#9aa9c0", "#293850", "#70a4ff",
            ),
            (false, "nord") => (
                "#2e3440", "#3b4252", "#434c5e", "#eceff4", "#c3cad5", "#525e72", "#88c0d0",
            ),
            (false, "dracula") => (
                "#21222c", "#282a36", "#343746", "#f8f8f2", "#b8b7c5", "#45475a", "#bd93f9",
            ),
            (false, "rose-pine") => (
                "#191724", "#1f1d2e", "#26233a", "#e0def4", "#a6a3c0", "#403c56", "#c4a7e7",
            ),
            (false, "solarized") => (
                "#002b36", "#073642", "#0b414b", "#fdf6e3", "#93a1a1", "#31535b", "#2aa198",
            ),
            (false, "forest") => (
                "#101b18", "#17251f", "#20342b", "#edf5ee", "#a3b5a7", "#344c3d", "#75c58d",
            ),
            (false, "sunset") => (
                "#211a24", "#30222c", "#412c34", "#fff2e8", "#c8aeb0", "#573a42", "#ff9e64",
            ),
            (false, _) => (
                "#11151d", "#191f2b", "#252c38", "#eff3fa", "#a6b1c2", "#2b3444", "#3887ed",
            ),
        };
    format!(
        "window {{ background: {background}; color: {text}; }} .launcher-shell {{ padding: 10px; }} .launcher-header {{ padding: 17px 21px; background: {surface}; border: 1px solid {border}; border-radius: 14px; }} .launcher-title {{ font-size: 23px; font-weight: 700; letter-spacing: -0.3px; }} .launcher-subtitle {{ color: {muted}; }} .launcher-sidebar {{ padding: 8px; background: {surface}; border: 1px solid {border}; border-radius: 14px; }} stacksidebar row {{ padding: 8px 10px; border-radius: 9px; }} stacksidebar row:selected {{ background: {surface_alt}; }} .launcher-content {{ padding: 5px 0 5px 3px; }} .launcher-footer {{ padding: 12px 16px; background: {surface}; border: 1px solid {border}; border-radius: 14px; }} .launcher-page {{ padding: 8px 10px 16px; }} .launcher-page-title {{ font-size: 21px; font-weight: 700; letter-spacing: -0.2px; }} .launcher-page-copy {{ color: {muted}; }} .launcher-hero {{ padding: 26px; background: {surface_alt}; border: 1px solid {border}; border-radius: 15px; }} .launcher-status, .launcher-stat, .launcher-card {{ padding: 16px; background: {surface}; border: 1px solid {border}; border-radius: 12px; }} .launcher-card checkbutton, .launcher-page > checkbutton {{ padding: 5px 2px; }} .launcher-flag-row {{ padding: 10px 12px; border-bottom: 1px solid {border}; }} button {{ min-height: 34px; padding: 4px 12px; border-radius: 9px; }} button.suggested-action {{ background: {accent}; }} entry, textview, spinbutton, dropdown, combobox {{ border-radius: 8px; }} .launcher-eyebrow {{ color: {accent}; font-size: 11px; font-weight: 700; letter-spacing: 1px; }} .launcher-stat-title {{ color: {muted}; font-size: 11px; font-weight: 700; letter-spacing: 0.6px; }} .launcher-stat-value {{ font-size: 16px; font-weight: 650; }} .launcher-pill {{ padding: 5px 9px; background: {surface_alt}; border-radius: 999px; }}"
    )
}

fn set_interface_color_scheme(settings: &gtk::Settings, mode: &str, system_dark: bool) {
    if let Some(property) = settings.find_property("gtk-interface-color-scheme") {
        if let Some(enum_class) = gtk::glib::EnumClass::with_type(property.value_type()) {
            let nick = match mode {
                "dark" => "dark",
                "light" => "light",
                _ => "default",
            };
            if let Some(value) = enum_class.value_by_nick(nick) {
                settings.set_property_from_value(
                    "gtk-interface-color-scheme",
                    &value.to_value(&enum_class),
                );
            }
        }
    } else {
        // GTK before 4.20 lacks gtk-interface-color-scheme.
        settings.set_property(
            "gtk-application-prefer-dark-theme",
            match mode {
                "dark" => true,
                "light" => false,
                _ => system_dark,
            },
        );
    }
}

fn parse_flag_value(text: &str, original: &serde_json::Value) -> Option<serde_json::Value> {
    match original {
        serde_json::Value::String(_) => Some(text.to_owned().into()),
        serde_json::Value::Number(_) => serde_json::from_str(text)
            .ok()
            .filter(serde_json::Value::is_number),
        serde_json::Value::Bool(_) => text.parse::<bool>().ok().map(Into::into),
        _ => serde_json::from_str(text).ok(),
    }
}

fn render_fast_flag_rows(
    table: &gtk::Box,
    flags: Rc<RefCell<BTreeMap<String, serde_json::Value>>>,
    disabled: Rc<RefCell<HashSet<String>>>,
    parent: gtk::Window,
    status: gtk::Label,
) {
    while let Some(child) = table.first_child() {
        table.remove(&child);
    }
    for (key, value) in flags.borrow().iter() {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        row.add_css_class("launcher-flag-row");
        let key_label = gtk::Label::new(Some(key));
        key_label.set_halign(gtk::Align::Start);
        key_label.set_hexpand(true);
        key_label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        row.append(&key_label);
        if let Some(boolean) = value.as_bool() {
            let enabled = gtk::CheckButton::with_label("True");
            enabled.set_active(boolean);
            let key = key.clone();
            let values = flags.clone();
            enabled.connect_toggled(move |control| {
                values
                    .borrow_mut()
                    .insert(key.clone(), control.is_active().into());
            });
            row.append(&enabled);
        } else {
            let enabled = gtk::CheckButton::with_label("Enabled");
            enabled.set_active(!disabled.borrow().contains(key));
            let key_for_toggle = key.clone();
            let off = disabled.clone();
            enabled.connect_toggled(move |control| {
                if control.is_active() {
                    off.borrow_mut().remove(&key_for_toggle);
                } else {
                    off.borrow_mut().insert(key_for_toggle.clone());
                }
            });
            row.append(&enabled);
            let entry = gtk::Entry::new();
            entry.set_hexpand(true);
            entry.set_width_chars(16);
            entry.set_text(&match value {
                serde_json::Value::String(text) => text.clone(),
                _ => value.to_string(),
            });
            let original = value.clone();
            let key_for_value = key.clone();
            let values = flags.clone();
            entry.connect_changed(move |entry| {
                if let Some(value) = parse_flag_value(&entry.text(), &original) {
                    values.borrow_mut().insert(key_for_value.clone(), value);
                }
            });
            row.append(&entry);
        }
        let remove = gtk::Button::with_label("Remove");
        let key_for_remove = key.clone();
        let values = flags.clone();
        let off = disabled.clone();
        let table_ref = table.clone();
        let parent_ref = parent.clone();
        let status_ref = status.clone();
        remove.connect_clicked(move |_| {
            let confirm = gtk::AlertDialog::builder()
                .message(format!("Remove {key_for_remove}?"))
                .detail("This flag will be deleted from your saved FastFlags.")
                .buttons(["Cancel", "Remove"])
                .cancel_button(0)
                .default_button(1)
                .build();
            let values = values.clone();
            let off = off.clone();
            let table = table_ref.clone();
            let parent = parent_ref.clone();
            let status = status_ref.clone();
            let key = key_for_remove.clone();
            confirm.choose(
                Some(&parent_ref),
                None::<&gtk::gio::Cancellable>,
                move |response| {
                    if matches!(response, Ok(1)) {
                        values.borrow_mut().remove(&key);
                        off.borrow_mut().remove(&key);
                        status.set_text(&format!(
                            "Removed {key}. Save FastFlags to apply the change."
                        ));
                        render_fast_flag_rows(
                            &table,
                            values.clone(),
                            off.clone(),
                            parent.clone(),
                            status.clone(),
                        );
                    }
                },
            );
        });
        row.append(&remove);
        table.append(&row);
    }
}

fn import_fast_flags(
    incoming: serde_json::Map<String, serde_json::Value>,
    flags: Rc<RefCell<BTreeMap<String, serde_json::Value>>>,
    disabled: Rc<RefCell<HashSet<String>>>,
    table: gtk::Box,
    parent: gtk::Window,
    status: gtk::Label,
) {
    let conflicts = incoming
        .iter()
        .filter_map(|(key, imported)| {
            flags
                .borrow()
                .get(key)
                .filter(|current| *current != imported)
                .map(|_| key.clone())
        })
        .collect::<Vec<_>>();
    let merge = {
        let flags = flags.clone();
        let disabled = disabled.clone();
        let parent = parent.clone();
        let table = table.clone();
        let status = status.clone();
        move |override_conflicts: bool| {
            for (key, value) in &incoming {
                if override_conflicts || !flags.borrow().contains_key(key) {
                    flags.borrow_mut().insert(key.clone(), value.clone());
                    disabled.borrow_mut().remove(key);
                }
            }
            render_fast_flag_rows(
                &table,
                flags.clone(),
                disabled.clone(),
                parent.clone(),
                status.clone(),
            );
            status.set_text(
                "Imported flags. Review the list, then save to apply them on the next launch.",
            );
        }
    };
    if conflicts.is_empty() {
        merge(true);
        return;
    }
    let dialog = gtk::AlertDialog::builder()
        .message(format!(
            "{} flag(s) conflict with your current values",
            conflicts.len()
        ))
        .detail(conflicts.join(", "))
        .buttons(["Cancel", "Keep current", "Override conflicts"])
        .cancel_button(0)
        .default_button(2)
        .build();
    dialog.choose(
        Some(&parent),
        None::<&gtk::gio::Cancellable>,
        move |response| match response {
            Ok(1) => merge(false),
            Ok(2) => merge(true),
            _ => {}
        },
    );
}

pub(crate) fn xml_frame_cap() -> Option<u32> {
    let contents = std::fs::read_to_string(game_settings_path()).ok()?;
    parse_game_values(&contents)
        .ok()?
        .get("FramerateCap")?
        .parse()
        .ok()
}

pub(crate) fn configured_frame_cap(settings: &Settings) -> Option<u32> {
    settings.fps_limit.or_else(xml_frame_cap)
}

pub(crate) fn run_ui() -> Result<bool, String> {
    let startup_settings = load();
    // GTK reads the renderer variable before initialization. Cairo is the
    // software fallback; GL is GTK's accelerated renderer.
    unsafe {
        std::env::set_var(
            "GSK_RENDERER",
            if startup_settings.hardware_acceleration {
                "gl"
            } else {
                "cairo"
            },
        );
    }
    let app = gtk::Application::builder()
        .application_id("org.rustyblox.Settings")
        .build();
    let play_requested = Rc::new(Cell::new(false));
    let play_requested_activate = play_requested.clone();
    app.connect_activate(move |app| {
        let settings = load();
        let initial_frame_cap = effective_frame_cap(&settings);
        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .title("rusty-blox")
            .default_width(1100)
            .default_height(760)
            .build();
        let css = gtk::CssProvider::new();
        css.load_from_data(&theme_css(
            &settings.theme_mode,
            &settings.theme_preset,
            &settings.theme_accent,
        ));
        let system_dark_preference = gtk::Settings::default()
            .map(|gtk_settings| {
                gtk_settings.find_property("gtk-interface-color-scheme").is_none()
                    && gtk_settings.property::<bool>("gtk-application-prefer-dark-theme")
            })
            .unwrap_or(false);
        if let Some(gtk_settings) = gtk::Settings::default() {
            set_interface_color_scheme(&gtk_settings, &settings.theme_mode, system_dark_preference);
            gtk_settings.set_property("gtk-enable-animations", settings.animations);
        }
        if let Some(display) = gtk::gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display,
                &css,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
        let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
        root.add_css_class("launcher-shell");
        root.set_margin_top(14);
        root.set_margin_bottom(14);
        root.set_margin_start(14);
        root.set_margin_end(14);
        let header = gtk::Box::new(gtk::Orientation::Vertical, 4);
        header.add_css_class("launcher-header");
        let title = gtk::Label::new(Some("rusty-blox"));
        title.add_css_class("launcher-title");
        title.set_halign(gtk::Align::Start);
        header.append(&title);
        let subtitle = gtk::Label::new(Some("Roblox desktop launcher · configure once, then play"));
        subtitle.add_css_class("launcher-subtitle");
        subtitle.set_halign(gtk::Align::Start);
        header.append(&subtitle);
        root.append(&header);
        let stack = gtk::Stack::new();
        stack.set_transition_type(gtk::StackTransitionType::Crossfade);
        stack.set_hexpand(true);
        stack.set_vexpand(true);
        let sidebar = gtk::StackSidebar::new();
        sidebar.add_css_class("launcher-sidebar");
        sidebar.set_stack(&stack);
        sidebar.set_size_request(150, -1);
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        content.add_css_class("launcher-content");
        content.set_vexpand(true);
        content.append(&sidebar);
        content.append(&stack);

        let home_page = gtk::Box::new(gtk::Orientation::Vertical, 18);
        home_page.set_margin_start(8);
        home_page.set_margin_end(8);
        let hero = gtk::Box::new(gtk::Orientation::Vertical, 12);
        hero.add_css_class("launcher-hero");
        let hero_eyebrow = gtk::Label::new(Some("PLAYER DASHBOARD"));
        hero_eyebrow.add_css_class("launcher-eyebrow");
        hero_eyebrow.set_halign(gtk::Align::Start);
        hero.append(&hero_eyebrow);
        let hero_title = gtk::Label::new(Some("Your Roblox, ready to launch."));
        hero_title.add_css_class("launcher-title");
        hero_title.set_halign(gtk::Align::Start);
        hero.append(&hero_title);
        let hero_copy = gtk::Label::new(Some(
            "Manage your client, tune the runtime, and jump back into the game.",
        ));
        hero_copy.add_css_class("launcher-subtitle");
        hero_copy.set_halign(gtk::Align::Start);
        hero.append(&hero_copy);
        home_page.append(&hero);

        let managed_dir = crate::client::managed_install_dir();
        let has_managed_client = managed_dir
            .as_ref()
            .is_some_and(|path| crate::client::discover_managed_client(path).is_some());
        let has_sober_client = crate::client::discover_sober_apks().is_some();
        let install_status = gtk::Label::new(Some(if has_managed_client {
            "Roblox is installed and ready to play."
        } else if has_sober_client {
            "A Sober Roblox install was found. It will be imported when you play."
        } else {
            "Roblox is not installed yet. Download the latest client here to get started."
        }));
        install_status.set_wrap(true);
        install_status.set_halign(gtk::Align::Start);
        let install_card = gtk::Box::new(gtk::Orientation::Vertical, 12);
        install_card.add_css_class("launcher-status");
        install_card.append(&install_status);
        let update_button = gtk::Button::with_label(if has_managed_client {
            "Check for updates"
        } else {
            "Download Roblox"
        });
        update_button.set_halign(gtk::Align::Start);
        install_card.append(&update_button);
        home_page.append(&install_card);
        let update_in_progress = Rc::new(Cell::new(false));
        let update_in_progress_ref = update_in_progress.clone();
        let client_ready = Rc::new(Cell::new(has_managed_client || has_sober_client));
        let client_ready_for_update = client_ready.clone();
        let install_status_ref = install_status.clone();
        let update_button_ref = update_button.clone();
        update_button.connect_clicked(move |_| {
            update_in_progress_ref.set(true);
            update_button_ref.set_sensitive(false);
            update_button_ref.set_label("Downloading and verifying…");
            install_status_ref.set_text("Downloading Roblox. The package signature will be checked before installation.");
            let (sender, receiver) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let result = std::env::current_exe()
                    .map_err(|error| error.to_string())
                    .and_then(|executable| {
                        std::process::Command::new(executable)
                            .arg("--update-apk")
                            .output()
                            .map_err(|error| format!("start updater: {error}"))
                    })
                    .and_then(|output| {
                        if output.status.success() {
                            Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
                        } else {
                            Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
                        }
                    });
                let _ = sender.send(result);
            });
            let status = install_status.clone();
            let button = update_button_ref.clone();
            let update_in_progress = update_in_progress_ref.clone();
            let client_ready = client_ready_for_update.clone();
            gtk::glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
                match receiver.try_recv() {
                    Ok(Ok(message)) => {
                        update_in_progress.set(false);
                        client_ready.set(true);
                        status.set_text(if message.is_empty() {
                            "Roblox is installed and ready to play."
                        } else {
                            &message
                        });
                        button.set_label("Check for updates");
                        button.set_sensitive(true);
                        gtk::glib::ControlFlow::Break
                    }
                    Ok(Err(error)) => {
                        update_in_progress.set(false);
                        status.set_text(&format!("Roblox update failed: {error}"));
                        button.set_label("Try again");
                        button.set_sensitive(true);
                        gtk::glib::ControlFlow::Break
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => gtk::glib::ControlFlow::Continue,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        update_in_progress.set(false);
                        status.set_text("The updater stopped before reporting a result.");
                        button.set_label("Try again");
                        button.set_sensitive(true);
                        gtk::glib::ControlFlow::Break
                    }
                }
            });
        });

        let quick_stats = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let fps_label = initial_frame_cap.to_string();
        for (title, value) in [
            (
                "ACCOUNT",
                settings.session.as_deref().unwrap_or("No saved session"),
            ),
            ("RENDERER", settings.renderer.as_str()),
            ("FPS LIMIT", fps_label.as_str()),
        ] {
            let stat = gtk::Box::new(gtk::Orientation::Vertical, 6);
            stat.add_css_class("launcher-stat");
            stat.set_hexpand(true);
            let title_label = gtk::Label::new(Some(title));
            title_label.add_css_class("launcher-stat-title");
            title_label.set_halign(gtk::Align::Start);
            let value_label = gtk::Label::new(Some(value));
            value_label.add_css_class("launcher-stat-value");
            value_label.set_halign(gtk::Align::Start);
            stat.append(&title_label);
            stat.append(&value_label);
            quick_stats.append(&stat);
        }
        home_page.append(&quick_stats);
        stack.add_titled(&home_page, Some("home"), "Home");

        let appearance_page = gtk::Box::new(gtk::Orientation::Vertical, 14);
        appearance_page.add_css_class("launcher-page");
        appearance_page.set_margin_start(10);
        appearance_page.set_margin_end(10);
        let appearance_heading = gtk::Label::new(Some("Appearance and effects"));
        appearance_heading.add_css_class("launcher-page-title");
        appearance_heading.set_halign(gtk::Align::Start);
        appearance_page.append(&appearance_heading);
        let appearance_copy = gtk::Label::new(Some("Choose a color scheme, mix accent colors, and tune the launcher's visual effects. System mode follows your desktop palette."));
        appearance_copy.add_css_class("launcher-page-copy");
        appearance_copy.set_halign(gtk::Align::Start);
        appearance_page.append(&appearance_copy);
        let appearance_card = gtk::Box::new(gtk::Orientation::Vertical, 14);
        appearance_card.add_css_class("launcher-card");
        let theme_mode = choice_dropdown(THEME_MODES, &settings.theme_mode);
        appearance_card.append(&labeled("Color scheme", &theme_mode));
        let theme_preset = choice_dropdown(THEME_PRESETS, &settings.theme_preset);
        appearance_card.append(&labeled("Theme preset", &theme_preset));
        let theme_accent = choice_dropdown(THEME_ACCENTS, &settings.theme_accent);
        appearance_card.append(&labeled("Accent color", &theme_accent));
        let animations = gtk::CheckButton::with_label("Enable interface animations");
        animations.set_active(settings.animations);
        appearance_card.append(&animations);
        let hardware_acceleration = gtk::CheckButton::with_label("Use hardware acceleration for the launcher UI");
        hardware_acceleration.set_active(settings.hardware_acceleration);
        appearance_card.append(&hardware_acceleration);
        appearance_page.append(&appearance_card);
        let appearance_note = gtk::Label::new(Some("Theme colors and animations update immediately. The renderer setting applies after restarting rusty-blox."));
        appearance_note.set_wrap(true);
        appearance_note.set_halign(gtk::Align::Start);
        appearance_page.append(&appearance_note);
        let appearance_status = gtk::Label::new(None);
        appearance_status.set_halign(gtk::Align::Start);
        appearance_page.append(&appearance_status);
        let appearance_save = gtk::Button::with_label("Save appearance");
        let theme_mode_live = theme_mode.clone();
        let theme_preset_live = theme_preset.clone();
        let theme_accent_live = theme_accent.clone();
        let css_live = css.clone();
        let animations_live = animations.clone();
        let system_dark_preference_live = system_dark_preference;
        let update_theme: Rc<dyn Fn()> = Rc::new(move || {
            let mode = selected_choice_id(&theme_mode_live, THEME_MODES).unwrap_or("dark");
            let preset = selected_choice_id(&theme_preset_live, THEME_PRESETS).unwrap_or("default");
            let accent = selected_choice_id(&theme_accent_live, THEME_ACCENTS).unwrap_or("preset");
            css_live.load_from_data(&theme_css(mode, preset, accent));
            if let Some(gtk_settings) = gtk::Settings::default() {
                set_interface_color_scheme(&gtk_settings, mode, system_dark_preference_live);
                gtk_settings.set_property("gtk-enable-animations", animations_live.is_active());
            }
        });
        let theme_mode_update = update_theme.clone();
        theme_mode.connect_selected_notify(move |_| theme_mode_update());
        let theme_preset_update = update_theme.clone();
        theme_preset.connect_selected_notify(move |_| theme_preset_update());
        let theme_accent_update = update_theme.clone();
        theme_accent.connect_selected_notify(move |_| theme_accent_update());
        animations.connect_toggled(move |_| update_theme());
        let mode_for_save = theme_mode.clone();
        let preset_for_save = theme_preset.clone();
        let accent_for_save = theme_accent.clone();
        let animations_for_save = animations.clone();
        let acceleration_for_save = hardware_acceleration.clone();
        let appearance_status_for_save = appearance_status.clone();
        appearance_save.connect_clicked(move |_| {
            let mut value = load();
            value.theme_mode = selected_choice_id(&mode_for_save, THEME_MODES).unwrap_or("dark").to_owned();
            value.theme_preset = selected_choice_id(&preset_for_save, THEME_PRESETS).unwrap_or("default").to_owned();
            value.theme_accent = selected_choice_id(&accent_for_save, THEME_ACCENTS).unwrap_or("preset").to_owned();
            value.animations = animations_for_save.is_active();
            value.hardware_acceleration = acceleration_for_save.is_active();
            match save(&value) {
                Ok(()) => appearance_status_for_save.set_text("Appearance saved."),
                Err(error) => appearance_status_for_save.set_text(&error),
            }
        });
        appearance_save.set_halign(gtk::Align::Start);
        appearance_page.append(&appearance_save);
        stack.add_titled(&appearance_page, Some("appearance"), "Appearance");

        let settings_page = gtk::Box::new(gtk::Orientation::Vertical, 12);
        settings_page.add_css_class("launcher-page");
        let heading = gtk::Label::new(Some("Performance and client settings"));
        heading.add_css_class("launcher-page-title");
        heading.set_halign(gtk::Align::Start);
        settings_page.append(&heading);
        let settings_copy = gtk::Label::new(Some("Runtime, graphics, logging, and account preferences."));
        settings_copy.add_css_class("launcher-page-copy");
        settings_copy.set_halign(gtk::Align::Start);
        settings_page.append(&settings_copy);

        let logs_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
        logs_box.add_css_class("launcher-card");
        let logs_heading = gtk::Label::new(Some("Log categories"));
        logs_heading.add_css_class("heading");
        logs_heading.set_halign(gtk::Align::Start);
        logs_box.append(&logs_heading);
        let log_controls: Vec<(String, gtk::CheckButton)> = LOG_CATEGORIES.iter().map(|(id, label, description)| {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let control = gtk::CheckButton::with_label(label);
            control.set_active(settings.log_categories.iter().any(|category| category == id));
            let hint = gtk::Label::new(Some(description));
            hint.set_halign(gtk::Align::Start);
            hint.set_hexpand(true);
            row.append(&control);
            row.append(&hint);
            logs_box.append(&row);
            ((*id).to_owned(), control)
        }).collect();
        settings_page.append(&logs_box);
        let wasd_last_pressed = gtk::CheckButton::with_label(
            "Prioritize the last pressed WASD direction while opposite keys are held",
        );
        wasd_last_pressed.set_active(settings.wasd_last_pressed);
        settings_page.append(&wasd_last_pressed);

        let jni_backend = choice_dropdown(
            JNI_BACKENDS,
            if settings.rust_jnivm { "rust" } else { "cpp" },
        );
        settings_page.append(&labeled("JNI backend", &jni_backend));
        let jni_fallback = gtk::CheckButton::with_label(
            "Use C++ libjnivm for methods and fields Rust does not handle",
        );
        jni_fallback.set_active(settings.jnivm_cpp_fallback);
        jni_fallback.set_sensitive(settings.rust_jnivm);
        let fallback_ref = jni_fallback.clone();
        jni_backend.connect_selected_notify(move |backend| {
            fallback_ref.set_sensitive(selected_choice_id(backend, JNI_BACKENDS) == Some("rust"));
        });
        settings_page.append(&jni_fallback);

        let host_libc = gtk::CheckButton::with_label(
            "Use host libc for otherwise unresolved imports (ABI-unsafe diagnostic mode)",
        );
        host_libc.set_active(settings.host_libc);
        settings_page.append(&host_libc);

        let gamemode = gtk::CheckButton::with_label("Enable Feral GameMode while Roblox runs");
        gamemode.set_active(settings.gamemode);
        settings_page.append(&gamemode);
        let presence = gtk::CheckButton::with_label("Enable Discord Rich Presence");
        presence.set_active(settings.discord_presence);
        settings_page.append(&presence);
        let app_id = gtk::Entry::builder()
            .placeholder_text("Discord Application ID")
            .text(&settings.discord_application_id)
            .build();
        app_id.set_sensitive(settings.discord_presence);
        let app_id_ref = app_id.clone();
        presence.connect_toggled(move |button| app_id_ref.set_sensitive(button.is_active()));
        settings_page.append(&app_id);

        let renderer = choice_dropdown(RENDERERS, &settings.renderer);
        settings_page.append(&labeled("Rendering pipeline", &renderer));
        let vsync = gtk::CheckButton::with_label("Enable VSync");
        vsync.set_active(settings.vsync);
        settings_page.append(&vsync);
        let present = choice_dropdown(PRESENT_MODES, &settings.present_mode);
        present.set_sensitive(settings.vsync && settings.renderer == "vulkan");
        let present_ref = present.clone();
        let vsync_ref = vsync.clone();
        let renderer_ref = renderer.clone();
        vsync.connect_toggled(move |_| {
            present_ref.set_sensitive(
                vsync_ref.is_active()
                    && selected_choice_id(&renderer_ref, RENDERERS) == Some("vulkan"),
            )
        });
        let present_ref = present.clone();
        let vsync_ref = vsync.clone();
        let renderer_ref = renderer.clone();
        renderer.connect_selected_notify(move |_| {
            present_ref.set_sensitive(
                vsync_ref.is_active()
                    && selected_choice_id(&renderer_ref, RENDERERS) == Some("vulkan"),
            )
        });
        settings_page.append(&labeled("Vulkan presentation mode", &present));
        let gl_interval = choice_dropdown(GL_SWAP_INTERVALS, &settings.gl_swap_interval);
        gl_interval.set_sensitive(settings.vsync && settings.renderer == "opengl");
        let gl_interval_ref = gl_interval.clone();
        let vsync_ref = vsync.clone();
        let renderer_ref = renderer.clone();
        vsync.connect_toggled(move |_| {
            gl_interval_ref.set_sensitive(
                vsync_ref.is_active()
                    && selected_choice_id(&renderer_ref, RENDERERS) == Some("opengl"),
            )
        });
        let gl_interval_ref = gl_interval.clone();
        let vsync_ref = vsync.clone();
        let renderer_ref = renderer.clone();
        renderer.connect_selected_notify(move |_| {
            gl_interval_ref.set_sensitive(
                vsync_ref.is_active()
                    && selected_choice_id(&renderer_ref, RENDERERS) == Some("opengl"),
            )
        });
        settings_page.append(&labeled("OpenGL ES swap interval", &gl_interval));
        let fps = gtk::SpinButton::with_range(0.0, 360.0, 1.0);
        fps.set_value(initial_frame_cap as f64);
        settings_page.append(&labeled("FPS limit (0 = unlimited; shared with Game settings)", &fps));
        let note = gtk::Label::new(Some(
            "OpenGL ES uses the selected EGL swap interval. Adaptive mode uses interval -1 and depends on driver support.",
        ));
        note.set_wrap(true);
        note.set_halign(gtk::Align::Start);
        settings_page.append(&note);
        let status = gtk::Label::new(None);
        status.set_halign(gtk::Align::Start);
        settings_page.append(&status);
        let sessions_root = if let Some(managed) = crate::client::managed_install_dir() {
            if let Err(error) = crate::client::migrate_legacy_sessions(&managed) {
                eprintln!("rusty-blox: could not migrate saved sessions: {error}");
            }
            crate::client::sessions_root(&managed)
        } else {
            PathBuf::from("rusty-blox").join("sessions")
        };
        let initial_sessions =
            roblox_runtime::session::Session::list(&sessions_root).unwrap_or_default();
        let labels: Vec<_> = std::iter::once("No saved session".to_owned())
            .chain(initial_sessions.iter().map(|session| session.name().to_owned()))
            .collect();
        let label_refs: Vec<_> = labels.iter().map(String::as_str).collect();
        let session_model = gtk::StringList::new(&label_refs);
        let session_picker =
            gtk::DropDown::new(Some(session_model.clone()), None::<gtk::Expression>);
        let disabled_fast_flags = Rc::new(RefCell::new(
            settings.disabled_fast_flags.iter().cloned().collect::<HashSet<_>>(),
        ));
        let session_names = Rc::new(RefCell::new(
            labels.into_iter().skip(1).collect::<Vec<String>>(),
        ));
        let selected_session = settings
            .session
            .as_ref()
            .and_then(|name| session_names.borrow().iter().position(|item| item == name))
            .map(|index| index as u32 + 1)
            .unwrap_or(0);
        session_picker.set_selected(selected_session);
        let picker_for_save = session_picker.clone();
        let session_names_for_save = session_names.clone();
        let status_for_save = status.clone();
        let save_button = gtk::Button::with_label("Save settings");
        let window_ref = window.clone();
        let fps_for_save = fps.clone();
        let disabled_for_save = disabled_fast_flags.clone();
        let mode_for_settings = theme_mode.clone();
        let preset_for_settings = theme_preset.clone();
        let accent_for_settings = theme_accent.clone();
        let animations_for_settings = animations.clone();
        let acceleration_for_settings = hardware_acceleration.clone();
        save_button.connect_clicked(move |_| {
            let limit = fps_for_save.value_as_int().max(0) as u32;
            let value = Settings {
                gamemode: gamemode.is_active(),
                discord_presence: presence.is_active(),
                discord_application_id: app_id.text().to_string(),
                renderer: selected_choice_id(&renderer, RENDERERS)
                    .unwrap_or("auto")
                    .to_owned(),
                vsync: vsync.is_active(),
                present_mode: selected_choice_id(&present, PRESENT_MODES)
                    .unwrap_or("fifo")
                    .to_owned(),
                gl_swap_interval: selected_choice_id(&gl_interval, GL_SWAP_INTERVALS)
                    .unwrap_or("on")
                    .to_owned(),
                rust_jnivm: selected_choice_id(&jni_backend, JNI_BACKENDS) == Some("rust"),
                jnivm_cpp_fallback: jni_fallback.is_active(),
                host_libc: host_libc.is_active(),
                fps_limit: (limit > 0).then_some(limit),
                session: picker_for_save
                    .selected()
                    .checked_sub(1)
                    .and_then(|index| session_names_for_save.borrow().get(index as usize).cloned()),
                log_categories: log_controls
                    .iter()
                    .filter(|(_, control)| control.is_active())
                    .map(|(id, _)| id.clone())
                    .collect(),
                wasd_last_pressed: wasd_last_pressed.is_active(),
                disabled_fast_flags: disabled_for_save.borrow().iter().cloned().collect(),
                theme_mode: selected_choice_id(&mode_for_settings, THEME_MODES).unwrap_or("dark").to_owned(),
                theme_preset: selected_choice_id(&preset_for_settings, THEME_PRESETS).unwrap_or("default").to_owned(),
                theme_accent: selected_choice_id(&accent_for_settings, THEME_ACCENTS).unwrap_or("preset").to_owned(),
                animations: animations_for_settings.is_active(),
                hardware_acceleration: acceleration_for_settings.is_active(),
            };
            let settings_error = save(&value).err();
            let mut errors = sync_frame_cap_fallbacks(limit);
            if let Some(error) = settings_error {
                errors.insert(0, format!("settings: {error}"));
            }
            if errors.is_empty() {
                status_for_save.set_text("Saved. Changes apply on the next launch.");
            } else {
                status_for_save.set_text(&format!("Saved with frame-cap warnings: {}", errors.join("; ")));
            }
        });

        let fast_flags_page = gtk::Box::new(gtk::Orientation::Vertical, 12);
        fast_flags_page.set_margin_start(10);
        fast_flags_page.set_margin_end(10);
        fast_flags_page.add_css_class("launcher-page");
        let fast_flags_heading = gtk::Label::new(Some("FastFlags"));
        fast_flags_heading.add_css_class("launcher-page-title");
        fast_flags_heading.set_halign(gtk::Align::Start);
        fast_flags_page.append(&fast_flags_heading);
        let fast_flags_copy = gtk::Label::new(Some("Choose which client flags are active, or import a JSON collection."));
        fast_flags_copy.add_css_class("launcher-page-copy");
        fast_flags_copy.set_halign(gtk::Align::Start);
        fast_flags_page.append(&fast_flags_copy);
        let fast_flags_path = fast_flags_path();
        let mut initial_flags = std::fs::read(&fast_flags_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default();
        let fast_flags = Rc::new(RefCell::new(
            std::mem::take(&mut initial_flags).into_iter().collect::<BTreeMap<_, _>>(),
        ));
        let fast_flags_toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let fast_flags_timing = gtk::Label::new(Some("NEXT LAUNCH"));
        fast_flags_timing.add_css_class("launcher-pill");
        fast_flags_timing.set_hexpand(true);
        fast_flags_timing.set_halign(gtk::Align::Start);
        fast_flags_toolbar.append(&fast_flags_timing);
        let fast_flags_import = gtk::Button::with_label("Import JSON");
        fast_flags_toolbar.append(&fast_flags_import);
        fast_flags_page.append(&fast_flags_toolbar);
        let fast_flags_table = gtk::Box::new(gtk::Orientation::Vertical, 0);
        fast_flags_table.add_css_class("launcher-card");
        let fast_flags_scroll = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .hexpand(true)
            .child(&fast_flags_table)
            .build();
        fast_flags_page.append(&fast_flags_scroll);
        let fast_flags_path_note = gtk::Label::new(Some(&format!(
            "Saved to {}. Disabled numerical and string flags are kept here and omitted at launch; boolean values are applied as shown.",
            fast_flags_path.display()
        )));
        fast_flags_path_note.set_wrap(true);
        fast_flags_path_note.set_halign(gtk::Align::Start);
        fast_flags_page.append(&fast_flags_path_note);
        let fast_flags_status = gtk::Label::new(None);
        fast_flags_status.set_halign(gtk::Align::Start);
        fast_flags_page.append(&fast_flags_status);
        let fast_flags_save = gtk::Button::with_label("Save FastFlags");
        fast_flags_save.add_css_class("suggested-action");
        fast_flags_save.set_halign(gtk::Align::Start);
        fast_flags_page.append(&fast_flags_save);
        render_fast_flag_rows(
            &fast_flags_table,
            fast_flags.clone(),
            disabled_fast_flags.clone(),
            window.clone().upcast(),
            fast_flags_status.clone(),
        );

        let import_parent: gtk::Window = window.clone().upcast();
        let import_table = fast_flags_table.clone();
        let import_values = fast_flags.clone();
        let import_disabled = disabled_fast_flags.clone();
        let import_status = fast_flags_status.clone();
        fast_flags_import.connect_clicked(move |_| {
            let dialog = gtk::Window::builder()
                .title("Import FastFlags JSON")
                .transient_for(&import_parent)
                .modal(true)
                .default_width(620)
                .default_height(440)
                .build();
            let content = gtk::Box::new(gtk::Orientation::Vertical, 10);
            content.set_margin_top(12);
            content.set_margin_bottom(12);
            content.set_margin_start(12);
            content.set_margin_end(12);
            let hint = gtk::Label::new(Some("Paste a JSON object of FastFlags. Existing keys will be checked for conflicts."));
            hint.set_halign(gtk::Align::Start);
            content.append(&hint);
            let editor = gtk::TextView::new();
            editor.set_monospace(true);
            editor.set_wrap_mode(gtk::WrapMode::None);
            editor.buffer().set_text("{}\n");
            let scroll = gtk::ScrolledWindow::builder()
                .vexpand(true)
                .hexpand(true)
                .child(&editor)
                .build();
            content.append(&scroll);
            let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            actions.set_halign(gtk::Align::End);
            let cancel = gtk::Button::with_label("Cancel");
            let import = gtk::Button::with_label("Import");
            import.add_css_class("suggested-action");
            actions.append(&cancel);
            actions.append(&import);
            content.append(&actions);
            dialog.set_child(Some(&content));
            let close_dialog = dialog.clone();
            cancel.connect_clicked(move |_| close_dialog.close());
            let values = import_values.clone();
            let disabled = import_disabled.clone();
            let table = import_table.clone();
            let parent = import_parent.clone();
            let status = import_status.clone();
            let buffer = editor.buffer();
            let close_dialog = dialog.clone();
            import.connect_clicked(move |_| {
                let (start, end) = buffer.bounds();
                let text = buffer.text(&start, &end, true).to_string();
                match serde_json::from_str::<serde_json::Value>(&text) {
                    Ok(value) if value.is_object() => {
                        import_fast_flags(
                            value.as_object().unwrap().clone(),
                            values.clone(),
                            disabled.clone(),
                            table.clone(),
                            parent.clone(),
                            status.clone(),
                        );
                        close_dialog.close();
                    }
                    Ok(_) => status.set_text("FastFlags import must be a JSON object."),
                    Err(error) => status.set_text(&format!("Invalid JSON: {error}")),
                }
            });
            dialog.present();
        });

        let flags_for_save = fast_flags.clone();
        let disabled_for_flags = disabled_fast_flags.clone();
        let flags_status_ref = fast_flags_status.clone();
        let flags_path_for_save = fast_flags_path.clone();
        let fast_flags_fps = fps.clone();
        fast_flags_save.connect_clicked(move |_| {
            let mut flags = flags_for_save.borrow_mut();
            if !disabled_for_flags
                .borrow()
                .contains("DFIntTaskSchedulerTargetFps")
            {
                let limit = fast_flags_fps.value_as_int().max(0) as u32;
                if limit == 0 {
                    flags.remove("DFIntTaskSchedulerTargetFps");
                    flags.remove("FFlagTaskSchedulerLimitTargetFpsTo2402");
                } else {
                    flags.insert("DFIntTaskSchedulerTargetFps".into(), limit.to_string().into());
                    if limit > 240 {
                        flags.insert("FFlagTaskSchedulerLimitTargetFpsTo2402".into(), false.into());
                    } else {
                        flags.remove("FFlagTaskSchedulerLimitTargetFpsTo2402");
                    }
                }
            }
            match serde_json::to_vec_pretty(&serde_json::Value::Object(flags.clone().into_iter().collect())) {
                Ok(bytes) => match write_file(&flags_path_for_save, &bytes) {
                    Ok(()) => {
                        let mut app_settings = load();
                        app_settings.disabled_fast_flags = disabled_for_flags.borrow().iter().cloned().collect();
                        match save(&app_settings) {
                            Ok(()) => flags_status_ref.set_text("Saved. Flags apply on the next launch."),
                            Err(error) => flags_status_ref.set_text(&format!("FastFlags saved, but disabled flag settings could not be saved: {error}")),
                        }
                    }
                    Err(error) => flags_status_ref.set_text(&error),
                },
                Err(error) => flags_status_ref.set_text(&error.to_string()),
            }
        });

        let game_settings_page = gtk::Box::new(gtk::Orientation::Vertical, 12);
        game_settings_page.add_css_class("launcher-page");
        let game_heading = gtk::Label::new(Some("Game settings"));
        game_heading.add_css_class("launcher-page-title");
        game_heading.set_halign(gtk::Align::Start);
        game_settings_page.append(&game_heading);
        let game_copy = gtk::Label::new(Some("Adjust in-game preferences stored in Roblox's local settings."));
        game_copy.add_css_class("launcher-page-copy");
        game_copy.set_halign(gtk::Align::Start);
        game_settings_page.append(&game_copy);
        let game_path = game_settings_path();
        let (game_values, game_load_message) = match std::fs::read_to_string(&game_path) {
            Ok(contents) => match parse_game_values(&contents) {
                Ok(values) => (values, None),
                Err(error) => (HashMap::new(), Some(format!("Could not read game settings: {error}"))),
            },
            Err(error) => (
                HashMap::new(),
                Some(format!("Could not read {}: {error}", game_path.display())),
            ),
        };
        let game_form = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let mut game_boolean_controls = Vec::new();
        for (key, label) in GAME_BOOLEAN_OPTIONS {
            let control = gtk::CheckButton::with_label(label);
            control.set_active(
                game_values
                    .get(*key)
                    .is_some_and(|value| value.eq_ignore_ascii_case("true")),
            );
            game_form.append(&control);
            game_boolean_controls.push(((*key).to_owned(), control));
        }
        let mut game_slider_controls = Vec::new();
        for (key, label, min, max, step, default) in GAME_SLIDER_OPTIONS {
            let (xml_key, setting_default) = slider_xml_value(key, *default);
            let alternate_mouse_sensitivity = (*key == "MouseSensitivity")
                .then(|| game_values.get("MouseSensitivityFirstPerson.X"))
                .flatten();
            let current = if *key == "FramerateCap" {
                initial_frame_cap as f64
            } else {
                game_values
                    .get(&xml_key)
                    .or(alternate_mouse_sensitivity)
                    .and_then(|value| value.parse::<f64>().ok())
                    .filter(|value| value.is_finite())
                    .unwrap_or(setting_default)
            }
            .clamp(*min, *max);
            let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, *min, *max, *step);
            scale.set_value(current);
            scale.set_draw_value(true);
            scale.set_digits(if *step < 1.0 { 2 } else { 0 });
            scale.set_hexpand(true);
            scale.add_mark(*min, gtk::PositionType::Bottom, Some(&format!("{min:.0}")));
            scale.add_mark(*max, gtk::PositionType::Bottom, Some(&format!("{max:.0}")));
            game_form.append(&labeled(label, &scale));
            if *key == "MouseSensitivity" {
                let hint = gtk::Label::new(Some(
                    "Linked across the default, first-person, and third-person sensitivity values.",
                ));
                hint.set_wrap(true);
                hint.set_halign(gtk::Align::Start);
                game_form.append(&hint);
            }
            game_slider_controls.push(((*key).to_owned(), scale));
        }
        if let Some((_, game_fps)) = game_slider_controls
            .iter()
            .find(|(key, _)| key == "FramerateCap")
        {
            let game_fps = game_fps.clone();
            let game_fps_for_general = game_fps.clone();
            let general_fps = fps.clone();
            fps.connect_value_changed(move |spin| game_fps_for_general.set_value(spin.value()));
            let general_fps_ref = general_fps.clone();
            game_fps.connect_value_changed(move |scale| general_fps_ref.set_value(scale.value()));
        }
        let game_scroll = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .hexpand(true)
            .child(&game_form)
            .build();
        game_settings_page.append(&game_scroll);
        let game_path_note = gtk::Label::new(Some(&format!(
            "These options are saved to {} and take effect on the next launch. Frame cap is shared with General settings and synced to FastFlags.",
            game_path.display()
        )));
        game_path_note.set_wrap(true);
        game_path_note.set_halign(gtk::Align::Start);
        game_settings_page.append(&game_path_note);
        let game_status = gtk::Label::new(game_load_message.as_deref());
        game_status.set_halign(gtk::Align::Start);
        game_settings_page.append(&game_status);
        let game_save = gtk::Button::with_label("Save game settings");
        let game_status_ref = game_status.clone();
        game_save.connect_clicked(move |_| {
            let mut updates = HashMap::new();
            for (key, control) in &game_boolean_controls {
                updates.insert(key.clone(), control.is_active().to_string());
            }
            for (key, scale) in &game_slider_controls {
                let value = scale.value();
                let formatted = if matches!(key.as_str(), "MouseSensitivity" | "GamepadCameraSensitivity" | "MasterVolume" | "PartyVoiceVolume" | "VoiceChatVolume" | "HapticStrength" | "PreferredTransparency") {
                    format!("{value:.2}")
                } else {
                    format!("{value:.0}")
                };
                for xml_key in slider_xml_keys(key) {
                    updates.insert(xml_key.to_owned(), formatted.clone());
                }
            }
            let frame_cap = game_slider_controls
                .iter()
                .find(|(key, _)| key == "FramerateCap")
                .map(|(_, scale)| scale.value().round().max(0.0) as u32)
                .unwrap_or(0);
            let mut errors = match update_game_settings(&game_path, &updates) {
                Ok(missing) if missing.is_empty() => Vec::new(),
                Ok(missing) => vec![format!("XML is missing: {}", missing.join(", "))],
                Err(error) => vec![format!("XML: {error}")],
            };
            let mut saved_settings = load();
            saved_settings.fps_limit = (frame_cap > 0).then_some(frame_cap);
            if let Err(error) = save(&saved_settings) {
                errors.push(format!("settings: {error}"));
            }
            errors.extend(sync_frame_cap_fallbacks(frame_cap));
            if errors.is_empty() {
                game_status_ref.set_text("Saved. Roblox will use these values on the next launch.");
            } else {
                game_status_ref.set_text(&format!("Saved with frame-cap warnings: {}", errors.join("; ")));
            }
        });

        let session_page = gtk::Box::new(gtk::Orientation::Vertical, 12);
        session_page.add_css_class("launcher-page");
        let session_heading = gtk::Label::new(Some("Roblox sessions"));
        session_heading.add_css_class("launcher-page-title");
        session_heading.set_halign(gtk::Align::Start);
        session_page.append(&session_heading);
        let session_copy = gtk::Label::new(Some("Keep separate sign-ins for the accounts you use."));
        session_copy.add_css_class("launcher-page-copy");
        session_copy.set_halign(gtk::Align::Start);
        session_page.append(&session_copy);
        session_page.append(&labeled("Use this session on the next launch", &session_picker));
        let set_default = gtk::Button::with_label("Set as default session");
        let picker_for_default = session_picker.clone();
        let session_names_for_default = session_names.clone();
        let session_status = gtk::Label::new(None);
        session_status.set_halign(gtk::Align::Start);
        let status_for_default = session_status.clone();
        set_default.connect_clicked(move |_| {
            let mut value = load();
            value.session = picker_for_default
                .selected()
                .checked_sub(1)
                .and_then(|index| session_names_for_default.borrow().get(index as usize).cloned());
            match save(&value) {
                Ok(()) => {
                    let message = value.session.as_ref().map_or_else(
                        || "Roblox will start without a saved session on the next launch.".to_owned(),
                        |name| format!("'{name}' will be used by default on the next launch."),
                    );
                    status_for_default.set_text(&message);
                }
                Err(error) => status_for_default.set_text(&error),
            }
        });
        session_page.append(&session_status);
        let add_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let session_name = gtk::Entry::builder().placeholder_text("New session name").hexpand(true).build();
        let add_button = gtk::Button::with_label("Add session");
        add_row.append(&session_name);
        add_row.append(&add_button);
        session_page.append(&add_row);
        let session_note = gtk::Label::new(Some("A new session starts logged out. Select it and launch Roblox to sign in; its login is saved separately."));
        session_note.set_wrap(true);
        session_note.set_halign(gtk::Align::Start);
        session_page.append(&session_note);
        let picker_ref = session_picker.clone();
        let model_ref = session_model.clone();
        let session_names_ref = session_names.clone();
        let name_ref = session_name.clone();
        let root_ref = sessions_root.clone();
        let status_ref = session_status.clone();
        add_button.connect_clicked(move |_| {
            let name = name_ref.text().trim().to_owned();
            match roblox_runtime::session::Session::open(&root_ref, &name) {
                Ok(session) => {
                    let mut names = session_names_ref.borrow_mut();
                    let index = if let Some(index) = names.iter().position(|name| name == session.name()) {
                        index
                    } else {
                        model_ref.append(session.name());
                        names.push(session.name().to_owned());
                        names.len() - 1
                    };
                    picker_ref.set_selected(index as u32 + 1);
                    status_ref.set_text(&format!("Added session '{}'. It will be saved when you choose Save & Play.", session.name()));
                    name_ref.set_text("");
                }
                Err(error) => status_ref.set_text(&error),
            }
        });
        let settings_scroll = gtk::ScrolledWindow::builder()
            .hexpand(true)
            .vexpand(true)
            .child(&settings_page)
            .build();
        stack.add_titled(&settings_scroll, Some("general"), "General");
        stack.add_titled(&fast_flags_page, Some("fast-flags"), "FastFlags");
        stack.add_titled(&game_settings_page, Some("game"), "Game");
        stack.add_titled(&session_page, Some("sessions"), "Sessions");
        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        footer.set_margin_top(2);
        let play_status = gtk::Label::new(Some("Your changes are saved before Roblox starts."));
        play_status.set_halign(gtk::Align::Start);
        play_status.set_hexpand(true);
        footer.append(&play_status);
        let play_button = gtk::Button::with_label("Play Roblox");
        play_button.add_css_class("suggested-action");
        play_button.set_size_request(150, 42);
        let play_button_label = play_button.clone();
        stack.connect_visible_child_notify(move |stack| {
            let label = if stack.visible_child_name().as_deref() == Some("home") {
                "Play Roblox"
            } else {
                "Save & Play"
            };
            play_button_label.set_label(label);
        });
        let save_all = save_button.clone();
        let save_flags = fast_flags_save.clone();
        let save_game = game_save.clone();
        let main_status = status.clone();
        let flags_status = fast_flags_status.clone();
        let saved_game_status = game_status.clone();
        let play_status_ref = play_status.clone();
        let window_for_play = window.clone();
        let play_requested = play_requested_activate.clone();
        let update_in_progress = update_in_progress.clone();
        let client_ready = client_ready.clone();
        play_button.connect_clicked(move |_| {
            if update_in_progress.get() {
                play_status_ref.set_text("Wait for the Roblox download or update to finish first.");
                return;
            }
            if !client_ready.get() {
                play_status_ref.set_text("Download Roblox first, or install it through Sober and restart rusty-blox.");
                return;
            }
            save_all.emit_clicked();
            save_flags.emit_clicked();
            save_game.emit_clicked();
            let main_message = main_status.text().to_string();
            let flags_message = flags_status.text().to_string();
            let game_message = saved_game_status.text().to_string();
            let failed = main_message.contains("settings:")
                || main_message.contains("FastFlags JSON")
                || !flags_message.starts_with("Saved")
                || game_message.starts_with("Could not read")
                || game_message.starts_with("XML:")
                || game_message.starts_with("Saved with frame-cap warnings: XML:");
            if failed {
                let messages = [main_message, flags_message, game_message]
                    .into_iter()
                    .filter(|message| !message.is_empty())
                    .collect::<Vec<_>>();
                play_status_ref.set_text(&messages.join(" · "));
                return;
            }
            play_requested.set(true);
            window_for_play.close();
        });
        footer.append(&play_button);
        footer.add_css_class("launcher-footer");
        root.append(&content);
        root.append(&footer);
        window_ref.set_child(Some(&root));
        window_ref.present();
    });
    // `--settings` was already consumed by rusty-blox. Do not pass it on to
    // GTK's own command-line parser, which rejects unknown application args.
    app.run_with_args(&["rusty-blox-settings"]);
    Ok(play_requested.get())
}

fn write_file(path: &std::path::Path, contents: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or("settings file has no parent directory")?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("create {}: {error}", parent.display()))?;
    let temp = path.with_extension("tmp");
    std::fs::write(&temp, contents)
        .map_err(|error| format!("write {}: {error}", temp.display()))?;
    std::fs::rename(&temp, path).map_err(|error| format!("save {}: {error}", path.display()))
}

fn effective_frame_cap(settings: &Settings) -> u32 {
    if let Some(limit) = configured_frame_cap(settings) {
        return limit.min(360);
    }
    let flags_path = fast_flags_path();
    std::fs::read(flags_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|flags| flags.get("DFIntTaskSchedulerTargetFps").cloned())
        .and_then(|value| match value {
            serde_json::Value::String(value) => value.parse::<u32>().ok(),
            serde_json::Value::Number(value) => value.as_u64().map(|value| value as u32),
            _ => None,
        })
        .unwrap_or(0)
        .min(360)
}

fn sync_frame_cap_fallbacks(limit: u32) -> Vec<String> {
    let mut errors = Vec::new();

    let xml_updates = HashMap::from([("FramerateCap".to_owned(), limit.to_string())]);
    match update_game_settings(&game_settings_path(), &xml_updates) {
        Ok(missing) if missing.is_empty() => {}
        Ok(_) => errors.push("XML does not contain FramerateCap".to_owned()),
        Err(error) => errors.push(format!("XML: {error}")),
    }

    let flags_path = fast_flags_path();
    let flags_result = match std::fs::read(&flags_path) {
        Ok(bytes) => match serde_json::from_slice::<serde_json::Value>(&bytes) {
            Ok(flags) => Ok(flags),
            Err(error) => Err(format!("FastFlags JSON: {error}")),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(serde_json::Value::Object(Default::default()))
        }
        Err(error) => Err(format!("read FastFlags: {error}")),
    };
    match flags_result {
        Err(error) => errors.push(error),
        Ok(flags) if !flags.is_object() => {
            errors.push("FastFlags must be a JSON object".to_owned());
        }
        Ok(mut flags) => {
            let Some(flags) = flags.as_object_mut() else {
                unreachable!("object was checked above")
            };
            let fps_flag_disabled = load()
                .disabled_fast_flags
                .iter()
                .any(|key| key == "DFIntTaskSchedulerTargetFps");
            if !fps_flag_disabled {
                if limit == 0 {
                    flags.remove("DFIntTaskSchedulerTargetFps");
                    flags.remove("FFlagTaskSchedulerLimitTargetFpsTo2402");
                } else {
                    flags.insert(
                        "DFIntTaskSchedulerTargetFps".to_owned(),
                        limit.to_string().into(),
                    );
                    if limit > 240 {
                        flags.insert(
                            "FFlagTaskSchedulerLimitTargetFpsTo2402".to_owned(),
                            "False".into(),
                        );
                    } else {
                        flags.remove("FFlagTaskSchedulerLimitTargetFpsTo2402");
                    }
                }
            }
            match serde_json::to_vec_pretty(&flags) {
                Ok(contents) => {
                    if let Err(error) = write_file(&flags_path, &contents) {
                        errors.push(format!("FastFlags: {error}"));
                    }
                }
                Err(error) => errors.push(format!("serialize FastFlags: {error}")),
            }
        }
    }
    errors
}

fn slider_xml_value(key: &str, default: f64) -> (String, f64) {
    match key {
        "StartScreenWidth" => ("StartScreenSize.X".to_owned(), default),
        "StartScreenHeight" => ("StartScreenSize.Y".to_owned(), default),
        _ => (key.to_owned(), default),
    }
}

fn slider_xml_keys(key: &str) -> Vec<&'static str> {
    match key {
        "MouseSensitivity" => vec![
            "MouseSensitivity",
            "MouseSensitivityFirstPerson.X",
            "MouseSensitivityFirstPerson.Y",
            "MouseSensitivityThirdPerson.X",
            "MouseSensitivityThirdPerson.Y",
        ],
        "GraphicsQualityLevel" => vec!["GraphicsQualityLevel", "SavedQualityLevel"],
        "StartScreenWidth" => vec!["StartScreenSize.X"],
        "StartScreenHeight" => vec!["StartScreenSize.Y"],
        "GamepadCameraSensitivity" => vec!["GamepadCameraSensitivity"],
        "FramerateCap" => vec!["FramerateCap"],
        "MasterVolume" => vec!["MasterVolume"],
        "PartyVoiceVolume" => vec!["PartyVoiceVolume"],
        "VoiceChatVolume" => vec!["VoiceChatVolume"],
        "HapticStrength" => vec!["HapticStrength"],
        "PreferredTransparency" => vec!["PreferredTransparency"],
        _ => Vec::new(),
    }
}

#[derive(Clone, Default)]
struct GameXmlContext {
    element: Vec<u8>,
    setting: Option<String>,
    vector: bool,
    component: Option<char>,
}

fn start_context(
    element: &quick_xml::events::BytesStart<'_>,
    parent: Option<&GameXmlContext>,
) -> GameXmlContext {
    let element_name = element.name().as_ref().to_vec();
    let setting_attr = element.attributes().flatten().find_map(|attribute| {
        (attribute.key.as_ref() == b"name")
            .then(|| String::from_utf8(attribute.value.into_owned()).ok())
            .flatten()
    });
    let setting = setting_attr.or_else(|| parent.and_then(|context| context.setting.clone()));
    let vector = element_name == b"Vector2" && setting.is_some();
    let component = if parent.is_some_and(|context| context.vector) {
        match element_name.as_slice() {
            b"X" => Some('X'),
            b"Y" => Some('Y'),
            _ => None,
        }
    } else {
        None
    };
    GameXmlContext {
        element: element_name,
        setting,
        vector,
        component,
    }
}

fn value_key(context: &GameXmlContext) -> Option<String> {
    let setting = context.setting.as_ref()?;
    if let Some(component) = context.component {
        return Some(format!("{setting}.{component}"));
    }
    matches!(
        context.element.as_slice(),
        b"bool" | b"float" | b"int" | b"int64" | b"token" | b"string"
    )
    .then(|| setting.clone())
}

fn parse_game_values(contents: &str) -> Result<HashMap<String, String>, String> {
    validate_xml(contents)?;
    let mut reader = quick_xml::Reader::from_str(contents);
    reader.config_mut().trim_text(false);
    let mut contexts: Vec<GameXmlContext> = Vec::new();
    let mut values = HashMap::new();
    loop {
        match reader.read_event() {
            Ok(quick_xml::events::Event::Start(element)) => {
                contexts.push(start_context(&element, contexts.last()));
            }
            Ok(quick_xml::events::Event::Text(text)) => {
                if let Some(context) = contexts.last().and_then(value_key) {
                    let value = text
                        .decode()
                        .map_err(|error| format!("decode game setting value: {error}"))?;
                    if !value.trim().is_empty() {
                        values.insert(context, value.trim().to_owned());
                    }
                }
            }
            Ok(quick_xml::events::Event::End(_)) => {
                contexts.pop();
            }
            Ok(quick_xml::events::Event::Eof) => return Ok(values),
            Ok(_) => {}
            Err(error) => return Err(error.to_string()),
        }
    }
}

fn update_game_settings(
    path: &std::path::Path,
    updates: &HashMap<String, String>,
) -> Result<Vec<String>, String> {
    let contents = std::fs::read_to_string(path)
        .map_err(|error| format!("read {}: {error}", path.display()))?;
    validate_xml(&contents)?;
    let mut reader = quick_xml::Reader::from_str(&contents);
    reader.config_mut().trim_text(false);
    let mut writer = quick_xml::Writer::new(Vec::new());
    let mut contexts: Vec<GameXmlContext> = Vec::new();
    let mut written = std::collections::HashSet::new();
    loop {
        let event = reader.read_event().map_err(|error| error.to_string())?;
        match event {
            quick_xml::events::Event::Start(element) => {
                contexts.push(start_context(&element, contexts.last()));
                writer
                    .write_event(quick_xml::events::Event::Start(element.into_owned()))
                    .map_err(|error| error.to_string())?;
            }
            quick_xml::events::Event::Text(text) => {
                let replacement = contexts
                    .last()
                    .and_then(value_key)
                    .and_then(|key| updates.get(&key).map(|value| (key, value)));
                if let Some((key, value)) = replacement {
                    writer
                        .write_event(quick_xml::events::Event::Text(
                            quick_xml::events::BytesText::new(value).into_owned(),
                        ))
                        .map_err(|error| error.to_string())?;
                    written.insert(key);
                } else {
                    writer
                        .write_event(quick_xml::events::Event::Text(text.into_owned()))
                        .map_err(|error| error.to_string())?;
                }
            }
            quick_xml::events::Event::End(element) => {
                writer
                    .write_event(quick_xml::events::Event::End(element.into_owned()))
                    .map_err(|error| error.to_string())?;
                contexts.pop();
            }
            quick_xml::events::Event::Eof => break,
            event => writer
                .write_event(event.into_owned())
                .map_err(|error| error.to_string())?,
        }
    }
    let output = writer.into_inner();
    let missing = updates
        .keys()
        .filter(|key| !written.contains(*key))
        .cloned()
        .collect();
    write_file(path, &output)?;
    Ok(missing)
}

fn validate_xml(contents: &str) -> Result<(), String> {
    if contents.trim().is_empty() {
        return Err("document is empty".into());
    }
    let mut reader = quick_xml::Reader::from_str(contents);
    reader.config_mut().trim_text(false);
    let mut elements: Vec<Vec<u8>> = Vec::new();
    let mut roots = 0usize;
    loop {
        match reader.read_event() {
            Ok(quick_xml::events::Event::Start(element)) => {
                if elements.is_empty() {
                    roots += 1;
                }
                elements.push(element.name().as_ref().to_vec());
            }
            Ok(quick_xml::events::Event::Empty(_)) if elements.is_empty() => roots += 1,
            Ok(quick_xml::events::Event::End(element)) => {
                let name = element.name();
                if elements.pop().as_deref() != Some(name.as_ref()) {
                    return Err("closing tag does not match its opening tag".into());
                }
            }
            Ok(quick_xml::events::Event::Eof) => {
                if !elements.is_empty() || roots != 1 {
                    return Err("document must have one complete root element".into());
                }
                return Ok(());
            }
            Ok(_) => {}
            Err(error) => return Err(error.to_string()),
        }
    }
}

fn labeled(widget_label: &str, widget: &impl IsA<gtk::Widget>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let label = gtk::Label::new(Some(widget_label));
    label.set_halign(gtk::Align::Start);
    row.append(&label);
    row.append(widget);
    row
}
