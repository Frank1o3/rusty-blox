use gtk::prelude::*;
use gtk4 as gtk;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

const JNI_BACKENDS: &[(&str, &str)] = &[
    ("cpp", "C++ libjnivm (default)"),
    ("rust", "Rust JNI VM"),
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
    choices
        .get(dropdown.selected() as usize)
        .map(|(id, _)| *id)
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

pub(crate) fn run_ui() -> Result<(), String> {
    let app = gtk::Application::builder()
        .application_id("org.rustyblox.Settings")
        .build();
    app.connect_activate(|app| {
        let settings = load();
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
        let fps = gtk::SpinButton::with_range(0.0, 1000.0, 1.0);
        fps.set_value(settings.fps_limit.unwrap_or(0) as f64);
        settings_page.append(&labeled("FPS limit (0 = unlimited)", &fps));
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
        save_button.connect_clicked(move |_| {
            let limit = fps.value_as_int().max(0) as u32;
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
            };
            match save(&value) {
                Ok(()) => status_for_save.set_text("Saved. Changes apply on the next launch."),
                Err(error) => status_for_save.set_text(&error),
            }
        });
        settings_page.append(&save_button);
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

fn labeled(widget_label: &str, widget: &impl IsA<gtk::Widget>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let label = gtk::Label::new(Some(widget_label));
    label.set_halign(gtk::Align::Start);
    row.append(&label);
    row.append(widget);
    row
}
