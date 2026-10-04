use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const URL: &str =
    "https://clientsettingscdn.roblox.com/v2/settings-compressed/application/GoogleAndroidApp.zst";
const MAX_AGE: Duration = Duration::from_secs(6 * 60 * 60);

pub(crate) fn load(explicit: Option<&Path>, cache_dir: &Path) -> Result<String, String> {
    if let Some(path) = explicit {
        let body = std::fs::read_to_string(path)
            .map_err(|error| format!("read client settings {}: {error}", path.display()))?;
        return validate(body, "--client-settings");
    }

    let cached = cache_path(cache_dir);
    if let Some(body) = read_fresh(&cached) {
        return validate(body, "client settings cache");
    }

    let fetched = (|| -> Result<String, String> {
        let config = ureq::config::Config::builder()
            .timeout_global(Some(Duration::from_secs(10)))
            .build();
        let client = config.new_agent();
        let response = client
            .get(URL)
            .call()
            .map_err(|error| format!("GET {URL}: {error}"))?;
        let bytes = response
            .into_body()
            .read_to_vec()
            .map_err(|error| format!("read client settings response: {error}"))?;
        let decoded = zstd::stream::decode_all(std::io::Cursor::new(bytes))
            .map_err(|error| format!("decompress client settings: {error}"))?;
        String::from_utf8(decoded)
            .map_err(|error| format!("client settings are not UTF-8: {error}"))
    })();

    match fetched {
        Ok(body) => {
            let body = validate(body, "Roblox settings CDN")?;
            if let Some(parent) = cached.parent() {
                if let Err(error) = std::fs::create_dir_all(parent) {
                    eprintln!("[runtime] could not create client settings cache: {error}");
                }
            }
            if let Err(error) = std::fs::write(&cached, &body) {
                eprintln!("[runtime] could not cache client settings: {error}");
            }
            Ok(body)
        }
        Err(error) => {
            if let Ok(body) = std::fs::read_to_string(&cached) {
                eprintln!("[runtime] client settings fetch failed ({error}); using stale cache");
                return validate(body, "stale client settings cache");
            }
            Err(format!("no usable client settings (fetch failed: {error})"))
        }
    }
}

fn cache_path(cache_dir: &Path) -> PathBuf {
    cache_dir.join("client-settings-google-android.json")
}

fn read_fresh(path: &Path) -> Option<String> {
    let metadata = std::fs::metadata(path).ok()?;
    let age = SystemTime::now()
        .duration_since(metadata.modified().ok()?)
        .ok()?;
    if age >= MAX_AGE {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

fn validate(body: String, source: &str) -> Result<String, String> {
    let value: serde_json::Value = serde_json::from_str(&body)
        .map_err(|error| format!("{source} is not valid JSON: {error}"))?;
    if !value
        .get("applicationSettings")
        .is_some_and(serde_json::Value::is_object)
    {
        return Err(format!("{source} has no applicationSettings object"));
    }
    eprintln!("[runtime] client settings: {} bytes ({source})", body.len());
    Ok(body)
}
