use gtk::prelude::*;
use gtk4 as gtk;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

const JNI_BACKENDS: &[(&str, &str)] = &[("cpp", "C++ libjnivm (default)"), ("rust", "Rust JNI VM")];
const LOG_LEVELS: &[(&str, &str)] = &[
    ("1", "1 — Standard startup logging"),
    ("2", "2 — Standard + runtime"),
    ("3", "3 — Standard + runtime + JNI VM"),
    ("4", "4 — All logs (including key presses)"),
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
    pub fps_limit: Option<u32>,
    pub session: Option<String>,
    pub log_level: u8,
    pub wasd_last_pressed: bool,
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
            fps_limit: None,
            session: None,
            log_level: 1,
            wasd_last_pressed: false,
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

#[cfg(feature = "aimbot")]
pub(crate) fn aimbot_config_path() -> PathBuf {
    path().with_file_name("aimbot.json")
}

#[cfg(feature = "aimbot")]
pub(crate) fn load_aimbot_config() -> extra::config::AimbotConfig {
    let path = aimbot_config_path();
    let config = match std::fs::read(&path) {
        Ok(bytes) => match serde_json::from_slice::<extra::config::AimbotConfig>(&bytes) {
            Ok(config) => config,
            Err(error) => {
                eprintln!(
                    "rusty-blox: could not parse {}: {error}; using defaults",
                    path.display()
                );
                return extra::config::AimbotConfig::default();
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let config = extra::config::AimbotConfig::default();
            match serde_json::to_vec_pretty(&config) {
                Ok(bytes) => {
                    if let Err(error) = write_file(&path, &bytes) {
                        eprintln!("rusty-blox: could not create {}: {error}", path.display());
                    }
                }
                Err(error) => eprintln!(
                    "rusty-blox: could not serialize {} defaults: {error}",
                    path.display()
                ),
            }
            config
        }
        Err(error) => {
            eprintln!(
                "rusty-blox: could not read {}: {error}; using defaults",
                path.display()
            );
            return extra::config::AimbotConfig::default();
        }
    };

    if let Err(error) = config.validate() {
        eprintln!(
            "rusty-blox: invalid {}: {error}; using defaults",
            path.display()
        );
        extra::config::AimbotConfig::default()
    } else {
        config
    }
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

pub(crate) fn save_fast_flags(
    path: &std::path::Path,
    flags: &serde_json::Value,
) -> Result<(), String> {
    let contents = serde_json::to_vec_pretty(flags).map_err(|error| error.to_string())?;
    write_file(path, &contents)
}

pub(crate) fn run_ui() -> Result<(), String> {
    let app = gtk::Application::builder()
        .application_id("org.rustyblox.Settings")
        .build();
    app.connect_activate(|app| {
        let settings = load();
        let initial_frame_cap = effective_frame_cap(&settings);
        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .title("rusty-blox Settings")
            .default_width(520)
            .default_height(620)
            .build();
        let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
        root.set_margin_top(20);
        root.set_margin_bottom(20);
        root.set_margin_start(20);
        root.set_margin_end(20);
        let stack = gtk::Stack::new();
        let switcher = gtk::StackSwitcher::new();
        switcher.set_stack(Some(&stack));
        root.append(&switcher);
        let settings_page = gtk::Box::new(gtk::Orientation::Vertical, 12);
        let heading = gtk::Label::new(Some("Performance and client settings"));
        heading.add_css_class("title-2");
        heading.set_halign(gtk::Align::Start);
        settings_page.append(&heading);

        let log_level = choice_dropdown(LOG_LEVELS, &settings.log_level.to_string());
        settings_page.append(&labeled("Runtime log level", &log_level));
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
        let sessions_root = crate::client::managed_install_dir()
            .unwrap_or_else(|| PathBuf::from(".").join("rusty-blox"))
            .join("sessions");
        let initial_sessions =
            roblox_runtime::session::Session::list(&sessions_root).unwrap_or_default();
        let labels: Vec<_> = std::iter::once("No saved session".to_owned())
            .chain(initial_sessions.iter().map(|session| session.name().to_owned()))
            .collect();
        let label_refs: Vec<_> = labels.iter().map(String::as_str).collect();
        let session_model = gtk::StringList::new(&label_refs);
        let session_picker =
            gtk::DropDown::new(Some(session_model.clone()), None::<gtk::Expression>);
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
                fps_limit: (limit > 0).then_some(limit),
                session: picker_for_save
                    .selected()
                    .checked_sub(1)
                    .and_then(|index| session_names_for_save.borrow().get(index as usize).cloned()),
                log_level: selected_choice_id(&log_level, LOG_LEVELS)
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(1),
                wasd_last_pressed: wasd_last_pressed.is_active(),
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
        settings_page.append(&save_button);

        let fast_flags_page = gtk::Box::new(gtk::Orientation::Vertical, 12);
        let fast_flags_heading = gtk::Label::new(Some("FastFlags"));
        fast_flags_heading.add_css_class("title-2");
        fast_flags_heading.set_halign(gtk::Align::Start);
        fast_flags_page.append(&fast_flags_heading);
        let fast_flags_path = fast_flags_path();
        let fast_flags_text = std::fs::read_to_string(&fast_flags_path)
            .unwrap_or_else(|_| "{}\n".to_owned());
        let fast_flags_editor = gtk::TextView::new();
        fast_flags_editor.set_monospace(true);
        fast_flags_editor.set_wrap_mode(gtk::WrapMode::None);
        fast_flags_editor.buffer().set_text(&fast_flags_text);
        let fast_flags_scroll = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .hexpand(true)
            .child(&fast_flags_editor)
            .build();
        fast_flags_page.append(&fast_flags_scroll);
        let fast_flags_path_note = gtk::Label::new(Some(&format!(
            "Saved to {}. These flags are used on the next launch.",
            fast_flags_path.display()
        )));
        fast_flags_path_note.set_wrap(true);
        fast_flags_path_note.set_halign(gtk::Align::Start);
        fast_flags_page.append(&fast_flags_path_note);
        let fast_flags_status = gtk::Label::new(None);
        fast_flags_status.set_halign(gtk::Align::Start);
        fast_flags_page.append(&fast_flags_status);
        let fast_flags_save = gtk::Button::with_label("Save FastFlags");
        let fast_flags_buffer = fast_flags_editor.buffer();
        let fast_flags_status_ref = fast_flags_status.clone();
        fast_flags_save.connect_clicked(move |_| {
            let (start, end) = fast_flags_buffer.bounds();
            let text = fast_flags_buffer.text(&start, &end, true).to_string();
            let parsed = serde_json::from_str::<serde_json::Value>(&text);
            match parsed {
                Ok(value) if value.is_object() => match write_file(&fast_flags_path, text.as_bytes()) {
                    Ok(()) => fast_flags_status_ref.set_text("Saved. Flags apply on the next launch."),
                    Err(error) => fast_flags_status_ref.set_text(&error),
                },
                Ok(_) => fast_flags_status_ref.set_text("FastFlags must be a JSON object."),
                Err(error) => fast_flags_status_ref.set_text(&format!("Invalid JSON: {error}")),
            }
        });
        fast_flags_page.append(&fast_flags_save);

        let game_settings_page = gtk::Box::new(gtk::Orientation::Vertical, 12);
        let game_heading = gtk::Label::new(Some("Game settings"));
        game_heading.add_css_class("title-2");
        game_heading.set_halign(gtk::Align::Start);
        game_settings_page.append(&game_heading);
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
        game_settings_page.append(&game_save);

        let session_page = gtk::Box::new(gtk::Orientation::Vertical, 12);
        let session_heading = gtk::Label::new(Some("Roblox sessions"));
        session_heading.add_css_class("title-2");
        session_heading.set_halign(gtk::Align::Start);
        session_page.append(&session_heading);
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
        session_page.append(&set_default);
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
                    status_ref.set_text(&format!("Added session '{}'. Select it and set it as the default to use it on launch.", session.name()));
                    name_ref.set_text("");
                }
                Err(error) => status_ref.set_text(&error),
            }
        });
        stack.add_titled(&settings_page, Some("general"), "General");
        stack.add_titled(&fast_flags_page, Some("fast-flags"), "FastFlags");
        stack.add_titled(&game_settings_page, Some("game"), "Game");
        stack.add_titled(&session_page, Some("sessions"), "Sessions");
        root.append(&stack);
        window_ref.set_child(Some(&root));
        window_ref.present();
    });
    // `--settings` was already consumed by rusty-blox. Do not pass it on to
    // GTK's own command-line parser, which rejects unknown application args.
    app.run_with_args(&["rusty-blox-settings"]);
    Ok(())
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
