//! Icon in the notification area.
//!
//! Uses the StatusNotifierItem protocol: the panel (quickshell, waybar, KDE)
//! draws the icon, and the application responds to clicks on it. The host is not
//! available in every environment, so the icon is not assumed — until it starts,
//! closing the window exits the application, or it would disappear into nowhere.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use ksni::TrayMethods;

/// What the user wants after clicking the icon.
#[derive(Debug, Clone, Copy)]
pub enum Action {
    /// Show the window.
    Show,
    /// Exit the entire application, including the core and tunnel.
    Quit,
}

/// The interface-side handle for the icon.
#[derive(Clone)]
pub struct Tray {
    updates: async_channel::Sender<Option<String>>,
    alive: Arc<AtomicBool>,
}

impl Tray {
    /// Whether the icon has started. Until then, the window cannot be hidden in it.
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }

    /// Name of the connected server, or `None` when there is no connection.
    pub fn set_connected(&self, name: Option<String>) {
        let _ = self.updates.try_send(name);
    }
}

/// The icon and its tooltip.
struct Icon {
    actions: async_channel::Sender<Action>,
    connected: Option<String>,
}

impl ksni::Tray for Icon {
    fn id(&self) -> String {
        crate::APP_ID.into()
    }

    fn icon_name(&self) -> String {
        crate::APP_ID.into()
    }

    fn title(&self) -> String {
        "Throne GTK".into()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "Throne GTK".into(),
            description: match &self.connected {
                Some(name) => format!("Подключено: {name}"),
                None => "Не подключено".into(),
            },
            ..Default::default()
        }
    }

    /// Clicking the icon is the most common way to restore the window.
    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.actions.try_send(Action::Show);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::*;
        vec![
            StandardItem {
                label: "Открыть окно".into(),
                activate: Box::new(|icon: &mut Self| {
                    let _ = icon.actions.try_send(Action::Show);
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Выйти".into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|icon: &mut Self| {
                    let _ = icon.actions.try_send(Action::Quit);
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// Starts the icon in its own thread. Returns the handle immediately without
/// waiting for the panel: it may respond slowly, while the window must already
/// be shown by then. Until the panel responds, [`Tray::is_alive`] returns `false`.
pub fn start(actions: async_channel::Sender<Action>) -> Tray {
    let (updates_tx, updates_rx) = async_channel::unbounded::<Option<String>>();
    let alive = Arc::new(AtomicBool::new(false));

    let thread = std::thread::Builder::new()
        .name("throne-tray".into())
        .spawn({
            let alive = alive.clone();
            move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(e) => {
                        tracing::warn!("значок в трее не заведён: {e}");
                        return;
                    }
                };

                runtime.block_on(async move {
                    let icon = Icon {
                        actions,
                        connected: None,
                    };
                    let handle = match icon.spawn().await {
                        Ok(handle) => handle,
                        // This is normal: the environment has no icon panel.
                        Err(e) => {
                            tracing::info!("значок в трее недоступен: {e}");
                            return;
                        }
                    };
                    alive.store(true, Ordering::Relaxed);
                    tracing::info!("значок в трее заведён");

                    while let Ok(connected) = updates_rx.recv().await {
                        handle
                            .update(|icon: &mut Icon| icon.connected = connected)
                            .await;
                    }
                });
            }
        });

    if let Err(e) = thread {
        tracing::warn!("поток значка не запустился: {e}");
    }

    Tray {
        updates: updates_tx,
        alive,
    }
}
