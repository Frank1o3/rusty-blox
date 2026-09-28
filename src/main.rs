mod client;

use std::path::PathBuf;

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

    let runtime_config = roblox_runtime::RuntimeConfig {
        apk_paths: imported.apk_paths.clone(),
        native_lib_dir: imported.native_lib_dir.clone(),
        data_dir,
        cache_dir,
        fast_flags: Default::default(),
        options: roblox_runtime::RuntimeOptions::default(),
    };
    let system_dir = runtime_config.prepare_android_environment()?;
    let asset_dir = runtime_config.prepare_asset_tree()?;
    let imports = runtime_config.engine_imports()?;
    let engine = runtime_config.load_engine()?;
    let strong_imports = imports
        .values()
        .filter(|binding: &&roblox_runtime::ImportBinding| {
            **binding == roblox_runtime::ImportBinding::Strong
        })
        .count();

    for apk in &runtime_config.apk_paths {
        println!("APK: {}", apk.display());
    }
    println!(
        "Native libraries: {}",
        runtime_config.native_lib_dir.display()
    );
    println!("Android system files: {}", system_dir.display());
    println!("APK assets: {}", asset_dir.display());
    println!(
        "Engine imports: {} required, {} optional",
        strong_imports,
        imports.len() - strong_imports
    );
    let (code_base, code_size) = engine.code_region();
    println!(
        "Engine mapped (constructors deferred): base=0x{:x}, executable={code_size} bytes at 0x{code_base:x}",
        engine.base()
    );
    Ok(())
}
