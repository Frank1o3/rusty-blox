mod client;

use std::path::PathBuf;

fn main() {
    if let Err(error) = run() {
        eprintln!("rusty-blox: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let apk = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .or_else(client::discover_sober_apk)
        .ok_or("no APK argument and no Sober x86-64 APK was found")?;
    let managed_dir = client::managed_install_dir().ok_or("HOME and XDG_DATA_HOME are unset")?;
    let imported = client::import_apk(&apk, &managed_dir)?;

    println!("APK: {}", imported.apk.display());
    println!("Native libraries: {}", imported.native_lib_dir.display());
    Ok(())
}
