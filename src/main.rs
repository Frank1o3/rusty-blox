macro_rules! eprintln {
    () => {{
        roblox_logging::emit(String::new());
    }};
    ($($arg:tt)*) => {{
        roblox_logging::emit(format!($($arg)*));
    }};
}

mod app;
mod client;
mod client_settings;
mod desktop;
mod host_window;
mod settings;
mod startup;
mod text_overlay;
#[cfg(feature = "webview")]
mod webview;

use std::path::PathBuf;
use winit::event_loop::EventLoop;

fn main() {
    if let Err(error) = run() {
        roblox_logging::emit_error(format!("rusty-blox: {error}"));
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
    let log_level = user_settings.log_level.clamp(1, 4);
    unsafe {
        std::env::set_var("RUSTY_BLOX_LOG_LEVEL", log_level.to_string());
    }
    let env_requests_rust = std::env::var("USE_EXPERIMENTAL_JNIVM")
        .is_ok_and(|value| matches!(value.trim().to_ascii_lowercase().as_str(), "true" | "1"));
    let use_rust_jnivm = user_settings.rust_jnivm || env_requests_rust;
    // The JNI backend is selected during native startup, before the client
    // creates worker threads. Keep the existing environment opt-in as a
    // developer override (dev.sh uses it) while making the saved UI choice the
    // normal launch path.
    unsafe {
        std::env::set_var(
            "USE_EXPERIMENTAL_JNIVM",
            if use_rust_jnivm { "1" } else { "0" },
        );
    }
    roblox_runtime::set_jnivm_cpp_fallback(user_settings.jnivm_cpp_fallback);
    let session_name = session_name.or_else(|| user_settings.session.clone());
    let fast_flags_path = fast_flags_path.unwrap_or_else(settings::fast_flags_path);
    if !fast_flags_path.exists() {
        let parent = fast_flags_path
            .parent()
            .ok_or("FastFlags path has no parent")?;
        std::fs::create_dir_all(parent)?;
        std::fs::write(&fast_flags_path, "{}\n")?;
    }
    let mut fast_flags: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&fast_flags_path)?)?;
    if let Some(limit) = settings::configured_frame_cap(&user_settings) {
        let flags = fast_flags
            .as_object_mut()
            .ok_or("Fast Flags document must be a JSON object")?;
        if limit == 0 {
            flags.remove("DFIntTaskSchedulerTargetFps");
            flags.remove("FFlagTaskSchedulerLimitTargetFpsTo2402");
        } else {
            flags.insert(
                "DFIntTaskSchedulerTargetFps".to_owned(),
                limit.to_string().into(),
            );
            if limit > 240 {
                flags.insert(
                    "FFlagTaskSchedulerLimitTargetFpsTo2402".to_owned(),
                    "False".into(),
                );
            } else {
                flags.remove("FFlagTaskSchedulerLimitTargetFpsTo2402");
            }
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
    client::migrate_legacy_sessions(&managed_dir)?;
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
            opengl_swap_interval: match user_settings.gl_swap_interval.as_str() {
                "off" => 0,
                "adaptive" => -1,
                _ => 1,
            },
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

struct InstanceLock {
    _file: std::fs::File,
}
impl InstanceLock {
    fn acquire(path: &std::path::Path) -> Result<Self, Box<dyn std::error::Error>> {
        use std::io::{Read, Seek, SeekFrom, Write};
        use std::os::fd::AsRawFd;

        // Keep the inode in place and let the kernel own the lock. Removing a
        // PID file on shutdown races with another launcher creating a fresh
        // one, while a crash between create_new and write leaves an empty file
        // that the old PID-only check misread as a live instance forever.
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(path)
            .map_err(|error| format!("open instance lock: {error}"))?;
        // SAFETY: `as_raw_fd` is a live descriptor owned by `file`; flock does
        // not take ownership and the descriptor remains open for the lock's
        // lifetime.
        let locked = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if locked != 0 {
            let error = std::io::Error::last_os_error();
            if error
                .raw_os_error()
                .is_some_and(|code| code == libc::EWOULDBLOCK || code == libc::EAGAIN)
            {
                return Err("a rusty-blox Roblox instance is already running".into());
            }
            return Err(format!("lock instance file: {error}").into());
        }

        // Honor a PID marker left by a build that predates flock, but only
        // when /proc confirms that PID is actually rusty-blox. The old code
        // used the PID file as its lock, so it may still be running while this
        // build is upgraded.
        let mut previous = String::new();
        file.read_to_string(&mut previous)?;
        if previous
            .trim()
            .parse::<u32>()
            .is_ok_and(legacy_rusty_blox_process_is_live)
        {
            return Err("a rusty-blox Roblox instance is already running".into());
        }

        file.set_len(0)?;
        file.seek(SeekFrom::Start(0))?;
        writeln!(file, "{}", std::process::id())?;
        file.sync_data()?;
        Ok(Self { _file: file })
    }
}

fn legacy_rusty_blox_process_is_live(pid: u32) -> bool {
    let Ok(command) = std::fs::read(format!("/proc/{pid}/cmdline")) else {
        return false;
    };
    let Some(executable) = command
        .split(|byte| *byte == 0)
        .next()
        .filter(|arg| !arg.is_empty())
    else {
        return false;
    };
    executable == b"rusty-blox" || executable.ends_with(b"/rusty-blox")
}
