use std::collections::HashSet;
use std::path::PathBuf;

use crate::host_window::SurfaceOwner;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{DeviceEvent, ElementState, Ime, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};
use winit::platform::x11::{WindowAttributesExtX11, WindowType};
use winit::window::{CursorGrabMode, Window, WindowId};

pub(crate) struct ClientApp {
    config: Option<roblox_runtime::RuntimeConfig>,
    asset_dir: PathBuf,
    session_dir: Option<PathBuf>,
    window: Option<Window>,
    surface_owner: Option<SurfaceOwner>,
    engine: Option<roblox_runtime::LoadedEngine>,
    game_activity: Option<i64>,
    failure: Option<String>,
    cursor: (f32, f32),
    modifiers: ModifiersState,
    cursor_locked: bool,
    window_focused: bool,
    cursor_inside: bool,
    text_generation: Option<u32>,
    text_value: String,
    text_cursor: usize,
    text_selection_anchor: Option<usize>,
    fast_flags_path: PathBuf,
    fast_flags: serde_json::Value,
    forwarded_keys: HashSet<i32>,
    movement_pressed: Vec<KeyCode>,
    movement_active: [Option<KeyCode>; 2],
    settings: crate::settings::Settings,
    game_mode: Option<crate::desktop::GameMode>,
    discord_presence: Option<crate::desktop::DiscordPresence>,
    #[cfg(feature = "webview")]
    webview: Option<crate::webview::WebViewHost>,
}

impl ClientApp {
    fn forward_locked_mouse_move(&self, position: (f32, f32), delta: (f32, f32)) {
        let Some(native) = self.engine.as_ref().and_then(|engine| {
            engine.symbol("Java_com_roblox_engine_jni_NativeInputInterface_nativePassMouseMove")
        }) else {
            return;
        };
        // SAFETY: this is the live mouse-move export from the loaded engine.
        if let Err(error) = unsafe {
            roblox_runtime::jni::game_activity::pass_mouse_move(
                native, position.0, position.1, delta.0, delta.1,
            )
        } {
            eprintln!("rusty-blox: locked mouse forwarding failed: {error}");
        }
    }

    pub(crate) fn new(
        config: roblox_runtime::RuntimeConfig,
        asset_dir: PathBuf,
        settings: crate::settings::Settings,
        fast_flags_path: PathBuf,
    ) -> Self {
        #[cfg(feature = "webview")]
        eprintln!("rusty-blox: embedded WebKitGTK web view host enabled");
        let fast_flags = config.fast_flags.clone();
        let session_dir = config
            .session
            .as_ref()
            .map(|session| session.directory().to_path_buf());
        Self {
            config: Some(config),
            asset_dir,
            session_dir,
            window: None,
            surface_owner: None,
            engine: None,
            game_activity: None,
            failure: None,
            cursor: (0.0, 0.0),
            modifiers: winit::keyboard::ModifiersState::empty(),
            cursor_locked: false,
            window_focused: false,
            cursor_inside: false,
            text_generation: None,
            text_value: String::new(),
            text_cursor: 0,
            text_selection_anchor: None,
            fast_flags_path,
            fast_flags,
            forwarded_keys: HashSet::new(),
            movement_pressed: Vec::new(),
            movement_active: [None, None],
            settings,
            game_mode: None,
            discord_presence: None,
            #[cfg(feature = "webview")]
            webview: None,
        }
    }

    pub(crate) fn take_failure(&mut self) -> Option<String> {
        self.failure.take()
    }
}

