//! Запуск вместе с сеансом рабочего стола.
//!
//! Состояние — это наличие ярлыка в `~/.config/autostart`, а не строка в
//! базе: тот же каталог правят настройки рабочего стола, и второе мнение
//! рядом с ними рано или поздно с ними разошлось бы.

use std::path::Path;

use anyhow::{Context, Result};

use crate::paths;

/// Запускается ли программа вместе с сеансом.
pub fn is_enabled() -> bool {
    paths::autostart_file().exists()
}

/// Кладёт ярлык автозапуска или убирает его.
pub fn set_enabled(enabled: bool) -> Result<()> {
    let path = paths::autostart_file();

    if !enabled {
        return match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            // Ярлыка и так нет — например, его убрали настройками рабочего
            // стола. Просить об этом ещё раз не о чем.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e).with_context(|| format!("не удалось убрать {}", path.display())),
        };
    }

    // Путь берётся у работающей программы, а не у ярлыка в списке программ:
    // так автозапуск одинаково работает и после `just install`, и при
    // отладочном запуске из `target`.
    let exec = std::env::current_exe().context("не удалось определить путь к программе")?;

    let dir = path.parent().expect("у ярлыка есть каталог");
    std::fs::create_dir_all(dir)
        .with_context(|| format!("не удалось создать {}", dir.display()))?;
    std::fs::write(&path, entry(&exec))
        .with_context(|| format!("не удалось записать {}", path.display()))
}

fn entry(exec: &Path) -> String {
    let id = crate::APP_ID;
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Throne GTK\n\
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
            quote(Path::new("/home/u/.local/lib/throne-gtk/throne-gtk")),
            "/home/u/.local/lib/throne-gtk/throne-gtk"
        );
    }

    #[test]
    fn path_with_space_gets_quoted() {
        assert_eq!(
            quote(Path::new("/home/Иван Петров/bin/throne-gtk")),
            "\"/home/Иван Петров/bin/throne-gtk\""
        );
    }

    #[test]
    fn quotes_inside_path_are_escaped() {
        assert_eq!(
            quote(Path::new(r#"/home/a"b/throne-gtk"#)),
            r#""/home/a\"b/throne-gtk""#
        );
    }

    /// Полный круг по настоящей файловой системе: включили — ярлык на месте
    /// и запускает то, что нужно; выключили — ярлыка нет; выключили дважды —
    /// по-прежнему нет и никто не ругается.
    #[test]
    fn round_trip_over_config_home() {
        let dir = std::env::temp_dir().join(format!("throne-gtk-autostart-{}", std::process::id()));
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
        let text = entry(Path::new("/opt/throne-gtk"));
        assert!(text.starts_with("[Desktop Entry]\n"));
        assert!(text.contains("\nExec=/opt/throne-gtk\n"));
        assert!(text.contains(&format!("\nIcon={}\n", crate::APP_ID)));
        assert!(text.ends_with('\n'));
    }
}
