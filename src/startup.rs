use winit::dpi::PhysicalSize;

pub(crate) fn initialize_client(
    engine: &roblox_runtime::LoadedEngine,
    config: &roblox_runtime::RuntimeConfig,
    asset_dir: &std::path::Path,
    game_activity: i64,
    size: PhysicalSize<u32>,
) -> Result<(), String> {
    let assets = asset_dir
        .to_str()
        .ok_or_else(|| "asset directory path is not UTF-8".to_owned())?;
    let width = crate::host_window::dimension_i32(size.width)?;
    let height = crate::host_window::dimension_i32(size.height)?;
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

pub(crate) fn config_path(config: &roblox_runtime::RuntimeConfig, child: &str) -> String {
    config.data_dir.join(child).to_string_lossy().into_owned()
}