impl ClientApp {
    fn launch(&mut self, event_loop: &ActiveEventLoop) -> Result<(), String> {
        // Use Roblox's saved StartScreenSize as the initial window dimensions.
        // The compositor still decides whether the toplevel floats or tiles.
        let base = configured_start_size(
            &self
                .config
                .as_ref()
                .ok_or("runtime config already consumed")?
                .data_dir,
        )
        .unwrap_or_else(|| PhysicalSize::new(1280, 720));
        let size = event_loop
            .primary_monitor()
            .map(|monitor| {
                let available = monitor.size();
                let scale = ((available.width.saturating_sub(64) as f64 / base.width as f64)
                    .min(available.height.saturating_sub(96) as f64 / base.height as f64))
                .min(1.0);
                PhysicalSize::new(
                    (base.width as f64 * scale).round().max(1.0) as u32,
                    (base.height as f64 * scale).round().max(1.0) as u32,
                )
            })
            .unwrap_or(base);
        let attributes = Window::default_attributes()
            .with_title("roblox-runtime")
            .with_resizable(true)
            .with_maximized(false)
            .with_inner_size(size)
            // Dialog toplevels are normally placed as floating windows by
            // X11 window managers. Wayland compositors do not expose a
            // standardized client request to force floating placement.
            .with_x11_window_type(vec![WindowType::Dialog]);
        let window = event_loop
            .create_window(attributes)
            .map_err(|error| format!("create host window: {error}"))?;
        // Some window managers restore their previous size after applying the
        // initial attributes. Reassert the requested client size once mapped.
        window.set_maximized(false);
        let _ = window.request_inner_size(size);
        window.set_ime_allowed(true);
        let size = window.inner_size();
        let owner = crate::host_window::SurfaceOwner::install(&window, size)?;
        self.window = Some(window);
        self.surface_owner = Some(owner);
        let config = self
            .config
            .take()
            .ok_or("runtime config already consumed")?;
        let backend = config
            .prepare_graphics()
            .map_err(|error| format!("prepare graphics: {error}"))?;
        self.game_mode = Some(crate::desktop::GameMode::register(self.settings.gamemode));
        self.discord_presence = Some(crate::desktop::DiscordPresence::connect(
            self.settings.discord_presence,
            &self.settings.discord_application_id,
        ));
        println!(
            "Host window ready: {}x{} ({backend:?})",
            size.width, size.height
        );

        let mut engine = config
            .load_engine()
            .map_err(|error| format!("map libroblox.so: {error}"))?;
        engine
            .prepare_before_constructors()
            .map_err(|error| format!("prepare engine storage: {error}"))?;
        engine
            .run_constructors()
            .map_err(|error| format!("run engine constructors: {error}"))?;
        let jni_version = engine
            .initialize_jni()
            .map_err(|error| format!("initialize engine JNI: {error}"))?;
        println!("JNI_OnLoad returned 0x{jni_version:x}");

        engine
            .prepare_startup_directories()
            .map_err(|error| format!("set Roblox startup directories: {error}"))?;

        let settings = crate::client_settings::load(
            config.options.client_settings.as_deref(),
            &config.cache_dir,
        )?;
        engine
            .install_startup_bootstrap(settings, config.fast_flags.to_string())
            .map_err(|error| format!("install GameActivity bootstrap: {error}"))?;

        let internal = crate::startup::config_path(&config, "files");
        let external = crate::startup::config_path(&config, "external");
        if !roblox_runtime::android::looper::prepare_for_current_thread() {
            return Err("prepare Android looper for the GameActivity thread failed".into());
        }
        let game_activity = engine
            .initialize_game_activity(&internal, &internal, &external, size.width, size.height)
            .map_err(|error| format!("initialize GameActivity: {error}"))?;
        println!("GameActivity initialized; handle={game_activity}");
        crate::startup::initialize_client(&engine, &config, &self.asset_dir, game_activity, size)?;
        roblox_runtime::webview::arm(&engine);
        self.engine = Some(engine);
        self.game_activity = Some(game_activity);
        Ok(())
    }
}

impl Drop for ClientApp {
    fn drop(&mut self) {
        if let Err(error) =
            crate::settings::save_fast_flags(&self.fast_flags_path, &self.fast_flags)
        {
            eprintln!("rusty-blox: could not save active FastFlags: {error}");
        }
        if let Some(engine) = &self.engine {
            if let Some(session_dir) = &self.session_dir {
                if let Err(error) = roblox_runtime::session::save(engine, session_dir) {
                    eprintln!("rusty-blox: could not save Roblox session: {error}");
                }
            }
        }
        roblox_runtime::graphics::clear_surface();
        drop(self.engine.take());
        drop(self.surface_owner.take());
        drop(self.window.take());
    }
}

