mod client;

use std::path::PathBuf;
use std::ptr::NonNull;

use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};
use winit::raw_window_handle::{
    HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle,
};
use winit::window::{Window, WindowId};

struct ClientApp {
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

enum SurfaceOwner {
    X11,
    Wayland(WaylandEglWindow),
}

struct WaylandEglWindow {
    _library: libloading::Library,
    handle: NonNull<std::ffi::c_void>,
    resize: unsafe extern "C" fn(*mut std::ffi::c_void, i32, i32, i32, i32),
    destroy: unsafe extern "C" fn(*mut std::ffi::c_void),
}

impl WaylandEglWindow {
    fn create(surface: NonNull<std::ffi::c_void>, width: u32, height: u32) -> Result<Self, String> {
        type Create =
            unsafe extern "C" fn(*mut std::ffi::c_void, i32, i32) -> *mut std::ffi::c_void;
        // SAFETY: the library is kept alive by the returned owner and all
        // function pointers are obtained from that library.
        let library = unsafe { libloading::Library::new("libwayland-egl.so.1") }
            .map_err(|error| format!("load libwayland-egl: {error}"))?;
        // SAFETY: names and signatures are the public wayland-egl ABI.
        let create: libloading::Symbol<Create> = unsafe { library.get(b"wl_egl_window_create\0") }
            .map_err(|error| format!("resolve wl_egl_window_create: {error}"))?;
        // SAFETY: as above.
        let resize: libloading::Symbol<
            unsafe extern "C" fn(*mut std::ffi::c_void, i32, i32, i32, i32),
        > = unsafe { library.get(b"wl_egl_window_resize\0") }
            .map_err(|error| format!("resolve wl_egl_window_resize: {error}"))?;
        // SAFETY: as above.
        let destroy: libloading::Symbol<unsafe extern "C" fn(*mut std::ffi::c_void)> =
            unsafe { library.get(b"wl_egl_window_destroy\0") }
                .map_err(|error| format!("resolve wl_egl_window_destroy: {error}"))?;
        // SAFETY: `surface` is owned by the live winit Window; dimensions fit
        // the signed Wayland EGL ABI after the checks below.
        let handle = NonNull::new(unsafe {
            create(
                surface.as_ptr(),
                dimension_i32(width)?,
                dimension_i32(height)?,
            )
        })
        .ok_or_else(|| "wl_egl_window_create returned null".to_owned())?;
        Ok(Self {
            resize: *resize,
            destroy: *destroy,
            handle,
            _library: library,
        })
    }

    fn resize_to(&self, width: u32, height: u32) -> Result<(), String> {
        // SAFETY: this EGL window remains alive until this owner is dropped.
        unsafe {
            (self.resize)(
                self.handle.as_ptr(),
                dimension_i32(width)?,
                dimension_i32(height)?,
                0,
                0,
            )
        };
        Ok(())
    }
}

impl Drop for WaylandEglWindow {
    fn drop(&mut self) {
        // SAFETY: the handle was created by this library and is destroyed once.
        unsafe { (self.destroy)(self.handle.as_ptr()) };
    }
}

impl SurfaceOwner {
    fn install(window: &Window, size: PhysicalSize<u32>) -> Result<Self, String> {
        let display = window
            .display_handle()
            .map_err(|error| format!("get host display handle: {error}"))?
            .as_raw();
        let raw_window = window
            .window_handle()
            .map_err(|error| format!("get host window handle: {error}"))?
            .as_raw();
        match (display, raw_window) {
            (RawDisplayHandle::Xlib(display), RawWindowHandle::Xlib(window_handle)) => {
                let display = display
                    .display
                    .ok_or_else(|| "winit returned a null Xlib display".to_owned())?;
                // SAFETY: winit owns the X connection and window, and ClientApp
                // retains the Window until after the runtime surface is cleared.
                let surface = unsafe {
                    roblox_runtime::graphics::HostSurface::xlib(
                        display.as_ptr(),
                        window_handle.window as u64,
                        size.width,
                        size.height,
                    )
                }
                .map_err(|error| error.to_string())?;
                roblox_runtime::graphics::install_surface(surface);
                Ok(Self::X11)
            }
            (RawDisplayHandle::Wayland(display), RawWindowHandle::Wayland(window_handle)) => {
                let egl = WaylandEglWindow::create(window_handle.surface, size.width, size.height)?;
                // SAFETY: winit owns the display/surface, while this owner
                // keeps the derived wl_egl_window alive through engine shutdown.
                let surface = unsafe {
                    roblox_runtime::graphics::HostSurface::wayland(
                        display.display.as_ptr(),
                        window_handle.surface.as_ptr(),
                        egl.handle.as_ptr(),
                        size.width,
                        size.height,
                    )
                }
                .map_err(|error| error.to_string())?;
                roblox_runtime::graphics::install_surface(surface);
                Ok(Self::Wayland(egl))
            }
            (display, window) => Err(format!(
                "unsupported winit handles: display {display:?}, window {window:?}; expected Xlib or Wayland"
            )),
        }
    }

