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
            Some(name) => format!("Throne GTK — подключено: {name}"),
            None => "Throne GTK — не подключено".into(),
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
    let show = MenuItem::new("Открыть окно", true, None);
    let separator = PredefinedMenuItem::separator();
    let quit = MenuItem::new("Выйти", true, None);
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
        .with_tooltip("Throne GTK — не подключено")
        .with_icon(template_icon()?)
        .with_icon_as_template(true)
        .build()
        .map_err(|e| e.to_string())
}

/// A 18pt crown rendered at 2x. AppKit treats its alpha channel as a template,
/// automatically choosing the right colour for light and dark menu bars.
fn template_icon() -> Result<Icon, String> {
    const SIZE: u32 = 36;
    let mut rgba = vec![0_u8; (SIZE * SIZE * 4) as usize];
    let crown = [
        (4, 9),
        (11, 17),
        (18, 5),
        (25, 17),
        (32, 9),
        (28, 27),
        (8, 27),
    ];

    for y in 0..SIZE as i32 {
        for x in 0..SIZE as i32 {
            if inside_polygon(x, y, &crown) || (7..=29).contains(&x) && (28..=31).contains(&y) {
                let offset = ((y as u32 * SIZE + x as u32) * 4) as usize;
                rgba[offset + 3] = 255;
            }
        }
    }

    Icon::from_rgba(rgba, SIZE, SIZE).map_err(|e| e.to_string())
}

fn inside_polygon(x: i32, y: i32, points: &[(i32, i32)]) -> bool {
    let mut inside = false;
    let mut previous = points.len() - 1;
    for current in 0..points.len() {
        let (xi, yi) = points[current];
        let (xj, yj) = points[previous];
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        previous = current;
    }
    inside
}