impl winit::application::ApplicationHandler for ClientApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() || self.failure.is_some() {
            return;
        }
        if let Err(error) = self.launch(event_loop) {
            self.failure = Some(error);
            event_loop.exit();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                roblox_runtime::graphics::clear_surface();
                event_loop.exit();
            }
            WindowEvent::Focused(focused) => {
                self.window_focused = focused;
                if !focused {
                    self.clear_movement_keys(event_loop);
                }
                if !focused && self.cursor_locked {
                    if let Some(window) = self.window.as_ref() {
                        let _ = window.set_cursor_grab(CursorGrabMode::None);
                    }
                    self.cursor_locked = false;
                }
                self.update_system_cursor_visibility();
            }
            WindowEvent::CursorEntered { .. } => {
                self.cursor_inside = true;
                self.update_system_cursor_visibility();
            }
            WindowEvent::CursorLeft { .. } => {
                self.cursor_inside = false;
                self.update_system_cursor_visibility();
            }
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            WindowEvent::CursorMoved { position, .. } => {
                let next = (position.x as f32, position.y as f32);
                let delta = (next.0 - self.cursor.0, next.1 - self.cursor.1);
                self.cursor = next;
                if !self.cursor_locked {
                    self.forward_mouse_move(next, delta, event_loop);
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if let Some(button) = android_mouse_button(button) {
                    self.forward_mouse_button(
                        self.cursor,
                        state == ElementState::Pressed,
                        button,
                        event_loop,
                    );
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let vertical = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(position) => (position.y / 40.0) as f32,
                };
                self.forward_mouse_wheel(self.cursor, vertical, event_loop);
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    if event.state == ElementState::Pressed {
                        if self.modifiers.control_key() && self.text_shortcut(code) {
                            return;
                        }
                        self.editing_key(code);
                        // `Ime::Commit` covers composed input, but winit's
                        // ordinary key events also carry printable text. On
                        // desktop keyboard layouts that don't activate an IME,
                        // relying on `Ime::Commit` alone leaves Roblox's text
                        // box with no characters (the corresponding game key
                        // is intentionally suppressed below).
                        let text = event
                            .text
                            .as_deref()
                            .or_else(|| event.logical_key.to_text());
                        if roblox_runtime::jni::game_activity::focused_textbox().is_some() {
                            let characters = text
                                .map(|text| text.chars().filter(|ch| !ch.is_control()).count())
                                .unwrap_or(0);
                            eprintln!(
                                "[input] focused text key; printable characters={characters}"
                            );
                        }
                        if let Some(text) = text {
                            let printable: String = text
                                .chars()
                                .filter(|character| !character.is_control())
                                .collect();
                            self.commit_text(&printable);
                        }
                    }
                    let text_box_focused =
                        roblox_runtime::jni::game_activity::focused_textbox().is_some();
                    if text_box_focused
                        && (self.movement_active[0].is_some() || self.movement_active[1].is_some())
                    {
                        self.clear_movement_keys(event_loop);
                    }
                    let movement_key = self.settings.wasd_last_pressed
                        && !text_box_focused
                        && movement_axis(code).is_some();
                    if movement_key {
                        self.update_movement_key(code, event.state, event_loop);
                    } else if let (Some(key_code), Some(evdev_code)) =
                        (android_key_code(code), evdev_key_code(code))
                    {
                        self.forward_key(
                            event.state == ElementState::Pressed,
                            key_code,
                            evdev_code,
                            event
                                .logical_key
                                .to_text()
                                .and_then(|s| s.chars().next())
                                .map_or(0, |ch| ch as i32),
                            self.modifiers,
                            event.repeat,
                            event_loop,
                        );
                    } else if event.state == ElementState::Pressed {
                        eprintln!("[input] key not mapped: {code:?}");
                    }
                }
            }
            WindowEvent::Ime(Ime::Commit(text)) => self.commit_text(&text),
            WindowEvent::Ime(Ime::Enabled) => self.update_ime_cursor_area(),
            WindowEvent::Resized(size) if size.width > 0 && size.height > 0 => {
                let Some(owner) = &self.surface_owner else {
                    return;
                };
                if let Err(error) = owner.resize(size.width, size.height) {
                    self.failure = Some(error);
                    event_loop.exit();
                    return;
                }
                if let (Some(engine), Some(handle)) = (&self.engine, self.game_activity) {
                    let Some(assets) = self.asset_dir.to_str() else {
                        self.failure = Some("asset directory path is not UTF-8".into());
                        event_loop.exit();
                        return;
                    };
                    if let Err(error) =
                        engine.resize_surface(handle, assets, 1, size.width, size.height)
                    {
                        self.failure = Some(error.to_string());
                        event_loop.exit();
                    }
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        #[cfg(feature = "webview")]
        {
            crate::webview::WebViewHost::pump_events();
            if let Some(event) = roblox_runtime::webview::take_event() {
                match event {
                    roblox_runtime::webview::WebViewEvent::Open(request) => {
                        eprintln!("rusty-blox: delivering Roblox web view request to GTK");
                        if self.webview.is_none() {
                            match crate::webview::WebViewHost::new(self.session_dir.as_deref()) {
                                Ok(webview) => self.webview = Some(webview),
                                Err(error) => {
                                    eprintln!("rusty-blox: could not create web view: {error}")
                                }
                            }
                        }
                        if let Some(webview) = &self.webview {
                            webview.open(request);
                        }
                    }
                    roblox_runtime::webview::WebViewEvent::Close => {
                        if let Some(webview) = &self.webview {
                            webview.close();
                        } else {
                            eprintln!(
                                "rusty-blox: Roblox closed a web view before the host opened one"
                            );
                        }
                    }
                }
            }
        }
        #[cfg(not(feature = "webview"))]
        if let Some(request) = roblox_runtime::webview::take_request() {
            if !external_web_url_allowed(&request.url) {
                eprintln!("rusty-blox: blocked an unsafe Roblox web view URL");
            } else if let Err(error) = gtk4::gio::AppInfo::launch_default_for_uri(
                &request.url,
                None::<&gtk4::gio::AppLaunchContext>,
            ) {
                eprintln!(
                    "rusty-blox: could not open Roblox's page in the system browser: {error}"
                );
            }
        }

        if self.game_activity.is_some() {
            let _ = roblox_runtime::android::looper::poll_for_current_thread(0);
            self.refresh_text_focus();
            self.sync_text_overlay();
            self.sync_cursor_lock();
            if let Some(engine) = &self.engine {
                if let Some(session_dir) = &self.session_dir {
                    roblox_runtime::session::flush_if_due(engine, session_dir);
                }
            }
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(
            std::time::Instant::now() + std::time::Duration::from_millis(16),
        ));
    }

    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _device_id: winit::event::DeviceId,
        event: DeviceEvent,
    ) {
        if !self.cursor_locked {
            return;
        }
        if let DeviceEvent::MouseMotion { delta } = event {
            let Some(window) = self.window.as_ref() else {
                return;
            };
            let size = window.inner_size();
            let position = (size.width as f32 / 2.0, size.height as f32 / 2.0);
            self.forward_locked_mouse_move(position, (delta.0 as f32, delta.1 as f32));
        }
    }
}

#[cfg(not(feature = "webview"))]
fn external_web_url_allowed(uri: &str) -> bool {
    let Ok(parsed) = gtk4::glib::Uri::parse(uri, gtk4::glib::UriFlags::NONE) else {
        return false;
    };
    parsed.scheme().eq_ignore_ascii_case("https")
        && parsed.host().is_some_and(|host| !host.is_empty())
        && parsed.userinfo().is_none()
        && (parsed.port() == -1 || parsed.port() == 443)
}

impl ClientApp {
    fn sync_text_overlay(&mut self) {
        let Some(owner) = self.surface_owner.as_mut() else {
            return;
        };
        if roblox_runtime::jni::game_activity::focused_textbox().is_none() {
            owner.hide_text_overlay();
            return;
        }
        let Some(info) = roblox_runtime::jni::game_activity::focused_textbox_info() else {
            owner.hide_text_overlay();
            return;
        };
        let scale = self.window.as_ref().map_or(1.0, Window::scale_factor);
        owner.update_text_overlay(&self.text_value, self.text_cursor, info, scale);
    }

