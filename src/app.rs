use std::path::PathBuf;

use crate::host_window::SurfaceOwner;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};
use winit::window::{Window, WindowId};

pub(crate) struct ClientApp {
    config: Option<roblox_runtime::RuntimeConfig>,
    asset_dir: PathBuf,
    window: Option<Window>,
    surface_owner: Option<SurfaceOwner>,
    engine: Option<roblox_runtime::LoadedEngine>,
    game_activity: Option<i64>,
    failure: Option<String>,
    cursor: (f32, f32),
    modifiers: ModifiersState,
}

impl ClientApp {
    pub(crate) fn new(config: roblox_runtime::RuntimeConfig, asset_dir: PathBuf) -> Self {
        Self {
            config: Some(config),
            asset_dir,
            window: None,
            surface_owner: None,
            engine: None,
            game_activity: None,
            failure: None,
            cursor: (0.0, 0.0),
            modifiers: winit::keyboard::ModifiersState::empty(),
        }
    }

    pub(crate) fn take_failure(&mut self) -> Option<String> {
        self.failure.take()
    }
}

impl ClientApp {
    fn launch(&mut self, event_loop: &ActiveEventLoop) -> Result<(), String> {
        let window = event_loop
            .create_window(
                Window::default_attributes()
                    .with_title("rusty-blox")
                    .with_inner_size(PhysicalSize::new(1280, 720)),
            )
            .map_err(|error| format!("create host window: {error}"))?;
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
                self.forward_mouse_move(next, delta, event_loop);
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
                    if let Some(key_code) = android_key_code(code) {
                        self.forward_key(
                            event.state == ElementState::Pressed,
                            key_code,
                            self.modifiers,
                            event.repeat,
                            event_loop,
                        );
                    }
                }
            }
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
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(
            std::time::Instant::now() + std::time::Duration::from_millis(16),
        ));
    }
}

impl ClientApp {
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
        let result = self.engine.as_ref().and_then(|engine| {
            engine
                .symbol("Java_com_roblox_engine_jni_NativeInputInterface_nativePassKeyEvent")
                .map(|native| {
                    // SAFETY: this export belongs to the live engine library.
                    unsafe {
                        roblox_runtime::jni::game_activity::pass_key_event(
                            native,
                            down,
                            key_code,
                            android_modifiers,
                            repeat,
                        )
                    }
                })
        });
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
        KeyCode::Enter | KeyCode::NumpadEnter => 66,
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
