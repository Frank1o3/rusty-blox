//! Persist Roblox's in-memory cookie jar so a signed-in session survives exit.
//! The file is private (0600) and must be treated like a password.

use std::path::Path;
use std::time::{Duration, Instant};

const SETTINGS: &str = "com/roblox/engine/jni/NativeSettingsInterface";
const DOMAINS: [&str; 4] = [
    "roblox.com",
    ".roblox.com",
    "apis.roblox.com",
    "auth.roblox.com",
];

pub(crate) fn restore(
    engine: &roblox_runtime::LoadedEngine,
    config: &roblox_runtime::RuntimeConfig,
) -> Result<(), String> {
    let path = config.data_dir.join("roblox-cookies");
    let Ok(contents) = std::fs::read_to_string(&path) else {
        return Ok(());
    };
    let Some(native) = engine
        .symbol("Java_com_roblox_engine_jni_NativeSettingsInterface_nativeSetMultipleCookies")
    else {
        return Ok(());
    };
    let mut restored = 0;
    for line in contents.lines().filter(|line| !line.starts_with('#')) {
        let Some((domain, cookies)) = line.split_once('\t') else {
            continue;
        };
        let cookies = unescape(cookies);
        if cookies.is_empty() {
            continue;
        }
        // SAFETY: this is the live static settings native and both strings stay alive.
        unsafe {
            roblox_runtime::jni::game_activity::call_static_strings(
                native,
                SETTINGS,
                &[domain, &cookies],
            )
        }
        .map_err(|e| format!("restore Roblox session cookies: {e}"))?;
        restored += 1;
    }
    if restored > 0 {
        eprintln!("[session] restored cookies for {restored} Roblox domains");
    }
    Ok(())
}

fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some(ch) => {
                out.push('\\');
                out.push(ch);
            }
            None => out.push('\\'),
        }
    }
    out
}

pub(crate) fn flush_if_due(engine: &roblox_runtime::LoadedEngine, data_dir: &Path) {
    static LAST: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);
    let due = LAST
        .lock()
        .map(|mut last| {
            let now = Instant::now();
            if last.is_some_and(|at| now.duration_since(at) < Duration::from_secs(30)) {
                false
            } else {
                *last = Some(now);
                true
            }
        })
        .unwrap_or(false);
    if due {
        let _ = save(engine, data_dir);
    }
}

pub(crate) fn save(engine: &roblox_runtime::LoadedEngine, data_dir: &Path) -> Result<(), String> {
    let Some(native) = engine
        .symbol("Java_com_roblox_engine_jni_NativeSettingsInterface_nativeGetCookiesForDomain")
    else {
        return Ok(());
    };
    let mut output = String::from("# rusty-blox session cookie store; treat as a password\n");
    for domain in DOMAINS {
        // SAFETY: this is the live static settings native and the domain is valid UTF-8.
        let jar = unsafe {
            roblox_runtime::jni::game_activity::cookies_for_domain(native, SETTINGS, domain)
        }
        .map_err(|e| format!("read Roblox session cookies: {e}"))?;
        let cookies = jar
            .expose()
            .split("; ")
            .filter_map(|record| {
                let fields: Vec<_> = record.split('\t').collect();
                if fields.len() < 7 {
                    return None;
                }
                let name = fields[fields.len() - 2];
                let value = fields[fields.len() - 1];
                (!name.is_empty()).then(|| format!("{name}={value}"))
            })
            .collect::<Vec<_>>()
            .join("; ");
        if !cookies.is_empty() {
            output.push_str(domain);
            output.push('\t');
            output.push_str(
                &cookies
                    .replace('\\', "\\\\")
                    .replace('\n', "\\n")
                    .replace('\t', "\\t"),
            );
            output.push('\n');
        }
    }
    if output.ends_with("password\n") {
        return Ok(());
    }
    let path = data_dir.join("roblox-cookies");
    let temp = data_dir.join("roblox-cookies.tmp");
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temp).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
    }
    file.write_all(output.as_bytes())
        .map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    std::fs::rename(temp, path).map_err(|e| e.to_string())?;
    Ok(())
}