    fn refresh_text_focus(&mut self) {
        let generation = roblox_runtime::jni::game_activity::textbox_generation();
        if self.text_generation == Some(generation) {
            return;
        }
        self.text_generation = Some(generation);
        if roblox_runtime::jni::game_activity::focused_textbox().is_some() {
            self.text_value = roblox_runtime::jni::game_activity::textbox_text();
            let state_generation = roblox_runtime::jni::game_activity::ime_state_generation();
            let selection = if state_generation != 0 {
                Some(roblox_runtime::jni::game_activity::ime_state_selection())
            } else {
                None
            };
            self.text_cursor = selection
                .map(|(_, end)| end.max(0) as usize)
                .unwrap_or_else(|| self.text_value.chars().count())
                .min(self.text_value.chars().count());
            self.text_selection_anchor = selection.and_then(|(start, end)| {
                let start = start.max(0) as usize;
                (start != end.max(0) as usize).then_some(start)
            });
            self.update_ime_cursor_area();
            eprintln!(
                "[input] text box focused; seeded {} characters",
                self.text_value.chars().count()
            );
        } else {
            self.text_value.clear();
            self.text_cursor = 0;
            self.text_selection_anchor = None;
        }
    }

    fn update_ime_cursor_area(&self) {
        let (Some(window), Some(info)) = (
            self.window.as_ref(),
            roblox_runtime::jni::game_activity::focused_textbox_info(),
        ) else {
            return;
        };
        window.set_ime_cursor_area(
            PhysicalPosition::new(info.x, info.y),
            PhysicalSize::new(info.width.max(1.0), info.height.max(1.0)),
        );
    }

    fn commit_text(&mut self, committed: &str) {
        self.refresh_text_focus();
        if roblox_runtime::jni::game_activity::focused_textbox().is_none() || committed.is_empty() {
            return;
        }
        if let Some((start, end)) = self.selection_range() {
            self.text_value
                .replace_range(self.char_byte_offset(start)..self.char_byte_offset(end), "");
            self.text_cursor = start;
            self.text_selection_anchor = None;
        }
        let byte_cursor = self.char_byte_offset(self.text_cursor);
        self.text_value.insert_str(byte_cursor, committed);
        self.text_cursor += committed.chars().count();
        self.send_text_to_engine();
        eprintln!(
            "[input] committed {} text characters; caret={}",
            committed.chars().count(),
            self.text_cursor
        );
    }

    fn editing_key(&mut self, code: KeyCode) {
        self.refresh_text_focus();
        if roblox_runtime::jni::game_activity::focused_textbox().is_none() {
            return;
        }
        let mut changed = false;
        if let Some((start, end)) = self.selection_range() {
            match code {
                KeyCode::Backspace | KeyCode::Delete => {
                    self.text_value.replace_range(
                        self.char_byte_offset(start)..self.char_byte_offset(end),
                        "",
                    );
                    self.text_cursor = start;
                    self.text_selection_anchor = None;
                    changed = true;
                }
                KeyCode::ArrowLeft | KeyCode::ArrowRight | KeyCode::Home | KeyCode::End => {
                    self.text_cursor = if matches!(code, KeyCode::ArrowLeft | KeyCode::Home) {
                        start
                    } else {
                        end
                    };
                    self.text_selection_anchor = None;
                }
                _ => return,
            }
            if changed
                || matches!(
                    code,
                    KeyCode::ArrowLeft | KeyCode::ArrowRight | KeyCode::Home | KeyCode::End
                )
            {
                self.send_text_to_engine();
            }
            return;
        }
        match code {
            KeyCode::ArrowLeft => {
                self.text_cursor = self.text_cursor.saturating_sub(1);
                self.text_selection_anchor = None;
            }
            KeyCode::ArrowRight => {
                self.text_cursor = (self.text_cursor + 1).min(self.text_value.chars().count());
                self.text_selection_anchor = None;
            }
            KeyCode::Home => {
                self.text_cursor = 0;
                self.text_selection_anchor = None;
            }
            KeyCode::End => {
                self.text_cursor = self.text_value.chars().count();
                self.text_selection_anchor = None;
            }
            KeyCode::Backspace if self.text_cursor > 0 => {
                self.text_cursor -= 1;
                let byte = self
                    .text_value
                    .char_indices()
                    .nth(self.text_cursor)
                    .unwrap()
                    .0;
                self.text_value.remove(byte);
                changed = true;
            }
            KeyCode::Delete if self.text_cursor < self.text_value.chars().count() => {
                let byte = self
                    .text_value
                    .char_indices()
                    .nth(self.text_cursor)
                    .unwrap()
                    .0;
                self.text_value.remove(byte);
                changed = true;
            }
            _ => return,
        }
        if changed
            || matches!(
                code,
                KeyCode::ArrowLeft | KeyCode::ArrowRight | KeyCode::Home | KeyCode::End
            )
        {
            self.send_text_to_engine();
        }
    }

