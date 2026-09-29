use std::collections::HashSet;
use std::path::PathBuf;

use crate::host_window::SurfaceOwner;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{DeviceEvent, ElementState, Ime, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};
use winit::window::{CursorGrabMode, Window, WindowId};

pub(crate) struct ClientApp {
    config: Option<roblox_runtime::RuntimeConfig>,
    asset_dir: PathBuf,
    data_dir: PathBuf,
    window: Option<Window>,
    surface_owner: Option<SurfaceOwner>,
    engine: Option<roblox_runtime::LoadedEngine>,
    game_activity: Option<i64>,
    failure: Option<String>,
    cursor: (f32, f32),
    modifiers: ModifiersState,
    cursor_locked: bool,
    text_generation: Option<u32>,
    text_value: String,
    text_cursor: usize,
    forwarded_keys: HashSet<i32>,
    settings: crate::settings::Settings,
    game_mode: Option<crate::desktop::GameMode>,
    discord_presence: Option<crate::desktop::DiscordPresence>,
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
    ) -> Self {
        let data_dir = config.data_dir.clone();
        Self {
            config: Some(config),
            asset_dir,
            data_dir,
            window: None,
            surface_owner: None,
            engine: None,
            game_activity: None,
            failure: None,
            cursor: (0.0, 0.0),
            modifiers: winit::keyboard::ModifiersState::empty(),
            cursor_locked: false,
            text_generation: None,
            text_value: String::new(),
            text_cursor: 0,
            forwarded_keys: HashSet::new(),
            settings,
            game_mode: None,
            discord_presence: None,
        }
    }

    pub(crate) fn take_failure(&mut self) -> Option<String> {
        self.failure.take()
    }
}

impl ClientApp {
    fn launch(&mut self, event_loop: &ActiveEventLoop) -> Result<(), String> {
        // Start at 1280x720 when the display can fit it, scaling down to keep
        // the window floating and fully visible on smaller screens. The user
        // can resize it after launch.
        let base = PhysicalSize::new(1280_u32, 720_u32);
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
        let window = event_loop
            .create_window(
                Window::default_attributes()
                    .with_title("roblox-runtime")
                    .with_resizable(true)
                    .with_maximized(false)
                    .with_inner_size(size),
            )
            .map_err(|error| format!("create host window: {error}"))?;
        // Some window managers restore the previous size after applying the
        // initial attributes. Reassert the requested floating size once the
        // native window exists; this remains a request, not a size constraint.
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
            .initialize_game_activity(&internal, &internal, &external)
            .map_err(|error| format!("initialize GameActivity: {error}"))?;
        println!("GameActivity initialized; handle={game_activity}");
        crate::startup::initialize_client(&engine, &config, &self.asset_dir, game_activity, size)?;
        self.engine = Some(engine);
        self.game_activity = Some(game_activity);
        Ok(())
    }
}

impl Drop for ClientApp {
    fn drop(&mut self) {
        if let Some(engine) = &self.engine {
            if let Err(error) = crate::session::save(engine, &self.data_dir) {
                eprintln!("rusty-blox: could not save Roblox session: {error}");
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
                        self.editing_key(code);
                    }
                    if let (Some(key_code), Some(evdev_code)) =
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
        if self.game_activity.is_some() {
            let _ = roblox_runtime::android::looper::poll_for_current_thread(0);
            self.refresh_text_focus();
            self.sync_cursor_lock();
            if let Some(engine) = &self.engine {
                crate::session::flush_if_due(engine, &self.data_dir);
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

impl ClientApp {
    fn refresh_text_focus(&mut self) {
        let generation = roblox_runtime::jni::game_activity::textbox_generation();
        if self.text_generation == Some(generation) {
            return;
        }
        self.text_generation = Some(generation);
        if roblox_runtime::jni::game_activity::focused_textbox().is_some() {
            self.text_value = roblox_runtime::jni::game_activity::textbox_text();
            let state_generation = roblox_runtime::jni::game_activity::ime_state_generation();
            self.text_cursor = if state_generation != 0 {
                roblox_runtime::jni::game_activity::ime_state_selection()
                    .1
                    .max(0) as usize
            } else {
                self.text_value.chars().count()
            }
            .min(self.text_value.chars().count());
            self.update_ime_cursor_area();
            eprintln!(
                "[input] text box focused; seeded {} characters",
                self.text_value.chars().count()
            );
        } else {
            self.text_value.clear();
            self.text_cursor = 0;
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
        let byte_cursor = self
            .text_value
            .char_indices()
            .nth(self.text_cursor)
            .map_or(self.text_value.len(), |(offset, _)| offset);
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
        match code {
            KeyCode::ArrowLeft => self.text_cursor = self.text_cursor.saturating_sub(1),
            KeyCode::ArrowRight => {
                self.text_cursor = (self.text_cursor + 1).min(self.text_value.chars().count())
            }
            KeyCode::Home => self.text_cursor = 0,
            KeyCode::End => self.text_cursor = self.text_value.chars().count(),
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
            Ok(wants_lock) => wants_lock,
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
                    window.set_cursor_visible(false);
                    self.cursor_locked = true;
                    eprintln!("rusty-blox: cursor captured by Roblox");
                }
                Err(error) => eprintln!("rusty-blox: cursor capture failed: {error}"),
            }
        } else {
            if let Err(error) = window.set_cursor_grab(CursorGrabMode::None) {
                eprintln!("rusty-blox: cursor release failed: {error}");
            }
            window.set_cursor_visible(true);
            self.cursor_locked = false;
            eprintln!("rusty-blox: cursor released by Roblox");
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

fn android_mouse_button(button: MouseButton) -> Option<i32> {
    match button {
        MouseButton::Left => Some(0),
        MouseButton::Right => Some(1),
        MouseButton::Middle => Some(2),
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
