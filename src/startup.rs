use winit::dpi::PhysicalSize;

pub(crate) fn initialize_client(
    engine: &roblox_runtime::LoadedEngine,
    config: &roblox_runtime::RuntimeConfig,
    asset_dir: &std::path::Path,
    game_activity: i64,
    size: PhysicalSize<u32>,
) -> Result<(), String> {
    roblox_runtime::session::initialize(config.session.as_ref().map(|session| session.directory()));

    // The engine's platform asset folder is the APK's `assets/content`
    // directory, not the extraction root. roblox-runtime's launcher passes this same
    // subdirectory to MainGameActivity and App Bridge.
    let content_dir = asset_dir.join("content");
    if !content_dir.is_dir() {
        return Err(format!(
            "APK asset content directory does not exist: {}",
            content_dir.display()
        ));
    }
    let assets = content_dir
        .to_str()
        .ok_or_else(|| "asset directory path is not UTF-8".to_owned())?;
    let width = crate::host_window::dimension_i32(size.width)?;
    let height = crate::host_window::dimension_i32(size.height)?;
    let files = config_path(config, "files");
    let cache = config
        .cache_dir
        .to_str()
        .ok_or_else(|| "cache directory path is not UTF-8".to_owned())?;

    eprintln!("[startup] init asset manager");
    call_native(
        engine,
        "Java_com_roblox_client_JNIAAssetManagerSetup_initNative",
        |f| {
            // SAFETY: the address is this engine's JNI export and the VM is live.
            unsafe { roblox_runtime::jni::game_activity::asset_manager_init(f) }
        },
    )?;
    eprintln!("[startup] init storage manager");
    call_native(
        engine,
        "Java_com_roblox_client_LocalStorageManager_initStorageManagerNativeV3",
        |f| {
            // SAFETY: same conditions as the asset manager call above.
            unsafe { roblox_runtime::jni::game_activity::storage_init(f, &files, cache) }
        },
    )?;
    eprintln!("[startup] set asset path");
    call_native(
        engine,
        "Java_com_roblox_client_startup_MainGameActivity_nativeSetAssetPath",
        |f| {
            // SAFETY: this is the engine's exported static JNI native and the
            // runtime's JavaVM is live. Android passes the extracted
            // `assets/content` path here before starting App Bridge.
            unsafe {
                roblox_runtime::jni::game_activity::call_static_strings(
                    f,
                    "com/roblox/client/startup/MainGameActivity",
                    &[assets],
                )
            }
        },
    )?;
    eprintln!("[startup] set init params");
    call_native(
        engine,
        "Java_com_roblox_client_startup_MainGameActivity_nativeAppBridgeSetInitParams",
        |f| {
            // SAFETY: this exported JNI native receives the live VM environment
            // through the runtime's guarded JNI trampoline.
            unsafe { roblox_runtime::jni::game_activity::set_init_params(f, assets, width, height) }
        },
    )?;

    // ActivityNativeMain normally receives these through Android's
    // Application.ActivityLifecycleCallbacks. The engine records per-Activity
    // state here and later reads it from the Main thread created by
    // nativeGameGlobalInit; starting that thread without the callbacks leaves
    // its JNI context unset and can segfault during app-bridge startup.
    const ACTIVITY_LIFECYCLE: &[&str] = &[
        "nativeOnPreCreated",
        "nativeOnCreated",
        "nativeOnPostCreated",
        "nativeOnPreStarted",
        "nativeOnStarted",
        "nativeOnPostStarted",
        "nativeOnPreResumed",
        "nativeOnResumed",
        "nativeOnPostResumed",
    ];
    const ACTIVITY_LIFECYCLE_PREFIX: &str =
        "Java_com_roblox_universalapp_activitylifecyclecallbacks_JNIActivityLifecycleCallbacks_";
    for stage in ACTIVITY_LIFECYCLE {
        let name = format!("{ACTIVITY_LIFECYCLE_PREFIX}{stage}");
        call_native(engine, &name, |f| {
            // SAFETY: this is one of the engine's exported Activity lifecycle
            // callbacks. Its JNI environment and Activity name are supplied by
            // the runtime's compatibility bridge.
            unsafe {
                roblox_runtime::jni::game_activity::activity_lifecycle(
                    f,
                    "com.roblox.client.ActivityNativeMain",
                )
            }
        })?;
    }
    eprintln!(
        "[startup] delivered {}/{} ActivityNativeMain lifecycle callbacks",
        ACTIVITY_LIFECYCLE.len(),
        ACTIVITY_LIFECYCLE.len()
    );

    eprintln!("[startup] nativeGameGlobalInit");
    call_native(
        engine,
        "Java_com_roblox_engine_jni_NativeGLInterface_nativeGameGlobalInit",
        |f| {
            // SAFETY: the export is resolved from this mapped engine.
            unsafe { roblox_runtime::jni::game_activity::appbridge_call_bare(f) }
        },
    )?;
    eprintln!("[startup] nativeGameGlobalInit returned");
    eprintln!("[startup] nativeUpdateAdapterInit");
    call_native(
        engine,
        "Java_com_roblox_engine_jni_NativeGLInterface_nativeUpdateAdapterInit",
        |f| {
            // SAFETY: the export is resolved from this mapped engine.
            unsafe { roblox_runtime::jni::game_activity::appbridge_call_bare(f) }
        },
    )?;
    eprintln!("[startup] nativeUpdateAdapterInit returned");
    eprintln!("[startup] nativeAppBridgeV2InitWithParams");
    call_native(
        engine,
        "Java_com_roblox_engine_jni_NativeGLInterface_nativeAppBridgeV2InitWithParams",
        |f| {
            // SAFETY: the export is resolved from this mapped engine.
            unsafe { roblox_runtime::jni::game_activity::appbridge_init(f, assets, width, height) }
        },
    )?;
    eprintln!("[startup] nativeAppBridgeV2InitWithParams returned");

    if let Some(session) = &config.session {
        roblox_runtime::session::restore(engine, session.directory())?;
    }

    call_native(
        engine,
        "Java_com_roblox_engine_jni_NativeGLInterface_nativeAppBridgeStartLuaAppDM",
        |f| {
            // SAFETY: the export is resolved from this mapped engine.
            unsafe { roblox_runtime::jni::game_activity::appbridge_call_bare(f) }
        },
    )?;
    eprintln!("[startup] nativeAppBridgeStartLuaAppDM returned");
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
    eprintln!("[startup] nativeAppBridgeV2StartAppWithParams returned");

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
        eprintln!("[startup] {name} returned");
    }

    eprintln!("[startup] delivering initial GameActivity surface");
    engine
        .start_game_activity(game_activity, size.width, size.height, 1)
        .map_err(|error| format!("deliver initial surface: {error}"))?;
    eprintln!("[startup] initial GameActivity surface delivered");
    match roblox_runtime::jni::game_activity::set_input_connection(game_activity)
        .map_err(|error| format!("install GameActivity InputConnection: {error}"))?
    {
        Some(()) => println!("GameActivity InputConnection installed"),
        None => eprintln!("GameActivity InputConnection native is not registered"),
    }
    Ok(())
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

pub(crate) fn config_path(config: &roblox_runtime::RuntimeConfig, child: &str) -> String {
    config.data_dir.join(child).to_string_lossy().into_owned()
}