    fn text_shortcut(&mut self, code: KeyCode) -> bool {
        self.refresh_text_focus();
        if roblox_runtime::jni::game_activity::focused_textbox().is_none() {
            return false;
        }
        match code {
            KeyCode::KeyA => {
                self.text_selection_anchor = Some(0);
                self.text_cursor = self.text_value.chars().count();
            }
            KeyCode::KeyC | KeyCode::KeyX => {
                if let Some((start, end)) = self.selection_range() {
                    let selected: String = self
                        .text_value
                        .chars()
                        .skip(start)
                        .take(end - start)
                        .collect();
                    if let Err(error) = clipboard_write(&selected) {
                        eprintln!("rusty-blox: clipboard copy failed: {error}");
                    }
                    if code == KeyCode::KeyX {
                        self.text_value.replace_range(
                            self.char_byte_offset(start)..self.char_byte_offset(end),
                            "",
                        );
                        self.text_cursor = start;
                        self.text_selection_anchor = None;
                        self.send_text_to_engine();
                    }
                }
            }
            KeyCode::KeyV => match clipboard_read() {
                Ok(text) => self.commit_text(&text),
                Err(error) => eprintln!("rusty-blox: clipboard paste failed: {error}"),
            },
            _ => return false,
        }
        true
    }

    fn selection_range(&self) -> Option<(usize, usize)> {
        let anchor = self.text_selection_anchor?;
        (anchor != self.text_cursor)
            .then_some((anchor.min(self.text_cursor), anchor.max(self.text_cursor)))
    }

    fn char_byte_offset(&self, index: usize) -> usize {
        self.text_value
            .char_indices()
            .nth(index)
            .map_or(self.text_value.len(), |(offset, _)| offset)
    }

    fn send_text_to_engine(&self) {
        let Some(engine) = self.engine.as_ref() else {
            return;
        };
        let cursor = self.text_cursor.min(i32::MAX as usize) as i32;
        if let Some(native) = engine.symbol(
            "Java_com_roblox_engine_jni_NativeGLInterface_syncTextboxTextAndCursorPosition2",
        ) {
            // SAFETY: this export belongs to the live loaded engine.
            if let Err(error) = unsafe {
                roblox_runtime::jni::game_activity::sync_textbox(native, &self.text_value, cursor)
            } {
                eprintln!("rusty-blox: synchronize text box failed: {error}");
            }
        } else {
            eprintln!("rusty-blox: text box synchronization native is missing");
        }
        if let Some(handle) = self.game_activity {
            if let Err(error) = roblox_runtime::jni::game_activity::text_input(
                handle,
                &self.text_value,
                cursor,
                cursor,
            ) {
                eprintln!("rusty-blox: GameActivity text update failed: {error}");
            }
        }
    }

    fn sync_cursor_lock(&mut self) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        let Some(native) = self.engine.as_ref().and_then(|engine| {
            engine.symbol(
                "Java_com_roblox_engine_jni_NativeInputInterface_nativeGetMainWindowIsMouseLockedCenter",
            )
        }) else {
            return;
        };
        // SAFETY: this is the live getter export from the loaded Roblox library.
        let wants_lock = match unsafe {
            roblox_runtime::jni::game_activity::call_static_bare_bool(
                native,
                "com/roblox/engine/jni/NativeInputInterface",
            )
        } {
            Ok(wants_lock) => wants_lock && self.window_focused,
            Err(error) => {
                eprintln!("rusty-blox: read Roblox mouse lock request failed: {error}");
                return;
            }
        };
        if wants_lock == self.cursor_locked {
            return;
        }

