use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const URL: &str =
    "https://clientsettingscdn.roblox.com/v2/settings-compressed/application/GoogleAndroidApp.zst";
const MAX_AGE: Duration = Duration::from_secs(6 * 60 * 60);

pub(crate) fn load(
    explicit: Option<&Path>,
    cache_dir: &Path,
    fast_flags: &serde_json::Value,
) -> Result<String, String> {
    let base = if let Some(path) = explicit {
        let body = std::fs::read_to_string(path)
            .map_err(|error| format!("read client settings {}: {error}", path.display()))?;
        validate(body, "--client-settings")?
    } else {
        let cached = cache_path(cache_dir);
        if let Some(body) = read_fresh(&cached) {
            validate(body, "client settings cache")?
        } else {
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
                    body
                }
                Err(error) => {
                    if let Ok(body) = std::fs::read_to_string(&cached) {
                        eprintln!(
                            "[runtime] client settings fetch failed ({error}); using stale cache"
                        );
                        validate(body, "stale client settings cache")?
                    } else {
                        return Err(format!("no usable client settings (fetch failed: {error})"));
                    }
                }
            }
        }
    };

    merge_fast_flags(base, fast_flags)
}

/// Put host FastFlags into the same applicationSettings document Roblox reads.
/// `nativePreloadFlagOverrides` is still called as a matching Android startup
/// hook, but Cordial's investigation found its accepted JSON shape unknown.
fn merge_fast_flags(body: String, fast_flags: &serde_json::Value) -> Result<String, String> {
    let Some(overrides) = fast_flags.as_object() else {
        return Err("FastFlags document must be a JSON object".to_owned());
    };
    if overrides.is_empty() {
        return Ok(body);
    }

    let mut document: serde_json::Value = serde_json::from_str(&body)
        .map_err(|error| format!("parse client settings before merging FastFlags: {error}"))?;
    let settings = document
        .get_mut("applicationSettings")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("client settings have no applicationSettings object")?;
    for (name, value) in overrides {
        let value = match value {
            serde_json::Value::String(value) => value.clone(),
            serde_json::Value::Bool(value) => {
                if *value {
                    "True".to_owned()
                } else {
                    "False".to_owned()
                }
            }
            serde_json::Value::Number(value) => value.to_string(),
            serde_json::Value::Null
            | serde_json::Value::Array(_)
            | serde_json::Value::Object(_) => {
                return Err(format!("FastFlag {name} must have a scalar value"));
            }
        };
        settings.insert(name.clone(), value.into());
    }
    eprintln!(
        "[runtime] merged {} FastFlags into applicationSettings",
        overrides.len()
    );
    serde_json::to_string(&document)
        .map_err(|error| format!("serialize client settings with FastFlags: {error}"))
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
