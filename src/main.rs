mod app;
mod client;
mod client_settings;
mod host_window;
mod startup;

use std::path::PathBuf;
use winit::event_loop::EventLoop;

fn main() {
    if let Err(error) = run() {
        eprintln!("rusty-blox: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let mut apk_arg = None;
    let mut fast_flags_path = None;
    let mut client_settings = None;
    let mut host_libc = false;
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--help" | "-h") => {
                println!(
                    "Usage: rusty-blox [--host-libc] [--fast-flags FILE] [--client-settings FILE] [APK]\n\
                     Without APK, the client looks for Sober's x86-64 installation.\n\
                     --host-libc enables the runtime's ABI-unsafe diagnostic resolver."
                );
                return Ok(());
            }
            Some("--host-libc") => host_libc = true,
            Some("--fast-flags") => {
                fast_flags_path = Some(PathBuf::from(
                    args.next().ok_or("--fast-flags needs a path")?,
                ));
            }
            Some("--client-settings") => {
                client_settings = Some(PathBuf::from(
                    args.next().ok_or("--client-settings needs a path")?,
                ));
            }
            Some(value) if value.starts_with('-') => {
                return Err(format!("unknown option: {value}").into());
            }
            _ if apk_arg.is_none() => apk_arg = Some(PathBuf::from(arg)),
            _ => return Err("only one base APK path may be supplied".into()),
        }
    }
    let fast_flags = if let Some(path) = fast_flags_path {
        serde_json::from_slice(&std::fs::read(path)?)?
    } else {
        serde_json::Value::Object(Default::default())
    };
    let apks = apk_arg
        .map(|base| {
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
        fast_flags,
        options: roblox_runtime::RuntimeOptions {
            client_settings,
            host_libc,
            ..Default::default()
        },
    };
    let system_dir = config.prepare_android_environment()?;
    let asset_dir = config.prepare_asset_tree()?;
    enter_run_directory(&config.data_dir, &asset_dir)?;
    println!("Android system files: {}", system_dir.display());
    println!("APK assets: {}", asset_dir.display());
    for apk in &config.apk_paths {
        println!("APK: {}", apk.display());
    }
    println!("Native libraries: {}", config.native_lib_dir.display());

    let event_loop = EventLoop::new()?;
    let mut app = app::ClientApp::new(config, asset_dir);
    event_loop.run_app(&mut app)?;
    if let Some(error) = app.take_failure() {
        return Err(error.into());
    }
    Ok(())
}

fn enter_run_directory(data_dir: &std::path::Path, asset_dir: &std::path::Path) -> std::io::Result<()> {
    // Android gives the engine a private working directory. Roblox resolves
    // appData, http, sounds, and ./exe/cacert.pem relative to this directory.
    let run_dir = data_dir.join("run");
    let exe_dir = run_dir.join("exe");
    std::fs::create_dir_all(&exe_dir)?;

    let ca_bundle = asset_dir.join("ssl/cacert.pem");
    if ca_bundle.is_file() {
        std::fs::copy(ca_bundle, exe_dir.join("cacert.pem"))?;
    } else {
        eprintln!("[runtime] APK CA bundle is missing: {}", ca_bundle.display());
    }

    let run_dir = std::fs::canonicalize(run_dir)?;
    std::env::set_current_dir(&run_dir)?;
    println!("Roblox working directory: {}", run_dir.display());
    Ok(())
}
