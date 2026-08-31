//! Where things are located.

use std::path::PathBuf;

fn config_home() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home().join(".config"))
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
}

pub fn config_dir() -> PathBuf {
    config_home().join("throne-gtk")
}

pub fn database() -> PathBuf {
    config_dir().join("throne-gtk.db")
}

/// Directory for the socket used to communicate with the core. It lives in the
/// user’s runtime directory: the socket must not survive a reboot or be visible
/// to other users.
pub fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(std::env::temp_dir)
        .join("throne-gtk")
}

/// The original Throne database — the import source.
pub fn throne_database() -> PathBuf {
    config_home().join("Throne/config/throne.db")
}

/// The core is looked up next to the executable: it checks that the parent
/// process is named `throne-gtk` and is in the same directory, otherwise it
/// simply refuses to run.
pub fn core_binary() -> PathBuf {
    if let Some(path) = std::env::var_os("THRONE_GTK_CORE") {
        return PathBuf::from(path);
    }
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("throne-gtk-core")))
        .unwrap_or_else(|| PathBuf::from("throne-gtk-core"))
}
