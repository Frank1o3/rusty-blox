//! Client-owned Roblox installation discovery and import.

use std::env;
use std::error::Error;
use std::fmt;
use std::fs::{self, File};
use std::io;
use std::path::{Component, Path, PathBuf};

const SOBER_APK_RELATIVE: &str =
    ".var/app/org.vinegarhq.Sober/data/sober/packages/x86_64/com.roblox.client/base.apk";
/// Runtime inputs imported into the client's managed data directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportedClient {
    pub apk: PathBuf,
    pub native_lib_dir: PathBuf,
}

/// Find the x86-64 APK managed by the Sober Flatpak for the current user.
pub fn discover_sober_apk() -> Option<PathBuf> {
    let path = env::var_os("HOME")
        .map(PathBuf::from)?
        .join(SOBER_APK_RELATIVE);
    path.is_file().then_some(path)
}

/// Choose the client's persistent installation directory.
pub fn managed_install_dir() -> Option<PathBuf> {
    if let Some(data_home) = env::var_os("XDG_DATA_HOME") {
        return Some(PathBuf::from(data_home).join("rusty-blox/roblox"));
    }
    env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share/rusty-blox/roblox"))
}

/// Import an APK and its x86-64 shared libraries into a client-owned location.
///
/// The import is assembled in a sibling staging directory and only published
/// after the APK and `libroblox.so` have both been written successfully.
pub fn import_apk(source: &Path, managed_dir: &Path) -> Result<ImportedClient, ImportError> {
    if !source.is_file() {
        return Err(ImportError::MissingApk(source.to_path_buf()));
    }

    let parent = managed_dir
        .parent()
        .ok_or_else(|| ImportError::InvalidManagedPath(managed_dir.to_path_buf()))?;
    fs::create_dir_all(parent)?;

    let staging = staging_path(managed_dir);
    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }
    fs::create_dir(&staging)?;

    let result = import_to_staging(source, &staging);
    if let Err(error) = result {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }

    publish(&staging, managed_dir)?;
    Ok(ImportedClient {
        apk: managed_dir.join("base.apk"),
        native_lib_dir: managed_dir.join("lib/x86_64"),
    })
}

fn import_to_staging(source: &Path, staging: &Path) -> Result<(), ImportError> {
    fs::copy(source, staging.join("base.apk"))?;
    let apk = File::open(source)?;
    let mut archive = zip::ZipArchive::new(apk)?;
    let native_dir = staging.join("lib/x86_64");
    fs::create_dir_all(&native_dir)?;

    let mut found_roblox = false;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let Some(name) = entry.enclosed_name() else {
            continue;
        };
        let Some(file_name) = native_library_name(&name) else {
            continue;
        };

        let destination = native_dir.join(file_name);
        let mut output = File::create(destination)?;
        io::copy(&mut entry, &mut output)?;
        found_roblox |= file_name == "libroblox.so";
    }

    if !found_roblox {
        return Err(ImportError::RobloxLibraryMissing);
    }
    Ok(())
}

fn native_library_name(path: &Path) -> Option<&str> {
    let mut components = path.components();
    if components.next()? != Component::Normal("lib".as_ref())
        || components.next()? != Component::Normal("x86_64".as_ref())
    {
        return None;
    }
    let Component::Normal(file_name) = components.next()? else {
        return None;
    };
    if components.next().is_some() || !file_name.to_string_lossy().ends_with(".so") {
        return None;
    }
    file_name.to_str()
}

fn staging_path(managed_dir: &Path) -> PathBuf {
    let name = managed_dir
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    managed_dir.with_file_name(format!(".{name}.staging-{}", std::process::id()))
}

fn publish(staging: &Path, managed_dir: &Path) -> Result<(), ImportError> {
    let backup = managed_dir.with_extension(format!("previous-{}", std::process::id()));
    if backup.exists() {
        fs::remove_dir_all(&backup)?;
    }
    let had_previous = managed_dir.exists();
    if had_previous {
        fs::rename(managed_dir, &backup)?;
    }
    if let Err(error) = fs::rename(staging, managed_dir) {
        if had_previous {
            let _ = fs::rename(&backup, managed_dir);
        }
        return Err(error.into());
    }
    if had_previous {
        fs::remove_dir_all(backup)?;
    }
    Ok(())
}

/// Errors encountered while preparing a managed Roblox installation.
#[derive(Debug)]
pub enum ImportError {
    Io(io::Error),
    Zip(zip::result::ZipError),
    MissingApk(PathBuf),
    InvalidManagedPath(PathBuf),
    RobloxLibraryMissing,
}

impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "I/O error importing Roblox: {error}"),
            Self::Zip(error) => write!(f, "invalid Roblox APK archive: {error}"),
            Self::MissingApk(path) => write!(f, "Roblox APK does not exist: {}", path.display()),
            Self::InvalidManagedPath(path) => {
                write!(
                    f,
                    "managed installation path has no parent: {}",
                    path.display()
                )
            }
            Self::RobloxLibraryMissing => {
                f.write_str("APK does not contain lib/x86_64/libroblox.so")
            }
        }
    }
}

impl Error for ImportError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Zip(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for ImportError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<zip::result::ZipError> for ImportError {
    fn from(value: zip::result::ZipError) -> Self {
        Self::Zip(value)
    }
}
