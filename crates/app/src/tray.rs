//! Значок в области уведомлений.
//!
//! Работает по протоколу StatusNotifierItem: значок рисует панель (quickshell,
//! waybar, KDE), а программа отвечает на щелчки по нему. Хост есть не в каждом
//! окружении, поэтому значок не считается данностью — пока он не поднялся,
//! закрытие окна выключает программу, иначе она исчезла бы в никуда.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use ksni::TrayMethods;

/// Чего хочет человек, щёлкнувший по значку.
#[derive(Debug, Clone, Copy)]
pub enum Action {
    /// Показать окно.
    Show,
    /// Выключить программу целиком, вместе с ядром и туннелем.
    Quit,
}

/// Ручка значка со стороны интерфейса.
#[derive(Clone)]
pub struct Tray {
    updates: async_channel::Sender<Option<String>>,
    alive: Arc<AtomicBool>,
}

impl Tray {
    /// Поднялся ли значок. Пока нет — прятать в него окно нельзя.
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }

    /// Имя сервера, к которому подключены, или `None`, если соединения нет.
    pub fn set_connected(&self, name: Option<String>) {
        let _ = self.updates.try_send(name);
    }
}

/// Значок и подсказка к нему.
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

    /// Щелчок по значку — самый частый способ вернуть окно.
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

/// Заводит значок в своём потоке. Возвращает ручку сразу, не дожидаясь
/// панели: та может отвечать не мгновенно, а окно к этому времени уже нужно
/// показать. Пока панель не ответила, [`Tray::is_alive`] возвращает `false`.
pub fn start(actions: async_channel::Sender<Action>) -> Tray {
    let (updates_tx, updates_rx) = async_channel::unbounded::<Option<String>>();
    let alive = Arc::new(AtomicBool::new(false));

    let thread = std::thread::Builder::new().name("throne-tray".into()).spawn({
        let alive = alive.clone();
        move || {
            let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
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
                    // Обычное дело: в окружении нет панели со значками.
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
