//! Fetching a Roblox Android x86-64 build from a mirror, verifying it, and
//! handing it to the client's staged APK importer.
//!
//! Roblox publishes no public Android APK download, so `--update-apk` asks a
//! mirror (APKPure by default) for the newest build. A mirror serves bytes
//! nobody here can vouch for, so nothing is installed until
//! [`signature`] proves Roblox's own signing certificate signed the archive —
//! see `certificates.json` for what "Roblox's" means concretely.
//!
//! The pieces, in call order: [`source`] asks the mirror which build is
//! newest and downloads it, [`net`] owns the network policy (https only,
//! allow-listed hosts, redirects followed by hand), [`signature`] checks the
//! result, and the verified files go through `client::import_apks`.

mod net;
mod signature;
mod source;

use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

/// Where the engine lives inside an APK, for whatever host architecture the
/// client was built for.
pub(crate) const LIBRARY_IN_APK: &str = "lib/x86_64/libroblox.so";

pub(crate) const USER_AGENT: &str = concat!("rusty-blox/", env!("CARGO_PKG_VERSION"), " (Linux)");

/// Fetch the newest build, verify its signature, and publish it through
/// [`crate::client::import_apks`].
///
/// The download happens in a staging directory beside the managed install and
/// is removed again afterwards, whatever the outcome: `import_apks` is the
/// only thing that turns staged files into an installation, and until it says
/// otherwise there is no such thing as most of an update.
pub(crate) fn update(managed_dir: &Path) -> Result<String, UpdateError> {
    let parent = managed_dir
        .parent()
        .ok_or_else(|| UpdateError::Local("managed APK path has no parent".into()))?;
    fs::create_dir_all(parent)?;
    let staging = parent.join(format!(".roblox-apk-staging-{}", std::process::id()));
    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }
    fs::create_dir(&staging)?;

    let result = update_into(&staging, managed_dir);
    let _ = fs::remove_dir_all(&staging);
    result
}

fn update_into(staging: &Path, managed_dir: &Path) -> Result<String, UpdateError> {
    let fetch = source::ApkPure;
    let mut progress = |item| match item {
        source::Progress::Asking { endpoint } => {
            eprintln!("Roblox APK: checking {endpoint} for the latest x86-64 build")
        }
        source::Progress::Fetch { file, done, total } => {
            if let Some(total) = total {
                eprintln!("Roblox APK: {file}: {done}/{total} bytes")
            } else {
                eprintln!("Roblox APK: {file}: {done} bytes")
            }
        }
        source::Progress::Verify { file } => {
            eprintln!("Roblox APK: verifying {file}")
        }
    };

    let version = fetch
        .newest(&mut progress)
        .map_err(|e| UpdateError::Source(e.to_string()))?;
    eprintln!("Roblox APK: downloading version {}", version.name);
    let archives = fetch
        .fetch(&version, &net::Cancel::default(), staging, &mut progress)
        .map_err(|e| UpdateError::Source(e.to_string()))?;

    let trusted = signature::pinned();
    for path in archives.distinct() {
        progress(source::Progress::Verify {
            file: path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
        });
        signature::verify_signed_by(path, &trusted)
            .map_err(|error| UpdateError::Rejected(error.to_string()))?;
    }

    let base = staging.join("base.apk");
    let split = staging.join("split_config.x86_64.apk");
    let apks: Vec<PathBuf> = if archives.base == archives.split {
        fs::rename(&archives.base, &base)?;
        vec![base]
    } else {
        fs::rename(&archives.base, &base)?;
        fs::rename(&archives.split, &split)?;
        vec![base, split]
    };
    let imported = crate::client::import_apks(&apks, managed_dir)
        .map_err(|error| UpdateError::Local(error.to_string()))?;
    drop(imported);
    Ok(version.name)
}

/// Why an update did not happen, with the distinction that matters to a
/// person staring at the failure: the source failed, the bytes arrived but
/// were rejected, or something local went wrong.
#[derive(Debug)]
pub(crate) enum UpdateError {
    /// The mirror could not be reached, or answered unusably.
    Source(String),
    /// The download arrived intact but Roblox did not sign it.
    Rejected(String),
    /// A local step (staging, importing) failed.
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

impl From<std::io::Error> for UpdateError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}