    fn resize(&self, width: u32, height: u32) -> Result<(), String> {
        if let Self::Wayland(egl) = self {
            egl.resize_to(width, height)?;
        }
        Ok(())
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
        let size = window.inner_size();
        let owner = SurfaceOwner::install(&window, size)?;
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

        let internal = config_path(&config, "files");
        let external = config_path(&config, "external");
        let game_activity = engine
            .initialize_game_activity(&internal, &internal, &external)
            .map_err(|error| format!("initialize GameActivity: {error}"))?;
        println!("GameActivity initialized; handle={game_activity}");
        initialize_client(&engine, &config, &self.asset_dir, game_activity, size)?;
        self.engine = Some(engine);
        self.game_activity = Some(game_activity);
        Ok(())
    }
}

fn initialize_client(
    engine: &roblox_runtime::LoadedEngine,
    config: &roblox_runtime::RuntimeConfig,
    asset_dir: &std::path::Path,
    game_activity: i64,
    size: PhysicalSize<u32>,
) -> Result<(), String> {
    let assets = asset_dir
        .to_str()
        .ok_or_else(|| "asset directory path is not UTF-8".to_owned())?;
    let width = dimension_i32(size.width)?;
    let height = dimension_i32(size.height)?;
    let files = config_path(config, "files");
    let cache = config
        .cache_dir
        .to_str()
        .ok_or_else(|| "cache directory path is not UTF-8".to_owned())?;

    call_native(
        engine,
        "Java_com_roblox_client_JNIAAssetManagerSetup_initNative",
        |f| {
            // SAFETY: the address is this engine's JNI export and the VM is live.
            unsafe { roblox_runtime::jni::game_activity::asset_manager_init(f) }
        },
    )?;
    call_native(
        engine,
        "Java_com_roblox_client_LocalStorageManager_initStorageManagerNativeV3",
        |f| {
            // SAFETY: same conditions as the asset manager call above.
            unsafe { roblox_runtime::jni::game_activity::storage_init(f, &files, cache) }
        },
    )?;
    call_native(
        engine,
        "Java_com_roblox_client_startup_MainGameActivity_nativeAppBridgeSetInitParams",
        |f| {
            // SAFETY: this exported JNI native receives the live VM environment
            // through the runtime's guarded JNI trampoline.
            unsafe { roblox_runtime::jni::game_activity::set_init_params(f, assets, width, height) }
        },
    )?;

    call_native(
        engine,
        "Java_com_roblox_engine_jni_NativeGLInterface_nativeGameGlobalInit",
        |f| {
            // SAFETY: the export is resolved from this mapped engine.
            unsafe { roblox_runtime::jni::game_activity::appbridge_call_bare(f) }
        },
    )?;
    call_native(
        engine,
        "Java_com_roblox_engine_jni_NativeGLInterface_nativeUpdateAdapterInit",
        |f| {
            // SAFETY: the export is resolved from this mapped engine.
            unsafe { roblox_runtime::jni::game_activity::appbridge_call_bare(f) }
        },
    )?;
    call_native(
        engine,
        "Java_com_roblox_engine_jni_NativeGLInterface_nativeAppBridgeV2InitWithParams",
        |f| {
            // SAFETY: the export is resolved from this mapped engine.
            unsafe { roblox_runtime::jni::game_activity::appbridge_init(f, assets, width, height) }
        },
    )?;
    call_native(
        engine,
        "Java_com_roblox_engine_jni_NativeGLInterface_nativeAppBridgeStartLuaAppDM",
        |f| {
            // SAFETY: the export is resolved from this mapped engine.
            unsafe { roblox_runtime::jni::game_activity::appbridge_call_bare(f) }
        },
    )?;
    call_native(
        engine,
        "Java_com_roblox_engine_jni_NativeGLInterface_nativeAppBridgeV2StartAppWithParams",
        |f| {
            // SAFETY: the export is resolved from this mapped engine.
            unsafe {
                roblox_runtime::jni::game_activity::appbridge_start_app(f, assets, width, height)
            }
        },
    )?;

    for (name, is_game) in [
        (
            "Java_com_roblox_engine_jni_NativeGLInterface_nativeAppBridgeV2UpdateSurfaceAppWithPlatformParams",
            false,
        ),
        (
            "Java_com_roblox_engine_jni_NativeGLInterface_nativeAppBridgeV2UpdateSurfaceGameWithPlatformParams",
            true,
        ),
    ] {
        call_native(engine, name, |f| {
            // SAFETY: the export is resolved from this mapped engine.
            unsafe {
                roblox_runtime::jni::game_activity::appbridge_update_surface(
                    f, assets, width, height, is_game,
                )
            }
        })?;
    }

    engine
        .start_game_activity(game_activity, size.width, size.height, 1)
        .map_err(|error| format!("deliver initial surface: {error}"))
}

fn call_native(
    engine: &roblox_runtime::LoadedEngine,
    name: &str,
    call: impl FnOnce(*mut std::ffi::c_void) -> Result<(), String>,
) -> Result<(), String> {
    let native = engine
        .symbol(name)
        .ok_or_else(|| format!("required startup native is not exported: {name}"))?;
    call(native).map_err(|error| format!("{name}: {error}"))
}

impl Drop for ClientApp {
    fn drop(&mut self) {
        roblox_runtime::graphics::clear_surface();
        drop(self.engine.take());
        drop(self.surface_owner.take());
        drop(self.window.take());
    }
}

impl ApplicationHandler for ClientApp {
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
        event_loop.set_control_flow(ControlFlow::Wait);
    }
}