        if wants_lock {
            let result = window
                .set_cursor_grab(CursorGrabMode::Locked)
                .or_else(|_| window.set_cursor_grab(CursorGrabMode::Confined));
            match result {
                Ok(()) => {
                    self.cursor_locked = true;
                    self.update_system_cursor_visibility();
                    eprintln!("rusty-blox: cursor captured by Roblox");
                }
                Err(error) => eprintln!("rusty-blox: cursor capture failed: {error}"),
            }
        } else {
            if let Err(error) = window.set_cursor_grab(CursorGrabMode::None) {
                eprintln!("rusty-blox: cursor release failed: {error}");
            }
            self.cursor_locked = false;
            self.update_system_cursor_visibility();
            eprintln!("rusty-blox: cursor released by Roblox");
        }
    }

    fn update_system_cursor_visibility(&self) {
        if let Some(window) = self.window.as_ref() {
            // Roblox draws its own pointer. Hide the host pointer over or while
            // focused on the game window, including when Roblox releases grab.
            window.set_cursor_visible(
                !(self.window_focused || self.cursor_inside || self.cursor_locked),
            );
        }
    }

    fn forward_mouse_move(
        &mut self,
        position: (f32, f32),
        delta: (f32, f32),
        event_loop: &ActiveEventLoop,
    ) {
        let result = self.engine.as_ref().and_then(|engine| {
            engine
                .symbol("Java_com_roblox_engine_jni_NativeInputInterface_nativePassMouseMove")
                .map(|native| {
                    // SAFETY: this export belongs to the live engine library.
                    unsafe {
                        roblox_runtime::jni::game_activity::pass_mouse_move(
                            native, position.0, position.1, delta.0, delta.1,
                        )
                    }
                })
        });
        self.record_input_result(result, event_loop);
    }

    fn forward_mouse_button(
        &mut self,
        position: (f32, f32),
        down: bool,
        button: i32,
        event_loop: &ActiveEventLoop,
    ) {
        let result = self.engine.as_ref().and_then(|engine| {
            engine
                .symbol("Java_com_roblox_engine_jni_NativeInputInterface_nativePassMouseButton")
                .map(|native| {
                    // SAFETY: this export belongs to the live engine library.
                    unsafe {
                        roblox_runtime::jni::game_activity::pass_mouse_button(
                            native, position.0, position.1, down, button,
                        )
                    }
                })
        });
        self.record_input_result(result, event_loop);
    }

    fn forward_mouse_wheel(
        &mut self,
        position: (f32, f32),
        delta: f32,
        event_loop: &ActiveEventLoop,
    ) {
        let result = self.engine.as_ref().and_then(|engine| {
            engine
                .symbol("Java_com_roblox_engine_jni_NativeInputInterface_nativePassMouseWheel")
                .map(|native| {
                    // SAFETY: this export belongs to the live engine library.
                    unsafe {
                        roblox_runtime::jni::game_activity::pass_mouse_wheel(
                            native, position.0, position.1, delta,
                        )
                    }
                })
        });
        self.record_input_result(result, event_loop);
    }

    fn forward_key(
        &mut self,
        down: bool,
        key_code: i32,
        evdev_code: i32,
        unicode_char: i32,
        modifiers: ModifiersState,
        repeat: bool,
        event_loop: &ActiveEventLoop,
    ) {
        let mut android_modifiers = 0;
        if modifiers.shift_key() {
            android_modifiers |= 1;
        }
        if modifiers.alt_key() {
            android_modifiers |= 2;
        }
        if modifiers.control_key() {
            android_modifiers |= 0x1000;
        }
        let text_box_focused = roblox_runtime::jni::game_activity::focused_textbox().is_some();
        let text_key = is_evdev_text_key(evdev_code);
        let send_to_game = if down {
            !(text_box_focused && text_key)
        } else {
            self.forwarded_keys.remove(&evdev_code)
        };
        let result = send_to_game
            .then(|| {
                self.engine.as_ref().and_then(|engine| {
                    engine
                        .symbol("Java_com_roblox_engine_jni_NativeGLInterface_nativePassKeyEvent")
                        .map(|native| {
                            // SAFETY: this export belongs to the live engine library.
                            unsafe {
                                roblox_runtime::jni::game_activity::pass_key_event(
                                    native,
                                    down,
                                    evdev_code,
                                    android_modifiers,
                                    repeat,
                                )
                            }
                        })
                })
            })
            .flatten();
        if down && send_to_game && result.is_some() {
            self.forwarded_keys.insert(evdev_code);
        }
        if down {
            let native_status = if text_box_focused && text_key {
                "suppressed for text box"
            } else {
                match &result {
                    Some(Ok(())) => "sent",
                    Some(Err(_)) => "failed",
                    None => "export missing",
                }
            };
            eprintln!(
                "[input] key down: Android={key_code} evdev={evdev_code} unicode={unicode_char} NativeInputInterface={native_status}"
            );
        }
        self.record_input_result(result, event_loop);
    }

    fn update_movement_key(
        &mut self,
        code: KeyCode,
        state: ElementState,
        event_loop: &ActiveEventLoop,
    ) {
        if state == ElementState::Pressed {
            // Key repeat must not make a held direction jump ahead of a newer
            // opposing key.
            if !self.movement_pressed.contains(&code) {
                self.movement_pressed.push(code);
            }
        } else {
            self.movement_pressed.retain(|pressed| *pressed != code);
        }

        for axis in 0..2 {
            let next = self
                .movement_pressed
                .iter()
                .rev()
                .copied()
                .find(|pressed| movement_axis(*pressed) == Some(axis));
            if self.movement_active[axis] == next {
                continue;
            }
            if let Some(previous) = self.movement_active[axis] {
                self.send_movement_key(previous, false, event_loop);
            }
            if let Some(next) = next {
                self.send_movement_key(next, true, event_loop);
            }
            self.movement_active[axis] = next;
        }
    }

    fn clear_movement_keys(&mut self, event_loop: &ActiveEventLoop) {
        let active = std::mem::replace(&mut self.movement_active, [None, None]);
        for code in active.into_iter().flatten() {
            self.send_movement_key(code, false, event_loop);
        }
        self.movement_pressed.clear();
    }

    fn send_movement_key(&mut self, code: KeyCode, down: bool, event_loop: &ActiveEventLoop) {
        let (Some(key_code), Some(evdev_code)) = (android_key_code(code), evdev_key_code(code))
        else {
            return;
        };
        let unicode_char = match code {
            KeyCode::KeyW => 'w',
            KeyCode::KeyA => 'a',
            KeyCode::KeyS => 's',
            KeyCode::KeyD => 'd',
            _ => return,
        } as i32;
        self.forward_key(
            down,
            key_code,
            evdev_code,
            unicode_char,
            self.modifiers,
            false,
            event_loop,
        );
    }

    fn record_input_result(
        &mut self,
        result: Option<Result<(), String>>,
        _event_loop: &ActiveEventLoop,
    ) {
        if let Some(Err(error)) = result {
            eprintln!("rusty-blox: input forwarding failed: {error}");
        }
    }
}

