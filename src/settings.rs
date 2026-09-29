use gtk::prelude::*;
use gtk4 as gtk;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Settings {
    pub gamemode: bool,
    pub discord_presence: bool,
    pub discord_application_id: String,
    pub renderer: String,
    pub vsync: bool,
    pub present_mode: String,
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

        let renderer = gtk::ComboBoxText::new();
        for (id, label) in [
            ("auto", "Automatic"),
            ("vulkan", "Vulkan"),
            ("opengl", "OpenGL ES"),
        ] {
            renderer.append(Some(id), label);
        }
        renderer.set_active_id(Some(&settings.renderer));
        settings_page.append(&labeled("Rendering pipeline", &renderer));
        let vsync = gtk::CheckButton::with_label("Enable VSync");
        vsync.set_active(settings.vsync);
        settings_page.append(&vsync);
        let present = gtk::ComboBoxText::new();
        for (id, label) in [
            ("fifo", "FIFO (steady VSync)"),
            ("fifo-relaxed", "FIFO relaxed"),
            ("mailbox", "Mailbox (low latency)"),
            ("immediate", "Immediate (tearing allowed)"),
            ("auto", "Automatic"),
        ] {
            present.append(Some(id), label);
        }
        present.set_active_id(Some(&settings.present_mode));
        present.set_sensitive(settings.vsync && settings.renderer == "vulkan");
        let present_ref = present.clone();
        let vsync_ref = vsync.clone();
        let renderer_ref = renderer.clone();
        vsync.connect_toggled(move |_| {
            present_ref.set_sensitive(
                vsync_ref.is_active() && renderer_ref.active_id().as_deref() == Some("vulkan"),
            )
        });
        let present_ref = present.clone();
        let vsync_ref = vsync.clone();
        let renderer_ref = renderer.clone();
        renderer.connect_changed(move |_| {
            present_ref.set_sensitive(
                vsync_ref.is_active() && renderer_ref.active_id().as_deref() == Some("vulkan"),
            )
        });
        settings_page.append(&labeled("Vulkan presentation mode", &present));
        let fps = gtk::SpinButton::with_range(0.0, 1000.0, 1.0);
        fps.set_value(settings.fps_limit.unwrap_or(0) as f64);
        settings_page.append(&labeled("FPS limit (0 = unlimited)", &fps));
        let note = gtk::Label::new(Some(
            "OpenGL ES follows the game's swap interval. Vulkan mode choices apply only to Vulkan.",
        ));
        note.set_wrap(true);
        note.set_halign(gtk::Align::Start);
        settings_page.append(&note);
        let status = gtk::Label::new(None);
        status.set_halign(gtk::Align::Start);
        settings_page.append(&status);
        let session_picker = gtk::ComboBoxText::new();
        let picker_for_save = session_picker.clone();
        let status_for_save = status.clone();
        let save_button = gtk::Button::with_label("Save settings");
        let window_ref = window.clone();
        save_button.connect_clicked(move |_| {
            let limit = fps.value_as_int().max(0) as u32;
            let value = Settings {
                gamemode: gamemode.is_active(),
                discord_presence: presence.is_active(),
                discord_application_id: app_id.text().to_string(),
                renderer: renderer
                    .active_id()
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| "auto".into()),
                vsync: vsync.is_active(),
                present_mode: present
                    .active_id()
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| "fifo".into()),
                fps_limit: (limit > 0).then_some(limit),
                session: match picker_for_save.active_id().as_deref() { Some("") | None => None, Some(name) => Some(name.to_owned()) },
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
        session_picker.append(Some(""), "No saved session");
        let sessions_root = crate::client::managed_install_dir()
            .unwrap_or_else(|| PathBuf::from(".").join("rusty-blox"))
            .join("sessions");
        let initial_sessions = roblox_runtime::session::Session::list(&sessions_root).unwrap_or_default();
        for session in initial_sessions { session_picker.append(Some(session.name()), session.name()); }
        session_picker.set_active_id(settings.session.as_deref().or(Some("")));
        session_page.append(&labeled("Use this session on the next launch", &session_picker));
        let set_default = gtk::Button::with_label("Set as default session");
        let picker_for_default = session_picker.clone();
        let session_status = gtk::Label::new(None);
        session_status.set_halign(gtk::Align::Start);
        let status_for_default = session_status.clone();
        set_default.connect_clicked(move |_| {
            let mut value = load();
            value.session = picker_for_default
                .active_id()
                .map(|name| name.to_string())
                .filter(|name| !name.is_empty());
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
        let name_ref = session_name.clone();
        let root_ref = sessions_root.clone();
        let status_ref = session_status.clone();
        add_button.connect_clicked(move |_| {
            let name = name_ref.text().trim().to_owned();
            match roblox_runtime::session::Session::open(&root_ref, &name) {
                Ok(session) => {
                    if picker_ref.active_id().as_deref() != Some(session.name()) { picker_ref.append(Some(session.name()), session.name()); }
                    picker_ref.set_active_id(Some(session.name()));
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