fn dimension_i32(value: u32) -> Result<i32, String> {
    i32::try_from(value).map_err(|_| "host window dimension exceeds platform limits".into())
}

fn config_path(config: &roblox_runtime::RuntimeConfig, child: &str) -> String {
    config.data_dir.join(child).to_string_lossy().into_owned()
}

fn main() {
    if let Err(error) = run() {
        eprintln!("rusty-blox: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let apks = std::env::args_os()
        .nth(1)
        .map(|path| {
            let base = PathBuf::from(path);
            let mut apks = vec![base.clone()];
            let x86_split = base.with_file_name("split_config.x86_64.apk");
            if x86_split.is_file() {
                apks.push(x86_split);
            }
            apks
        })
        .or_else(client::discover_sober_apks)
        .ok_or("no APK argument and no Sober x86-64 APK was found")?;
    let managed_dir = client::managed_install_dir().ok_or("HOME and XDG_DATA_HOME are unset")?;
    let imported = client::import_apks(&apks, &managed_dir)?;
    let client_root = managed_dir
        .parent()
        .ok_or("managed installation directory has no parent")?;
    let data_dir = client_root.join("data");
    let cache_dir = client_root.join("cache");
    std::fs::create_dir_all(&data_dir)?;
    std::fs::create_dir_all(&cache_dir)?;

    let config = roblox_runtime::RuntimeConfig {
        apk_paths: imported.apk_paths,
        native_lib_dir: imported.native_lib_dir,
        data_dir,
        cache_dir,
        fast_flags: Default::default(),
        options: roblox_runtime::RuntimeOptions::default(),
    };
    let system_dir = config.prepare_android_environment()?;
    let asset_dir = config.prepare_asset_tree()?;
    println!("Android system files: {}", system_dir.display());
    println!("APK assets: {}", asset_dir.display());
    for apk in &config.apk_paths {
        println!("APK: {}", apk.display());
    }
    println!("Native libraries: {}", config.native_lib_dir.display());

    let event_loop = EventLoop::new()?;
    let mut app = ClientApp {
        config: Some(config),
        asset_dir,
        window: None,
        surface_owner: None,
        engine: None,
        game_activity: None,
        failure: None,
    };
    event_loop.run_app(&mut app)?;
    if let Some(error) = app.failure.take() {
        return Err(error.into());
    }
    Ok(())
}
