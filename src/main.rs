mod app;
mod client;
mod client_settings;
mod desktop;
mod host_window;
mod settings;
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
    let mut settings_mode = false;
    let mut session_name: Option<String> = None;
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--help" | "-h") => {
                println!(
                    "Usage: rusty-blox [--settings] [--session NAME|--add-session NAME] [--host-libc] [--fast-flags FILE] [--client-settings FILE] [APK]\n\
                     Without APK, the client looks for Sober's x86-64 installation.\n\
                     Without --session, Roblox starts without a saved login. --add-session NAME creates/selects a named login profile.\n\
                     --host-libc enables the runtime's ABI-unsafe diagnostic resolver."
                );
                return Ok(());
            }
            Some("--host-libc") => host_libc = true,
            Some("--settings") => settings_mode = true,
            Some("--session") | Some("--add-session") => {
                session_name = Some(
                    args.next()
                        .ok_or("--session needs a name")?
                        .to_string_lossy()
                        .into_owned(),
                );
            }
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
    if settings_mode {
        return settings::run_ui().map_err(Into::into);
    }
    let lock_root = client::managed_install_dir().ok_or("HOME and XDG_DATA_HOME are unset")?;
    std::fs::create_dir_all(&lock_root)?;
    let _instance_lock = InstanceLock::acquire(&lock_root.join("roblox-instance.lock"))?;
    let user_settings = settings::load();
    let session_name = session_name.or_else(|| user_settings.session.clone());
    let fast_flags = if let Some(path) = fast_flags_path {
        serde_json::from_slice(&std::fs::read(path)?)?
    } else {
        serde_json::Value::Object(Default::default())
    };
    let mut fast_flags = fast_flags;
    if let Some(limit) = user_settings.fps_limit {
        let flags = fast_flags
            .as_object_mut()
            .ok_or("Fast Flags document must be a JSON object")?;
        flags
            .entry("DFIntTaskSchedulerTargetFps")
            .or_insert_with(|| limit.to_string().into());
        if limit > 240 {
            flags
                .entry("FFlagTaskSchedulerLimitTargetFpsTo2402")
                .or_insert_with(|| "False".into());
        }
    }
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
            graphics_backend: match user_settings.renderer.as_str() {
                "vulkan" => roblox_runtime::graphics::BackendPreference::Vulkan,
                "opengl" => roblox_runtime::graphics::BackendPreference::OpenGlEs,
                _ => roblox_runtime::graphics::BackendPreference::Automatic,
            },
            present_mode: Some(if user_settings.vsync {
                user_settings.present_mode.clone()
            } else {
                "immediate".into()
            }),
            vsync: user_settings.vsync,
            host_libc,
            ..Default::default()
        },
        session: session_name
            .as_deref()
            .map(|name| {
                roblox_runtime::session::Session::open(&client_root.join("sessions"), name)
                    .map_err(std::io::Error::other)
            })
            .transpose()?,
    };
    let system_dir = config.prepare_android_environment()?;
    let asset_dir = config.prepare_asset_tree()?;
    let run_dir = config.prepare_engine_working_directory(&asset_dir)?;
    println!("Roblox working directory: {}", run_dir.display());
    println!("Android system files: {}", system_dir.display());
    println!("APK assets: {}", asset_dir.display());
    for apk in &config.apk_paths {
        println!("APK: {}", apk.display());
    }
    println!("Native libraries: {}", config.native_lib_dir.display());

    let event_loop = EventLoop::new()?;
    let mut app = app::ClientApp::new(config, asset_dir, user_settings);
    event_loop.run_app(&mut app)?;
    if let Some(error) = app.take_failure() {
        return Err(error.into());
    }
    Ok(())
}

struct InstanceLock(std::path::PathBuf);
impl InstanceLock {
    fn acquire(path: &std::path::Path) -> Result<Self, Box<dyn std::error::Error>> {
        use std::io::Write;
        let mut file = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let pid = std::fs::read_to_string(path)
                    .ok()
                    .and_then(|s| s.trim().parse::<u32>().ok());
                if pid.is_some_and(|pid| !std::path::Path::new(&format!("/proc/{pid}")).exists()) {
                    let _ = std::fs::remove_file(path);
                    return Self::acquire(path);
                }
                return Err("a rusty-blox Roblox instance is already running".into());
            }
            Err(error) => return Err(format!("create Roblox instance lock: {error}").into()),
        };
        writeln!(file, "{}", std::process::id())?;
        Ok(Self(path.to_path_buf()))
    }
}
impl Drop for InstanceLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