fn configured_start_size(data_dir: &std::path::Path) -> Option<PhysicalSize<u32>> {
    let path = data_dir.join("files/appData/GlobalBasicSettings_13.xml");
    let xml = std::fs::read_to_string(path).ok()?;
    let mut reader = quick_xml::Reader::from_str(&xml);
    let mut in_start_size = false;
    let mut component: Option<u8> = None;
    let mut width = None;
    let mut height = None;

    loop {
        match reader.read_event().ok()? {
            quick_xml::events::Event::Start(element) => {
                if !in_start_size && element.name().as_ref() == b"Vector2" {
                    in_start_size = element.attributes().flatten().any(|attribute| {
                        attribute.key.as_ref() == b"name"
                            && attribute.value.as_ref() == b"StartScreenSize"
                    });
                } else if in_start_size {
                    component = match element.name().as_ref() {
                        b"X" => Some(b'X'),
                        b"Y" => Some(b'Y'),
                        _ => None,
                    };
                }
            }
            quick_xml::events::Event::Text(text) if in_start_size => {
                if let Some(value) = text.decode().ok()?.trim().parse::<u32>().ok() {
                    match component {
                        Some(b'X') => width = Some(value),
                        Some(b'Y') => height = Some(value),
                        _ => {}
                    }
                }
            }
            quick_xml::events::Event::End(element) => match element.name().as_ref() {
                b"X" | b"Y" => component = None,
                b"Vector2" if in_start_size => {
                    in_start_size = false;
                    if let (Some(width), Some(height)) = (width, height) {
                        if width > 0 && height > 0 {
                            return Some(PhysicalSize::new(width, height));
                        }
                    }
                }
                _ => {}
            },
            quick_xml::events::Event::Eof => return None,
            _ => {}
        }
    }
}

fn clipboard_write(text: &str) -> Result<(), String> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    for (program, args) in [
        ("wl-copy", vec![]),
        ("xclip", vec!["-selection", "clipboard"]),
        ("xsel", vec!["--clipboard", "--input"]),
    ] {
        let Ok(mut child) = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .spawn()
        else {
            continue;
        };
        if let Some(mut stdin) = child.stdin.take() {
            if let Err(error) = stdin.write_all(text.as_bytes()) {
                return Err(error.to_string());
            }
        }
        return child
            .wait()
            .map_err(|error| error.to_string())?
            .success()
            .then_some(())
            .ok_or_else(|| format!("{program} exited unsuccessfully"));
    }
    Err("no clipboard tool found (install wl-clipboard, xclip, or xsel)".into())
}

fn clipboard_read() -> Result<String, String> {
    use std::process::Command;
    for (program, args) in [
        ("wl-paste", vec!["--no-newline"]),
        ("xclip", vec!["-selection", "clipboard", "-o"]),
        ("xsel", vec!["--clipboard", "--output"]),
    ] {
        let Ok(output) = Command::new(program).args(args).output() else {
            continue;
        };
        if output.status.success() {
            return String::from_utf8(output.stdout).map_err(|error| error.to_string());
        }
    }
    Err("no readable clipboard tool found (install wl-clipboard, xclip, or xsel)".into())
}

fn android_mouse_button(button: MouseButton) -> Option<i32> {
    match button {
        MouseButton::Left => Some(0),
        MouseButton::Right => Some(1),
        MouseButton::Middle => Some(2),
        _ => None,
    }
}

fn movement_axis(code: KeyCode) -> Option<usize> {
    match code {
        KeyCode::KeyA | KeyCode::KeyD => Some(0),
        KeyCode::KeyW | KeyCode::KeyS => Some(1),
        _ => None,
    }
}

fn android_key_code(code: KeyCode) -> Option<i32> {
    Some(match code {
        KeyCode::Digit0 => 7,
        KeyCode::Digit1 => 8,
        KeyCode::Digit2 => 9,
        KeyCode::Digit3 => 10,
        KeyCode::Digit4 => 11,
        KeyCode::Digit5 => 12,
        KeyCode::Digit6 => 13,
        KeyCode::Digit7 => 14,
        KeyCode::Digit8 => 15,
        KeyCode::Digit9 => 16,
        KeyCode::Numpad0 => 144,
        KeyCode::Numpad1 => 145,
        KeyCode::Numpad2 => 146,
        KeyCode::Numpad3 => 147,
        KeyCode::Numpad4 => 148,
        KeyCode::Numpad5 => 149,
        KeyCode::Numpad6 => 150,
        KeyCode::Numpad7 => 151,
        KeyCode::Numpad8 => 152,
        KeyCode::Numpad9 => 153,
        KeyCode::NumpadDivide => 154,
        KeyCode::NumpadMultiply => 155,
        KeyCode::NumpadSubtract => 156,
        KeyCode::NumpadAdd => 157,
        KeyCode::NumpadDecimal => 158,
        KeyCode::NumpadComma => 159,
        KeyCode::NumpadEqual => 161,
        KeyCode::KeyA => 29,
        KeyCode::KeyB => 30,
        KeyCode::KeyC => 31,
        KeyCode::KeyD => 32,
        KeyCode::KeyE => 33,
        KeyCode::KeyF => 34,
        KeyCode::KeyG => 35,
        KeyCode::KeyH => 36,
        KeyCode::KeyI => 37,
        KeyCode::KeyJ => 38,
        KeyCode::KeyK => 39,
        KeyCode::KeyL => 40,
        KeyCode::KeyM => 41,
        KeyCode::KeyN => 42,
        KeyCode::KeyO => 43,
        KeyCode::KeyP => 44,
        KeyCode::KeyQ => 45,
        KeyCode::KeyR => 46,
        KeyCode::KeyS => 47,
        KeyCode::KeyT => 48,
        KeyCode::KeyU => 49,
        KeyCode::KeyV => 50,
        KeyCode::KeyW => 51,
        KeyCode::KeyX => 52,
        KeyCode::KeyY => 53,
        KeyCode::KeyZ => 54,
        KeyCode::Comma => 55,
        KeyCode::Period => 56,
        KeyCode::AltLeft => 57,
        KeyCode::AltRight => 58,
        KeyCode::ShiftLeft => 59,
        KeyCode::ShiftRight => 60,
        KeyCode::Tab => 61,
        KeyCode::Space => 62,
        KeyCode::Enter => 66,
        KeyCode::NumpadEnter => 160,
        KeyCode::Backspace => 67,
        KeyCode::Backquote => 68,
        KeyCode::Minus => 69,
        KeyCode::Equal => 70,
        KeyCode::BracketLeft => 71,
        KeyCode::BracketRight => 72,
        KeyCode::Backslash => 73,
        KeyCode::Semicolon => 74,
        KeyCode::Quote => 75,
        KeyCode::Slash => 76,
        KeyCode::ContextMenu => 82,
        KeyCode::PageUp => 92,
        KeyCode::PageDown => 93,
        KeyCode::Escape => 111,
        KeyCode::Delete => 112,
        KeyCode::ControlLeft => 113,
        KeyCode::ControlRight => 114,
        KeyCode::CapsLock => 115,
        KeyCode::Home => 3,
        KeyCode::ArrowUp => 19,
        KeyCode::ArrowDown => 20,
        KeyCode::ArrowLeft => 21,
        KeyCode::ArrowRight => 22,
        KeyCode::End => 123,
        KeyCode::Insert => 124,
        KeyCode::F1 => 131,
        KeyCode::F2 => 132,
        KeyCode::F3 => 133,
        KeyCode::F4 => 134,
        KeyCode::F5 => 135,
        KeyCode::F6 => 136,
        KeyCode::F7 => 137,
        KeyCode::F8 => 138,
        KeyCode::F9 => 139,
        KeyCode::F10 => 140,
        KeyCode::F11 => 141,
        KeyCode::F12 => 142,
        _ => return None,
    })
}

