//! Запуск вместе с сеансом рабочего стола.
//!
//! Состояние — это наличие ярлыка в `~/.config/autostart`, а не строка в
//! базе: тот же каталог правят настройки рабочего стола, и второе мнение
//! рядом с ними рано или поздно с ними разошлось бы.

use std::path::Path;

use anyhow::{bail, Context, Result};
use std::io::Write;

use crate::paths;

/// Запускается ли программа вместе с сеансом.
pub fn is_enabled() -> bool {
    paths::autostart_file().exists()
}

/// Кладёт ярлык автозапуска или убирает его.
pub fn set_enabled(enabled: bool) -> Result<()> {
    let path = paths::autostart_file();
    let exec = std::env::current_exe().context("не удалось определить путь к программе")?;
    set_file_enabled(&path, &exec, enabled)
}

fn set_file_enabled(path: &Path, exec: &Path, enabled: bool) -> Result<()> {
    let expected = entry(exec);
    if !enabled {
        match std::fs::symlink_metadata(path) {
            Ok(meta) if meta.file_type().is_symlink() || !meta.is_file() => {
                bail!(
                    "Refusing to remove a non-regular autostart entry: {}",
                    path.display()
                );
            }
            Ok(_) if std::fs::read_to_string(path)? != expected => {
                bail!(
                    "Autostart entry was customized; disable it manually: {}",
                    path.display()
                );
            }
            Ok(_) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.into()),
        }
        return match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            // Ярлыка и так нет — например, его убрали настройками рабочего
            // стола. Просить об этом ещё раз не о чем.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e).with_context(|| format!("не удалось убрать {}", path.display())),
        };
    }

    let dir = path.parent().expect("у ярлыка есть каталог");
    refuse_legacy_entries(dir)?;
    std::fs::create_dir_all(dir)
        .with_context(|| format!("не удалось создать {}", dir.display()))?;
    let mut file = match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let meta = std::fs::symlink_metadata(path)?;
            if !meta.is_file()
                || meta.file_type().is_symlink()
                || std::fs::read_to_string(path)? != expected
            {
                bail!(
                    "Refusing to overwrite an existing autostart entry: {}",
                    path.display()
                );
            }
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };
    file.write_all(expected.as_bytes())?;
    file.sync_all()?;
    Ok(())
}

/// Never guess whether an old file belongs to us, or remove customized entries.
/// The desktop's Startup Applications settings can disable the old entry first.
fn refuse_legacy_entries(dir: &Path) -> Result<()> {
    for name in ["dev.nalart.ThroneGtk.desktop", "throne-gtk.desktop"] {
        let old = dir.join(name);
        match std::fs::symlink_metadata(&old) {
            Ok(_) => bail!("Disable the former Throne GTK entry in Startup Applications, then remove or move its autostart file before enabling Nolurix: {}. The old file has not been changed.", old.display()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

fn entry(exec: &Path) -> String {
    let id = crate::APP_ID;
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Nolurix\n\
         Comment=Клиент прокси на ядре sing-box и Xray\n\
         Exec={exec}\n\
         Icon={id}\n\
         Terminal=false\n\
         Categories=Network;\n\
         StartupNotify=false\n\
         StartupWMClass={id}\n\
         X-GNOME-Autostart-enabled=true\n",
        exec = quote(exec),
    )
}

/// `Exec` разбирается как командная строка, поэтому путь с пробелом или
/// служебным знаком обязан быть в кавычках — иначе рабочий стол прочтёт его
/// как команду с аргументами.
fn quote(exec: &Path) -> String {
    let raw = exec.to_string_lossy();
    let reserved = |c: char| c.is_whitespace() || "\"'\\<>~|&;$*?#()`".contains(c);
    if !raw.contains(reserved) {
        return raw.into_owned();
    }
    let escaped = raw.replace('\\', r"\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_path_goes_without_quotes() {
        assert_eq!(
            quote(Path::new("/home/u/.local/lib/nolurix/nolurix")),
            "/home/u/.local/lib/nolurix/nolurix"
        );
    }

    #[test]
    fn path_with_space_gets_quoted() {
        assert_eq!(
            quote(Path::new("/home/Иван Петров/bin/nolurix")),
            "\"/home/Иван Петров/bin/nolurix\""
        );
    }

    #[test]
    fn quotes_inside_path_are_escaped() {
        assert_eq!(
            quote(Path::new(r#"/home/a"b/nolurix"#)),
            r#""/home/a\"b/nolurix""#
        );
    }

    /// Полный круг по настоящей файловой системе: включили — ярлык на месте
    /// и запускает то, что нужно; выключили — ярлыка нет; выключили дважды —
    /// по-прежнему нет и никто не ругается.
    #[test]
    fn round_trip_over_config_home() {
        let dir = std::env::temp_dir().join(format!("nolurix-autostart-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::env::set_var("XDG_CONFIG_HOME", &dir);

        assert!(!is_enabled());

        set_enabled(true).unwrap();
        assert!(is_enabled());
        let text = std::fs::read_to_string(paths::autostart_file()).unwrap();
        let exec = std::env::current_exe().unwrap();
        assert!(text.contains(&format!("Exec={}", quote(&exec))));

        set_enabled(false).unwrap();
        assert!(!is_enabled());
        set_enabled(false).unwrap();

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn entry_carries_exec_and_id() {
        let text = entry(Path::new("/opt/nolurix"));
        assert!(text.starts_with("[Desktop Entry]\n"));
        assert!(text.contains("\nExec=/opt/nolurix\n"));
        assert!(text.contains(&format!("\nIcon={}\n", crate::APP_ID)));
        assert!(text.ends_with('\n'));
    }

    #[test]
    fn legacy_entry_blocks_new_without_changing_user_files() {
        let dir =
            std::env::temp_dir().join(format!("nolurix-autostart-legacy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let old = dir.join("dev.nalart.ThroneGtk.desktop");
        let new = dir.join("dev.nalart.Nolurix.desktop");
        std::fs::write(&old, "user-customized legacy entry").unwrap();
        let error = set_file_enabled(&new, Path::new("/opt/nolurix"), true).unwrap_err();
        assert!(error.to_string().contains("Startup Applications"));
        assert!(!new.exists());
        assert_eq!(
            std::fs::read_to_string(&old).unwrap(),
            "user-customized legacy entry"
        );
        std::fs::remove_file(&old).unwrap();
        std::fs::write(&new, "user-customized new entry").unwrap();
        assert!(set_file_enabled(&new, Path::new("/opt/nolurix"), true).is_err());
        assert!(set_file_enabled(&new, Path::new("/opt/nolurix"), false).is_err());
        assert_eq!(
            std::fs::read_to_string(&new).unwrap(),
            "user-customized new entry"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
