//! Где что лежит.

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

/// Каталог для сокета связи с ядром. Живёт в runtime-каталоге пользователя:
/// сокет не должен переживать перезагрузку и не должен быть виден другим
/// пользователям.
pub fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(std::env::temp_dir)
        .join("throne-gtk")
}

/// База оригинального Throne — источник для импорта.
pub fn throne_database() -> PathBuf {
    config_home().join("Throne/config/throne.db")
}

/// Ядро ищется рядом с исполняемым файлом: оно проверяет, что родительский
/// процесс называется `throne-gtk` и лежит в той же папке, поэтому иначе
/// просто откажется работать.
pub fn core_binary() -> PathBuf {
    if let Some(path) = std::env::var_os("THRONE_GTK_CORE") {
        return PathBuf::from(path);
    }
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("throne-gtk-core")))
        .unwrap_or_else(|| PathBuf::from("throne-gtk-core"))
}
