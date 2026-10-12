//! Download and verify Roblox's Android x86-64 APK before installing it.
//!
//! The downloader and APK signature verifier are adapted from Cordial's
//! `cordial-update` crate (GPL-3.0-or-later), commit
//! `7abbaf5d5f6cc81ca97e448812985d3f180b2420`. Roblox does not expose a public
//! Android APK download URL, so the update source is APKPure. APKs are accepted
//! only when their APK Signature Scheme v2/v3 certificate matches Roblox's
//! pinned certificate. A mirror-provided checksum is never treated as proof.

mod apk_signature;
mod provider;
mod url_policy;

use std::error::Error;
use std::fmt;
use std::fs;
use std::path::Path;

use provider::{ApkPure, Cancel, Provider};

const LIBRARY_IN_APK: &str = "lib/x86_64/libroblox.so";
const USER_AGENT: &str = concat!("rusty-blox/", env!("CARGO_PKG_VERSION"), " (Linux)");

#[derive(Debug)]
enum Unreachable {
    Transport {
        url: String,
        why: String,
    },
    Status {
        url: String,
        status: u16,
        body: String,
    },
    Malformed {
        url: String,
        why: String,
    },
    Cancelled,
    NoSource {
        why: String,
    },
}

impl fmt::Display for Unreachable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport { url, why } => write!(f, "could not reach {url}: {why}"),
            Self::Status { url, status, body } if body.trim().is_empty() => {
                write!(f, "{url} answered HTTP {status}")
            }
            Self::Status { url, status, body } => {
                write!(f, "{url} answered HTTP {status}: {}", body.trim())
            }
            Self::Malformed { url, why } => write!(f, "{url} returned unusable data: {why}"),
            Self::Cancelled => f.write_str("APK download was cancelled"),
            Self::NoSource { why } => f.write_str(why),
        }
    }
}

impl Error for Unreachable {}

/// Fetch the newest x86-64 build, verify its Roblox signature, and publish it
/// through the client's existing staged APK importer.
pub(crate) fn update(managed_dir: &Path) -> Result<String, UpdateError> {
    let parent = managed_dir
        .parent()
        .ok_or_else(|| UpdateError::Local("managed APK path has no parent".into()))?;
    fs::create_dir_all(parent)?;
    let staging = parent.join(format!(".roblox-apk-update-{}", std::process::id()));
    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }
    fs::create_dir(&staging)?;

    let result = update_into(&staging, managed_dir);
    let _ = fs::remove_dir_all(&staging);
    result
}

fn update_into(staging: &Path, managed_dir: &Path) -> Result<String, UpdateError> {
    let provider = ApkPure::default();
    let mut progress = |item| match item {
        provider::Progress::Asking { provider } => {
            eprintln!("Roblox APK: checking {provider} for the latest x86-64 build")
        }
        provider::Progress::Fetching { file, done, total } => {
            if let Some(total) = total {
                eprintln!("Roblox APK: {file}: {done}/{total} bytes")
            } else {
                eprintln!("Roblox APK: {file}: {done} bytes")
            }
        }
        provider::Progress::Verifying { file } => {
            eprintln!("Roblox APK: verifying {file}")
        }
    };
    let version = provider.newest(&mut progress)?;
    eprintln!("Roblox APK: downloading version {}", version.name);
    let archives = provider.fetch(&version, &Cancel::new(), staging, &mut progress)?;

    let trusted = apk_signature::pinned();
    for path in archives.distinct() {
        progress(provider::Progress::Verifying {
            file: path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
        });
        apk_signature::verify_signed_by(path, &trusted)
            .map_err(|error| UpdateError::Rejected(error.to_string()))?;
    }

    let base = staging.join("base.apk");
    let split = staging.join("split_config.x86_64.apk");
    if archives.base == archives.split {
        fs::rename(&archives.base, &base)?;
        let imported = crate::client::import_apks(&[base], managed_dir)
            .map_err(|error| UpdateError::Local(error.to_string()))?;
        drop(imported);
    } else {
        fs::rename(&archives.base, &base)?;
        fs::rename(&archives.split, &split)?;
        let imported = crate::client::import_apks(&[base, split], managed_dir)
            .map_err(|error| UpdateError::Local(error.to_string()))?;
        drop(imported);
    }
    Ok(version.name)
}

#[derive(Debug)]
pub(crate) enum UpdateError {
    Source(String),
    Rejected(String),
    Local(String),
    Io(std::io::Error),
}

impl fmt::Display for UpdateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Source(error) => write!(f, "Roblox APK source failed: {error}"),
            Self::Rejected(error) => write!(f, "Roblox APK was rejected: {error}"),
            Self::Local(error) => f.write_str(error),
            Self::Io(error) => write!(f, "Roblox APK update failed: {error}"),
        }
    }
}

impl Error for UpdateError {}

impl From<Unreachable> for UpdateError {
    fn from(value: Unreachable) -> Self {
        Self::Source(value.to_string())
    }
}

impl From<std::io::Error> for UpdateError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}