// NativeGLInterface.nativePassKeyEvent expects Linux evdev codes, while the
// GameActivity KeyEvent path above uses Android keycodes.
fn evdev_key_code(code: KeyCode) -> Option<i32> {
    Some(match code {
        KeyCode::Digit1 => 2,
        KeyCode::Digit2 => 3,
        KeyCode::Digit3 => 4,
        KeyCode::Digit4 => 5,
        KeyCode::Digit5 => 6,
        KeyCode::Digit6 => 7,
        KeyCode::Digit7 => 8,
        KeyCode::Digit8 => 9,
        KeyCode::Digit9 => 10,
        KeyCode::Digit0 => 11,
        KeyCode::Numpad1 => 79,
        KeyCode::Numpad2 => 80,
        KeyCode::Numpad3 => 81,
        KeyCode::Numpad4 => 75,
        KeyCode::Numpad5 => 76,
        KeyCode::Numpad6 => 77,
        KeyCode::Numpad7 => 71,
        KeyCode::Numpad8 => 72,
        KeyCode::Numpad9 => 73,
        KeyCode::Numpad0 => 82,
        KeyCode::NumpadDecimal => 83,
        KeyCode::NumpadAdd => 78,
        KeyCode::NumpadSubtract => 74,
        KeyCode::NumpadMultiply => 55,
        KeyCode::NumpadDivide => 98,
        KeyCode::NumpadComma => 121,
        KeyCode::NumpadEqual => 117,
        KeyCode::KeyQ => 16,
        KeyCode::KeyW => 17,
        KeyCode::KeyE => 18,
        KeyCode::KeyR => 19,
        KeyCode::KeyT => 20,
        KeyCode::KeyY => 21,
        KeyCode::KeyU => 22,
        KeyCode::KeyI => 23,
        KeyCode::KeyO => 24,
        KeyCode::KeyP => 25,
        KeyCode::KeyA => 30,
        KeyCode::KeyS => 31,
        KeyCode::KeyD => 32,
        KeyCode::KeyF => 33,
        KeyCode::KeyG => 34,
        KeyCode::KeyH => 35,
        KeyCode::KeyJ => 36,
        KeyCode::KeyK => 37,
        KeyCode::KeyL => 38,
        KeyCode::KeyZ => 44,
        KeyCode::KeyX => 45,
        KeyCode::KeyC => 46,
        KeyCode::KeyV => 47,
        KeyCode::KeyB => 48,
        KeyCode::KeyN => 49,
        KeyCode::KeyM => 50,
        KeyCode::Escape => 1,
        KeyCode::Tab => 15,
        KeyCode::Enter => 28,
        KeyCode::NumpadEnter => 96,
        KeyCode::Backspace => 14,
        KeyCode::Space => 57,
        KeyCode::ShiftLeft => 42,
        KeyCode::ShiftRight => 54,
        KeyCode::ControlLeft => 29,
        KeyCode::ControlRight => 97,
        KeyCode::AltLeft => 56,
        KeyCode::AltRight => 100,
        KeyCode::ArrowUp => 103,
        KeyCode::ArrowDown => 108,
        KeyCode::ArrowLeft => 105,
        KeyCode::ArrowRight => 106,
        KeyCode::F1 => 59,
        KeyCode::F2 => 60,
        KeyCode::F3 => 61,
        KeyCode::F4 => 62,
        KeyCode::F5 => 63,
        KeyCode::F6 => 64,
        KeyCode::F7 => 65,
        KeyCode::F8 => 66,
        KeyCode::F9 => 67,
        KeyCode::F10 => 68,
        KeyCode::F11 => 87,
        KeyCode::F12 => 88,
        _ => return None,
    })
}

fn is_evdev_text_key(code: i32) -> bool {
    matches!(code, 2..=13 | 16..=27 | 30..=41 | 44..=55 | 57 | 71..=83 | 98 | 117 | 121)
}
