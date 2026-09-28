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

    for apk in &imported.apk_paths {
        println!("APK: {}", apk.display());
    }
    println!("Native libraries: {}", imported.native_lib_dir.display());
    Ok(())
}
