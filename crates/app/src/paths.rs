//! Where things are located.

use std::path::PathBuf;

fn config_home() -> PathBuf {
    #[cfg(target_os = "windows")]
    return std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join("AppData/Roaming"));

    #[cfg(target_os = "macos")]
    return home().join("Library/Application Support");

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home().join(".config"))
}

fn home() -> PathBuf {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
}

pub fn config_dir() -> PathBuf {
    config_home().join("nolurix")
}

/// Only default storage migrates. Explicit bridge data directories never import
/// unrelated default data, even when the override happens to name this directory.
pub fn migrate_default() -> anyhow::Result<()> {
    config_for_override(None).map(|_| ())
}

pub fn config_for_override(explicit: Option<&std::path::Path>) -> anyhow::Result<PathBuf> {
    nolurix_store::migration::prepare_config_dir(
        &config_home().join("throne-gtk"),
        &config_dir(),
        explicit,
    )
}

pub fn database() -> PathBuf {
    config_dir().join("nolurix.db")
}

/// Directory for the Unix socket used to communicate with the core. Windows
/// uses a named pipe instead, but callers keep one platform-independent API.
pub fn runtime_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    return std::env::temp_dir().join("nolurix");

    #[cfg(not(target_os = "windows"))]
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(std::env::temp_dir)
        .join("nolurix")
}

/// The original Throne database — the import source.
pub fn throne_database() -> PathBuf {
    config_home().join("Throne/config/throne.db")
}

/// The core is looked up next to the executable: it checks that the parent
/// process is named `nolurix` and is in the same directory, otherwise it
/// simply refuses to run.
pub fn core_binary() -> PathBuf {
    if let Some(path) = std::env::var_os("NOLURIX_CORE") {
        return PathBuf::from(path);
    }
    std::env::current_exe()
        .ok()
        .and_then(|p| {
            p.parent()
                .map(|d| d.join(format!("nolurix-core{}", std::env::consts::EXE_SUFFIX)))
        })
        .unwrap_or_else(|| PathBuf::from(format!("nolurix-core{}", std::env::consts::EXE_SUFFIX)))
}
