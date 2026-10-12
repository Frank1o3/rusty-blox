use gtk::prelude::*;
use gtk4 as gtk;
use serde::{Deserialize, Serialize};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
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
            .default_width(960)
            .default_height(720)
            .build();
        let css = gtk::CssProvider::new();
        css.load_from_data(
            "window { background: #11151d; color: #eff3fa; } .launcher-header { padding: 20px 24px; background: #191f2b; border-bottom: 1px solid #2b3444; } .launcher-title { font-size: 24px; font-weight: 700; } .launcher-subtitle { color: #a6b1c2; } .launcher-footer { padding: 14px 20px; background: #191f2b; border-top: 1px solid #2b3444; } .launcher-page-title { font-size: 20px; font-weight: 650; } .launcher-hero { padding: 28px; background: linear-gradient(135deg, #1c2b40, #18202c); border: 1px solid #35465f; border-radius: 18px; } .launcher-status { padding: 16px; background: #191f2b; border: 1px solid #2b3444; border-radius: 12px; } .launcher-stat { padding: 14px; background: #191f2b; border: 1px solid #2b3444; border-radius: 12px; }",
        );
        if let Some(display) = gtk::gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display,
                &css,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
        let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
        root.set_margin_top(20);
        root.set_margin_bottom(20);
        root.set_margin_start(20);
        root.set_margin_end(20);
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
        sidebar.set_stack(&stack);
        sidebar.set_size_request(150, -1);
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        content.set_vexpand(true);
        content.append(&sidebar);
        content.append(&stack);

        let home_page = gtk::Box::new(gtk::Orientation::Vertical, 18);
        home_page.set_margin_start(8);
        home_page.set_margin_end(8);
        let hero = gtk::Box::new(gtk::Orientation::Vertical, 12);
        hero.add_css_class("launcher-hero");
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
            title_label.add_css_class("launcher-subtitle");
            title_label.set_halign(gtk::Align::Start);
            let value_label = gtk::Label::new(Some(value));
            value_label.set_halign(gtk::Align::Start);
            stat.append(&title_label);
            stat.append(&value_label);
            quick_stats.append(&stat);
        }
        home_page.append(&quick_stats);
        stack.add_titled(&home_page, Some("home"), "Home");

        let settings_page = gtk::Box::new(gtk::Orientation::Vertical, 12);
        let heading = gtk::Label::new(Some("Performance and client settings"));
        heading.add_css_class("launcher-page-title");
        heading.set_halign(gtk::Align::Start);
        settings_page.append(&heading);

        let logs_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
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
        let fast_flags_heading = gtk::Label::new(Some("FastFlags"));
        fast_flags_heading.add_css_class("launcher-page-title");
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
        let fast_flags_fps = fps.clone();
        fast_flags_save.connect_clicked(move |_| {
            let (start, end) = fast_flags_buffer.bounds();
            let text = fast_flags_buffer.text(&start, &end, true).to_string();
            let parsed = serde_json::from_str::<serde_json::Value>(&text);
            match parsed {
                Ok(mut value) if value.is_object() => {
                    let limit = fast_flags_fps.value_as_int().max(0) as u32;
                    let Some(flags) = value.as_object_mut() else { unreachable!() };
                    if limit == 0 {
                        flags.remove("DFIntTaskSchedulerTargetFps");
                        flags.remove("FFlagTaskSchedulerLimitTargetFpsTo2402");
                    } else {
                        flags.insert("DFIntTaskSchedulerTargetFps".into(), limit.to_string().into());
                        if limit > 240 { flags.insert("FFlagTaskSchedulerLimitTargetFpsTo2402".into(), "False".into()); }
                        else { flags.remove("FFlagTaskSchedulerLimitTargetFpsTo2402"); }
                    }
                    match serde_json::to_vec_pretty(&value) {
                        Ok(bytes) => match write_file(&fast_flags_path, &bytes) {
                            Ok(()) => fast_flags_status_ref.set_text("Saved. Flags apply on the next launch."),
                            Err(error) => fast_flags_status_ref.set_text(&error),
                        },
                        Err(error) => fast_flags_status_ref.set_text(&error.to_string()),
                    }
                }
                Ok(_) => fast_flags_status_ref.set_text("FastFlags must be a JSON object."),
                Err(error) => fast_flags_status_ref.set_text(&format!("Invalid JSON: {error}")),
            }
        });

        let game_settings_page = gtk::Box::new(gtk::Orientation::Vertical, 12);
        let game_heading = gtk::Label::new(Some("Game settings"));
        game_heading.add_css_class("launcher-page-title");
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

        let session_page = gtk::Box::new(gtk::Orientation::Vertical, 12);
        let session_heading = gtk::Label::new(Some("Roblox sessions"));
        session_heading.add_css_class("launcher-page-title");
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
        footer.set_margin_top(12);
        footer.set_margin_bottom(12);
        footer.set_margin_start(20);
        footer.set_margin_end(20);
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
