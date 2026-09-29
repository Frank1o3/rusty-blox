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
        let heading = gtk::Label::new(Some("Performance and client settings"));
        heading.add_css_class("title-2");
        heading.set_halign(gtk::Align::Start);
        root.append(&heading);

        let gamemode = gtk::CheckButton::with_label("Enable Feral GameMode while Roblox runs");
        gamemode.set_active(settings.gamemode);
        root.append(&gamemode);
        let presence = gtk::CheckButton::with_label("Enable Discord Rich Presence");
        presence.set_active(settings.discord_presence);
        root.append(&presence);
        let app_id = gtk::Entry::builder()
            .placeholder_text("Discord Application ID")
            .text(&settings.discord_application_id)
            .build();
        app_id.set_sensitive(settings.discord_presence);
        let app_id_ref = app_id.clone();
        presence.connect_toggled(move |button| app_id_ref.set_sensitive(button.is_active()));
        root.append(&app_id);

        let renderer = gtk::ComboBoxText::new();
        for (id, label) in [
            ("auto", "Automatic"),
            ("vulkan", "Vulkan"),
            ("opengl", "OpenGL ES"),
        ] {
            renderer.append(Some(id), label);
        }
        renderer.set_active_id(Some(&settings.renderer));
        root.append(&labeled("Rendering pipeline", &renderer));
        let vsync = gtk::CheckButton::with_label("Enable VSync");
        vsync.set_active(settings.vsync);
        root.append(&vsync);
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
        root.append(&labeled("Vulkan presentation mode", &present));
        let fps = gtk::SpinButton::with_range(0.0, 1000.0, 1.0);
        fps.set_value(settings.fps_limit.unwrap_or(0) as f64);
        root.append(&labeled("FPS limit (0 = unlimited)", &fps));
        let note = gtk::Label::new(Some(
            "OpenGL ES follows the game's swap interval. Vulkan mode choices apply only to Vulkan.",
        ));
        note.set_wrap(true);
        note.set_halign(gtk::Align::Start);
        root.append(&note);
        let status = gtk::Label::new(None);
        status.set_halign(gtk::Align::Start);
        root.append(&status);
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
            };
            match save(&value) {
                Ok(()) => status.set_text("Saved. Changes apply on the next launch."),
                Err(error) => status.set_text(&error),
            }
        });
        root.append(&save_button);
        window_ref.set_child(Some(&root));
        window_ref.present();
    });
    app.run();
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
