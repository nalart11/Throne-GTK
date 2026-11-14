//! Native macOS menu-bar icon.
//!
//! AppKit requires the status item to be created on the main thread after the
//! event loop starts. `start` therefore returns a handle immediately and defers
//! the actual construction to GLib's first idle iteration.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::glib;
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

#[derive(Debug, Clone, Copy)]
pub enum Action {
    Show,
    Quit,
}

#[derive(Clone)]
pub struct Tray {
    inner: Rc<Inner>,
}

struct Inner {
    icon: RefCell<Option<TrayIcon>>,
    alive: Cell<bool>,
}

impl Tray {
    pub fn is_alive(&self) -> bool {
        self.inner.alive.get()
    }

    pub fn set_connected(&self, name: Option<String>) {
        let tooltip = match name {
            Some(name) => crate::tr_format!(
                "Nolurix — подключено: {name}",
                "Nolurix — connected: {name}"
            ),
            None => crate::i18n::tr("Nolurix — не подключено", "Nolurix — disconnected").into(),
        };
        if let Some(icon) = self.inner.icon.borrow().as_ref() {
            if let Err(e) = icon.set_tooltip(Some(tooltip)) {
                tracing::warn!("не удалось обновить подсказку значка: {e}");
            }
        }
    }
}

pub fn start(actions: async_channel::Sender<Action>) -> Tray {
    let inner = Rc::new(Inner {
        icon: RefCell::new(None),
        alive: Cell::new(false),
    });

    glib::idle_add_local_once({
        let inner = inner.clone();
        move || match build_icon(actions) {
            Ok(icon) => {
                inner.icon.replace(Some(icon));
                inner.alive.set(true);
                tracing::info!("значок macOS menu bar заведён");
            }
            Err(e) => tracing::warn!("значок macOS menu bar не заведён: {e}"),
        }
    });

    Tray { inner }
}

fn build_icon(actions: async_channel::Sender<Action>) -> Result<TrayIcon, String> {
    let menu = Menu::new();
    let show = MenuItem::new(crate::i18n::tr("Открыть окно", "Open window"), true, None);
    let separator = PredefinedMenuItem::separator();
    let quit = MenuItem::new(crate::i18n::tr("Выйти", "Quit"), true, None);
    menu.append_items(&[&show, &separator, &quit])
        .map_err(|e| format!("не удалось собрать меню: {e}"))?;

    let show_id = show.id().clone();
    let quit_id = quit.id().clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        let action = if event.id == show_id {
            Some(Action::Show)
        } else if event.id == quit_id {
            Some(Action::Quit)
        } else {
            None
        };
        if let Some(action) = action {
            let _ = actions.try_send(action);
        }
    }));

    TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip(crate::i18n::tr(
            "Nolurix — не подключено",
            "Nolurix — disconnected",
        ))
        .with_icon(template_icon()?)
        .with_icon_as_template(true)
        .build()
        .map_err(|e| e.to_string())
}

/// The app's channel-line logo, rendered at 2x from the symbolic app icon.
/// AppKit treats its alpha channel as a template, choosing the right colour
/// for light and dark menu bars.
fn template_icon() -> Result<Icon, String> {
    const SIZE: u32 = 36;
    // Reproduce the asset from the canonical symbolic logo by running
    // png="${TMPDIR:-/tmp}/nolurix-tray.png"
    // rsvg-convert -w 36 -h 36 resources/icons/dev.nalart.Nolurix-symbolic.svg -o "$png"
    // swift crates/app/src/generate_tray_icon.swift "$png" crates/app/src/nolurix_tray_icon.rgba
    const RGBA: &[u8; SIZE as usize * SIZE as usize * 4] = include_bytes!("nolurix_tray_icon.rgba");
    Icon::from_rgba(RGBA.to_vec(), SIZE, SIZE).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn template_asset_is_36px_rgba_with_template_alpha() {
        const RGBA: &[u8; 36 * 36 * 4] = include_bytes!("nolurix_tray_icon.rgba");
        let alpha = RGBA.chunks_exact(4).map(|pixel| pixel[3]);
        let mut any_visible = false;
        let mut any_transparent = false;
        for value in alpha {
            any_visible |= value > 0;
            any_transparent |= value < 255;
        }
        assert!(any_visible, "logo alpha must not be empty");
        assert!(
            any_transparent,
            "template logo must retain transparent pixels"
        );
    }
}
